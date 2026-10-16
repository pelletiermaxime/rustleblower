use hyper_util::rt::TokioIo;
use prost::Message;
use rustleblower::{
    alerts,
    backend::Backend,
    config::{Config, Decision},
    protocol,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tonic::transport::Endpoint;
fn options(dir: &Path) -> Config {
    Config {
        socket: dir.join("ui.sock"),
        database: dir.join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    }
}
async fn client(
    socket: &Path,
    name: &str,
) -> anyhow::Result<protocol::ui_client::UiClient<tonic::transport::Channel>> {
    let path = socket.to_owned();
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
    let mut client = protocol::ui_client::UiClient::new(channel);
    client
        .subscribe(protocol::ClientConfig {
            name: name.into(),
            ..Default::default()
        })
        .await?;
    Ok(client)
}
fn alert(id: u64, action: i32, text: &str) -> protocol::Alert {
    protocol::Alert {
        id,
        action,
        r#type: 1,
        priority: 2,
        what: 6,
        data: Some(protocol::alert::Data::Text(text.into())),
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actions_session_ownership_typed_payloads_persistence_and_export() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut desktop = backend.handle.desktop_notices.take_receiver().unwrap();
    let mut alpha = client(&config.socket, "alpha").await?;
    let mut beta = client(&config.socket, "beta").await?;
    assert_eq!(backend.handle.status().session.unwrap().name, "alpha");
    alpha
        .post_alert(alert(1, 1, "Visible alpha warning"))
        .await?;
    let response = beta
        .post_alert(alert(u64::MAX, 2, "Stored beta only"))
        .await?
        .into_inner();
    assert_eq!(response.id, u64::MAX);
    assert_eq!(
        backend.handle.status().last_alert.as_deref(),
        Some("Visible alpha warning")
    );
    beta.post_alert(alert(3, 0, "No action")).await?;
    assert_eq!(desktop.try_recv()?.node, "alpha");
    assert!(
        desktop.try_recv().is_err(),
        "Save-only and NONE do not notify"
    );
    let mut preferences = backend.handle.preferences.snapshot();
    preferences.desktop_notifications = false;
    backend.handle.preferences.save(preferences)?;
    alpha
        .post_alert(alert(9, 1, "Muted desktop, still archived"))
        .await?;
    assert!(desktop.try_recv().is_err());
    beta.post_alert(protocol::Alert {
        id: 4,
        action: 2,
        r#type: 91,
        priority: 72,
        what: 83,
        data: Some(protocol::alert::Data::Proc(protocol::Process {
            pid: u64::MAX,
            path: "/bin/typed".into(),
            args: vec!["one two".into(), "Ω".into()],
            env: [("ALERT_ONLY".into(), "kept".into())].into(),
            ..Default::default()
        })),
    })
    .await?;
    beta.post_alert(protocol::Alert {
        id: 5,
        action: 2,
        data: Some(protocol::alert::Data::Conn(protocol::Connection {
            process_path: "/bin/connect".into(),
            dst_ip: "2001:db8::1".into(),
            dst_port: 443,
            ..Default::default()
        })),
        ..Default::default()
    })
    .await?;
    beta.post_alert(protocol::Alert {
        id: 6,
        action: 2,
        data: Some(protocol::alert::Data::Rule(protocol::Rule {
            name: "alert.rule".into(),
            action: "reject".into(),
            ..Default::default()
        })),
        ..Default::default()
    })
    .await?;
    beta.post_alert(protocol::Alert {
        id: 7,
        action: 2,
        data: Some(protocol::alert::Data::Fwrule(protocol::FwRule {
            chain: "OUTPUT".into(),
            uuid: "rule-id".into(),
            expressions: vec![protocol::Expressions {
                statement: Some(protocol::Statement {
                    op: "match".into(),
                    values: vec![protocol::StatementValues {
                        key: "port".into(),
                        value: "443".into(),
                    }],
                    ..Default::default()
                }),
            }],
            ..Default::default()
        })),
        ..Default::default()
    })
    .await?;
    backend.handle.history.flush().await?;
    let page = alerts::page(&config.database, &alerts::Query::default())?;
    assert_eq!(page.rows.len(), 7);
    assert!(page.rows.iter().filter(|row| row.node == "beta").count() == 5);
    let row = page.rows.iter().find(|row| row.kind == "process").unwrap();
    let detail = alerts::detail(&config.database, row.id)?.unwrap();
    let doc: serde_json::Value = serde_json::from_str(&detail.document)?;
    assert_eq!(doc["payload"]["pid"].as_u64(), Some(u64::MAX));
    assert_eq!(doc["payload"]["args"][0], "one two");
    assert_eq!(doc["payload"]["env"]["ALERT_ONLY"], "kept");
    assert_eq!(row.severity, 91);
    assert_eq!(row.priority, 72);
    let filtered = alerts::Query {
        node: "beta".into(),
        search: "ALERT_ONLY".into(),
        ..Default::default()
    };
    assert_eq!(alerts::page(&config.database, &filtered)?.rows.len(), 1);
    let change = backend
        .handle
        .history
        .change_alert(alerts::Change::Read(row.id))
        .unwrap();
    assert_eq!(change.await?.map_err(anyhow::Error::msg)?, 1);
    assert_eq!(
        alerts::page(
            &config.database,
            &alerts::Query {
                unread: true,
                ..Default::default()
            }
        )?
        .rows
        .len(),
        6
    );
    let target = dir.path().join("alerts.jsonl");
    let stop = std::sync::atomic::AtomicBool::new(false);
    let report = alerts::export(
        &config.database,
        &alerts::Query::default(),
        &target,
        alerts::Format::JsonLines,
        false,
        &stop,
    )?;
    assert_eq!(report.rows, 7);
    assert!(report.bytes > 0);
    let output = std::fs::read_to_string(&target)?;
    assert_eq!(output.lines().count(), 7);
    assert!(output.contains("ALERT_ONLY"));
    assert!(
        alerts::export(
            &config.database,
            &alerts::Query::default(),
            &target,
            alerts::Format::Csv,
            false,
            &stop
        )
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&target)?, output);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        alerts::export(
            &config.database,
            &alerts::Query::default(),
            &target,
            alerts::Format::Csv,
            true,
            &stop
        )
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&target)?, output);
    stop.store(false, std::sync::atomic::Ordering::Relaxed);
    assert!(
        alerts::export(
            &config.database,
            &alerts::Query::default(),
            &config.socket,
            alerts::Format::Csv,
            true,
            &stop
        )
        .is_err()
    );
    alpha.ping(protocol::PingRequest::default()).await?;
    assert!(
        alerts::export(
            &config.database,
            &filtered,
            &config.database,
            alerts::Format::Csv,
            true,
            &stop
        )
        .is_err()
    );
    let db = rusqlite::Connection::open(&config.database)?;
    let wire: Vec<u8> = db.query_row("SELECT payload FROM alerts WHERE id=?", [row.id], |r| {
        r.get(0)
    })?;
    let decoded = protocol::Alert::decode(wire.as_slice())?;
    assert_eq!(decoded.priority, 72);
    let unknown = alpha
        .post_alert(alert(8, 99, "unknown action"))
        .await?
        .into_inner();
    assert_eq!(unknown.id, 8);
    assert_eq!(
        backend
            .handle
            .history
            .health
            .alerts_skipped
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    drop(db);
    drop(alpha);
    drop(beta);
    backend.join();
    let reopened = Backend::start(config.clone())?;
    assert_eq!(
        alerts::page(&config.database, &alerts::Query::default())?
            .rows
            .len(),
        7
    );
    reopened.join();
    assert!(!config.socket.exists());
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn alerts_under_writer_lock_never_delay_ping_or_permission() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut daemon = client(&config.socket, "held-writer").await?;
    backend.handle.history.flush().await?;
    let db = rusqlite::Connection::open(&config.database)?;
    db.execute_batch("BEGIN IMMEDIATE")?;
    let started = Instant::now();
    for id in 0..40 {
        daemon.post_alert(alert(id, 2, "held writer")).await?;
    }
    assert!(started.elapsed() < Duration::from_secs(1));
    let mut pinger = daemon.clone();
    let mut asker = daemon.clone();
    let task = tokio::spawn(async move {
        asker
            .ask_rule(protocol::Connection {
                process_path: "/bin/independent".into(),
                ..Default::default()
            })
            .await
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while backend.handle.prompts.snapshot().is_empty() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let start = Instant::now();
    assert_eq!(
        pinger
            .ping(protocol::PingRequest {
                id: 42,
                stats: None
            })
            .await?
            .into_inner()
            .id,
        42
    );
    let prompt = backend.handle.prompts.snapshot()[0].id;
    assert!(backend.handle.decide(prompt, Decision::Allow));
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(500), task)
            .await???
            .into_inner()
            .action,
        "allow"
    );
    assert!(start.elapsed() < Duration::from_millis(500));
    db.execute_batch("ROLLBACK")?;
    drop(db);
    backend.handle.history.flush().await?;
    let stored = alerts::page(&config.database, &alerts::Query::default())?
        .rows
        .len();
    let skipped = backend
        .handle
        .history
        .health
        .alerts_skipped
        .load(std::sync::atomic::Ordering::Relaxed) as usize;
    assert_eq!(stored + skipped, 40);
    assert!(skipped > 0);
    drop(daemon);
    drop(pinger);
    backend.join();
    assert!(!config.socket.exists());
    Ok(())
}
