//! Real HTTP/2 frames reproduce the daemon's invalid host-shaped UDS authority.
//! A tonic client with a valid HTTP endpoint cannot exercise this failure.
use anyhow::{Context, Result, bail};
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
};
use std::{path::Path, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

fn integer(out: &mut Vec<u8>, mut value: usize, prefix: u8) {
    let limit = (1usize << prefix) - 1;
    if value < limit {
        out.push(value as u8);
        return;
    }
    out.push(limit as u8);
    value -= limit;
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn string(out: &mut Vec<u8>, value: &str) {
    integer(out, value.len(), 7);
    out.extend_from_slice(value.as_bytes());
}
fn literal(out: &mut Vec<u8>, name_index: usize, value: &str) {
    integer(out, name_index, 4);
    string(out, value);
}
fn frame(kind: u8, flags: u8, stream: u32, body: &[u8]) -> Vec<u8> {
    let len = body.len() as u32;
    let mut out = vec![(len >> 16) as u8, (len >> 8) as u8, len as u8, kind, flags];
    out.extend_from_slice(&stream.to_be_bytes());
    out.extend_from_slice(body);
    out
}
async fn ping(socket: &Path, authority: &str, path: &str) -> Result<Vec<u8>> {
    let mut socket = UnixStream::connect(socket).await?;
    let mut headers = vec![0x83, 0x86]; // indexed :method POST, :scheme http
    literal(&mut headers, 4, path);
    literal(&mut headers, 1, authority);
    literal(&mut headers, 31, "application/grpc");
    headers.push(0);
    string(&mut headers, "te");
    string(&mut headers, "trailers");
    let mut request = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec();
    request.extend(frame(4, 0, 0, &[]));
    request.extend(frame(1, 4, 1, &headers));
    request.extend(frame(0, 1, 1, &[0, 0, 0, 0, 2, 8, 42])); // gRPC PingRequest {id:42}
    socket.write_all(&request).await?;
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut data = vec![];
        loop {
            let mut header = [0; 9];
            socket.read_exact(&mut header).await?;
            let len =
                ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
            anyhow::ensure!(len <= 16384, "unexpected frame length");
            let stream = u32::from_be_bytes(header[5..9].try_into().unwrap()) & 0x7fff_ffff;
            let mut body = vec![0; len];
            socket.read_exact(&mut body).await?;
            if header[3] == 4 && header[4] & 1 == 0 {
                socket.write_all(&frame(4, 1, 0, &[])).await?;
            }
            if stream == 1 {
                if header[3] == 3 {
                    bail!(
                        "RST_STREAM {}",
                        u32::from_be_bytes(body[..4].try_into().unwrap())
                    );
                }
                if header[3] == 0 {
                    data.extend(body);
                }
                if matches!(header[3], 0 | 1) && header[4] & 1 != 0 {
                    return Ok(data);
                }
            }
        }
    })
    .await
    .context("response deadline")?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepts_daemon_socket_authorities_without_relaxing_other_http_paths() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = Config {
        transport: Default::default(),
        socket: dir.path().join("ui.sock"),
        database: dir.path().join("history.sqlite3"),
        prompt_timeout: 1,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    };
    let backend = Backend::start(config.clone())?;
    for authority in [
        "localhost",
        "/tmp/osui.sock",
        "tmp%2Fosui.sock",
        "%2ftmp%2fosui.sock",
    ] {
        assert_eq!(
            ping(&config.socket, authority, "/protocol.UI/Ping").await?,
            [0, 0, 0, 0, 2, 8, 42],
            "{authority}"
        );
    }
    for (authority, path) in [
        ("/tmp/osui.sock", "/unrelated.Service/Ping"),
        ("bad%host", "/protocol.UI/Ping"),
    ] {
        let error = ping(&config.socket, authority, path).await.unwrap_err();
        assert!(error.to_string().contains("RST_STREAM 1"), "{error:#}");
    }
    backend.join();
    Ok(())
}
