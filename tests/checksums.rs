use prost::Message;
use rusqlite::params;
use rustleblower::{
    protocol::{Connection, Operator, Rule},
    rules::checksums::{self, Context},
};

fn connection() -> Connection {
    Connection {
        process_path: "/usr/bin/checksum-fixture".into(),
        process_checksums: [
            ("process.hash.md5".into(), "a".repeat(32)),
            ("process.hash.sha1".into(), "b".repeat(40)),
        ]
        .into(),
        ..Default::default()
    }
}
fn rule(index: usize) -> Rule {
    Rule {
        name: format!("checksum.{index:03}"),
        created: 123456,
        description: "Keep metadata and unrelated conditions".into(),
        enabled: true,
        precedence: true,
        nolog: true,
        action: "reject".into(),
        duration: "5m".into(),
        operator: Some(Operator {
            r#type: "list".into(),
            operand: "list".into(),
            sensitive: true,
            list: vec![
                Operator {
                    r#type: "simple".into(),
                    operand: "process.path".into(),
                    data: connection().process_path,
                    sensitive: true,
                    ..Default::default()
                },
                Operator {
                    r#type: "simple".into(),
                    operand: "process.hash.md5".into(),
                    data: "c".repeat(32),
                    ..Default::default()
                },
                Operator {
                    r#type: "simple".into(),
                    operand: "process.hash.sha1".into(),
                    data: "d".repeat(40),
                    ..Default::default()
                },
                Operator {
                    r#type: "regexp".into(),
                    operand: "dest.host".into(),
                    data: "^updates\\.example\\.test$".into(),
                    sensitive: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
    }
}

#[test]
fn checksum_comparison_and_replacement_preserve_rule_intent_and_missing_fields() {
    let con = connection();
    let context = Context::from_connection(&con).unwrap();
    let original = rule(0);
    assert_eq!(checksums::differences(&original, &context).len(), 2);
    let updated = checksums::updated(&original, &context).unwrap();
    assert!(checksums::differences(&updated, &context).is_empty());
    let mut expected = original.clone();
    expected.operator.as_mut().unwrap().list[1].data = "a".repeat(32);
    expected.operator.as_mut().unwrap().list[2].data = "b".repeat(40);
    expected.operator.as_mut().unwrap().data = updated.operator.as_ref().unwrap().data.clone();
    assert_eq!(updated, expected);
    let json: serde_json::Value = serde_json::from_str(&updated.operator.unwrap().data).unwrap();
    assert_eq!(json[1]["data"], "a".repeat(32));
    assert_eq!(json[2]["data"], "b".repeat(40));
    let mut missing = con.clone();
    missing.process_checksums.remove("process.hash.sha1");
    let updated =
        checksums::updated(&original, &Context::from_connection(&missing).unwrap()).unwrap();
    assert_eq!(
        updated.operator.unwrap().list[2].data,
        "d".repeat(40),
        "unreported hashes are never replaced"
    );
    missing
        .process_checksums
        .insert("process.hash.md5".into(), "malformed".into());
    assert!(Context::from_connection(&missing).is_none());
    let mut candidate = original.clone();
    candidate.enabled = false;
    assert!(checksums::differences(&candidate, &context).is_empty());
    candidate = original.clone();
    candidate.operator.as_mut().unwrap().list[0].data = "/unrelated".into();
    assert!(checksums::differences(&candidate, &context).is_empty());
    candidate = original.clone();
    candidate.operator.as_mut().unwrap().list[1].r#type = "regexp".into();
    candidate.operator.as_mut().unwrap().list[2].r#type = "regexp".into();
    assert!(checksums::differences(&candidate, &context).is_empty());
    candidate = original.clone();
    candidate.operator.as_mut().unwrap().list[1].data = "A".repeat(32);
    candidate.operator.as_mut().unwrap().list[2].data = "B".repeat(40);
    assert!(checksums::differences(&candidate, &context).is_empty());
    candidate.operator.as_mut().unwrap().list[1].sensitive = true;
    assert_eq!(checksums::differences(&candidate, &context).len(), 1);
}

fn database() -> (tempfile::TempDir, std::path::PathBuf, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalogue.sqlite3");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE application_rules(node TEXT,name TEXT,process TEXT,wire BLOB,PRIMARY KEY(node,name)); CREATE TABLE prompt_rules(node TEXT,name TEXT,process TEXT,wire BLOB);").unwrap();
    (dir, path, db)
}
fn put(db: &rusqlite::Connection, node: &str, rule: &Rule) {
    db.execute(
        "INSERT OR REPLACE INTO application_rules(node,name,process,wire) VALUES(?,?,?,?)",
        params![
            node,
            rule.name,
            rule.operator.as_ref().unwrap().list[0].data,
            rule.encode_to_vec()
        ],
    )
    .unwrap();
}

#[test]
fn checksum_pages_enforce_count_and_bytes_and_do_not_include_unconfirmed_proposals() {
    let (_dir, path, db) = database();
    let context = Context::from_connection(&connection()).unwrap();
    for index in 0..37 {
        put(&db, "node", &rule(index));
    }
    put(&db, "other-node", &rule(99));
    let proposal = rule(100);
    db.execute(
        "INSERT INTO prompt_rules VALUES(?,?,?,?)",
        params![
            "node",
            proposal.name,
            connection().process_path,
            proposal.encode_to_vec()
        ],
    )
    .unwrap();
    let mut after = String::new();
    let mut names = Vec::new();
    loop {
        let page = checksums::page(&path, "node", &context, &after).unwrap();
        assert!(page.changes.len() <= checksums::PAGE_RULES);
        names.extend(page.changes.iter().map(|change| change.rule.name.clone()));
        if !page.has_more {
            break;
        }
        assert!(page.after > after);
        after = page.after;
    }
    assert_eq!(
        names,
        (0..37).map(|index| rule(index).name).collect::<Vec<_>>()
    );
    db.execute("DELETE FROM application_rules", []).unwrap();
    for index in 0..14 {
        let mut large = rule(index);
        large.description = "x".repeat(48 * 1024);
        put(&db, "node", &large);
    }
    let page = checksums::page(&path, "node", &context, "").unwrap();
    assert!(
        page.has_more && page.changes.len() < checksums::PAGE_RULES && !page.changes.is_empty()
    );
    let next = checksums::page(&path, "node", &context, &page.after).unwrap();
    assert!(!next.changes.is_empty());
}

#[test]
fn checksum_preparation_refuses_stale_deleted_disabled_or_wrong_node_rules() {
    let (_dir, path, db) = database();
    let original = rule(0);
    let context = Context::from_connection(&connection()).unwrap();
    put(&db, "node", &original);
    assert!(checksums::prepare(&path, "node", &original, &context).is_ok());
    let mut edited = original.clone();
    edited.description = "Concurrent user edit".into();
    put(&db, "node", &edited);
    assert!(
        checksums::prepare(&path, "node", &original, &context)
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    assert!(checksums::prepare(&path, "other-node", &original, &context).is_err());
    edited.enabled = false;
    put(&db, "node", &edited);
    assert!(checksums::prepare(&path, "node", &edited, &context).is_err());
    db.execute("DELETE FROM application_rules", []).unwrap();
    assert!(checksums::prepare(&path, "node", &original, &context).is_err());
}
