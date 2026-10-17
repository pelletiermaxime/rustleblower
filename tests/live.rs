use anyhow::Result;
use hyper_util::rt::TokioIo;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    live::{Kind, MAX_UPDATE},
    protocol::{self, ui_client::UiClient},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tonic::transport::{Channel, Endpoint};
async fn client(path: &Path) -> Result<UiClient<Channel>> {
    let path = path.to_owned();
    Ok(UiClient::new(
        Endpoint::try_from("http://localhost")?
            .connect_with_connector(tower::service_fn(move |_| {
                let path = path.clone();
                async move {
                    tokio::net::UnixStream::connect(path)
                        .await
                        .map(TokioIo::new)
                }
            }))
            .await?,
    ))
}
fn config(path: &Path) -> Config {
    Config {
        transport: Default::default(),
        socket: path.join("ui.sock"),
        database: path.join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    }
}
async fn wait_for(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await
        }
    })
    .await
    .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_updates_stop_generation_errors_bounds_and_permission_isolation() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = config(dir.path());
    let backend = Backend::start(cfg.clone())?;
    let mut alpha = client(&cfg.socket).await?;
    alpha
        .subscribe(protocol::ClientConfig {
            name: "alpha".into(),
            ..Default::default()
        })
        .await?;
    let a = backend.handle.node("alpha").unwrap().id;
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let mut stream = alpha
        .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await?
        .into_inner();
    let mut beta = client(&cfg.socket).await?;
    beta.subscribe(protocol::ClientConfig {
        name: "beta".into(),
        ..Default::default()
    })
    .await?;
    let (btx, brx) = tokio::sync::mpsc::channel(16);
    let bstream = beta
        .notifications(tokio_stream::wrappers::ReceiverStream::new(brx))
        .await?
        .into_inner();
    let kind = Kind::Sockets {
        seconds: 5,
        state: 1,
        proto: 6,
        family: 10,
    };
    let lease = backend
        .handle
        .live
        .start(a, kind.clone())
        .map_err(anyhow::Error::msg)?;
    let command = stream.message().await?.unwrap();
    assert_eq!(command.r#type, 13);
    // daemon/ui/notifications_tasks.go::monitorTaskEvents routes IDs <=10000
    // to generic alerts instead of the requesting Notifications stream.
    assert!(
        command.id > 10_000,
        "The daemon would route task {} to alerts, leaving Live without updates",
        command.id
    );
    let doc: serde_json::Value = serde_json::from_str(&command.data)?;
    assert_eq!(
        doc,
        serde_json::json!({"name":"sockets-monitor","data":{"interval":"5s","state":1,"proto":6,"family":10}})
    );
    assert!(backend.handle.live.start(a, kind.clone()).is_err());
    btx.send(protocol::NotificationReply {
        id: command.id,
        data: "crossed".into(),
        ..Default::default()
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(lease.snapshot().revision, 0);
    for i in 1..=25 {
        tx.send(protocol::NotificationReply {
            id: command.id,
            data: format!("{{\"Table\":[],\"Sequence\":{i}}}"),
            ..Default::default()
        })
        .await?;
    }
    wait_for(|| lease.snapshot().revision == 25).await;
    assert!(lease.snapshot().raw.unwrap().contains("25"));
    assert!(lease.snapshot().running);
    assert!(!backend.handle.history.health.catalogues.uncertain(a));
    let started = Instant::now();
    tokio::time::timeout(
        Duration::from_millis(500),
        alpha.ping(protocol::PingRequest {
            id: 1,
            ..Default::default()
        }),
    )
    .await??;
    let mut asking = beta.clone();
    let request = tokio::spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/bin/live-fixture".into(),
                ..Default::default()
            })
            .await
    });
    wait_for(|| !backend.handle.prompts.snapshot().is_empty()).await;
    assert!(
        backend
            .handle
            .decide(backend.handle.prompts.snapshot()[0].id, Decision::Allow)
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(500), request)
            .await???
            .into_inner()
            .action,
        "allow"
    );
    assert!(started.elapsed() < Duration::from_millis(500));
    lease.stop();
    let stop = stream.message().await?.unwrap();
    assert_eq!(stop.r#type, 14);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&stop.data)?, doc);
    assert!(!lease.snapshot().running);
    assert!(lease.snapshot().status.contains("does not acknowledge"));
    tx.send(protocol::NotificationReply {
        id: command.id,
        data: "late".into(),
        ..Default::default()
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(lease.snapshot().revision, 25);
    let process = backend
        .handle
        .live
        .start(
            a,
            Kind::Process {
                seconds: 2,
                pid: 42,
            },
        )
        .map_err(anyhow::Error::msg)?;
    let p = stream.message().await?.unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&p.data)?["data"]["pid"],
        "42"
    );
    tx.send(protocol::NotificationReply {
        id: p.id,
        data: "process exited".into(),
        code: 1,
    })
    .await?;
    wait_for(|| !process.snapshot().running).await;
    assert!(process.snapshot().error.unwrap().contains("exited"));
    assert_eq!(stream.message().await?.unwrap().r#type, 14);
    assert!(!backend.handle.history.health.catalogues.uncertain(a));
    let large = backend
        .handle
        .live
        .start(a, kind.clone())
        .map_err(anyhow::Error::msg)?;
    let n = stream.message().await?.unwrap();
    tx.send(protocol::NotificationReply {
        id: n.id,
        data: "x".repeat(MAX_UPDATE + 1),
        ..Default::default()
    })
    .await?;
    wait_for(|| large.snapshot().error.is_some()).await;
    assert_eq!(stream.message().await?.unwrap().r#type, 14);
    let old = backend
        .handle
        .live
        .start(a, kind)
        .map_err(anyhow::Error::msg)?;
    let old_n = stream.message().await?.unwrap();
    let mut replacement = client(&cfg.socket).await?;
    replacement
        .subscribe(protocol::ClientConfig {
            name: "alpha".into(),
            ..Default::default()
        })
        .await?;
    wait_for(|| !old.snapshot().running).await;
    assert!(old.snapshot().status.contains("disconnected"));
    tx.send(protocol::NotificationReply {
        id: old_n.id,
        data: "stale generation".into(),
        ..Default::default()
    })
    .await
    .ok();
    assert!(old.snapshot().raw.is_none());
    drop(lease);
    drop(process);
    drop(large);
    drop(old);
    drop(alpha);
    drop(beta);
    drop(replacement);
    drop(stream);
    drop(bstream);
    drop(tx);
    drop(btx);
    backend.join();
    assert!(!cfg.socket.exists());
    Ok(())
}
