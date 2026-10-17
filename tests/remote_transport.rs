//! Network cases must run through scripts/owned_net.py, never the host network.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    protocol::{self, ui_client::UiClient},
    transport::{self, Auth, Endpoint, Peer, Settings},
};
use std::{
    collections::BTreeSet,
    net::IpAddr,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint as GrpcEndpoint, Identity};

fn owned() -> Result<()> {
    let mapping = std::fs::read_to_string("/proc/self/uid_map")?;
    let fields = mapping.split_whitespace().collect::<Vec<_>>();
    ensure!(
        fields.len() == 3 && fields[0] == "0" && fields[1] != "0" && fields[2] == "1",
        "Run network tests through scripts/owned_net.py"
    );
    let links = Command::new("ip").args(["-j", "link", "show"]).output()?;
    ensure!(
        links.status.success(),
        "Cannot inspect owned namespace links"
    );
    let interfaces: Vec<serde_json::Value> = serde_json::from_slice(&links.stdout)?;
    ensure!(
        interfaces.len() == 1 && interfaces[0]["ifname"] == "lo",
        "Transport fixture requires only owned loopback"
    );
    Ok(())
}
fn config(path: &Path) -> Config {
    Config {
        socket: "127.0.0.1:0".into(),
        transport: Default::default(),
        database: path.join("history.sqlite3"),
        prompt_timeout: 3,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    }
}
fn openssl(root: &Path, args: &[&str]) -> Result<()> {
    let result = Command::new("openssl")
        .current_dir(root)
        .args(args)
        .output()?;
    ensure!(
        result.status.success(),
        "Owned OpenSSL fixture generation failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}
fn certificates(root: &Path) -> Result<()> {
    for name in ["ca", "other"] {
        openssl(
            root,
            &[
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                &format!("/CN=Owned Rustleblower {name}"),
                "-addext",
                "basicConstraints=critical,CA:TRUE",
                "-addext",
                "keyUsage=critical,keyCertSign,cRLSign",
                "-keyout",
                &format!("{name}.key"),
                "-out",
                &format!("{name}.pem"),
            ],
        )?;
    }
    for (name, ca, usage) in [
        ("server", "ca", "serverAuth"),
        ("client", "ca", "clientAuth"),
        ("untrusted", "other", "clientAuth"),
    ] {
        openssl(
            root,
            &[
                "req",
                "-new",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-subj",
                &format!("/CN=Owned Rustleblower {name}"),
                "-keyout",
                &format!("{name}.key"),
                "-out",
                &format!("{name}.csr"),
            ],
        )?;
        std::fs::write(
            root.join(format!("{name}.ext")),
            format!(
                "basicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage={usage}\nsubjectAltName=DNS:localhost,IP:127.0.0.1,IP:::1\n"
            ),
        )?;
        openssl(
            root,
            &[
                "x509",
                "-req",
                "-in",
                &format!("{name}.csr"),
                "-CA",
                &format!("{ca}.pem"),
                "-CAkey",
                &format!("{ca}.key"),
                "-CAcreateserial",
                "-days",
                "1",
                "-extfile",
                &format!("{name}.ext"),
                "-out",
                &format!("{name}.pem"),
            ],
        )?;
    }
    Ok(())
}
fn settings(path: &Path, auth: Auth) -> Settings {
    Settings {
        socket_auth: auth,
        tls_cert: Some(path.join("server.pem")),
        tls_key: Some(path.join("server.key")),
        tls_ca_cert: (auth == Auth::TlsMutual).then(|| path.join("ca.pem")),
    }
}
async fn client(
    address: &str,
    path: &Path,
    ca: Option<&str>,
    identity: Option<&str>,
    domain: &str,
) -> Result<UiClient<Channel>> {
    let mut endpoint = GrpcEndpoint::from_shared(format!(
        "{}://{address}",
        if ca.is_some() { "https" } else { "http" }
    ))?
    .connect_timeout(Duration::from_secs(2))
    .timeout(Duration::from_secs(3));
    if let Some(ca) = ca {
        let mut tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(std::fs::read(
                path.join(format!("{ca}.pem")),
            )?))
            .domain_name(domain);
        if let Some(identity) = identity {
            tls = tls.identity(Identity::from_pem(
                std::fs::read(path.join(format!("{identity}.pem")))?,
                std::fs::read(path.join(format!("{identity}.key")))?,
            ));
        }
        endpoint = endpoint.tls_config(tls)?;
    }
    Ok(UiClient::new(endpoint.connect().await?))
}
async fn ping(
    address: &str,
    path: &Path,
    ca: Option<&str>,
    identity: Option<&str>,
    domain: &str,
) -> Result<()> {
    let mut client = client(address, path, ca, identity, domain).await?;
    ensure!(
        client
            .ping(protocol::PingRequest {
                id: 77,
                ..Default::default()
            })
            .await?
            .into_inner()
            .id
            == 77,
        "Wrong ping reply"
    );
    Ok(())
}
async fn wait_until(mut test: impl FnMut() -> bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !test() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .context("Fixture state deadline")?;
    Ok(())
}

#[test]
fn endpoint_auth_and_trusted_peer_identity_do_not_depend_on_reported_name() -> Result<()> {
    assert_eq!(
        Endpoint::parse(Path::new("unix:///tmp/owned.sock"))?,
        Endpoint::Unix("/tmp/owned.sock".into())
    );
    for address in ["127.0.0.1:50051", "[::1]:50051", "tcp://[::]:0"] {
        assert!(matches!(
            Endpoint::parse(Path::new(address))?,
            Endpoint::Tcp(_)
        ));
    }
    for invalid in [
        "relative.sock",
        "https://127.0.0.1:443",
        "unix://relative.sock",
        "127.0.0.1:99999",
    ] {
        assert!(Endpoint::parse(Path::new(invalid)).is_err());
    }
    let cli = Config::try_parse_from([
        "rustleblower",
        "--socket",
        "[::1]:50051",
        "--socket-auth",
        "tls-mutual",
        "--tls-ca-cert",
        "ca.pem",
        "--tls-cert",
        "server.pem",
        "--tls-key",
        "server.key",
    ])?;
    assert_eq!(cli.transport.socket_auth, Auth::TlsMutual);
    let local = BTreeSet::from(["192.0.2.1".parse::<IpAddr>()?]);
    let remote = Peer::tcp("192.0.2.2".parse()?, &local);
    assert!(!remote.local);
    assert_eq!(remote.node_key("local")?, "192.0.2.2/local");
    assert_ne!(remote.node_key("local")?, Peer::unix().node_key("local")?);
    assert!(Peer::tcp("192.0.2.1".parse()?, &local).local);
    assert!(Peer::tcp("::ffff:127.0.0.1".parse()?, &local).local);
    assert_eq!(
        Peer::tcp("::ffff:192.0.2.2".parse()?, &local).node_key("local")?,
        remote.node_key("local")?
    );
    assert!(remote.node_key(&"x".repeat(128)).is_err());
    assert!(remote.node_key("line\nbreak").is_err());
    let linked = BTreeSet::from(["fe80::1".parse::<IpAddr>()?]);
    let first = Peer::tcp_socket("[fe80::1%2]:50051".parse()?, &linked);
    let second = Peer::tcp_socket("[fe80::1%3]:50051".parse()?, &linked);
    assert!(!first.local && !second.local);
    assert_ne!(first.node_key("local")?, second.node_key("local")?);
    assert_eq!(first.node_key("local")?, "fe80::1%2/local");
    Ok(())
}

#[test]
fn invalid_or_unbounded_credentials_fail_closed_and_startup_preserves_paths() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path();
    let endpoint = Endpoint::Tcp("127.0.0.1:0".parse()?);
    assert!(
        Settings {
            tls_cert: Some(path.join("unused.pem")),
            ..Default::default()
        }
        .tls(&endpoint)
        .is_err()
    );
    assert!(
        Settings {
            socket_auth: Auth::TlsSimple,
            ..Default::default()
        }
        .tls(&endpoint)
        .is_err()
    );
    assert!(
        settings(path, Auth::TlsSimple)
            .tls(&Endpoint::Unix(path.join("ui.sock")))
            .is_err()
    );
    std::fs::write(path.join("server.pem"), vec![b'x'; 65537])?;
    std::fs::write(path.join("server.key"), "not-a-key")?;
    assert!(settings(path, Auth::TlsSimple).tls(&endpoint).is_err());
    std::fs::write(path.join("server.pem"), "not-a-certificate")?;
    let mut cfg = config(path);
    cfg.transport = settings(path, Auth::TlsSimple);
    assert!(Backend::start(cfg.clone()).is_err());
    cfg.socket = "[::1]:50051".into();
    cfg.transport = settings(path, Auth::TlsMutual);
    let launch = rustleblower::startup::Launch::new(PathBuf::from("/bin/true"), &cfg)?;
    assert_eq!(launch.arguments.len(), 16);
    assert!(
        launch
            .arguments
            .windows(2)
            .any(|v| v == ["--socket", "[::1]:50051"])
    );
    assert!(
        launch
            .arguments
            .windows(2)
            .any(|v| v == ["--socket-auth", "tls-mutual"])
    );
    assert!(
        launch
            .arguments
            .windows(2)
            .any(|v| v[0] == "--tls-key" && v[1] == path.join("server.key").to_string_lossy())
    );
    assert!(!launch.command()?.contains("not-a-key"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plaintext_tcp_preserves_ping_prompt_subscription_and_reconnect_routing() -> Result<()> {
    owned()?;
    let dir = tempfile::tempdir()?;
    let backend = Backend::start(config(dir.path()))?;
    let address = backend.handle.listener.clone();
    let mut node = client(&address, dir.path(), None, None, "").await?;
    node.subscribe(protocol::ClientConfig {
        name: "local".into(),
        ..Default::default()
    })
    .await?;
    let first = backend
        .handle
        .status()
        .session
        .context("Missing TCP session")?;
    assert_eq!(first.name, "127.0.0.1/local");
    assert!(first.local);
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let _stream = node
        .notifications(tokio_stream::wrappers::ReceiverStream::new(receiver))
        .await?
        .into_inner();
    wait_until(|| {
        backend
            .handle
            .session(first.id)
            .is_some_and(|s| s.notifications)
    })
    .await?;
    let mut asking = node.clone();
    let request = tokio::spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/tcp-prompt".into(),
                ..Default::default()
            })
            .await
    });
    wait_until(|| !backend.handle.prompts.snapshot().is_empty()).await?;
    assert!(
        backend
            .handle
            .decide(backend.handle.prompts.snapshot()[0].id, Decision::Allow)
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), request)
            .await???
            .into_inner()
            .action,
        "allow"
    );
    ping(&address, dir.path(), None, None, "").await?;
    let mut replacement = client(&address, dir.path(), None, None, "").await?;
    replacement
        .subscribe(protocol::ClientConfig {
            name: "local".into(),
            ..Default::default()
        })
        .await?;
    let second = backend
        .handle
        .status()
        .session
        .context("Missing replacement")?;
    assert_ne!(second.id, first.id);
    assert_eq!(second.name, first.name);
    assert!(backend.handle.session(first.id).is_none());
    drop(sender);
    drop(replacement);
    drop(node);
    drop(_stream);
    backend.join();
    assert!(!dir.path().join("ui.sock").exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tls_and_mutual_tls_accept_only_the_configured_certificate_contract() -> Result<()> {
    owned()?;
    let dir = tempfile::tempdir()?;
    certificates(dir.path())?;
    for auth in [Auth::TlsSimple, Auth::TlsMutual] {
        let storage = dir.path().join(auth.key());
        std::fs::create_dir(&storage)?;
        let mut cfg = config(&storage);
        cfg.transport = settings(dir.path(), auth);
        let backend = Backend::start(cfg)?;
        let address = backend.handle.listener.clone();
        let identity = (auth == Auth::TlsMutual).then_some("client");
        ping(&address, dir.path(), Some("ca"), identity, "localhost").await?;
        assert!(
            ping(&address, dir.path(), Some("other"), identity, "localhost")
                .await
                .is_err()
        );
        assert!(
            ping(
                &address,
                dir.path(),
                Some("ca"),
                identity,
                "wrong-owned-host"
            )
            .await
            .is_err()
        );
        assert!(ping(&address, dir.path(), None, None, "").await.is_err());
        if auth == Auth::TlsMutual {
            assert!(
                ping(&address, dir.path(), Some("ca"), None, "localhost")
                    .await
                    .is_err()
            );
            assert!(
                ping(
                    &address,
                    dir.path(),
                    Some("ca"),
                    Some("untrusted"),
                    "localhost"
                )
                .await
                .is_err()
            );
        }
        // Failed handshakes must not tear down the listener or retain permits.
        ping(&address, dir.path(), Some("ca"), identity, "localhost").await?;
        let mut node = client(&address, dir.path(), Some("ca"), identity, "localhost").await?;
        node.subscribe(protocol::ClientConfig {
            name: "tls-owned".into(),
            ..Default::default()
        })
        .await?;
        let session = backend
            .handle
            .status()
            .session
            .context("TLS did not preserve connect metadata")?;
        assert!(session.local);
        assert_eq!(session.name, "127.0.0.1/tls-owned");
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let mut stream = node
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await?
            .into_inner();
        wait_until(|| {
            backend
                .handle
                .session(session.id)
                .is_some_and(|s| s.notifications)
        })
        .await?;
        backend.handle.history.flush().await?;
        let mut accepted = backend
            .handle
            .control
            .submit_node(session.id, rustleblower::nodes::Change::Interception(true))
            .map_err(anyhow::Error::msg)?;
        let command = tokio::time::timeout(Duration::from_secs(1), stream.message())
            .await??
            .context("No held command")?;
        assert_eq!(command.r#type, protocol::Action::EnableInterception as i32);
        assert!(matches!(
            accepted.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        let mut asking = node.clone();
        let pending = tokio::spawn(async move {
            asking
                .ask_rule(protocol::Connection {
                    process_path: "/owned/tls-held-ack-prompt".into(),
                    ..Default::default()
                })
                .await
        });
        wait_until(|| !backend.handle.prompts.snapshot().is_empty()).await?;
        assert!(
            backend
                .handle
                .decide(backend.handle.prompts.snapshot()[0].id, Decision::Reject)
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(500), pending)
                .await???
                .into_inner()
                .action,
            "reject"
        );
        let reply = tokio::time::timeout(
            Duration::from_millis(500),
            node.ping(protocol::PingRequest {
                id: 88,
                ..Default::default()
            }),
        )
        .await??
        .into_inner();
        assert_eq!(reply.id, 88);
        assert!(matches!(
            accepted.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        tx.send(protocol::NotificationReply {
            id: command.id,
            code: protocol::NotificationReplyCode::Ok as i32,
            data: String::new(),
        })
        .await?;
        tokio::time::timeout(Duration::from_secs(1), accepted)
            .await??
            .map_err(anyhow::Error::msg)?;
        drop(stream);
        drop(tx);
        drop(node);
        backend.join();
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_tls_connections_have_finite_admission_and_initial_deadlines() -> Result<()> {
    owned()?;
    let dir = tempfile::tempdir()?;
    certificates(dir.path())?;
    let mut cfg = config(dir.path());
    cfg.transport = settings(dir.path(), Auth::TlsSimple);
    let backend = Backend::start(cfg)?;
    let address = backend.handle.listener.clone();
    let mut sockets = Vec::new();
    for _ in 0..transport::MAX_INITIAL_CONNECTIONS + 4 {
        sockets.push(tokio::net::TcpStream::connect(&address).await?);
    }
    wait_until(|| backend.handle.initial_connection_count() == transport::MAX_INITIAL_CONNECTIONS)
        .await?;
    assert!(backend.handle.connection_count() <= transport::MAX_INITIAL_CONNECTIONS);
    tokio::time::timeout(
        transport::INITIAL_DEADLINE + Duration::from_secs(3),
        async {
            while backend.handle.connection_count() != 0 {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        },
    )
    .await
    .context("Unfinished handshakes retained connections past their deadline")?;
    ping(&address, dir.path(), Some("ca"), None, "localhost").await?;
    drop(sockets);
    backend.join();
    Ok(())
}

#[tokio::test]
async fn admission_pools_refuse_before_decode_and_do_not_queue_behind_other_methods() -> Result<()>
{
    use tower::Service;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let recorded = calls.clone();
    let inner = tower::service_fn(move |_: http::Request<tonic::body::Body>| {
        recorded.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        async {
            std::future::pending::<()>().await;
            Ok::<_, std::convert::Infallible>(http::Response::new(tonic::body::Body::empty()))
        }
    });
    let mut admission = transport::Admission::new(inner);
    let request = |method: &str| {
        http::Request::builder()
            .uri(format!("/protocol.UI/{method}"))
            .body(tonic::body::Body::empty())
            .unwrap()
    };
    let mut pending = Vec::new();
    for (method, cap) in ["AskRule", "Ping", "Subscribe", "PostAlert", "Notifications"]
        .into_iter()
        .zip(transport::RPC_CAPACITIES)
    {
        let mut held = Vec::new();
        for _ in 0..cap {
            held.push(admission.call(request(method)));
        }
        let response =
            tokio::time::timeout(Duration::from_millis(50), admission.call(request(method)))
                .await??;
        assert_eq!(response.headers().get("grpc-status").unwrap(), "8");
        pending.push(held);
    }
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 54);
    let unknown = admission.call(request("not-a-method")).await?;
    assert_eq!(unknown.headers().get("grpc-status").unwrap(), "12");
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 54);
    // Cancelled/unpolled futures release admission; a clone shares the same cap.
    pending[0].clear();
    let mut clone = admission.clone();
    let replacement = clone.call(request("AskRule"));
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 55);
    drop(replacement);
    drop(pending);
    Ok(())
}
