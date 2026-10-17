use anyhow::Result;
use prost::Message;
use rustleblower::{
    history::Worker,
    protocol::{Operator, Rule},
    rules::{
        self, Query,
        transfer::{self, ExportScope},
    },
    view_preferences::{self as views, RuleField, Sort},
};
use std::time::Duration;
#[test]
fn finite_columns_filters_and_whole_document_validation() -> Result<()> {
    let settings = views::Settings::default();
    settings.validate()?;
    assert_eq!(settings.nodes.columns.len(), 6);
    assert_eq!(settings.rules.columns.len(), 11);
    assert_eq!(settings.alerts.columns.len(), 10);
    let mut invalid = settings.clone();
    invalid.nodes.columns.push(invalid.nodes.columns[0].clone());
    assert!(invalid.validate().is_err());
    let mut invalid = settings.clone();
    invalid.rules.columns[0].key = "arbitrary_sql_expression()".into();
    assert!(invalid.validate().is_err());
    let mut invalid = settings.clone();
    invalid.alerts.columns[0].width = 2001;
    assert!(invalid.validate().is_err());
    let mut invalid = settings;
    invalid.alerts.severity = Some(99);
    assert!(invalid.validate().is_err());
    Ok(())
}
#[tokio::test]
async fn shared_views_worker_reopens_and_refuses_stale_other_view_and_invalid_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = views::path_for(&dir.path().join("history.sqlite3"));
    assert!(path.ends_with("history.sqlite3.views.json"));
    let worker = views::Worker::start(path.clone());
    let handle = worker.handle();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !handle.snapshot().ready {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let mut settings = views::Settings::default();
    settings.nodes.search = "owned-node".into();
    settings.rules.sort = Sort {
        field: RuleField::Created,
        ascending: false,
    };
    let saved = handle
        .request(views::Command::Save {
            expected_revision: None,
            settings: Box::new(settings.clone()),
        })
        .await?
        .map_err(anyhow::Error::msg)?;
    let mut later = settings.clone();
    later.alerts.unread = true;
    assert!(
        handle
            .request(views::Command::Save {
                expected_revision: None,
                settings: Box::new(later)
            })
            .await?
            .is_err()
    );
    worker.join();
    let worker = views::Worker::start(path.clone());
    let handle = worker.handle();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !handle.snapshot().ready {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert_eq!(handle.snapshot().settings, settings);
    std::fs::write(&path, b"external invalid view settings")?;
    let state = handle
        .request(views::Command::Reload)
        .await?
        .map_err(anyhow::Error::msg)?;
    assert!(state.warning.is_some());
    assert_eq!(state.settings, settings);
    assert!(
        handle
            .request(views::Command::Save {
                expected_revision: saved.revision,
                settings: Box::new(settings)
            })
            .await?
            .is_err()
    );
    assert_eq!(std::fs::read(&path)?, b"external invalid view settings");
    worker.join();
    Ok(())
}
fn fixture(index: usize) -> Rule {
    Rule {
        name: format!("rule.{index:04}"),
        created: 1700000000 + ((index * 37) % 650) as i64,
        action: if index.is_multiple_of(2) {
            "allow"
        } else {
            "deny"
        }
        .into(),
        enabled: index.is_multiple_of(2),
        duration: if index.is_multiple_of(3) {
            "5m"
        } else {
            "always"
        }
        .into(),
        precedence: index.is_multiple_of(5),
        nolog: index.is_multiple_of(7),
        description: format!("description {:04}", 650 - index),
        operator: Some(Operator {
            r#type: "simple".into(),
            operand: "process.path".into(),
            data: format!("/owned/bin/{}", index % 9),
            ..Default::default()
        }),
    }
}
#[tokio::test]
async fn global_rule_sort_is_numeric_tied_stable_across_pages_and_frozen_exports() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("history.sqlite3");
    let worker = Worker::start(&database, 1, 16)?;
    let mut db = rusqlite::Connection::open(&database)?;
    let transaction = db.transaction()?;
    for index in 0..650 {
        let rule = fixture(index);
        transaction.execute("INSERT INTO application_rules(node,name,wire,process,description,duration) VALUES('owned',?1,?2,?3,?4,?5)",rusqlite::params![rule.name,rule.encode_to_vec(),rules::executable(&rule).unwrap(),rule.description,rule.duration])?;
    }
    transaction.commit()?;
    let mut query = Query {
        node: "owned".into(),
        sort: Some(Sort {
            field: RuleField::Created,
            ascending: true,
        }),
        ..Default::default()
    };
    let mut names = Vec::new();
    let mut created = Vec::new();
    loop {
        let page = worker
            .history
            .query_rules(query.clone())
            .unwrap()
            .await?
            .map_err(anyhow::Error::msg)?;
        assert!(!page.rules.is_empty());
        names.extend(page.rules.iter().map(|rule| rule.name.clone()));
        created.extend(page.rules.iter().map(|rule| rule.created));
        if !page.has_more {
            break;
        }
        query.after = page.rules.last().unwrap().name.clone();
    }
    assert_eq!(names.len(), 650);
    assert_eq!(created, (1700000000..1700000650).collect::<Vec<_>>());
    query.after.clear();
    let scope = ExportScope {
        query: query.clone(),
        names: None,
    };
    let (text, count) = transfer::export_text(&database, &scope)?;
    assert_eq!(count, 650);
    let exported = transfer::Import::from_text(&text)?;
    assert_eq!(
        exported
            .rules
            .iter()
            .map(|rule| rule.name.clone())
            .collect::<Vec<_>>(),
        names
    );
    let selected = ExportScope {
        query: query.clone(),
        names: Some(std::collections::BTreeSet::from([
            names[199].clone(),
            names[401].clone(),
        ])),
    };
    let (text, count) = transfer::export_text(&database, &selected)?;
    assert_eq!(count, 2);
    assert_eq!(
        transfer::Import::from_text(&text)?
            .rules
            .iter()
            .map(|rule| rule.name.clone())
            .collect::<Vec<_>>(),
        vec![names[199].clone(), names[401].clone()]
    );
    for field in views::RULE_FIELDS {
        let mut query = query.clone();
        query.sort = Some(Sort {
            field: *field,
            ascending: false,
        });
        let page = worker
            .history
            .query_rules(query)
            .unwrap()
            .await?
            .map_err(anyhow::Error::msg)?;
        assert_eq!(page.rules.len(), 200);
        assert!(page.has_more);
    }
    let durations = [
        "always",
        "1h",
        "5m",
        "30s",
        "unknown",
        "once",
        "until restart",
        "90s",
        "60m",
    ];
    for (index, duration) in durations.iter().enumerate() {
        let mut rule = fixture(index);
        rule.name = format!("duration.{index}");
        rule.duration = (*duration).into();
        db.execute("INSERT INTO application_rules(node,name,wire,process,description,duration) VALUES('durations',?1,?2,'','',?3)",rusqlite::params![rule.name,rule.encode_to_vec(),rule.duration])?;
    }
    let page = worker
        .history
        .query_rules(Query {
            node: "durations".into(),
            sort: Some(Sort {
                field: RuleField::Duration,
                ascending: true,
            }),
            ..Default::default()
        })
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        page.rules
            .iter()
            .map(|rule| rule.duration.as_str())
            .collect::<Vec<_>>(),
        vec![
            "once",
            "30s",
            "90s",
            "5m",
            "1h",
            "60m",
            "until restart",
            "always",
            "unknown"
        ]
    );
    drop(db);
    worker.join();
    Ok(())
}

#[test]
fn eight_detail_profiles_allow_all_twenty_one_columns_with_explicit_encoded_budget() -> Result<()> {
    use rustleblower::history::query::{Comparison, Field};
    let mut settings = views::Settings::default();
    settings.nodes.search = "n".repeat(256);
    settings.rules.search = "r".repeat(256);
    settings.alerts.search = "a".repeat(256);
    settings.alerts.node = "s".repeat(128);
    settings.rules.filters = (0..8)
        .map(|_| rules::query::Filter {
            field: rules::query::Field::Name,
            comparison: Comparison::Contains,
            value: "v".repeat(256),
        })
        .collect();
    for profile in &mut settings.details {
        profile.search = "d".repeat(256);
        profile.columns = std::iter::once(views::DetailColumn::Hits)
            .chain(
                rustleblower::history::query::FIELDS
                    .iter()
                    .copied()
                    .map(views::DetailColumn::Event),
            )
            .map(|column| views::DetailWidth {
                column,
                width: 2000,
            })
            .collect();
        profile.sort = Sort {
            field: views::DetailColumn::Event(Field::DestinationIp),
            ascending: true,
        };
    }
    settings.validate()?;
    let bytes = serde_json::to_vec(&settings)?.len();
    println!("all_filled_detail_profile_bytes={bytes}");
    assert!(bytes + 64 <= views::MAX_DOCUMENT_BYTES);
    assert_eq!(settings.details.len(), 8);
    assert!(
        settings
            .details
            .iter()
            .all(|profile| profile.columns.len() == 21)
    );
    for kind in views::DETAIL_KINDS {
        let profile = settings.detail(kind.grouped()).unwrap();
        assert_eq!(profile.kind, kind);
        assert_eq!(
            views::DetailColumn::from_grouped(profile.sort.field.grouped()),
            profile.sort.field
        );
    }
    let mut invalid = settings.clone();
    invalid.details[1].kind = invalid.details[0].kind;
    assert!(invalid.validate().is_err());
    let mut invalid = settings.clone();
    let duplicate = invalid.details[0].columns[0];
    invalid.details[0].columns.push(duplicate);
    assert!(invalid.validate().is_err());
    settings.nodes.search = "\x01".repeat(256);
    settings.rules.search = "\x01".repeat(256);
    settings.alerts.search = "\x01".repeat(256);
    settings.alerts.node = "\x01".repeat(128);
    for filter in &mut settings.rules.filters {
        filter.value = "\x01".repeat(256);
    }
    for profile in &mut settings.details {
        profile.search = "\x01".repeat(256);
    }
    let escaped = serde_json::to_vec(&settings)?.len();
    println!("escaped_detail_profile_bytes={escaped}");
    assert!(escaped + 64 > views::MAX_DOCUMENT_BYTES);
    assert!(settings.validate().is_err());
    Ok(())
}
