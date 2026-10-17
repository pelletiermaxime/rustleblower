use anyhow::Result;
use prost::Message;
use rusqlite::{Connection, params};
use rustleblower::{
    history::{
        Worker,
        query::{Comparison, Group, Logic, Match},
    },
    protocol::{Operator, Rule},
    rules::{
        self, Query,
        query::{Field, Filter, Predicate},
        transfer::{self, ExportScope},
    },
};
use std::{collections::BTreeSet, path::PathBuf, sync::atomic::AtomicBool};
#[tokio::test]
async fn nested_formula_has_one_predicate_for_catalogue_selected_and_full_export() -> Result<()> {
    let (_dir, database) = fixture(650)?;
    let query = Query {
        node: "alpha".into(),
        filters: vec![
            filter(Field::Created, Comparison::AtLeast, "1700000100"),
            filter(Field::Action, Comparison::Equals, "allow"),
            filter(Field::Enabled, Comparison::Equals, "1"),
            filter(Field::Nolog, Comparison::Equals, "1"),
        ],
        logic: Logic {
            formula: "4 OR (1 AND (3 OR 2))".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let expected = (0..650)
        .filter(|i| i % 4 == 0 || (i >= &100 && (i % 3 != 0 || i % 2 == 0)))
        .map(rule)
        .collect::<Vec<_>>();
    assert!(expected.len() > 200);
    let worker = Worker::start(&database, 7, 16)?;
    assert_eq!(all(&worker, query.clone()).await?, expected);
    let (text, count) = transfer::export_text(
        &database,
        &ExportScope {
            query: query.clone(),
            names: None,
        },
    )?;
    assert_eq!(count, expected.len());
    assert_eq!(
        serde_json::from_str::<Vec<serde_json::Value>>(&text)?.len(),
        expected.len()
    );
    let selected = transfer::load_rules(
        &database,
        &ExportScope {
            query: query.clone(),
            names: Some(BTreeSet::from(["rule-0106".into(), "rule-0200".into()])),
        },
    )?;
    assert_eq!(selected, vec![rule(106), rule(200)]);
    assert!(
        transfer::load_rules(
            &database,
            &ExportScope {
                query: query.clone(),
                names: Some(BTreeSet::from(["rule-0105".into()]))
            }
        )
        .is_err()
    );
    for formula in ["1 OR 1", "4 OR 5", "4; DELETE FROM application_rules"] {
        let mut invalid = query.clone();
        invalid.logic.formula = formula.into();
        assert!(Predicate::new(&invalid).is_err());
    }
    assert_eq!(all(&worker, query).await?, expected);
    worker.join();
    Ok(())
}

fn rule(index: usize) -> Rule {
    let future = index.is_multiple_of(5);
    Rule {
        created: 1_700_000_000 + index as i64,
        name: format!("rule-{index:04}"),
        description: format!("literal_%' row {index}"),
        enabled: !index.is_multiple_of(3),
        precedence: index.is_multiple_of(7),
        nolog: index.is_multiple_of(4),
        action: if index.is_multiple_of(2) {
            "allow"
        } else {
            "deny"
        }
        .into(),
        duration: if index.is_multiple_of(2) {
            "always"
        } else {
            "5m"
        }
        .into(),
        operator: Some(Operator {
            r#type: if future { "future-op" } else { "simple" }.into(),
            operand: if future {
                "future.selector"
            } else {
                "process.path"
            }
            .into(),
            data: if future {
                format!("opaque {index} literal_%'")
            } else {
                format!("/bin/app-{index}")
            },
            sensitive: true,
            ..Default::default()
        }),
    }
}
fn filter(field: Field, comparison: Comparison, value: &str) -> Filter {
    Filter {
        field,
        comparison,
        value: value.into(),
    }
}
fn put(db: &Connection, node: &str, rule: &Rule) -> Result<Vec<u8>> {
    let mut wire = rule.encode_to_vec();
    // Unknown protobuf field100 remains cached byte-for-byte through reads.
    wire.extend_from_slice(&[0xa0, 0x06, 0x07]);
    db.execute("INSERT INTO application_rules(node,name,wire,process,description,duration) VALUES(?,?,?,?,?,?)",params![node,rule.name,wire,rules::executable(rule).unwrap_or(""),rule.description,rule.duration])?;
    Ok(wire)
}
fn fixture(count: usize) -> Result<(tempfile::TempDir, PathBuf)> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("history.sqlite3");
    Worker::start(&database, 7, 16)?.join();
    let db = Connection::open(&database)?;
    for node in ["alpha", "beta"] {
        for index in 0..count {
            put(&db, node, &rule(index))?;
        }
    }
    Ok((dir, database))
}
#[test]
fn typed_fields_literal_operators_created_and_boolean_boundaries_are_source_backed() -> Result<()> {
    for (field, comparison, value, count) in [
        (Field::Created, Comparison::AtLeast, "1700000100", 550),
        (
            Field::Created,
            Comparison::AtMost,
            "2023-11-14T22:15:00Z",
            101,
        ),
        (Field::Enabled, Comparison::Equals, "1", 433),
        (Field::Nolog, Comparison::Equals, "1", 163),
        (Field::Priority, Comparison::NotEquals, "1", 557),
        (Field::Action, Comparison::NotContains, "deny", 325),
        (Field::Description, Comparison::StartsWith, "LITERAL_%", 650),
        (Field::Description, Comparison::StartsWith, "literal%", 0),
        (Field::Data, Comparison::EndsWith, "_%'", 130),
        (Field::OperatorType, Comparison::Equals, "FUTURE-OP", 130),
        (Field::Operand, Comparison::Equals, "future.selector", 130),
        (Field::Process, Comparison::StartsWith, "/bin/app", 520),
        (Field::Node, Comparison::Equals, "beta", 0),
        (Field::Name, Comparison::Contains, "' OR 1=1 --", 0),
        (Field::Name, Comparison::NotContains, "%", 650),
    ] {
        let predicate = Predicate::new(&Query {
            filters: vec![filter(field, comparison, value)],
            ..Default::default()
        })?;
        assert_eq!(
            (0..650)
                .filter(|i| predicate.matches("alpha", &rule(*i)))
                .count(),
            count,
            "{field:?} {comparison:?} {value}"
        );
    }
    let missing = Predicate::new(&Query {
        filters: vec![filter(Field::OperatorType, Comparison::Missing, "")],
        ..Default::default()
    })?;
    assert!(missing.matches("alpha", &Rule::default()));
    for value in ["-1", "2", "true"] {
        assert!(
            filter(Field::Enabled, Comparison::Equals, value)
                .validate()
                .is_err()
        );
    }
    assert!(
        filter(
            Field::Created,
            Comparison::AtLeast,
            "2023-11-14T22:13:20.001Z"
        )
        .validate()
        .is_err()
    );
    assert!(
        filter(Field::Data, Comparison::Contains, &"é".repeat(129))
            .validate()
            .is_err()
    );
    assert!(
        Predicate::new(&Query {
            search: "x".repeat(257),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        Predicate::new(&Query {
            filters: vec![filter(Field::Enabled, Comparison::Equals, "1"); 9],
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        Predicate::new(&Query {
            filters: vec![filter(Field::Enabled, Comparison::Equals, "1")],
            logic: Logic {
                formula: String::new(),
                mode: Match::Any,
                groups: vec![Group {
                    count: 2,
                    mode: Match::All
                }]
            },
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        serde_json::from_str::<Filter>(
            r#"{"field":"name; DROP TABLE application_rules","comparison":"contains","value":"x"}"#
        )
        .is_err()
    );
    Ok(())
}
fn grouped() -> Query {
    Query {
        node: "alpha".into(),
        filters: vec![
            filter(Field::Created, Comparison::AtLeast, "1700000100"),
            filter(Field::Action, Comparison::Equals, "allow"),
            filter(Field::Enabled, Comparison::Equals, "1"),
        ],
        logic: Logic {
            formula: String::new(),
            mode: Match::All,
            groups: vec![
                Group {
                    count: 1,
                    mode: Match::All,
                },
                Group {
                    count: 2,
                    mode: Match::Any,
                },
            ],
        },
        ..Default::default()
    }
}
async fn all(worker: &Worker, mut query: Query) -> Result<Vec<Rule>> {
    let mut rules = Vec::new();
    loop {
        let page = worker
            .history
            .query_rules(query.clone())
            .unwrap()
            .await?
            .map_err(anyhow::Error::msg)?;
        rules.extend(page.rules);
        if !page.has_more {
            return Ok(rules);
        }
        query.after = rules.last().unwrap().name.clone();
    }
}
#[tokio::test]
async fn catalogue_selected_full_filter_and_unknown_wire_share_one_predicate() -> Result<()> {
    let (_dir, database) = fixture(650)?;
    let db = Connection::open(&database)?;
    let original: Vec<u8> = db.query_row(
        "SELECT wire FROM application_rules WHERE node='alpha' AND name='rule-0105'",
        [],
        |r| r.get(0),
    )?;
    drop(db);
    let expected = (100..650)
        .filter(|i| i % 2 == 0 || i % 3 != 0)
        .map(rule)
        .collect::<Vec<_>>();
    let worker = Worker::start(&database, 7, 16)?;
    assert_eq!(all(&worker, grouped()).await?, expected);
    let scope = ExportScope {
        query: grouped(),
        names: None,
    };
    let (text, count) = transfer::export_text(&database, &scope)?;
    assert_eq!(count, expected.len());
    let documents: Vec<serde_json::Value> = serde_json::from_str(&text)?;
    assert_eq!(
        documents,
        expected
            .iter()
            .map(rules::document::rule_json)
            .collect::<Vec<_>>()
    );
    let selected = BTreeSet::from(["rule-0105".into(), "rule-0200".into()]);
    //105 does not meet the expression; selected operations must refuse mismatch.
    assert!(
        transfer::load_rules(
            &database,
            &ExportScope {
                query: grouped(),
                names: Some(selected)
            }
        )
        .is_err()
    );
    let selected = BTreeSet::from(["rule-0106".into(), "rule-0200".into()]);
    assert_eq!(
        transfer::load_rules(
            &database,
            &ExportScope {
                query: grouped(),
                names: Some(selected)
            }
        )?,
        vec![rule(106), rule(200)]
    );
    worker.join();
    let db = Connection::open(&database)?;
    assert_eq!(
        db.query_row(
            "SELECT wire FROM application_rules WHERE node='alpha' AND name='rule-0105'",
            [],
            |r| r.get::<_, Vec<u8>>(0)
        )?,
        original
    );
    Ok(())
}
#[test]
fn multi_node_capture_uses_actual_node_and_retains_immutable_filtered_documents() -> Result<()> {
    let (dir, database) = fixture(260)?;
    let stop = AtomicBool::new(false);
    let filter = Query {
        node: "alpha".into(),
        filters: vec![
            filter(Field::OperatorType, Comparison::Equals, "future-op"),
            filter(Field::Node, Comparison::Equals, "beta"),
        ],
        ..Default::default()
    };
    let review =
        rules::export_nodes::capture(&database, &["alpha".into(), "beta".into()], &filter, &stop)?;
    assert_eq!(review.rules, 52);
    assert_eq!(review.nodes[0].rules, 0);
    assert_eq!(review.nodes[1].rules, 52);
    let db = Connection::open(&database)?;
    let original: Vec<u8> = db.query_row(
        "SELECT wire FROM application_rules WHERE node='beta' AND name='rule-0000'",
        [],
        |r| r.get(0),
    )?;
    assert!(original.ends_with(&[0xa0, 0x06, 0x07]));
    // Capture is frozen before review/publishing and retains the original rule.
    db.execute(
        "UPDATE application_rules SET wire=? WHERE node='beta' AND name='rule-0000'",
        [rule(999).encode_to_vec()],
    )?;
    drop(db);
    let published = rules::export_nodes::publish(&review, dir.path(), &stop)?;
    let beta = published.join(&review.nodes[1].directory);
    let imported = std::fs::read_dir(&beta)?
        .map(|entry| {
            Ok(serde_json::from_slice::<serde_json::Value>(
                &std::fs::read(entry?.path())?,
            )?)
        })
        .collect::<Result<Vec<_>>>()?;
    let expected = (0..260)
        .filter(|i| i % 5 == 0)
        .map(rule)
        .collect::<Vec<_>>();
    assert_eq!(imported.len(), expected.len());
    for rule in expected {
        assert!(imported.contains(&rules::document::rule_json(&rule)));
    }
    assert!(
        imported
            .iter()
            .all(|r| r["operator"]["type"] == "future-op")
    );
    Ok(())
}
#[tokio::test]
async fn typed_admission_rejects_invalid_groups_and_pending_rule_reviews_stay_guarded() -> Result<()>
{
    let (_dir, database) = fixture(2)?;
    let worker = Worker::start(&database, 7, 16)?;
    let invalid = Query {
        node: "alpha".into(),
        filters: vec![filter(Field::Action, Comparison::Equals, "allow")],
        logic: Logic {
            formula: String::new(),
            mode: Match::Any,
            groups: vec![Group {
                count: 0,
                mode: Match::All,
            }],
        },
        ..Default::default()
    };
    assert!(worker.history.query_rules(invalid).unwrap().await?.is_err());
    worker.join();
    let db = Connection::open(&database)?;
    let mut pending = rule(5);
    pending.name = "pending-future".into();
    db.execute(
        "INSERT INTO prompt_rules(node,name,wire,process,description,duration) VALUES(?,?,?,?,?,?)",
        params![
            "alpha",
            pending.name,
            pending.encode_to_vec(),
            "",
            pending.description,
            pending.duration
        ],
    )?;
    drop(db);
    let query = Query {
        node: "alpha".into(),
        filters: vec![filter(Field::Name, Comparison::Equals, "pending-future")],
        ..Default::default()
    };
    assert!(
        transfer::load_rules(
            &database,
            &ExportScope {
                query: query.clone(),
                names: None
            }
        )
        .unwrap_err()
        .to_string()
        .contains("awaiting")
    );
    let (text, count) = transfer::export_text(&database, &ExportScope { query, names: None })?;
    assert_eq!(count, 1);
    let documents: Vec<serde_json::Value> = serde_json::from_str(&text)?;
    assert_eq!(documents, vec![rules::document::rule_json(&pending)]);
    assert!(
        transfer::Import::from_text(&text).is_err(),
        "Existing unsupported-operator import guard stays intact"
    );
    Ok(())
}

#[tokio::test]
async fn receipt_time_is_local_nullable_and_shared_by_pages_exports_and_real_observation()
-> Result<()> {
    let (_dir, database) = fixture(650)?;
    let db = Connection::open(&database)?;
    // Simulate untouched rows from schema9; migration never invents receipt time.
    db.execute("DELETE FROM rule_observations WHERE name='rule-0000'", [])?;
    db.execute(
        "UPDATE rule_observations SET received=1700000000+CAST(substr(name,6) AS INTEGER)",
        [],
    )?;
    let original_wire: Vec<u8> = db.query_row(
        "SELECT wire FROM application_rules WHERE node='alpha' AND name='rule-0001'",
        [],
        |r| r.get(0),
    )?;
    let query = Query {
        node: "alpha".into(),
        filters: vec![filter(
            Field::Received,
            Comparison::AtLeast,
            "2023-11-14T22:15:00Z",
        )],
        ..Default::default()
    };
    let worker = Worker::start(&database, 7, 16)?;
    let expected = (100..650).map(rule).collect::<Vec<_>>();
    assert_eq!(all(&worker, query.clone()).await?, expected);
    let (json, count) = transfer::export_text(
        &database,
        &ExportScope {
            query: query.clone(),
            names: None,
        },
    )?;
    assert_eq!(count, 550);
    assert_eq!(
        serde_json::from_str::<Vec<serde_json::Value>>(&json)?.len(),
        550
    );
    let scope = rustleblower::rules::export_nodes::capture(
        &database,
        &["alpha".into(), "beta".into()],
        &query,
        &AtomicBool::new(false),
    )?;
    assert_eq!(scope.rules, 1100);
    let missing = Query {
        node: "alpha".into(),
        filters: vec![filter(Field::Received, Comparison::Missing, "")],
        ..Default::default()
    };
    assert_eq!(all(&worker, missing.clone()).await?, vec![rule(0)]);
    // A received daemon event updates the clock on just its existing cached rule.
    worker.history.ingest(
        "alpha".into(),
        vec![rustleblower::protocol::Event {
            unixnano: rustleblower::history::now_nanos(),
            rule: Some(rule(0)),
            ..Default::default()
        }],
        "observed",
    );
    worker.history.flush().await?;
    assert!(all(&worker, missing).await?.is_empty());
    let received: i64 = db.query_row(
        "SELECT received FROM rule_observations WHERE node='alpha' AND name='rule-0000'",
        [],
        |r| r.get(0),
    )?;
    assert!((rustleblower::history::now_seconds() - received).abs() < 5);
    let unchanged: Vec<u8> = db.query_row(
        "SELECT wire FROM application_rules WHERE node='alpha' AND name='rule-0001'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(original_wire, unchanged);
    worker.history.ingest(
        "alpha".into(),
        vec![rustleblower::protocol::Event {
            unixnano: rustleblower::history::now_nanos(),
            rule: Some(Rule {
                name: "uncached".into(),
                ..Default::default()
            }),
            ..Default::default()
        }],
        "observed",
    );
    worker.history.flush().await?;
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM rule_observations WHERE name='uncached'",
            [],
            |r| r.get::<_, u32>(0)
        )?,
        0
    );
    db.execute(
        "DELETE FROM application_rules WHERE node='alpha' AND name='rule-0001'",
        [],
    )?;
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM rule_observations WHERE node='alpha' AND name='rule-0001'",
            [],
            |r| r.get::<_, u32>(0)
        )?,
        0
    );
    worker.join();
    Ok(())
}
#[test]
fn schema9_migration_preserves_wire_and_unknown_receipt_instead_of_using_creation_time()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("old.sqlite3");
    let db = Connection::open(&database)?;
    db.execute_batch("CREATE TABLE application_rules(node TEXT NOT NULL,name TEXT NOT NULL,wire BLOB NOT NULL,process TEXT NOT NULL,description TEXT NOT NULL,duration TEXT NOT NULL,PRIMARY KEY(node,name)); PRAGMA user_version=9;")?;
    let wire = put(&db, "alpha", &rule(1))?;
    drop(db);
    Worker::start(&database, 7, 16)?.join();
    let db = Connection::open(&database)?;
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))?,
        11
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM rule_observations", [], |r| r
            .get::<_, u32>(0))?,
        0
    );
    assert_eq!(
        db.query_row("SELECT wire FROM application_rules", [], |r| r
            .get::<_, Vec<u8>>(0))?,
        wire
    );
    let filter = Query {
        filters: vec![filter(Field::Received, Comparison::Missing, "")],
        ..Default::default()
    };
    assert!(Predicate::new(&filter)?.matches("alpha", &rule(1)));
    Ok(())
}

#[tokio::test]
async fn nested_operator_fields_match_leaves_without_rewriting_json_or_negative_sibling_leaks()
-> Result<()> {
    let (_dir, database) = fixture(3)?;
    let db = Connection::open(&database)?;
    let mut nested = rule(2);
    nested.operator = Some(Operator {
        r#type: "list".into(),
        operand: "list".into(),
        list: vec![Operator {
            r#type: "list".into(),
            list: vec![
                Operator {
                    operand: "dest.host".into(),
                    data: "Hidden.EXAMPLE.test".into(),
                    ..Default::default()
                },
                Operator {
                    operand: "dest.port".into(),
                    data: "443".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    });
    db.execute(
        "DELETE FROM application_rules WHERE node='alpha' AND name=?",
        [&nested.name],
    )?;
    let wire = put(&db, "alpha", &nested)?;
    let worker = Worker::start(&database, 7, 16)?;
    for (field, comparison, value, expected) in [
        (
            Field::AnyData,
            Comparison::Contains,
            "hidden.example",
            vec![nested.clone()],
        ),
        (
            Field::AnyOperand,
            Comparison::Exact,
            "dest.host",
            vec![nested.clone()],
        ),
        (Field::Data, Comparison::Contains, "hidden.example", vec![]),
        (
            Field::AnyData,
            Comparison::NotContains,
            "hidden.example",
            vec![rule(0), rule(1)],
        ),
        (
            Field::AnyData,
            Comparison::NotEquals,
            "hidden.example.test",
            vec![rule(0), rule(1)],
        ),
    ] {
        let query = Query {
            node: "alpha".into(),
            filters: vec![filter(field, comparison, value)],
            ..Default::default()
        };
        assert_eq!(all(&worker, query.clone()).await?, expected);
        assert_eq!(
            transfer::export_text(&database, &ExportScope { query, names: None })?.1,
            expected.len()
        );
    }
    let missing = Query {
        filters: vec![filter(Field::AnyData, Comparison::Missing, "")],
        ..Default::default()
    };
    assert!(Predicate::new(&missing)?.matches("alpha", &Rule::default()));
    assert!(!Predicate::new(&missing)?.matches("alpha", &nested));
    assert_eq!(
        db.query_row(
            "SELECT wire FROM application_rules WHERE node='alpha' AND name=?",
            [&nested.name],
            |r| r.get::<_, Vec<u8>>(0)
        )?,
        wire
    );
    worker.join();
    Ok(())
}
