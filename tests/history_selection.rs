use anyhow::Result;
use rustleblower::{
    history::{
        self, Query, Row, Worker,
        export::{self, Format},
        query::{FIELDS, Field},
        selection::{MAX_OVERRIDES, Selection},
    },
    protocol::{Connection, Event, Rule},
};
use std::sync::atomic::AtomicBool;

fn event(index: usize, large: bool) -> Event {
    Event {
        unixnano: history::now_nanos() + index as i64,
        connection: Some(Connection {
            process_path: if large {
                format!("/{}", "x".repeat(4095))
            } else {
                format!("/bin/app-{:02}", index % 17)
            },
            process_cwd: if index.is_multiple_of(3) {
                String::new()
            } else {
                format!("/work/literal_%'{}", index % 5)
            },
            process_args: vec![format!("literal_%'quoted,\\\"\n{index}")],
            src_ip: format!("192.0.2.{}", index % 200),
            dst_host: format!("host-{}.example", index % 7),
            dst_ip: "198.51.100.1".into(),
            src_port: (index % 11) as u32,
            dst_port: (index % 13) as u32,
            user_id: (index % 3) as u32,
            process_id: (index % 19) as u32,
            protocol: if index.is_multiple_of(2) {
                "tcp"
            } else {
                "udp"
            }
            .into(),
            ..Default::default()
        }),
        rule: Some(Rule {
            name: format!("rule-{}", index % 5),
            action: if index.is_multiple_of(2) {
                "allow"
            } else {
                "deny"
            }
            .into(),
            duration: "always".into(),
            nolog: index.is_multiple_of(2),
            precedence: index.is_multiple_of(3),
            ..Default::default()
        }),
        ..Default::default()
    }
}
async fn fixture(count: usize, large: bool) -> Result<(tempfile::TempDir, Worker)> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(&dir.path().join("history.sqlite3"), 7, 64)?;
    for start in (0..count).step_by(32) {
        worker.history.ingest(
            "owned-node".into(),
            (start..(start + 32).min(count))
                .map(|index| event(index, large))
                .collect(),
            "observed",
        );
        worker.history.flush().await?;
    }
    assert_eq!(
        worker
            .history
            .health
            .dropped
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    Ok((dir, worker))
}
async fn all(worker: &Worker, mut query: Query) -> Result<(i64, Vec<Row>)> {
    let mut rows = vec![];
    let mut ceiling = 0;
    loop {
        let page = worker
            .history
            .query(query.clone())
            .unwrap()
            .await?
            .map_err(anyhow::Error::msg)?;
        ceiling = ceiling.max(page.ceiling);
        rows.extend(page.rows.iter().cloned());
        if !page.has_more {
            break;
        }
        query.cursor = page.next_cursor;
        query.ceiling = Some(ceiling);
    }
    Ok((ceiling, rows))
}
fn times(text: &str) -> Result<Vec<String>> {
    text.lines()
        .map(|line| {
            Ok(serde_json::from_str::<serde_json::Value>(line)?["time"]
                .as_str()
                .unwrap()
                .to_owned())
        })
        .collect()
}

#[tokio::test]
async fn ranges_follow_every_sql_sort_with_nulls_ties_reverse_directions_and_cross_page_overrides()
-> Result<()> {
    let (dir, worker) = fixture(550, false).await?;
    let database = dir.path().join("history.sqlite3");
    let db = rusqlite::Connection::open(&database)?;
    db.execute("UPDATE events SET metadata=json_set(metadata,'$.user',NULL,'$.pid',NULL,'$.source_port',NULL,'$.logged',NULL,'$.priority',NULL,'$.available',json('false')) WHERE id%4=0",[])?;
    for &field in FIELDS {
        for ascending in [false, true] {
            let query = Query {
                sort: Some((field, ascending)),
                ..Default::default()
            };
            let (ceiling, rows) = all(&worker, query.clone()).await?;
            assert_eq!(rows.len(), 550);
            let mut selection = Selection::capture(query, ceiling)?;
            selection.toggle(&rows[10])?;
            selection.extend_to(&rows[430])?;
            selection.toggle(&rows[250])?;
            selection.toggle(&rows[540])?;
            let expected = rows
                .iter()
                .enumerate()
                .filter(|(index, _)| ((10..=430).contains(index) && *index != 250) || *index == 540)
                .map(|(_, row)| row)
                .collect::<Vec<_>>();
            assert_eq!(
                rows.iter()
                    .filter(|row| selection.contains(row))
                    .map(|row| row.id)
                    .collect::<Vec<_>>(),
                expected.iter().map(|row| row.id).collect::<Vec<_>>(),
                "{field:?} {ascending}"
            );
            let (text, report) = export::copy_selection(
                &database,
                &selection,
                &[Field::Time],
                Format::JsonLines,
                &AtomicBool::new(false),
            )?;
            assert_eq!(report.rows, expected.len());
            assert_eq!(
                times(&text)?,
                expected
                    .iter()
                    .map(|row| Field::Time.text(row))
                    .collect::<Vec<_>>(),
                "SQL {field:?} {ascending}"
            );
            // Reverse endpoint order and shrinking Shift ranges preserve the anchor.
            let mut reverse = Selection::capture(selection.query().clone(), ceiling)?;
            reverse.toggle(&rows[430])?;
            reverse.extend_to(&rows[10])?;
            reverse.extend_to(&rows[400])?;
            let (text, report) = export::copy_selection(
                &database,
                &reverse,
                &[Field::Time],
                Format::JsonLines,
                &AtomicBool::new(false),
            )?;
            assert_eq!(report.rows, 31);
            assert_eq!(
                times(&text)?,
                rows[400..=430]
                    .iter()
                    .map(|row| Field::Time.text(row))
                    .collect::<Vec<_>>()
            );
        }
    }
    worker.join();
    Ok(())
}
#[tokio::test]
async fn all_matching_freezes_filter_ceiling_and_columns_without_materializing_ids() -> Result<()> {
    let (dir, worker) = fixture(550, false).await?;
    let database = dir.path().join("history.sqlite3");
    let query = Query {
        action: "allow".into(),
        sort: Some((Field::Process, true)),
        ..Default::default()
    };
    let (ceiling, rows) = all(&worker, query.clone()).await?;
    let mut selection = Selection::capture(query, ceiling)?;
    selection.select_all();
    assert_eq!(selection.override_count(), 0);
    assert!(selection.all_matching());
    selection.toggle(&rows[20])?;
    selection.toggle(&rows[220])?;
    worker
        .history
        .ingest("owned-node".into(), vec![event(9000, false)], "observed");
    worker.history.flush().await?;
    let expected = rows
        .iter()
        .enumerate()
        .filter(|(index, _)| !matches!(index, 20 | 220))
        .map(|(_, row)| row)
        .collect::<Vec<_>>();
    let columns = [Field::Time, Field::Process, Field::Command];
    let (text, report) = export::copy_selection(
        &database,
        &selection,
        &columns,
        Format::JsonLines,
        &AtomicBool::new(false),
    )?;
    assert_eq!(report.rows, expected.len());
    assert_eq!(
        times(&text)?,
        expected
            .iter()
            .map(|row| Field::Time.text(row))
            .collect::<Vec<_>>()
    );
    assert!(text.lines().all(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap()
            .as_object()
            .unwrap()
            .len()
            == 3
    }));
    let target = dir.path().join("selected.csv");
    let report = export::save_selection(
        &database,
        &selection,
        &columns,
        Format::Csv,
        &target,
        false,
        &AtomicBool::new(false),
    )?;
    assert_eq!(report.rows, expected.len());
    let bytes = std::fs::read(&target)?;
    assert!(
        String::from_utf8_lossy(&bytes)
            .starts_with("\"Time (UTC)\",\"Executable\",\"Command line\"\r\n")
    );
    assert!(
        export::save_selection(
            &database,
            &selection,
            &columns,
            Format::Csv,
            &target,
            false,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&target)?, bytes);
    assert!(
        export::save_selection(
            &database,
            &selection,
            &columns,
            Format::Csv,
            &target,
            true,
            &AtomicBool::new(true)
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&target)?, bytes);
    selection.clear();
    assert!(selection.is_empty());
    assert!(
        export::copy_selection(
            &database,
            &selection,
            &columns,
            Format::Csv,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    worker.join();
    Ok(())
}
fn row(id: i64) -> Row {
    Row {
        id,
        unixnano: id,
        action: "allow".into(),
        protocol: "tcp".into(),
        process: "/bin/owned".into(),
        destination: String::new(),
        port: 0,
        reason: "observed".into(),
        node: "owned".into(),
        metadata: Default::default(),
    }
}
#[test]
fn count_and_byte_bounds_refuse_atomically_and_future_rows_cannot_join_a_selection() -> Result<()> {
    let mut selection = Selection::capture(Query::default(), 10_000)?;
    for id in 1..=MAX_OVERRIDES {
        selection.toggle(&row(id as i64))?;
    }
    assert_eq!(selection.override_count(), MAX_OVERRIDES);
    assert!(selection.toggle(&row(MAX_OVERRIDES as i64 + 1)).is_err());
    assert!(!selection.contains(&row(MAX_OVERRIDES as i64 + 1)));
    assert!(selection.toggle(&row(10_001)).is_err());
    let mut selection = Selection::capture(
        Query {
            sort: Some((Field::Command, true)),
            ..Default::default()
        },
        10_000,
    )?;
    let mut accepted = 0;
    for id in 1..100 {
        let mut row = row(id);
        row.metadata.command = "x".repeat(8192);
        if selection.toggle(&row).is_err() {
            assert!(!selection.contains(&row));
            break;
        }
        accepted += 1;
    }
    assert!((1..=16).contains(&accepted));
    assert_eq!(selection.override_count(), accepted);
    assert!(Selection::capture(Query::default(), -1).is_err());
    Ok(())
}
#[tokio::test]
async fn expired_explicit_rows_refuse_publication_and_large_clipboard_uses_bounded_file_export()
-> Result<()> {
    let (dir, worker) = fixture(1100, true).await?;
    let database = dir.path().join("history.sqlite3");
    let (ceiling, rows) = all(&worker, Query::default()).await?;
    assert_eq!(rows.len(), 1100);
    let mut selection = Selection::capture(Query::default(), ceiling)?;
    selection.select_all();
    assert!(
        export::copy_selection(
            &database,
            &selection,
            &[Field::Process],
            Format::Csv,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    let target = dir.path().join("large.csv");
    let report = export::save_selection(
        &database,
        &selection,
        &[Field::Process],
        Format::Csv,
        &target,
        false,
        &AtomicBool::new(false),
    )?;
    assert_eq!(report.rows, 1100);
    assert!(report.bytes > 4 * 1024 * 1024);
    let mut explicit = Selection::capture(Query::default(), ceiling)?;
    explicit.toggle(&rows[0])?;
    let db = rusqlite::Connection::open(&database)?;
    db.execute_batch("PRAGMA foreign_keys=ON;")?;
    db.execute("DELETE FROM events WHERE id=?", [rows[0].id])?;
    let before = std::fs::read(&target)?;
    assert!(
        export::save_selection(
            &database,
            &explicit,
            &[Field::Process],
            Format::Csv,
            &target,
            true,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&target)?, before);
    assert!(
        export::copy_selection(
            &database,
            &explicit,
            &[Field::Process],
            Format::Csv,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    worker.join();
    Ok(())
}
