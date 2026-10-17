use anyhow::Result;
use hyper_util::rt::TokioIo;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    history::Query,
    prompts::MAX_PENDING,
    protocol::{self, ui_client::UiClient},
};
use std::{
    path::Path,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tonic::transport::{Channel, Endpoint};

fn config(dir: &Path) -> Config {
    Config {
        transport: Default::default(),
        socket: dir.join("ui.sock"),
        database: dir.join("history.sqlite3"),
        prompt_timeout: 1,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    }
}
async fn connect(path: &Path) -> Result<UiClient<Channel>> {
    let path = path.to_path_buf();
    let channel = Endpoint::try_from("http://[::]:50051")?
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
async fn client(path: &Path) -> Result<UiClient<Channel>> {
    let mut client = connect(path).await?;
    client
        .subscribe(protocol::ClientConfig {
            name: "test-daemon".into(),
            config: "{\"DefaultAction\":\"allow\"}".into(),
            ..Default::default()
        })
        .await?;
    Ok(client)
}
fn connection() -> protocol::Connection {
    protocol::Connection {
        process_path: "/usr/bin/test-client".into(),
        dst_ip: "192.0.2.42".into(),
        dst_port: 443,
        protocol: "tcp".into(),
        user_id: 1000,
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_and_shutdown_use_each_requests_captured_preference_overrides() -> Result<()> {
    use rustleblower::preferences::Popup;
    let dir = tempfile::tempdir()?;
    let mut config = config(dir.path());
    config.prompt_timeout = 5;
    let backend = Backend::start(config.clone())?;
    let daemon = client(&config.socket).await?;
    backend.handle.preferences.save(Popup {
        duration: "always".into(),
        timeout: Some(1),
        fallback: Some(Decision::Reject),
        ..Default::default()
    })?;
    let mut asking = daemon.clone();
    let old = tokio::spawn(async move { asking.ask_rule(connection()).await });
    wait_pending(&backend, 1).await;
    let old_deadline = backend.handle.prompts.snapshot()[0].deadline;
    backend.handle.preferences.save(Popup {
        timeout: Some(5),
        fallback: Some(Decision::Allow),
        ..Default::default()
    })?;
    let mut asking = daemon.clone();
    let new = tokio::spawn(async move { asking.ask_rule(connection()).await });
    wait_pending(&backend, 2).await;
    let prompts = backend.handle.prompts.snapshot();
    assert_eq!(prompts[0].deadline, old_deadline);
    assert_eq!(prompts[0].initial.fallback, Some(Decision::Reject));
    assert_eq!(prompts[1].initial.fallback, Some(Decision::Allow));
    assert!(prompts[1].deadline.duration_since(prompts[0].deadline) > Duration::from_secs(3));
    let old = old.await??.into_inner();
    assert_eq!(old.action, "reject");
    assert_eq!(old.duration, "once");
    assert_eq!(backend.handle.prompts.snapshot().len(), 1);
    backend.handle.shutdown();
    let new = new.await??.into_inner();
    assert_eq!(new.action, "allow");
    assert_eq!(new.duration, "once");
    drop(daemon);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_preserves_old_fallback_and_disabled_requests_use_new_override() -> Result<()> {
    use rustleblower::preferences::Popup;
    let dir = tempfile::tempdir()?;
    let mut config = config(dir.path());
    config.prompt_timeout = 5;
    let backend = Backend::start(config.clone())?;
    let daemon = client(&config.socket).await?;
    backend.handle.preferences.save(Popup {
        fallback: Some(Decision::Reject),
        ..Default::default()
    })?;
    let mut asking = daemon.clone();
    let old = tokio::spawn(async move { asking.ask_rule(connection()).await });
    wait_pending(&backend, 1).await;
    assert!(
        backend
            .handle
            .prompts
            .review(backend.handle.prompts.snapshot()[0].id)
    );
    backend.handle.preferences.save(Popup {
        popups: false,
        fallback: Some(Decision::Allow),
        ..Default::default()
    })?;
    let mut replacement = client(&config.socket).await?;
    let old = old.await??.into_inner();
    assert_eq!(old.action, "reject");
    assert_eq!(old.duration, "once");
    let started = Instant::now();
    let new = replacement.ask_rule(connection()).await?.into_inner();
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(new.action, "allow");
    assert_eq!(new.duration, "once");
    assert!(backend.handle.prompts.snapshot().is_empty());
    drop(replacement);
    drop(daemon);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_popups_reply_immediately_once_without_disturbing_existing_prompt() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = config(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut daemon = client(&config.socket).await?;
    let mut asking = daemon.clone();
    let old = tokio::spawn(async move { asking.ask_rule(connection()).await });
    wait_pending(&backend, 1).await;
    let prompt = backend.handle.prompts.snapshot()[0].id;
    let preferences = rustleblower::preferences::Popup {
        popups: false,
        duration: "always".into(),
        ..Default::default()
    };
    backend.handle.preferences.save(preferences.clone())?;
    assert_eq!(
        backend.handle.prompts.snapshot()[0].initial.choice.duration,
        "once",
        "already queued request retains its original defaults"
    );
    let started = Instant::now();
    let reply = daemon.ask_rule(connection()).await?.into_inner();
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(reply.action, "deny");
    assert_eq!(reply.duration, "once");
    assert_eq!(backend.handle.prompts.snapshot().len(), 1);
    assert_eq!(backend.handle.status().overloaded, 0);
    assert!(backend.handle.decide(prompt, Decision::Allow));
    assert_eq!(old.await??.into_inner().action, "allow");
    backend.handle.history.flush().await?;
    let page = backend
        .handle
        .history
        .query(Query::default())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert!(page.rows.iter().any(|row| row.reason == "popups-disabled"));
    drop(daemon);
    backend.join();
    assert_eq!(
        rustleblower::preferences::Store::open(&config.database)?.snapshot(),
        preferences
    );
    Ok(())
}
async fn wait_pending(backend: &Backend, n: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while backend.handle.prompts.snapshot().len() != n {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("pending prompt count");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_extends_only_the_interacted_request_and_retains_its_rpc_limit() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let backend = Backend::start(config(dir.path()))?;
    let daemon = client(&dir.path().join("ui.sock")).await?;
    let mut asking = daemon.clone();
    let mut request = tonic::Request::new(connection());
    request.set_timeout(Duration::from_secs(4));
    let reviewed = tokio::spawn(async move { asking.ask_rule(request).await });
    wait_pending(&backend, 1).await;
    let original = backend.handle.prompts.snapshot()[0].clone();
    assert!(backend.handle.prompts.review(original.id));
    assert!(!backend.handle.prompts.review(original.id));
    let prompt = backend.handle.prompts.snapshot()[0].clone();
    assert!(prompt.reviewing);
    assert_eq!(prompt.deadline, original.hard_deadline);
    assert!(prompt.deadline > original.deadline + Duration::from_secs(2));
    let mut asking = daemon.clone();
    let queued = tokio::spawn(async move { asking.ask_rule(connection()).await });
    wait_pending(&backend, 2).await;
    let queued_id = backend.handle.prompts.snapshot()[1].id;
    assert_eq!(queued.await??.into_inner().duration, "once");
    assert!(
        !reviewed.is_finished(),
        "review survives ordinary countdown"
    );
    assert!(
        !backend.handle.prompts.review(queued_id),
        "expired input cannot revive a prompt"
    );
    assert_eq!(
        backend.handle.prompts.snapshot()[0].deadline,
        prompt.deadline
    );
    assert!(backend.handle.decide(original.id, Decision::Allow));
    assert_eq!(reviewed.await??.into_inner().action, "allow");
    assert_eq!(backend.handle.status().answered, 1);
    assert_eq!(backend.handle.status().timed_out, 1);
    assert!(!backend.handle.prompts.review(original.id));
    drop(daemon);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reviewed_request_expires_before_rpc_limit_to_captured_once_fallback() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let backend = Backend::start(config(dir.path()))?;
    backend
        .handle
        .preferences
        .save(rustleblower::preferences::Popup {
            duration: "always".into(),
            fallback: Some(Decision::Reject),
            ..Default::default()
        })?;
    let mut daemon = client(&dir.path().join("ui.sock")).await?;
    let mut request = tonic::Request::new(connection());
    request.set_timeout(Duration::from_millis(1800));
    let start = Instant::now();
    let reply = tokio::spawn(async move { daemon.ask_rule(request).await });
    wait_pending(&backend, 1).await;
    let prompt = backend.handle.prompts.snapshot()[0].clone();
    assert!(backend.handle.prompts.review(prompt.id));
    backend
        .handle
        .preferences
        .save(rustleblower::preferences::Popup {
            fallback: Some(Decision::Allow),
            ..Default::default()
        })?;
    let rule = reply.await??.into_inner();
    assert!(start.elapsed() >= Duration::from_millis(1500));
    assert!(start.elapsed() < Duration::from_millis(1800));
    assert_eq!(rule.action, "reject");
    assert_eq!(rule.duration, "once");
    assert_eq!(backend.handle.status().timed_out, 1);
    assert!(!backend.handle.prompts.review(prompt.id));
    backend.handle.history.flush().await?;
    let db = rusqlite::Connection::open(dir.path().join("history.sqlite3"))?;
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM events WHERE reason='timeout' AND action='reject'",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        1
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM prompt_rules", [], |row| row
            .get::<_, i64>(0))?,
        0
    );
    drop(db);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lasting_popup_replies_wait_for_daemon_observation_before_becoming_editable() -> Result<()>
{
    use rustleblower::prompts::choice::Choice;
    let dir = tempfile::tempdir()?;
    let mut settings = config(dir.path());
    settings.prompt_timeout = 5;
    let backend = Backend::start(settings)?;
    let mut client = client(&dir.path().join("ui.sock")).await?;
    backend.handle.history.flush().await?;
    let mut asking = client.clone();
    let task = tokio::spawn(async move { asking.ask_rule(connection()).await });
    wait_pending(&backend, 1).await;
    let id = backend.handle.prompts.snapshot()[0].id;
    let choice = Choice {
        duration: "always".into(),
        target: "process.path".into(),
        constraints: vec!["dest.ip".into(), "dest.port".into()],
        priority: true,
        logging: false,
    };
    assert!(
        backend
            .handle
            .prompts
            .choose(id, Decision::Reject, &choice)
            .unwrap()
    );
    let reply = task.await??.into_inner();
    assert_eq!(reply.action, "reject");
    assert_eq!(reply.duration, "always");
    assert!(reply.precedence && reply.nolog);
    backend.handle.history.flush().await?;
    let query = rustleblower::rules::Query {
        node: "test-daemon".into(),
        ..Default::default()
    };
    let page = backend
        .handle
        .history
        .query_rules(query.clone())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(page.rules, vec![reply.clone()]);
    assert!(
        page.awaiting.contains(&reply.name),
        "the UI's own decision history must not confirm its reply"
    );
    let scope = rustleblower::rules::transfer::ExportScope {
        query: query.clone(),
        names: None,
    };
    assert!(
        rustleblower::rules::transfer::load_rules(backend.handle.history.database_path(), &scope)
            .is_err()
    );
    assert!(
        rustleblower::rules::transfer::find_confirmed_rule(
            backend.handle.history.database_path(),
            "test-daemon",
            &reply.name
        )?
        .is_none()
    );
    let mut observed = reply.clone();
    observed.description = "Daemon-reported metadata".into();
    observed.created += 1;
    client
        .ping(protocol::PingRequest {
            id: 99,
            stats: Some(protocol::Statistics {
                rules: 1,
                events: vec![protocol::Event {
                    unixnano: 12345,
                    connection: Some(connection()),
                    rule: Some(observed.clone()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
        })
        .await?;
    backend.handle.history.flush().await?;
    let page = backend
        .handle
        .history
        .query_rules(query.clone())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(page.rules, vec![observed]);
    assert!(page.awaiting.is_empty());
    assert_eq!(
        rustleblower::rules::transfer::load_rules(backend.handle.history.database_path(), &scope)?
            .len(),
        1
    );
    // Another unobserved choice is discarded by an authoritative fresh inventory.
    let mut asking = client.clone();
    let task = tokio::spawn(async move { asking.ask_rule(connection()).await });
    wait_pending(&backend, 1).await;
    let id = backend.handle.prompts.snapshot()[0].id;
    assert!(
        backend
            .handle
            .prompts
            .choose(id, Decision::Allow, &choice)
            .unwrap()
    );
    let _ = task.await??;
    client
        .subscribe(protocol::ClientConfig {
            name: "test-daemon".into(),
            ..Default::default()
        })
        .await?;
    backend.handle.history.flush().await?;
    assert!(
        backend
            .handle
            .history
            .query_rules(query)
            .unwrap()
            .await?
            .map_err(anyhow::Error::msg)?
            .rules
            .is_empty()
    );
    drop(client);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reject_timeout_is_still_once_and_creates_no_lasting_popup_entry() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut settings = config(dir.path());
    settings.timeout_action = Decision::Reject;
    let backend = Backend::start(settings)?;
    let mut client = client(&dir.path().join("ui.sock")).await?;
    let rule = client.ask_rule(connection()).await?.into_inner();
    assert_eq!(rule.action, "reject");
    assert_eq!(rule.duration, "once");
    backend.handle.history.flush().await?;
    let page = backend
        .handle
        .history
        .query_rules(rustleblower::rules::Query {
            node: "test-daemon".into(),
            ..Default::default()
        })
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert!(page.rules.is_empty());
    drop(client);
    backend.join();
    Ok(())
}

fn event_ping(id: u64) -> protocol::PingRequest {
    protocol::PingRequest {
        id,
        stats: Some(protocol::Statistics {
            daemon_version: "1.7.2".into(),
            events: vec![protocol::Event {
                unixnano: id as i64,
                connection: Some(connection()),
                ..Default::default()
            }],
            ..Default::default()
        }),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn early_ping_survives_subscribe_and_replay_without_history_loss() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let backend = Backend::start(config(dir.path()))?;
    let mut client = connect(&dir.path().join("ui.sock")).await?;
    // Reproduce the daemon's startup ordering deterministically, rather than
    // relying on whether concurrently issued RPCs happen to race in this run.
    let ping = event_ping(17);
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(500), client.ping(ping.clone()))
            .await??
            .into_inner()
            .id,
        17
    );
    assert!(backend.handle.status().session.is_none());
    assert!(backend.handle.history.health.warning().is_none());
    let settings = protocol::ClientConfig {
        name: "early-daemon".into(),
        config: "{\"DefaultAction\":\"allow\"}".into(),
        ..Default::default()
    };
    assert_eq!(
        client.subscribe(settings.clone()).await?.into_inner(),
        settings
    );
    assert_eq!(backend.handle.status().session.unwrap().version, "1.7.2");
    client.ping(ping).await?;
    backend.handle.history.flush().await?;
    let db = rusqlite::Connection::open(dir.path().join("history.sqlite3"))?;
    let (rows, node): (i64, String) =
        db.query_row("SELECT count(*), node FROM events", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    assert_eq!((rows, node), (1, "early-daemon".into()));
    assert_eq!(
        backend
            .handle
            .history
            .health
            .dropped
            .load(Ordering::Relaxed),
        0
    );
    assert!(backend.handle.history.health.warning().is_none());
    drop(db);
    drop(client);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn early_reports_are_bounded_and_released_when_connection_closes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let backend = Backend::start(config(dir.path()))?;
    let mut early = connect(&dir.path().join("ui.sock")).await?;
    for id in 1..=33 {
        early.ping(event_ping(id)).await?;
    }
    let health = &backend.handle.history.health;
    assert_eq!(health.queue_full.load(Ordering::Relaxed), 1);
    assert_eq!(health.dropped.load(Ordering::Relaxed), 1);
    drop(early);
    tokio::time::timeout(Duration::from_secs(2), async {
        while health.connection_lost.load(Ordering::Relaxed) != 32 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert_eq!(health.dropped.load(Ordering::Relaxed), 33);
    let warning = health.warning().unwrap();
    assert!(warning.contains("1 while recording couldn't keep up"));
    assert!(warning.contains("32 during a daemon disconnect"));
    let mut replacement = client(&dir.path().join("ui.sock")).await?;
    replacement.ping(event_ping(100)).await?;
    backend.handle.history.flush().await?;
    let page = backend
        .handle
        .history
        .query(Query::default())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        page.rows.len(),
        1,
        "early data from another connection must never leak into the replacement"
    );
    drop(replacement);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn superseded_connection_cannot_buffer_reports_for_a_later_subscribe() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let backend = Backend::start(config(dir.path()))?;
    let mut old = client(&dir.path().join("ui.sock")).await?;
    let replacement = client(&dir.path().join("ui.sock")).await?;
    old.ping(event_ping(99)).await?;
    old.subscribe(protocol::ClientConfig {
        name: "returning-daemon".into(),
        ..Default::default()
    })
    .await?;
    old.ping(event_ping(100)).await?;
    backend.handle.history.flush().await?;
    let page = backend
        .handle
        .history
        .query(Query::default())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].unixnano, 100);
    assert_eq!(
        backend
            .handle
            .history
            .health
            .connection_lost
            .load(Ordering::Relaxed),
        1
    );
    drop(old);
    drop(replacement);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wire_protocol_decision_and_persistence() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let backend = Backend::start(config(dir.path()))?;
    let mut client = client(&dir.path().join("ui.sock")).await?;
    let (notifications_tx, notifications_rx) = tokio::sync::mpsc::channel(4);
    let stream = client
        .notifications(tokio_stream::wrappers::ReceiverStream::new(
            notifications_rx,
        ))
        .await?;
    let ping = protocol::PingRequest {
        id: 42,
        stats: Some(protocol::Statistics {
            daemon_version: "1.7.2".into(),
            events: vec![protocol::Event {
                unixnano: 123,
                connection: Some(connection()),
                rule: Some(protocol::Rule {
                    action: "deny".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }),
    };
    assert_eq!(client.ping(ping.clone()).await?.into_inner().id, 42);
    client.ping(ping).await?;
    let mut asking = client.clone();
    let task = tokio::spawn(async move { asking.ask_rule(connection()).await });
    wait_pending(&backend, 1).await;
    let id = backend.handle.prompts.snapshot()[0].id;
    assert!(backend.handle.decide(id, Decision::Allow));
    assert!(!backend.handle.decide(id, Decision::Deny));
    let rule = task.await??.into_inner();
    assert_eq!(rule.action, "allow");
    assert_eq!(rule.duration, "once");
    let operator = rule.operator.unwrap();
    assert_eq!(operator.r#type, "list");
    assert!(
        operator
            .list
            .iter()
            .any(|op| op.operand == "dest.ip" && op.data == "192.0.2.42")
    );
    assert!(operator.list.iter().all(|op| op.sensitive));
    client
        .post_alert(protocol::Alert {
            id: 7,
            action: protocol::alert::Action::ShowAlert as i32,
            data: Some(protocol::alert::Data::Text("fixture warning".into())),
            ..Default::default()
        })
        .await?;
    assert_eq!(
        backend.handle.status().last_alert.as_deref(),
        Some("fixture warning")
    );
    backend.handle.history.flush().await?;
    let page = backend
        .handle
        .history
        .query(Query::default())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(page.rows.len(), 2, "one observed event plus one decision");
    drop(stream);
    drop(notifications_tx);
    drop(client);
    backend.join();
    assert!(!dir.path().join("ui.sock").exists());
    let db = rusqlite::Connection::open(dir.path().join("history.sqlite3"))?;
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))?,
        2
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_and_cancel_remove_requests() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let backend = Backend::start(config(dir.path()))?;
    let mut client = client(&dir.path().join("ui.sock")).await?;
    let start = Instant::now();
    let rule = client.ask_rule(connection()).await?.into_inner();
    assert!(start.elapsed() >= Duration::from_millis(900));
    assert_eq!(rule.action, "deny");
    assert_eq!(rule.duration, "once");
    assert_eq!(backend.handle.status().timed_out, 1);
    wait_pending(&backend, 0).await;
    let mut asking = client.clone();
    let task = tokio::spawn(async move { asking.ask_rule(connection()).await });
    wait_pending(&backend, 1).await;
    let id = backend.handle.prompts.snapshot()[0].id;
    assert!(backend.handle.prompts.review(id));
    task.abort();
    let _ = task.await;
    wait_pending(&backend, 0).await;
    assert!(!backend.handle.prompts.review(id));
    assert!(!backend.handle.decide(id, Decision::Allow));
    drop(client);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounded_overload_and_busy_history_do_not_block_prompt_or_ping() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut options = config(dir.path());
    options.prompt_timeout = 10;
    let backend = Backend::start(options)?;
    backend
        .handle
        .preferences
        .save(rustleblower::preferences::Popup {
            timeout: Some(110),
            ..Default::default()
        })?;
    let mut client = client(&dir.path().join("ui.sock")).await?;
    let db = rusqlite::Connection::open(dir.path().join("history.sqlite3"))?;
    db.execute_batch("BEGIN IMMEDIATE")?;
    let ping = protocol::PingRequest {
        id: 9,
        stats: Some(protocol::Statistics {
            events: vec![protocol::Event {
                unixnano: 1,
                connection: Some(connection()),
                ..Default::default()
            }],
            ..Default::default()
        }),
    };
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(500), client.ping(ping))
            .await??
            .into_inner()
            .id,
        9
    );
    let mut tasks = vec![];
    for _ in 0..MAX_PENDING {
        let mut c = client.clone();
        tasks.push(tokio::spawn(async move { c.ask_rule(connection()).await }));
    }
    wait_pending(&backend, MAX_PENDING).await;
    let overflow = tokio::time::timeout(Duration::from_millis(500), client.ask_rule(connection()))
        .await??
        .into_inner();
    assert_eq!(overflow.action, "deny");
    assert_eq!(backend.handle.prompts.snapshot().len(), MAX_PENDING);
    let ids: Vec<_> = backend
        .handle
        .prompts
        .snapshot()
        .iter()
        .map(|p| p.id)
        .collect();
    let start = Instant::now();
    for id in ids {
        assert!(backend.handle.decide(id, Decision::Allow));
    }
    for task in tasks {
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(500), task)
                .await???
                .into_inner()
                .action,
            "allow"
        );
    }
    assert!(start.elapsed() < Duration::from_secs(1));
    db.execute_batch("ROLLBACK")?;
    let too_many = protocol::PingRequest {
        id: 10,
        stats: Some(protocol::Statistics {
            events: vec![protocol::Event::default(); 5000],
            ..Default::default()
        }),
    };
    client.ping(too_many).await?;
    assert_eq!(
        backend
            .handle
            .history
            .health
            .event_limit
            .load(Ordering::Relaxed),
        5000
    );
    assert!(
        backend
            .handle
            .history
            .health
            .dropped
            .load(Ordering::Relaxed)
            >= 5000
    );
    drop(client);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_invalidates_old_prompts_and_stream_close_disconnects() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut options = config(dir.path());
    options.prompt_timeout = 10;
    let backend = Backend::start(options)?;
    let mut old = client(&dir.path().join("ui.sock")).await?;
    let asking = tokio::spawn(async move { old.ask_rule(connection()).await });
    wait_pending(&backend, 1).await;
    let id = backend.handle.prompts.snapshot()[0].id;
    let mut replacement = client(&dir.path().join("ui.sock")).await?;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), asking)
            .await???
            .into_inner()
            .action,
        "deny"
    );
    assert!(!backend.handle.decide(id, Decision::Allow));
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let stream = replacement
        .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await?;
    drop(stream);
    drop(tx);
    tokio::time::timeout(Duration::from_secs(2), async {
        while backend.handle.status().session.is_some() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    drop(replacement);
    backend.join();
    Ok(())
}

#[test]
fn refuses_to_replace_an_existing_socket_or_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = config(dir.path());
    std::fs::write(&cfg.socket, "owned elsewhere")?;
    assert!(Backend::start(cfg.clone()).is_err());
    assert_eq!(std::fs::read_to_string(cfg.socket)?, "owned elsewhere");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_deadline_and_shutdown_get_explicit_once_fallbacks() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut options = config(dir.path());
    options.prompt_timeout = 10;
    let backend = Backend::start(options)?;
    let mut client = client(&dir.path().join("ui.sock")).await?;
    let mut request = tonic::Request::new(connection());
    request.set_timeout(Duration::from_millis(400));
    let start = Instant::now();
    let rule = client.ask_rule(request).await?.into_inner();
    assert_eq!(rule.action, "deny");
    assert_eq!(rule.duration, "once");
    assert!(start.elapsed() < Duration::from_millis(400));
    let mut asking = client.clone();
    let task = tokio::spawn(async move { asking.ask_rule(connection()).await });
    wait_pending(&backend, 1).await;
    assert!(
        backend
            .handle
            .prompts
            .review(backend.handle.prompts.snapshot()[0].id)
    );
    backend.handle.shutdown();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await???
            .into_inner()
            .action,
        "deny"
    );
    drop(client);
    backend.join();
    Ok(())
}
