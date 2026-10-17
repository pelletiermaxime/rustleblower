use anyhow::Result;
use hyper_util::rt::TokioIo;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    history::{self, storage},
    protocol::{self, ui_client::UiClient},
};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tonic::transport::{Channel, Endpoint};
fn options(dir: &Path) -> Config {
    Config {
        transport: Default::default(),
        socket: dir.join("ui.sock"),
        database: dir.join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 32,
        headless: true,
    }
}
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
fn event(id: i64) -> protocol::Event {
    protocol::Event {
        unixnano: history::now_nanos() + id,
        connection: Some(protocol::Connection {
            process_path: format!("/bin/history-preference-{id}"),
            dst_ip: "192.0.2.1".into(),
            dst_port: 443,
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn count(db: &rusqlite::Connection, table: &str) -> u64 {
    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retention_is_transactional_persistent_stale_safe_and_independent_of_permissions()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = options(dir.path());
    let backend = Backend::start(cfg.clone())?;
    let history = &backend.handle.history;
    let mut daemon = client(&cfg.socket).await?;
    daemon
        .subscribe(protocol::ClientConfig {
            name: "retention-fixture".into(),
            ..Default::default()
        })
        .await?;
    history.ingest("fixture".into(), (0..12).map(event).collect(), "observed");
    for id in 0..3 {
        daemon
            .post_alert(protocol::Alert {
                id,
                action: 2,
                data: Some(protocol::alert::Data::Text(format!("retained alert {id}"))),
                ..Default::default()
            })
            .await?;
    }
    daemon
        .ping(protocol::PingRequest {
            stats: Some(protocol::Statistics {
                by_executable: [("old".into(), 3), ("new".into(), 4)].into(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await?;
    history.flush().await?;
    let db = rusqlite::Connection::open(&cfg.database)?;
    let old = history::now_seconds() - 2 * 86400;
    db.execute("UPDATE events SET received=? WHERE id<=4", [old])?;
    db.execute(
        "UPDATE alerts SET received=? WHERE id=(SELECT min(id) FROM alerts)",
        [old],
    )?;
    db.execute(
        "UPDATE statistics_counts SET updated=? WHERE key='old'",
        [old * 1_000_000_000],
    )?;
    let base = history.storage.snapshot();
    let candidate = storage::Config {
        days: 1,
        mib: 16,
        cleanup_minutes: 1,
    };
    let preview = history
        .preview_storage(candidate, base.revision)
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        (
            preview.expired_events,
            preview.expired_alerts,
            preview.expired_counters
        ),
        (4, 1, 1)
    );
    assert_eq!(count(&db, "events"), 12, "Preview never prunes");
    db.execute_batch("CREATE TRIGGER reject_preferences BEFORE UPDATE ON history_preferences BEGIN SELECT RAISE(ABORT,'synthetic settings failure'); END;")?;
    assert!(
        history
            .apply_storage(candidate, base.revision)
            .unwrap()
            .await?
            .is_err()
    );
    assert_eq!(history.storage.snapshot(), base);
    assert_eq!(
        (
            count(&db, "events"),
            count(&db, "alerts"),
            count(&db, "statistics_counts")
        ),
        (12, 3, 2),
        "Failed settings write rolls back cleanup too"
    );
    db.execute_batch("DROP TRIGGER reject_preferences;")?;
    // Hold SQLite beyond a permission deadline budget; saving must not block RPC replies.
    db.execute_batch("BEGIN IMMEDIATE")?;
    let applying = history.apply_storage(candidate, base.revision).unwrap();
    let start = Instant::now();
    tokio::time::timeout(
        Duration::from_millis(500),
        daemon.ping(protocol::PingRequest {
            id: 99,
            ..Default::default()
        }),
    )
    .await??;
    let mut asking = daemon.clone();
    let task = tokio::spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/bin/retention-permission".into(),
                ..Default::default()
            })
            .await
    });
    tokio::time::timeout(Duration::from_millis(400), async {
        while backend.handle.prompts.snapshot().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await
        }
    })
    .await?;
    assert!(
        backend
            .handle
            .decide(backend.handle.prompts.snapshot()[0].id, Decision::Allow)
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(400), task)
            .await???
            .into_inner()
            .action,
        "allow"
    );
    assert!(start.elapsed() < Duration::from_millis(500));
    db.execute_batch("ROLLBACK")?;
    let report = applying.await?.map_err(anyhow::Error::msg)?;
    assert_eq!(
        (
            report.removed_events,
            report.removed_alerts,
            report.removed_counters
        ),
        (4, 1, 1)
    );
    assert_eq!(report.snapshot.config, candidate);
    assert_eq!(history.storage.snapshot().config, candidate);
    assert!(
        history
            .apply_storage(
                storage::Config {
                    days: 2,
                    ..candidate
                },
                base.revision
            )
            .unwrap()
            .await?
            .unwrap_err()
            .contains("another editor")
    );
    drop(db);
    drop(daemon);
    backend.join();
    let reopened = Backend::start(cfg.clone())?;
    assert_eq!(reopened.handle.history.storage.snapshot().config, candidate);
    reopened.join();
    assert!(!cfg.socket.exists());
    Ok(())
}
#[tokio::test]
async fn physical_compaction_is_reported_without_rejecting_saved_limits() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = options(dir.path());
    let backend = Backend::start(cfg.clone())?;
    let db = rusqlite::Connection::open(&cfg.database)?;
    db.execute_batch("CREATE TABLE temporary_pressure(value BLOB);")?;
    for _ in 0..15 {
        db.execute(
            "INSERT INTO temporary_pressure VALUES(zeroblob(1048576))",
            [],
        )?;
    }
    db.execute_batch("DELETE FROM temporary_pressure;")?;
    let base = backend.handle.history.storage.snapshot();
    let candidate = storage::Config {
        mib: 16,
        ..base.config
    };
    let report = backend
        .handle
        .history
        .apply_storage(candidate, base.revision)
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(backend.handle.history.storage.snapshot().config.mib, 16);
    assert!(
        report.disk.pending(),
        "Freed pages are compacted gradually, not presented as an immediate filesystem quota"
    );
    assert!(report.disk.reusable > 0);
    drop(db);
    backend.join();
    Ok(())
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[tokio::test]
async fn explicit_cli_flags_override_saved_retention_and_survive_reopen() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = options(dir.path());
    let backend = Backend::start(cfg.clone())?;
    let base = backend.handle.history.storage.snapshot();
    backend
        .handle
        .history
        .apply_storage(
            storage::Config {
                days: 1,
                mib: 16,
                cleanup_minutes: 2,
            },
            base.revision,
        )
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    backend.join();
    let log = std::fs::File::create(dir.path().join("cli.log"))?;
    let mut child = OwnedChild(
        Command::new(env!("CARGO_BIN_EXE_rustleblower"))
            .args(["--headless", "--socket"])
            .arg(&cfg.socket)
            .arg("--database")
            .arg(&cfg.database)
            .args(["--history-days", "2", "--history-mib", "64"])
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()?,
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !cfg.socket.exists() {
            assert!(child.0.try_wait().unwrap().is_none());
            tokio::time::sleep(Duration::from_millis(10)).await
        }
    })
    .await?;
    let mut daemon = client(&cfg.socket).await?;
    daemon.ping(protocol::PingRequest::default()).await?;
    drop(daemon);
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.0.id().to_string()])
            .status()?
            .success()
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await
        }
    })
    .await?;
    assert!(!cfg.socket.exists());
    let backend = Backend::start(cfg)?;
    let current = backend.handle.history.storage.snapshot().config;
    assert_eq!(
        (current.days, current.mib, current.cleanup_minutes),
        (2, 64, 2)
    );
    backend.join();
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_cleanup_expires_data_without_new_ingestion() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = options(dir.path());
    let backend = Backend::start(cfg.clone())?;
    let history = &backend.handle.history;
    let base = history.storage.snapshot();
    history
        .apply_storage(
            storage::Config {
                days: 1,
                cleanup_minutes: 1,
                ..base.config
            },
            base.revision,
        )
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    history.ingest("idle".into(), vec![event(1)], "observed");
    history.flush().await?;
    let db = rusqlite::Connection::open(&cfg.database)?;
    db.execute(
        "UPDATE events SET received=?",
        [history::now_seconds() - 2 * 86400],
    )?;
    assert_eq!(count(&db, "events"), 1);
    // No Ping/events/history query is sent while waiting: only a separate read-only observation.
    tokio::time::timeout(Duration::from_secs(75), async {
        while count(&db, "events") != 0 {
            tokio::time::sleep(Duration::from_millis(100)).await
        }
    })
    .await?;
    drop(db);
    backend.join();
    assert!(!cfg.socket.exists());
    Ok(())
}
#[test]
fn display_settings_migrate_validate_and_merge_independent_controls() -> Result<()> {
    use history::{
        query::Field,
        settings::{Options, Store},
    };
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("history.sqlite3");
    let path = dir.path().join("history.sqlite3.history.json");
    let mut old = serde_json::to_value(Options::default())?;
    old["version"] = 1.into();
    old.as_object_mut().unwrap().remove("refresh_seconds");
    std::fs::write(&path, serde_json::to_vec(&old)?)?;
    let store = Store::open(&database)?;
    assert_eq!(store.snapshot().version, 3);
    assert_eq!(store.snapshot().refresh_seconds, 1);
    let mut stale_columns = store.snapshot();
    stale_columns.columns = vec![Field::Time, Field::User];
    stale_columns.table = true;
    store.save_refresh(0)?;
    store.save_columns(stale_columns)?;
    let reopened = Store::open(&database)?.snapshot();
    assert_eq!(reopened.refresh_seconds, 0);
    assert!(reopened.table);
    assert_eq!(reopened.columns, vec![Field::Time, Field::User]);
    assert!(store.save_refresh(61).is_err());
    assert_eq!(store.snapshot(), reopened);
    let mut future = serde_json::to_value(reopened)?;
    future["version"] = 4.into();
    std::fs::write(&path, serde_json::to_vec(&future)?)?;
    assert!(Store::open(&database).is_err());
    Ok(())
}

#[tokio::test]
async fn schema7_migrates_without_preferences_table_or_history_loss() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = options(dir.path());
    let backend = Backend::start(cfg.clone())?;
    backend
        .handle
        .history
        .ingest("legacy".into(), vec![event(1)], "observed");
    backend.handle.history.flush().await?;
    backend.join();
    let db = rusqlite::Connection::open(&cfg.database)?;
    db.execute_batch("DROP TABLE history_preferences; PRAGMA user_version=7;")?;
    drop(db);
    let reopened = Backend::start(cfg.clone())?;
    assert_eq!(
        reopened.handle.history.storage.snapshot().config,
        storage::Config {
            days: 7,
            mib: 32,
            cleanup_minutes: 5
        }
    );
    reopened.join();
    let db = rusqlite::Connection::open(&cfg.database)?;
    assert_eq!(count(&db, "events"), 1);
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))?,
        11
    );
    Ok(())
}
