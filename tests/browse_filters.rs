use anyhow::Result;
use rustleblower::{
    history::{
        self, Query, Worker,
        query::{Comparison, Field, Filter, Logic, Match},
        settings::{Presentation, Store},
    },
    protocol::{Connection, Event, Rule},
};
use std::sync::atomic::AtomicBool;

#[tokio::test]
async fn history_node_scope_survives_paging_any_conditions_export_and_restart() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("history.sqlite3");
    let worker = Worker::start(&database, 7, 16)?;
    let exact = "offline_%'node";
    let start = history::now_nanos();
    for (node, offset) in [(exact, 0), ("online-node", 1000)] {
        worker.history.ingest(
            node.into(),
            (0..250)
                .map(|n| Event {
                    unixnano: start + offset + n,
                    connection: Some(Connection {
                        process_path: format!("/owned/app-{n}"),
                        dst_host: "owned.test".into(),
                        ..Default::default()
                    }),
                    rule: Some(Rule {
                        action: if n % 2 == 0 { "deny" } else { "allow" }.into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .collect(),
            "owned-browse",
        );
    }
    worker.history.flush().await?;
    assert_eq!(
        history::query::node_choices(&database)?,
        [exact, "online-node"]
    );
    let mut query = Query {
        node: exact.into(),
        ..Default::default()
    };
    let first = worker
        .history
        .query(query.clone())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(first.rows.len(), 200);
    assert!(first.has_more && first.rows.iter().all(|row| row.node == exact));
    query.cursor = first.next_cursor;
    query.ceiling = Some(first.ceiling);
    let second = worker
        .history
        .query(query)
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(second.rows.len(), 50);
    assert!(!second.has_more && second.rows.iter().all(|row| row.node == exact));

    let presentation = Presentation {
        node: exact.into(),
        action: "deny".into(),
        filters: vec![
            Filter {
                field: Field::Node,
                comparison: Comparison::Equals,
                value: "online-node".into(),
            },
            Filter {
                field: Field::Process,
                comparison: Comparison::Contains,
                value: "/owned/".into(),
            },
        ],
        logic: Logic {
            mode: Match::Any,
            ..Default::default()
        },
        ..Default::default()
    };
    Store::open(&database)?.save_presentation(presentation.clone())?;
    let restored = Store::open(&database)?.snapshot().presentation;
    assert_eq!(restored, presentation);
    let query = restored.query();
    let page = worker
        .history
        .query(query.clone())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(page.rows.len(), 125);
    assert!(
        page.rows
            .iter()
            .all(|row| row.node == exact && row.action == "deny")
    );
    let target = dir.path().join("node.jsonl");
    let report = history::export::save(
        &database,
        &query,
        &[Field::Node, Field::Action],
        history::export::Format::JsonLines,
        &target,
        false,
        &AtomicBool::new(false),
    )?;
    assert_eq!(report.rows, 125);
    assert_eq!(std::fs::read_to_string(target)?.lines().count(), 125);
    let all = worker
        .history
        .query(Query::default())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert!(all.rows.iter().any(|row| row.node == "online-node"));
    assert!(
        worker
            .history
            .query(Query {
                node: "n".repeat(129),
                ..Default::default()
            })
            .unwrap()
            .await?
            .is_err()
    );
    // Existing version-three files without the new optional node field remain valid.
    let mut legacy = serde_json::to_value(restored)?;
    legacy.as_object_mut().unwrap().remove("node");
    assert!(
        serde_json::from_value::<Presentation>(legacy)?
            .node
            .is_empty()
    );
    worker.join();
    Ok(())
}
