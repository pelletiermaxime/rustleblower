use prost::Message;
use rusqlite::{Connection, params};
use rustleblower::{
    protocol::{Operator, Rule},
    rules::{
        Query, document,
        transfer::{self, ExportScope, Import},
    },
};

fn fixture(count: usize) -> (tempfile::TempDir, std::path::PathBuf, Vec<Rule>) {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("history.sqlite3");
    let db = Connection::open(&database).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE application_rules(node TEXT,name TEXT,wire BLOB,process TEXT,description TEXT,duration TEXT,PRIMARY KEY(node,name)); CREATE TABLE prompt_rules AS SELECT * FROM application_rules WHERE 0;").unwrap();
    let rules = (0..count)
        .map(|i| Rule {
            name: format!("app.{i:03}"),
            created: 123456,
            enabled: true,
            precedence: true,
            nolog: true,
            action: "allow".into(),
            duration: "always".into(),
            operator: Some(Operator {
                r#type: "simple".into(),
                operand: "process.path".into(),
                data: format!("/bin/application-{i}"),
                sensitive: true,
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    for rule in &rules {
        db.execute(
            "INSERT INTO application_rules VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                "node",
                rule.name,
                rule.encode_to_vec(),
                rule.operator.as_ref().unwrap().data,
                rule.description,
                rule.duration
            ],
        )
        .unwrap();
    }
    (dir, database, rules)
}
fn scope() -> ExportScope {
    ExportScope {
        query: Query {
            node: "node".into(),
            ..Default::default()
        },
        names: None,
    }
}

#[test]
fn export_spans_pages_and_roundtrips_all_policy_fields() {
    let (dir, database, rules) = fixture(260);
    let path = dir.path().join("rules.json");
    assert!(transfer::export_file(&database, &scope(), &database).is_err());
    assert_eq!(
        transfer::export_file(&database, &scope(), &path).unwrap(),
        260
    );
    assert_eq!(Import::from_path(&path).unwrap().rules, rules);
    let (text, count) = transfer::export_text(&database, &scope()).unwrap();
    assert_eq!(count, 260);
    assert_eq!(transfer::load_rules(&database, &scope()).unwrap(), rules);
    assert_eq!(Import::from_text(&text).unwrap().rules, rules);
    let (directory, count) = transfer::export_directory(&database, &scope(), dir.path()).unwrap();
    assert_eq!(count, 260);
    assert_eq!(Import::from_path(&directory).unwrap().rules, rules);
    assert_eq!(
        transfer::existing_names(&database, "node").unwrap().len(),
        260
    );
}

#[test]
fn filtered_selected_export_and_failed_export_preserve_existing_files() {
    let (dir, database, rules) = fixture(260);
    let path = dir.path().join("rules.json");
    let mut scope = scope();
    scope.query.search = "application-205".into();
    assert_eq!(transfer::export_file(&database, &scope, &path).unwrap(), 1);
    assert_eq!(
        Import::from_path(&path).unwrap().rules,
        vec![rules[205].clone()]
    );
    scope.query.search.clear();
    scope.names = Some(std::collections::BTreeSet::from([
        rules[205].name.clone(),
        "missing".into(),
    ]));
    let old = std::fs::read(&path).unwrap();
    assert!(transfer::export_file(&database, &scope, &path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), old);
    assert!(transfer::export_directory(&database, &scope, dir.path()).is_err());
    assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".rustleblower-export-")
    }));
}

#[test]
fn clipboard_accepts_adjacent_opensnitch_objects_and_rejects_duplicate_names() {
    let (_, _, rules) = fixture(2);
    let text = rules
        .iter()
        .map(|rule| document::rule_json(rule).to_string())
        .collect::<Vec<_>>()
        .join("\n\n");
    assert_eq!(Import::from_text(&text).unwrap().rules, rules);
    assert!(
        Import::from_text(&format!("{text}\n{}", document::rule_json(&rules[0])))
            .err()
            .unwrap()
            .to_string()
            .contains("Duplicate")
    );
    assert!(Import::from_text("[]").is_err());
    assert!(Import::from_text(&" ".repeat(document::MAX_DOCUMENT_BYTES + 1)).is_err());
}

#[test]
fn directory_exports_do_not_collide_for_names_with_json_suffixes() {
    let (dir, database, mut rules) = fixture(2);
    let db = Connection::open(&database).unwrap();
    for (i, name) in ["example", "example.json"].iter().enumerate() {
        rules[i].name = (*name).into();
        db.execute(
            "UPDATE application_rules SET name=?1,wire=?2 WHERE name=?3",
            params![name, rules[i].encode_to_vec(), format!("app.{i:03}")],
        )
        .unwrap();
    }
    let (directory, count) = transfer::export_directory(&database, &scope(), dir.path()).unwrap();
    assert_eq!(count, 2);
    assert_eq!(Import::from_path(&directory).unwrap().rules, rules);
    assert!(
        directory.join("example.json").exists() && directory.join("example.json.json").exists()
    );
}

#[test]
fn streaming_import_rejects_oversized_objects_and_rule_counts_before_expansion() {
    assert!(
        Import::from_text(&format!("{{\"ignored\":\"{}\"}}", "x".repeat(512 * 1024)))
            .err()
            .unwrap()
            .to_string()
            .contains("512 KiB")
    );
    let (_, _, rules) = fixture(1);
    let text = (0..10_001)
        .map(|i| {
            let mut rule = rules[0].clone();
            rule.name = format!("rule.{i}");
            document::rule_json(&rule).to_string()
        })
        .collect::<Vec<_>>()
        .join(",");
    let error = Import::from_text(&format!("[{text}]"))
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("10,000"), "{error}");
}
