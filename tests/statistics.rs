use anyhow::Result;
use hyper_util::rt::TokioIo;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    history,
    protocol::{self, ui_client::UiClient},
    statistics::{self, Kind, Query, Reset, Sort},
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use tonic::transport::{Channel, Endpoint};
fn options(dir: &Path) -> Config {
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
async fn client(path: &Path) -> Result<UiClient<Channel>> {
    let path = path.to_owned();
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
async fn subscribe(client: &mut UiClient<Channel>, name: &str) -> Result<()> {
    client
        .subscribe(protocol::ClientConfig {
            name: name.into(),
            ..Default::default()
        })
        .await?;
    Ok(())
}
fn stats(count: u64, uptime: u64) -> protocol::Statistics {
    protocol::Statistics {
        uptime,
        connections: 888,
        accepted: 999,
        dropped: 42,
        daemon_version: "1.7.2 fixture".into(),
        by_executable: [("/bin/App".into(), count), ("/bin/app".into(), 10)].into(),
        by_host: [("host.example.test".into(), 12)].into(),
        by_address: [("192.0.2.1".into(), 13)].into(),
        by_port: [("443".into(), 14)].into(),
        by_uid: [("1000".into(), 15)].into(),
        by_proto: [("tcp".into(), 16)].into(),
        ..Default::default()
    }
}
async fn ping(
    client: &mut UiClient<Channel>,
    backend: &Backend,
    stats: protocol::Statistics,
) -> Result<()> {
    client
        .ping(protocol::PingRequest {
            stats: Some(stats),
            ..Default::default()
        })
        .await?;
    backend.handle.history.flush().await?;
    Ok(())
}
fn rows(config: &Config) -> Vec<statistics::Row> {
    statistics::capture(
        &config.database,
        Query {
            sort: Sort::Name,
            ..Default::default()
        },
    )
    .unwrap()
    .page(0)
    .unwrap()
}
async fn reset(backend: &Backend, reset: Reset) -> Result<statistics::ResetResult> {
    backend
        .handle
        .history
        .reset_statistics(reset)
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)
}
fn scope(key: Option<&str>) -> Reset {
    Reset {
        kind: Kind::Applications,
        node: "node-a".into(),
        key: key.map(str::to_owned),
        clear_history: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reported_absolute_counters_baselines_restart_and_persistence() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut daemon = client(&config.socket).await?;
    // The production daemon may Ping before it subscribes.
    daemon
        .ping(protocol::PingRequest {
            stats: Some(stats(100, 20)),
            ..Default::default()
        })
        .await?;
    subscribe(&mut daemon, "node-a").await?;
    backend.handle.history.flush().await?;
    assert_eq!(
        rows(&config).iter().map(|row| row.hits).collect::<Vec<_>>(),
        [100, 10]
    );
    ping(&mut daemon, &backend, stats(100, 21)).await?;
    assert_eq!(rows(&config)[0].hits, 100);
    for (kind, want) in [
        (Kind::Hosts, 12),
        (Kind::Addresses, 13),
        (Kind::Ports, 14),
        (Kind::Users, 15),
        (Kind::Protocols, 16),
    ] {
        let snapshot = statistics::capture(
            &config.database,
            Query {
                kind,
                ..Default::default()
            },
        )?;
        assert_eq!(snapshot.total, want);
        assert_eq!(snapshot.nodes[0].totals.accepted, 999);
    }
    assert_eq!(reset(&backend, scope(Some("/bin/App"))).await?.counters, 1);
    ping(&mut daemon, &backend, stats(100, 22)).await?;
    assert_eq!(rows(&config)[0].hits, 0);
    assert_eq!(rows(&config)[1].hits, 10);
    ping(&mut daemon, &backend, stats(107, 23)).await?;
    assert_eq!(rows(&config)[0].hits, 7);
    // Omitted resident keys keep their last report, rather than becoming zero.
    ping(
        &mut daemon,
        &backend,
        protocol::Statistics {
            uptime: 24,
            by_host: [("new.example.test".into(), 3)].into(),
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(rows(&config)[0].hits, 7);
    let snapshot = statistics::capture(
        &config.database,
        Query {
            kind: Kind::Hosts,
            ..Default::default()
        },
    )?;
    assert_eq!(snapshot.len(), 2);
    drop(snapshot);
    drop(daemon);
    backend.join();
    let backend = Backend::start(config.clone())?;
    assert_eq!(rows(&config)[0].hits, 7);
    let mut daemon = client(&config.socket).await?;
    subscribe(&mut daemon, "node-a").await?;
    ping(&mut daemon, &backend, stats(108, 25)).await?;
    assert_eq!(rows(&config)[0].hits, 8);
    // A dropped per-key total starts its own fresh baseline.
    ping(&mut daemon, &backend, stats(2, 26)).await?;
    assert_eq!(rows(&config)[0].hits, 2);
    // A lower uptime discards stale keys from the previous daemon lifetime.
    ping(
        &mut daemon,
        &backend,
        protocol::Statistics {
            uptime: 1,
            by_executable: [("/bin/new".into(), 4)].into(),
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(rows(&config).len(), 1);
    assert_eq!(rows(&config)[0].key, "/bin/new");
    drop(daemon);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshots_are_exact_frozen_paged_and_exported_without_touching_sources() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut first = client(&config.socket).await?;
    subscribe(&mut first, "node-a").await?;
    let mut sample = protocol::Statistics {
        uptime: 10,
        ..Default::default()
    };
    for index in 0..431 {
        sample
            .by_executable
            .insert(format!("/bin/app-{index:03}"), index + 1);
    }
    sample
        .by_executable
        .insert("comma,\"quote\"\nrow".into(), u64::MAX);
    ping(&mut first, &backend, sample.clone()).await?;
    let mut second = client(&config.socket).await?;
    subscribe(&mut second, "node-b").await?;
    ping(&mut second, &backend, sample).await?;
    let snapshot = statistics::capture(
        &config.database,
        Query {
            sort: Sort::Name,
            ..Default::default()
        },
    )?;
    assert_eq!(snapshot.len(), 432);
    assert_eq!(snapshot.page(0)?.len(), 200);
    assert_eq!(snapshot.page(200)?.len(), 200);
    assert_eq!(snapshot.page(400)?.len(), 32);
    assert!(snapshot.page(600)?.is_empty());
    let huge = &snapshot.page(400)?[31];
    assert_eq!(huge.hits, u128::from(u64::MAX) * 2);
    assert_eq!(huge.nodes, 2);
    ping(
        &mut second,
        &backend,
        protocol::Statistics {
            uptime: 11,
            by_executable: [("/bin/app-000".into(), 99)].into(),
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(snapshot.page(0)?[0].hits, 2);
    assert_eq!(rows(&config)[0].hits, 100);
    let filtered = statistics::capture(
        &config.database,
        Query {
            node: "node-b".into(),
            search: "APP-000".into(),
            ..Default::default()
        },
    )?;
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered.total, 99);
    let target = dir.path().join("counts.csv");
    assert_eq!(
        snapshot.save(&target, false, false, &AtomicBool::new(false))?,
        432
    );
    let csv = std::fs::read_to_string(&target)?;
    assert!(csv.contains("\"comma,\"\"quote\"\"\nrow\""));
    assert!(csv.contains(&(u128::from(u64::MAX) * 2).to_string()));
    assert!(
        snapshot
            .save(&target, true, false, &AtomicBool::new(false))
            .is_err()
    );
    assert_eq!(csv, std::fs::read_to_string(&target)?);
    assert!(
        snapshot
            .save(&target, true, true, &AtomicBool::new(true))
            .is_err()
    );
    assert_eq!(csv, std::fs::read_to_string(&target)?);
    assert_eq!(
        snapshot.save(&target, true, true, &AtomicBool::new(false))?,
        432
    );
    let json = std::fs::read_to_string(&target)?;
    assert_eq!(json.lines().count(), 432);
    let last: serde_json::Value = serde_json::from_str(json.lines().last().unwrap())?;
    assert_eq!(last["hits"], (u128::from(u64::MAX) * 2).to_string());
    for path in [
        config.database.clone(),
        config.database.with_extension("sqlite3-wal"),
        config.database.with_extension("sqlite3-shm"),
        std::path::PathBuf::from(format!("{}.history.json", config.database.display())),
    ] {
        assert!(
            snapshot
                .save(&path, false, true, &AtomicBool::new(false))
                .is_err()
        );
    }
    drop(second);
    drop(first);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resets_delete_only_explicit_exact_history_scope_and_reject_oversized_maps() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut daemon = client(&config.socket).await?;
    subscribe(&mut daemon, "node-a").await?;
    let mut sample = stats(50, 3);
    sample.events = ["/bin/App", "/bin/app", "/bin/other"]
        .into_iter()
        .map(|path| protocol::Event {
            unixnano: 1,
            connection: Some(protocol::Connection {
                process_path: path.into(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    ping(&mut daemon, &backend, sample).await?;
    let mut reset_scope = scope(Some("/bin/App"));
    reset_scope.clear_history = true;
    let result = reset(&backend, reset_scope).await?;
    assert_eq!((result.counters, result.events), (1, 1));
    let page = backend
        .handle
        .history
        .query(history::Query::default())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(page.rows.len(), 2);
    assert!(page.rows.iter().all(|row| row.process != "/bin/App"));
    let invalid = Reset {
        clear_history: true,
        ..scope(None)
    };
    assert!(reset(&backend, invalid).await.is_err());
    let mut oversized = stats(999, 4);
    oversized.by_executable = [("x".repeat(4097), 1)].into();
    ping(&mut daemon, &backend, oversized).await?;
    assert_eq!(
        backend
            .handle
            .history
            .health
            .statistics_skipped
            .load(Ordering::Relaxed),
        1
    );
    assert_eq!(
        backend
            .handle
            .history
            .health
            .dropped
            .load(Ordering::Relaxed),
        0
    );
    assert_eq!(rows(&config)[0].reported, 50);
    assert_eq!(reset(&backend, scope(None)).await?.counters, 2);
    assert!(rows(&config).iter().all(|row| row.hits == 0));
    drop(daemon);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn counter_coalescing_and_disk_contention_do_not_delay_permission_decisions() -> Result<()> {
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut daemon = client(&config.socket).await?;
    subscribe(&mut daemon, "node-a").await?;
    backend.handle.history.flush().await?;
    let db = rusqlite::Connection::open(&config.database)?;
    db.execute_batch("BEGIN IMMEDIATE")?;
    let started = Instant::now();
    for i in 0..20 {
        let sample = protocol::Statistics {
            uptime: 100 + i,
            by_executable: (0..1000)
                .map(|key| (format!("/bin/key-{key:04}"), 100 + i))
                .collect(),
            ..Default::default()
        };
        daemon
            .ping(protocol::PingRequest {
                stats: Some(sample),
                ..Default::default()
            })
            .await?;
    }
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "Ping waited for the writer lock"
    );
    let mut asking = daemon.clone();
    let started = Instant::now();
    let reply = tokio::spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/bin/permission".into(),
                ..Default::default()
            })
            .await
    });
    let id = loop {
        if let Some(prompt) = backend.handle.prompts.snapshot().first() {
            break prompt.id;
        }
        assert!(started.elapsed() < Duration::from_millis(500));
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    assert!(
        backend
            .handle
            .prompts
            .choose(id, Decision::Allow, &Default::default())
            .unwrap()
    );
    let rule = reply.await??.into_inner();
    assert_eq!(rule.action, "allow");
    assert_eq!(rule.duration, "once");
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(
        backend
            .handle
            .history
            .health
            .statistics_skipped
            .load(Ordering::Relaxed)
            > 0
    );
    db.execute_batch("ROLLBACK")?;
    drop(db);
    backend.handle.history.flush().await?;
    let snapshot = statistics::capture(&config.database, Query::default())?;
    assert_eq!(snapshot.len(), 1000);
    assert!(snapshot.page(0)?.iter().all(|row| row.hits == 119));
    drop(daemon);
    backend.join();
    Ok(())
}
