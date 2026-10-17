use prost::Message;
use rusqlite::{Connection, params};
use rustleblower::{
    protocol::{Operator, Rule},
    rules::{Query, export_nodes, transfer::Import},
};
use std::sync::atomic::{AtomicBool, Ordering};
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE application_rules(node TEXT,name TEXT,wire BLOB,process TEXT,description TEXT,duration TEXT,PRIMARY KEY(node,name));CREATE TABLE prompt_rules AS SELECT * FROM application_rules WHERE 0; CREATE TABLE rule_observations(node TEXT,name TEXT,received INTEGER,PRIMARY KEY(node,name));CREATE TABLE node_info(name TEXT PRIMARY KEY);").unwrap();
    for node in ["../../alpha", "..__alpha", "offline"] {
        db.execute("INSERT INTO node_info VALUES (?)", [node])
            .unwrap();
    }
    for i in 0..260 {
        put(&db, "application_rules", "../../alpha", rule(i, "allow"));
    }
    put(&db, "application_rules", "..__alpha", rule(0, "deny"));
    put(&db, "prompt_rules", "offline", rule(205, "reject"));
    (dir, path)
}
fn rule(i: usize, action: &str) -> Rule {
    Rule {
        name: format!("app.{i:03}"),
        description: format!("Rule {i}"),
        created: 123456,
        enabled: true,
        precedence: true,
        nolog: true,
        action: action.into(),
        duration: if i == 205 { "30s" } else { "always" }.into(),
        operator: Some(Operator {
            r#type: "simple".into(),
            operand: "process.path".into(),
            data: format!("/bin/app-{i}"),
            sensitive: true,
            ..Default::default()
        }),
    }
}
fn put(db: &Connection, table: &str, node: &str, r: Rule) {
    db.execute(
        &format!("INSERT INTO {table} VALUES (?,?,?,?,?,?)"),
        params![
            node,
            r.name,
            r.encode_to_vec(),
            r.operator.unwrap().data,
            r.description,
            r.duration
        ],
    )
    .unwrap();
}
#[test]
fn frozen_export_preserves_colliding_node_rule_names_all_pages_and_offline_proposals() {
    let (dir, path) = fixture();
    let stop = AtomicBool::new(false);
    let names = export_nodes::catalogue(&path).unwrap();
    assert_eq!(names.len(), 3);
    let review = export_nodes::capture(&path, &names, &Query::default(), &stop).unwrap();
    assert_eq!(review.rules, 262);
    assert_eq!(review.nodes.iter().map(|n| n.awaiting).sum::<usize>(), 1);
    let db = Connection::open(&path).unwrap();
    db.execute("DELETE FROM application_rules", []).unwrap();
    db.execute("DELETE FROM prompt_rules", []).unwrap();
    let (busy, _, _): (i64, i64, i64) = db
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(busy, 0, "Review must not keep a live SQLite reader");
    drop(db);
    let out = export_nodes::publish(&review, dir.path(), &stop).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["format"], "rustleblower-rules-v1");
    assert_eq!(manifest["rules"], 262);
    let mut dirs = std::collections::BTreeSet::new();
    for n in &review.nodes {
        assert!(dirs.insert(n.directory.clone()));
        assert!(!n.directory.contains('/'));
        let mut got = Import::from_path(&out.join(&n.directory)).unwrap().rules;
        got.sort_by(|a, b| a.name.cmp(&b.name));
        let expected = match n.name.as_str() {
            "../../alpha" => (0..260).map(|i| rule(i, "allow")).collect::<Vec<_>>(),
            "..__alpha" => vec![rule(0, "deny")],
            _ => vec![rule(205, "reject")],
        };
        assert_eq!(got, expected);
    }
    let again = export_nodes::publish(&review, dir.path(), &stop).unwrap();
    assert_ne!(out, again);
    assert!(out.join("manifest.json").exists());
}
#[test]
fn captured_filter_applies_to_each_node_and_cancel_never_publishes() {
    let (dir, path) = fixture();
    let names = export_nodes::catalogue(&path).unwrap();
    let stop = AtomicBool::new(false);
    let review = export_nodes::capture(
        &path,
        &names,
        &Query {
            search: "app-205".into(),
            temporary: Some(true),
            ..Default::default()
        },
        &stop,
    )
    .unwrap();
    assert_eq!(review.rules, 2);
    assert_eq!(
        review.nodes.iter().map(|n| n.rules).collect::<Vec<_>>(),
        vec![1, 0, 1]
    );
    stop.store(true, Ordering::Release);
    assert!(export_nodes::publish(&review, dir.path(), &stop).is_err());
    assert!(export_nodes::capture(&path, &names, &Query::default(), &stop).is_err());
    assert!(!std::fs::read_dir(dir.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .contains("rustleblower-nodes")
    }));
    stop.store(false, Ordering::Release);
    let marker = dir.path().join("existing");
    std::fs::write(&marker, b"keep me").unwrap();
    assert!(export_nodes::publish(&review, &marker, &stop).is_err());
    assert_eq!(std::fs::read(marker).unwrap(), b"keep me");
}
#[test]
fn invalid_selection_oversized_blobs_and_changed_rule_identities_are_refused() {
    let (_dir, path) = fixture();
    let stop = AtomicBool::new(false);
    let query = Query::default();
    for names in [
        vec![],
        vec!["offline".into(); 2],
        vec!["missing".into()],
        (0..33).map(|i| format!("node{i}")).collect(),
    ] {
        assert!(export_nodes::capture(&path, &names, &query, &stop).is_err());
    }
    let db = Connection::open(&path).unwrap();
    db.execute(
        "UPDATE application_rules SET wire=zeroblob(65537) WHERE node='..__alpha'",
        [],
    )
    .unwrap();
    assert!(export_nodes::capture(&path, &["..__alpha".into()], &query, &stop).is_err());
    db.execute("UPDATE prompt_rules SET name='different-name'", [])
        .unwrap();
    assert!(export_nodes::capture(&path, &["offline".into()], &query, &stop).is_err());
}

#[test]
fn cached_node_choices_refuse_oversized_identities_and_more_than_32_nodes() {
    let (_dir, path) = fixture();
    let db = Connection::open(&path).unwrap();
    db.execute("INSERT INTO node_info VALUES (?)", ["x".repeat(4096)])
        .unwrap();
    assert!(export_nodes::catalogue(&path).is_err());
    db.execute("DELETE FROM node_info WHERE length(name)>128", [])
        .unwrap();
    for i in 0..30 {
        db.execute("INSERT INTO node_info VALUES (?)", [format!("node-{i}")])
            .unwrap();
    }
    assert!(export_nodes::catalogue(&path).is_err());
}
