use anyhow::Result;
use hyper_util::rt::TokioIo;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    nodes::{self, Change},
    protocol::{self, ui_client::UiClient},
};
use std::{path::Path, time::Duration};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};
fn options(dir: &Path) -> Config {
    Config {
        transport: Default::default(),
        socket: dir.join("ui.sock"),
        database: dir.join("history.sqlite3"),
        prompt_timeout: 5,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    }
}
async fn client(path: &Path) -> Result<UiClient<Channel>> {
    let path = path.to_path_buf();
    let channel = Endpoint::try_from("http://localhost")?
        .connect_with_connector(tower::service_fn(move |_| {
            let path = path.clone();
            async move {
                tokio::net::UnixStream::connect(path)
                    .await
                    .map(TokioIo::new)
            }
        }))
        .await?;
    Ok(UiClient::new(channel))
}
async fn subscribe(
    backend: &Backend,
    client: &mut UiClient<Channel>,
    name: &str,
    running: bool,
) -> Result<u64> {
    let config = protocol::ClientConfig {
        name: name.into(),
        is_firewall_running: running,
        config: "{\"Future\":true}".into(),
        ..Default::default()
    };
    assert_eq!(client.subscribe(config.clone()).await?.into_inner(), config);
    backend.handle.history.flush().await?;
    Ok(backend.handle.node(name).unwrap().id)
}
async fn stream(
    client: &mut UiClient<Channel>,
) -> Result<(
    tokio::sync::mpsc::Sender<protocol::NotificationReply>,
    tonic::Streaming<protocol::Notification>,
)> {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let stream = client
        .notifications(ReceiverStream::new(rx))
        .await?
        .into_inner();
    Ok((tx, stream))
}
async fn remember(
    backend: &Backend,
    session: u64,
    name: &str,
    enabled: bool,
    running: bool,
) -> Result<()> {
    let before = backend.handle.history.health.interception.get(name);
    backend
        .handle
        .history
        .remember_interception(session, name.into(), enabled, running, before)
        .map_err(anyhow::Error::msg)?
        .await?
        .map_err(anyhow::Error::msg)
}
async fn ack(
    tx: &tokio::sync::mpsc::Sender<protocol::NotificationReply>,
    id: u64,
    ok: bool,
) -> Result<()> {
    tx.send(protocol::NotificationReply {
        id,
        code: if ok {
            protocol::NotificationReplyCode::Ok
        } else {
            protocol::NotificationReplyCode::Error
        } as i32,
        data: if ok {
            ""
        } else {
            "owned interception rejection"
        }
        .into(),
    })
    .await?;
    Ok(())
}
async fn command(
    stream: &mut tonic::Streaming<protocol::Notification>,
) -> Result<protocol::Notification> {
    Ok(
        tokio::time::timeout(Duration::from_secs(3), stream.message())
            .await??
            .unwrap(),
    )
}
async fn none(stream: &mut tonic::Streaming<protocol::Notification>) -> Result<()> {
    assert!(
        tokio::time::timeout(Duration::from_millis(1150), stream.message())
            .await
            .is_err()
    );
    Ok(())
}
async fn decision(backend: &Backend, client: &mut UiClient<Channel>) -> Result<()> {
    let ping = tokio::time::timeout(
        Duration::from_millis(500),
        client.ping(protocol::PingRequest {
            id: 77,
            ..Default::default()
        }),
    )
    .await??
    .into_inner();
    assert_eq!(ping.id, 77);
    let mut asking = client.clone();
    let request = tokio::spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/independent-interception-prompt".into(),
                ..Default::default()
            })
            .await
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while backend.handle.prompts.snapshot().is_empty() {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        backend
            .handle
            .prompts
            .decide(backend.handle.prompts.snapshot()[0].id, Decision::Allow)
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(500), request)
            .await???
            .into_inner()
            .action,
        "allow"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opt_in_remembers_only_confirmed_choices_and_restores_pause_after_restart() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut daemon = client(&config.socket).await?;
    let session = subscribe(&backend, &mut daemon, "alpha", true).await?;
    let (tx, mut commands) = stream(&mut daemon).await?;
    assert_eq!(
        backend.handle.history.health.interception.get("alpha"),
        None
    );
    none(&mut commands).await?;
    remember(&backend, session, "alpha", true, true).await?;
    assert_eq!(
        backend.handle.history.health.interception.get("alpha"),
        Some(true)
    );
    let reply = backend
        .handle
        .control
        .submit_node(session, Change::Interception(false))
        .map_err(anyhow::Error::msg)?;
    let cmd = command(&mut commands).await?;
    assert_eq!(cmd.r#type, protocol::Action::DisableInterception as i32);
    assert_eq!(
        backend.handle.history.health.interception.get("alpha"),
        Some(true)
    );
    assert!(nodes::detail(&config.database, "alpha")?.row.running);
    decision(&backend, &mut daemon).await?;
    ack(&tx, cmd.id, false).await?;
    assert!(reply.await?.is_err());
    assert_eq!(
        backend.handle.history.health.interception.get("alpha"),
        Some(true)
    );
    drop(commands);
    drop(tx);
    let session = subscribe(&backend, &mut daemon, "alpha", true).await?;
    let (tx, mut commands) = stream(&mut daemon).await?;
    let reply = backend
        .handle
        .control
        .submit_node(session, Change::Interception(false))
        .map_err(anyhow::Error::msg)?;
    let cmd = command(&mut commands).await?;
    ack(&tx, cmd.id, true).await?;
    reply.await?.map_err(anyhow::Error::msg)?;
    assert_eq!(
        backend.handle.history.health.interception.get("alpha"),
        Some(false)
    );
    assert!(!nodes::detail(&config.database, "alpha")?.row.running);
    drop(commands);
    drop(tx);
    drop(daemon);
    backend.join();
    assert!(!config.socket.exists());
    let backend = Backend::start(config.clone())?;
    assert_eq!(
        backend.handle.history.health.interception.get("alpha"),
        Some(false)
    );
    let mut daemon = client(&config.socket).await?;
    let session = subscribe(&backend, &mut daemon, "alpha", true).await?;
    let (tx, mut commands) = stream(&mut daemon).await?;
    let cmd = command(&mut commands).await?;
    assert_eq!(cmd.r#type, protocol::Action::DisableInterception as i32);
    assert_eq!(cmd.client_name, "alpha");
    assert!(cmd.data.is_empty() && cmd.rules.is_empty());
    assert!(
        nodes::detail(&config.database, "alpha")?.row.running,
        "restoration must await ACK"
    );
    assert!(backend.handle.control.node_busy(session));
    decision(&backend, &mut daemon).await?;
    let lock = rusqlite::Connection::open(&config.database)?;
    lock.execute_batch("BEGIN IMMEDIATE;")?;
    ack(&tx, cmd.id, true).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        backend.handle.control.node_busy(session),
        "restoration stays busy until durable cache completion"
    );
    assert!(
        backend
            .handle
            .control
            .submit_node(session, Change::Interception(true))
            .err()
            .unwrap()
            .contains("restoration")
    );
    decision(&backend, &mut daemon).await?;
    lock.execute_batch("ROLLBACK;")?;
    drop(lock);
    backend.handle.history.flush().await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while nodes::detail(&config.database, "alpha")?.row.running {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    none(&mut commands).await?;
    let reply = backend
        .handle
        .control
        .submit_node(session, Change::Interception(true))
        .map_err(anyhow::Error::msg)?;
    let cmd = command(&mut commands).await?;
    ack(&tx, cmd.id, true).await?;
    reply.await?.map_err(anyhow::Error::msg)?;
    assert_eq!(
        backend.handle.history.health.interception.get("alpha"),
        Some(true)
    );
    remember(&backend, session, "alpha", false, true).await?;
    assert_eq!(
        backend.handle.history.health.interception.get("alpha"),
        None
    );
    none(&mut commands).await?;
    drop(commands);
    drop(tx);
    drop(daemon);
    backend.join();
    let db = rusqlite::Connection::open(&config.database)?;
    assert_eq!(
        db.query_row("SELECT count(*) FROM node_pause", [], |r| r
            .get::<_, i64>(0))?,
        0
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistence_is_node_scoped_never_starts_stopped_daemons_and_does_not_retry_error()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut alpha = client(&config.socket).await?;
    let mut beta = client(&config.socket).await?;
    let old = subscribe(&backend, &mut alpha, "alpha", false).await?;
    remember(&backend, old, "alpha", true, false).await?;
    let b = subscribe(&backend, &mut beta, "beta", false).await?;
    remember(&backend, b, "beta", true, false).await?;
    let (btx, mut bs) = stream(&mut beta).await?;
    none(&mut bs).await?;
    let next = subscribe(&backend, &mut alpha, "alpha", true).await?;
    assert_ne!(old, next);
    let (tx, mut a) = stream(&mut alpha).await?;
    let cmd = command(&mut a).await?;
    assert_eq!(cmd.client_name, "alpha");
    ack(&btx, cmd.id, true).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        nodes::detail(&config.database, "alpha")?.row.running,
        "a crossed ACK cannot confirm restoration"
    );
    ack(&tx, cmd.id, false).await?;
    none(&mut a).await?;
    assert!(backend.handle.history.health.catalogues.uncertain(next));
    assert_eq!(
        backend.handle.history.health.interception.get("alpha"),
        Some(false)
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !backend
        .handle
        .status()
        .last_alert
        .as_deref()
        .is_some_and(|s| s.contains("Could not restore saved pause"))
    {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(nodes::detail(&config.database, "alpha")?.row.running);
    assert!(!nodes::detail(&config.database, "beta")?.row.running);
    drop(a);
    drop(tx);
    let _session = subscribe(&backend, &mut alpha, "alpha", false).await?;
    let (tx, mut a) = stream(&mut alpha).await?;
    none(&mut a).await?;
    // A remembered running choice also does not auto-enable a stopped daemon.
    let session = subscribe(&backend, &mut beta, "beta", true).await?;
    remember(&backend, session, "beta", true, true).await?;
    drop(bs);
    drop(btx);
    subscribe(&backend, &mut beta, "beta", false).await?;
    let (btx, mut bs) = stream(&mut beta).await?;
    none(&mut bs).await?;
    drop(a);
    drop(tx);
    drop(bs);
    drop(btx);
    drop(alpha);
    drop(beta);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_preferences_and_late_sql_failures_preserve_the_confirmed_cache() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut daemon = client(&config.socket).await?;
    let session = subscribe(&backend, &mut daemon, "alpha", true).await?;
    remember(&backend, session, "alpha", true, true).await?;
    let stale = backend
        .handle
        .history
        .remember_interception(session, "alpha".into(), false, true, None)
        .map_err(anyhow::Error::msg)?;
    assert!(stale.await?.unwrap_err().contains("changed"));
    let stale = backend
        .handle
        .history
        .remember_interception(session, "alpha".into(), false, false, Some(true))
        .map_err(anyhow::Error::msg)?;
    assert!(stale.await?.unwrap_err().contains("changed"));
    let (tx, mut commands) = stream(&mut daemon).await?;
    let db = rusqlite::Connection::open(&config.database)?;
    db.execute_batch("CREATE TRIGGER refuse_pause BEFORE UPDATE ON node_pause WHEN NEW.running=0 BEGIN SELECT RAISE(ABORT,'owned pause persistence failure'); END;")?;
    let reply = backend
        .handle
        .control
        .submit_node(session, Change::Interception(false))
        .map_err(anyhow::Error::msg)?;
    let cmd = command(&mut commands).await?;
    ack(&tx, cmd.id, true).await?;
    assert!(
        reply
            .await?
            .unwrap_err()
            .contains("owned pause persistence failure")
    );
    assert!(nodes::detail(&config.database, "alpha")?.row.running);
    assert_eq!(
        backend.handle.history.health.interception.get("alpha"),
        Some(true)
    );
    assert!(db.query_row(
        "SELECT running FROM node_pause WHERE name='alpha'",
        [],
        |r| r.get::<_, bool>(0)
    )?);
    db.execute_batch("DROP TRIGGER refuse_pause;")?;
    drop(db);
    drop(commands);
    drop(tx);
    let next = subscribe(&backend, &mut daemon, "alpha", true).await?;
    assert_ne!(session, next);
    let stale = backend
        .handle
        .history
        .remember_interception(session, "alpha".into(), false, true, Some(true))
        .map_err(anyhow::Error::msg)?;
    assert!(stale.await?.is_err());
    assert_eq!(
        backend.handle.history.health.interception.get("alpha"),
        Some(true)
    );
    drop(daemon);
    backend.join();
    Ok(())
}
