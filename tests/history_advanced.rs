use anyhow::Result;
use rustleblower::{
    history::{
        self, Query, Worker,
        query::{Comparison, Field, Filter, Group, Logic, Match},
        settings::{Options, Presentation, Store},
    },
    protocol::{Connection, Event, Rule},
};
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn nested_formula_reorders_bound_values_across_pages_exports_and_saved_preferences()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("history.sqlite3");
    let worker = Worker::start(&database, 7, 16)?;
    worker
        .history
        .ingest("node".into(), (0..430).map(event).collect(), "observed");
    worker.history.flush().await?;
    ready(&worker).await?;
    let query = Query {
        filters: vec![
            filter(Field::Cwd, Comparison::StartsWith, "/work/group1"),
            filter(Field::Host, Comparison::Equals, "host-1.test"),
            filter(Field::Port, Comparison::AtLeast, "1003"),
            filter(Field::User, Comparison::Equals, "1000"),
        ],
        logic: Logic {
            formula: "4 OR (1 AND (3 OR 2))".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let expected = (0..430)
        .filter(|i| i % 4 == 0 || (i % 2 == 1 && (i % 7 >= 3 || i % 3 == 1)))
        .map(|i| 1_700_000_000_000_000_000 + i as i64)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(expected.len() > 200);
    let actual = rows(&worker, query.clone()).await?;
    assert_eq!(
        actual
            .iter()
            .map(|r| r.unixnano)
            .collect::<std::collections::BTreeSet<_>>(),
        expected
    );
    let target = dir.path().join("nested.jsonl");
    let report = history::export::save(
        &database,
        &query,
        &[Field::Cwd, Field::Host, Field::Port, Field::User],
        history::export::Format::JsonLines,
        &target,
        false,
        &AtomicBool::new(false),
    )?;
    assert_eq!(report.rows, expected.len());
    assert_eq!(
        std::fs::read_to_string(target)?.lines().count(),
        expected.len()
    );
    let store = Store::open(&database)?;
    let presentation = Presentation {
        filters: query.filters.clone(),
        logic: query.logic.clone(),
        ..Default::default()
    };
    store.save_presentation(presentation.clone())?;
    assert_eq!(
        Store::open(&database)?.snapshot().presentation,
        presentation
    );
    for formula in ["1 OR 1", "4 OR 5", "4; DELETE FROM events"] {
        let mut invalid = query.clone();
        invalid.logic.formula = formula.into();
        assert!(worker.history.query(invalid).unwrap().await?.is_err());
    }
    assert_eq!(rows(&worker, query).await?.len(), expected.len());
    worker.join();
    Ok(())
}

fn event(index: usize) -> Event {
    Event {
        unixnano: 1_700_000_000_000_000_000 + index as i64,
        connection: Some(Connection {
            process_path: format!("/bin/app-{}", index % 7),
            process_cwd: format!("/work/group{}/literal_%' END", index % 2),
            process_args: vec!["app".into(), format!("--item={index}")],
            dst_host: format!("host-{}.test", index % 3),
            dst_port: 1000 + (index % 7) as u32,
            protocol: "tcp".into(),
            user_id: 1000 + (index % 4) as u32,
            ..Default::default()
        }),
        rule: Some(Rule {
            action: "allow".into(),
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
async fn ready(worker: &Worker) -> Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while worker.history.health.indexing.load(Ordering::Acquire) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert!(
        worker
            .history
            .health
            .metadata_error
            .lock()
            .unwrap()
            .is_none()
    );
    Ok(())
}
async fn rows(worker: &Worker, mut query: Query) -> Result<Vec<history::Row>> {
    let mut result = Vec::new();
    loop {
        let page = worker
            .history
            .query(query.clone())
            .unwrap()
            .await?
            .map_err(anyhow::Error::msg)?;
        query.ceiling = Some(page.ceiling);
        query.cursor = page.next_cursor;
        result.extend(page.rows.iter().cloned());
        if !page.has_more {
            return Ok(result);
        }
    }
}
#[tokio::test]
async fn literal_text_operators_inclusive_numbers_and_times_use_bound_values() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(&dir.path().join("db"), 7, 16)?;
    worker
        .history
        .ingest("node".into(), (0..8).map(event).collect(), "observed");
    worker.history.flush().await?;
    ready(&worker).await?;
    for (field, comparison, value, count) in [
        (Field::Cwd, Comparison::StartsWith, "/WORK/GROUP1", 4),
        (Field::Cwd, Comparison::EndsWith, "_%' end", 8),
        (Field::Cwd, Comparison::NotContains, "group0", 4),
        (Field::Host, Comparison::Contains, "%", 0),
        (Field::Host, Comparison::NotContains, "%", 8),
        (Field::Host, Comparison::StartsWith, "%", 0),
        (Field::Host, Comparison::EndsWith, "%", 0),
        (Field::Cwd, Comparison::Contains, "' OR 1=1 --", 0),
        (
            Field::Cwd,
            Comparison::StartsWith,
            "'); DROP TABLE events; --",
            0,
        ),
        (Field::Port, Comparison::AtMost, "1003", 5),
        (Field::Port, Comparison::AtLeast, "1003", 4),
        (Field::Time, Comparison::AtMost, "1700000000000000003", 4),
        (
            Field::Time,
            Comparison::AtLeast,
            "2023-11-14T22:13:20.000000003Z",
            5,
        ),
    ] {
        let result = rows(
            &worker,
            Query {
                filters: vec![filter(field, comparison, value)],
                ..Default::default()
            },
        )
        .await?;
        assert_eq!(result.len(), count, "{field:?} {comparison:?} {value}");
    }
    assert!(
        filter(Field::Port, Comparison::AtLeast, "65536")
            .validate()
            .is_err()
    );
    assert!(
        filter(Field::Time, Comparison::AtMost, "bad time")
            .validate()
            .is_err()
    );
    assert!(
        filter(Field::Cwd, Comparison::Contains, &"x".repeat(257))
            .validate()
            .is_err()
    );
    assert_eq!(rows(&worker, Query::default()).await?.len(), 8);
    worker.join();
    Ok(())
}
#[tokio::test]
async fn two_level_groups_match_whole_sorted_pages_and_all_match_exports() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("db");
    let worker = Worker::start(&database, 7, 16)?;
    worker
        .history
        .ingest("node".into(), (0..430).map(event).collect(), "observed");
    worker.history.flush().await?;
    ready(&worker).await?;
    let query = Query {
        filters: vec![
            filter(Field::User, Comparison::Equals, "1001"),
            filter(Field::Cwd, Comparison::StartsWith, "/work/group1"),
            filter(Field::Port, Comparison::AtLeast, "1001"),
            filter(Field::Host, Comparison::EndsWith, ".test"),
        ],
        logic: Logic {
            formula: String::new(),
            mode: Match::Any,
            groups: vec![
                Group {
                    count: 2,
                    mode: Match::All,
                },
                Group {
                    count: 2,
                    mode: Match::All,
                },
            ],
        },
        sort: Some((Field::Cwd, true)),
        ..Default::default()
    };
    let expected = (0..430).filter(|i| i % 4 == 1 || i % 7 >= 1).count();
    let matching = rows(&worker, query.clone()).await?;
    assert_eq!(matching.len(), expected);
    assert!(matching.len() > 200);
    assert!(
        matching
            .windows(2)
            .all(|pair| (&pair[0].metadata.cwd, pair[0].id) < (&pair[1].metadata.cwd, pair[1].id))
    );
    let target = dir.path().join("grouped.jsonl");
    let report = history::export::save(
        &database,
        &query,
        &[Field::Cwd, Field::User, Field::Port],
        history::export::Format::JsonLines,
        &target,
        false,
        &AtomicBool::new(false),
    )?;
    assert_eq!(report.rows, expected);
    let text = std::fs::read_to_string(target)?;
    assert_eq!(text.lines().count(), expected);
    assert!(text.lines().all(|line| {
        serde_json::from_str::<serde_json::Value>(line).unwrap()["cwd"]
            .as_str()
            .unwrap()
            .starts_with("/work/")
    }));
    let first = worker
        .history
        .query(query.clone())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert!(first.has_more);
    let mut frozen = query.clone();
    frozen.ceiling = Some(first.ceiling);
    frozen.cursor = first.next_cursor;
    worker
        .history
        .ingest("node".into(), vec![event(10000)], "observed");
    worker.history.flush().await?;
    let mut ids = first.rows.iter().map(|r| r.id).collect::<Vec<_>>();
    ids.extend(rows(&worker, frozen).await?.iter().map(|r| r.id));
    assert_eq!(ids.len(), expected);
    assert!(ids.iter().all(|id| *id <= first.ceiling));
    assert_eq!(
        ids.iter().collect::<std::collections::BTreeSet<_>>().len(),
        expected
    );
    let and_of_or = Query {
        filters: query.filters.clone(),
        logic: Logic {
            formula: String::new(),
            mode: Match::All,
            groups: vec![
                Group {
                    count: 2,
                    mode: Match::Any,
                },
                Group {
                    count: 2,
                    mode: Match::Any,
                },
            ],
        },
        ..Default::default()
    };
    assert_eq!(rows(&worker, and_of_or).await?.len(), 215);
    worker.join();
    Ok(())
}
#[test]
fn bounded_groups_survive_legacy_settings_and_reject_expression_injection() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("db");
    let path = dir.path().join("db.history.json");
    let mut legacy = serde_json::to_value(Options::default())?;
    legacy["version"] = 1.into();
    legacy["presentation"]
        .as_object_mut()
        .unwrap()
        .remove("logic");
    std::fs::write(&path, serde_json::to_vec(&legacy)?)?;
    let store = Store::open(&database)?;
    assert_eq!(store.snapshot().presentation.logic, Logic::default());
    let mut value = Presentation {
        filters: vec![
            filter(Field::Cwd, Comparison::StartsWith, "/work"),
            filter(Field::Port, Comparison::AtMost, "443"),
            filter(Field::User, Comparison::Equals, "1000"),
        ],
        logic: Logic {
            formula: String::new(),
            mode: Match::Any,
            groups: vec![
                Group {
                    count: 2,
                    mode: Match::All,
                },
                Group {
                    count: 1,
                    mode: Match::Any,
                },
            ],
        },
        ..Default::default()
    };
    store.save_presentation(value.clone())?;
    let reopened = Store::open(&database)?.snapshot().presentation;
    assert_eq!(reopened, value);
    assert_eq!(reopened.query().logic, value.logic);
    for groups in [
        vec![Group {
            count: 0,
            mode: Match::All,
        }],
        vec![Group {
            count: 2,
            mode: Match::All,
        }],
        vec![Group {
            count: 4,
            mode: Match::All,
        }],
        vec![
            Group {
                count: 1,
                mode: Match::All
            };
            9
        ],
    ] {
        value.logic.groups = groups;
        assert!(store.save_presentation(value.clone()).is_err());
    }
    assert_eq!(store.snapshot().presentation, reopened);
    assert!(
        serde_json::from_str::<Logic>(r#"{"mode":"OR 1=1; DROP TABLE events;--","groups":[]}"#)
            .is_err()
    );
    assert!(serde_json::from_str::<Logic>(r#"{"sql":"true","groups":[]}"#).is_err());
    let mut logic = Logic::default();
    logic.append(false, 0);
    logic.append(true, 1);
    logic.append(false, 2);
    assert_eq!(
        logic.groups.iter().map(|g| g.count).collect::<Vec<_>>(),
        [1, 2]
    );
    logic.remove(0, 3);
    assert_eq!(logic.groups[0].count, 2);
    logic.validate(2)?;
    logic.remove(0, 2);
    logic.remove(0, 1);
    assert!(logic.groups.is_empty());
    Ok(())
}
#[tokio::test]
async fn already_indexed_metadata_backfills_cwd_resumes_and_preserves_legacy_unknowns() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("db");
    let worker = Worker::start(&path, 7, 16)?;
    worker
        .history
        .ingest("node".into(), (0..70).map(event).collect(), "observed");
    worker.history.flush().await?;
    worker.join();
    let db = rusqlite::Connection::open(&path)?;
    // First32 represent a completed chunk from an interrupted migration.
    db.execute(
        "UPDATE events SET metadata=json_set(metadata,'$.cwd','/already-upgraded') WHERE id<=32",
        [],
    )?;
    db.execute(
        "UPDATE events SET metadata=json_remove(metadata,'$.schema','$.cwd') WHERE id>32",
        [],
    )?;
    db.execute("UPDATE events SET metadata=json_set(metadata,'$.command','preserve indexed command','$.unknown_extension','keep') WHERE id=33",[])?;
    db.execute("DELETE FROM event_details WHERE event_id=70", [])?;
    db.execute("UPDATE events SET metadata=NULL WHERE id=70", [])?;
    drop(db);
    let worker = Worker::start(&path, 7, 16)?;
    ready(&worker).await?;
    let upgraded = rows(
        &worker,
        Query {
            filters: vec![filter(
                Field::Cwd,
                Comparison::StartsWith,
                "/already-upgraded",
            )],
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(upgraded.len(), 32);
    let recovered = rows(
        &worker,
        Query {
            filters: vec![filter(Field::Cwd, Comparison::StartsWith, "/work")],
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(recovered.len(), 37);
    let unknown = rows(
        &worker,
        Query {
            filters: vec![filter(Field::Cwd, Comparison::Missing, "")],
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(unknown.len(), 1);
    assert!(!unknown[0].metadata.available);
    let not_contains = rows(
        &worker,
        Query {
            filters: vec![filter(Field::Cwd, Comparison::NotContains, "/unrelated")],
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(not_contains.len(), 69);
    worker.history.flush().await?;
    worker.join();
    let db = rusqlite::Connection::open(&path)?;
    let json: String = db.query_row("SELECT metadata FROM events WHERE id=33", [], |r| r.get(0))?;
    let value: serde_json::Value = serde_json::from_str(&json)?;
    assert_eq!(value["command"], "preserve indexed command");
    assert_eq!(value["unknown_extension"], "keep");
    assert!(value["cwd"].as_str().unwrap().starts_with("/work"));
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        11
    );
    assert_eq!(
        db.query_row("SELECT bytes FROM usage", [], |r| r.get::<_, i64>(0))?,
        db.query_row("SELECT sum(stored_bytes) FROM events", [], |r| r
            .get::<_, i64>(0))?
    );
    let before = db.query_row("SELECT bytes FROM usage", [], |r| r.get::<_, i64>(0))?;
    drop(db);
    let worker = Worker::start(&path, 7, 16)?;
    ready(&worker).await?;
    worker.join();
    let db = rusqlite::Connection::open(path)?;
    assert_eq!(
        db.query_row("SELECT bytes FROM usage", [], |r| r.get::<_, i64>(0))?,
        before
    );
    Ok(())
}
