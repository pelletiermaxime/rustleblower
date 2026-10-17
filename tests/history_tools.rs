use anyhow::Result;
use rustleblower::{
    history::{
        self, Query, Worker,
        query::{Comparison, Field, Filter},
    },
    protocol::{Connection, Event, Rule},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn event(index: usize) -> Event {
    Event {
        unixnano: 1_700_000_000_000_000_000 + index as i64,
        connection: Some(Connection {
            process_path: format!("/bin/app-{:03}", index % 7),
            process_args: vec!["app".into(), "literal_%' \"quote\"\nnext".into()],
            src_ip: "192.0.2.10".into(),
            src_port: 40000 + (index % 5) as u32,
            dst_ip: "198.51.100.20".into(),
            dst_host: format!("host-{}.example.test", index % 3),
            dst_port: 443,
            protocol: "tcp".into(),
            user_id: if index.is_multiple_of(2) { 0 } else { 1001 },
            process_id: 100 + (index % 4) as u32,
            ..Default::default()
        }),
        rule: Some(Rule {
            name: format!("rule-{}", index % 3),
            action: if index.is_multiple_of(3) {
                "reject"
            } else {
                "allow"
            }
            .into(),
            duration: "5m".into(),
            nolog: index.is_multiple_of(2),
            precedence: true,
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn filter(field: Field, comparison: Comparison, value: &str) -> Filter {
    Filter {
        field,
        comparison,
        value: value.into(),
    }
}
async fn page(worker: &Worker, query: Query) -> Result<history::Page> {
    worker
        .history
        .query(query)
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)
}

#[tokio::test]
async fn typed_history_filters_and_sorted_keyset_pages_keep_a_stable_ceiling() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(&dir.path().join("history.sqlite3"), 7, 16)?;
    worker
        .history
        .ingest("node-a".into(), (0..430).map(event).collect(), "observed");
    worker.history.flush().await?;
    let query = Query {
        action: "reject".into(),
        filters: vec![
            filter(Field::User, Comparison::Equals, "1001"),
            filter(Field::Command, Comparison::Contains, "_%' \"quote\""),
            filter(Field::SourceIp, Comparison::Equals, "192.0.2.10"),
            filter(Field::Priority, Comparison::Equals, "1"),
        ],
        ..Default::default()
    };
    let filtered = page(&worker, query).await?;
    assert_eq!(filtered.rows.len(), 72);
    assert!(
        filtered
            .rows
            .iter()
            .all(|row| row.metadata.user == Some(1001) && row.action == "reject")
    );
    assert_eq!(
        page(
            &worker,
            Query {
                filters: vec![
                    filter(Field::User, Comparison::Equals, "0"),
                    filter(Field::Logged, Comparison::Equals, "0")
                ],
                ..Default::default()
            }
        )
        .await?
        .rows
        .len(),
        200
    );
    assert!(
        page(
            &worker,
            Query {
                filters: vec![filter(Field::Command, Comparison::Contains, "' OR 1=1 --")],
                ..Default::default()
            }
        )
        .await?
        .rows
        .is_empty()
    );
    assert!(
        filter(Field::Port, Comparison::Equals, "65536")
            .validate()
            .is_err()
    );
    assert!(
        filter(Field::Time, Comparison::After, "not a time")
            .validate()
            .is_err()
    );
    let mut query = Query {
        sort: Some((Field::Process, true)),
        ..Default::default()
    };
    let first = page(&worker, query.clone()).await?;
    assert!(first.has_more);
    let ceiling = first.ceiling;
    let mut names = first
        .rows
        .iter()
        .map(|row| (row.process.clone(), row.id))
        .collect::<Vec<_>>();
    query.cursor = first.next_cursor;
    query.ceiling = Some(ceiling);
    let mut newest = event(1000);
    newest.connection.as_mut().unwrap().process_path = "/bin/aaa-new".into();
    worker
        .history
        .ingest("node-a".into(), vec![newest], "observed");
    worker.history.flush().await?;
    loop {
        let next = page(&worker, query.clone()).await?;
        names.extend(next.rows.iter().map(|row| (row.process.clone(), row.id)));
        if !next.has_more {
            break;
        }
        query.cursor = next.next_cursor;
    }
    assert_eq!(names.len(), 430);
    assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        names
            .iter()
            .map(|(_, id)| *id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        430
    );
    assert!(names.iter().all(|(_, id)| *id <= ceiling));
    let time = chrono::DateTime::from_timestamp_nanos(event(200).unixnano).to_rfc3339();
    assert_eq!(
        page(
            &worker,
            Query {
                filters: vec![filter(Field::Time, Comparison::Before, &time)],
                ..Default::default()
            }
        )
        .await?
        .rows
        .len(),
        200
    );
    worker.join();
    Ok(())
}

#[tokio::test]
async fn migration_backfills_bounded_details_and_preserves_unknown_legacy_metadata() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("history.sqlite3");
    let worker = Worker::start(&path, 7, 16)?;
    worker
        .history
        .ingest("node".into(), vec![event(0)], "observed");
    worker.history.flush().await?;
    worker.join();
    let db = rusqlite::Connection::open(&path)?;
    db.execute_batch("ALTER TABLE events DROP COLUMN metadata; PRAGMA user_version=3;")?;
    db.execute("INSERT INTO events(node,fingerprint,unixnano,received,action,protocol,process,destination,port,reason,stored_bytes) VALUES('legacy',zeroblob(32),1,?,'deny','tcp','/bin/old','old.example',80,'observed',512)",[history::now_seconds()])?;
    drop(db);
    let worker = Worker::start(&path, 7, 16)?;
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while worker.history.health.indexing.load(Ordering::Acquire) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await?;
    let known = page(
        &worker,
        Query {
            filters: vec![filter(Field::User, Comparison::Equals, "0")],
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(known.rows.len(), 1);
    assert_eq!(known.rows[0].metadata.pid, Some(100));
    let unknown = page(
        &worker,
        Query {
            filters: vec![filter(Field::User, Comparison::Missing, "")],
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(unknown.rows.len(), 1);
    assert_eq!(unknown.rows[0].node, "legacy");
    assert!(Field::Truncated.text(&unknown.rows[0]).is_empty());
    worker.history.flush().await?;
    worker.join();
    let db = rusqlite::Connection::open(&path)?;
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?,
        11
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM events WHERE metadata IS NULL",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        0
    );
    assert_eq!(
        db.query_row("SELECT bytes FROM usage", [], |row| row.get::<_, i64>(0))?,
        db.query_row("SELECT sum(stored_bytes) FROM events", [], |row| row
            .get::<_, i64>(0))?
    );
    Ok(())
}

#[tokio::test]
async fn history_exports_quote_data_use_columns_and_preserve_destination_on_failure() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("history.sqlite3");
    let worker = Worker::start(&path, 7, 16)?;
    worker
        .history
        .ingest("node".into(), (0..430).map(event).collect(), "observed");
    worker.history.flush().await?;
    let stop = AtomicBool::new(false);
    let target = dir.path().join("history.csv");
    let query = Query {
        action: "reject".into(),
        ..Default::default()
    };
    let columns = [Field::Node, Field::Command, Field::User, Field::Rule];
    let report = history::export::save(
        &path,
        &query,
        &columns,
        history::export::Format::Csv,
        &target,
        false,
        &stop,
    )?;
    assert_eq!(report.rows, 144);
    let csv = std::fs::read_to_string(&target)?;
    assert!(csv.starts_with("\"Node\",\"Command line\",\"UID\",\"Matched rule\"\r\n"));
    assert!(csv.contains("\"\"quote\"\"\nnext"));
    let before = std::fs::read(&target)?;
    assert!(
        history::export::save(
            &path,
            &Query::default(),
            &columns,
            history::export::Format::Csv,
            &target,
            false,
            &stop
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&target)?, before);
    stop.store(true, Ordering::Relaxed);
    assert!(
        history::export::save(
            &path,
            &Query::default(),
            &columns,
            history::export::Format::Csv,
            &target,
            true,
            &stop
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&target)?, before);
    stop.store(false, Ordering::Relaxed);
    let json = dir.path().join("history.jsonl");
    let report = history::export::save(
        &path,
        &query,
        &columns,
        history::export::Format::JsonLines,
        &json,
        false,
        &stop,
    )?;
    let text = std::fs::read_to_string(json)?;
    assert_eq!(report.rows, text.lines().count());
    assert!(
        text.lines()
            .all(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["node"] == "node")
    );
    assert!(
        history::export::save(
            &path,
            &query,
            &columns,
            history::export::Format::Csv,
            &path,
            true,
            &stop
        )
        .is_err()
    );
    assert!(
        history::export::save(
            &path,
            &query,
            &columns,
            history::export::Format::Csv,
            std::path::Path::new("/"),
            true,
            &stop
        )
        .is_err()
    );
    let appearance = rustleblower::appearance::Store::new(&path);
    appearance.save(
        &appearance.load()?,
        rustleblower::appearance::ColorMode::Dark,
    )?;
    let saved_appearance = std::fs::read(&appearance.path)?;
    for suffix in [
        "-wal",
        "-shm",
        ".history.json",
        ".preferences.json",
        ".appearance.json",
        ".window.json",
        ".updates.json",
        ".reputation.json",
        ".raw-downloads.json",
        ".highlighting.json",
        ".catalogue-policy.json",
        ".raw-downloads-journal.json",
    ] {
        let reserved = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        assert!(
            history::export::save(
                &path,
                &query,
                &columns,
                history::export::Format::Csv,
                &reserved,
                true,
                &stop
            )
            .is_err()
        );
    }
    assert_eq!(std::fs::read(&appearance.path)?, saved_appearance);
    let options = history::settings::Options {
        table: true,
        columns: columns.to_vec(),
        ..Default::default()
    };
    worker.history.settings.save(options.clone())?;
    assert_eq!(history::settings::Store::open(&path)?.snapshot(), options);
    let result = page(&worker, query).await?;
    let clipboard = history::export::page_csv(&result.rows, &columns)?;
    assert_eq!(clipboard, csv);
    let worker = Arc::new(worker);
    let clone = worker.clone();
    let writer = std::thread::spawn(move || {
        for index in 430..462 {
            clone
                .history
                .ingest("node".into(), vec![event(index)], "observed");
        }
    });
    let concurrent = dir.path().join("concurrent.csv");
    let report = history::export::save(
        &path,
        &Query::default(),
        &[Field::Time],
        history::export::Format::Csv,
        &concurrent,
        false,
        &stop,
    )?;
    writer.join().unwrap();
    assert!((430..=462).contains(&report.rows));
    assert_eq!(
        std::fs::read_to_string(concurrent)?.lines().count(),
        report.rows + 1
    );
    Arc::try_unwrap(worker).ok().unwrap().join();
    Ok(())
}
