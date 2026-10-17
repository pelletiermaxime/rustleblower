use anyhow::Result;
use hyper_util::rt::TokioIo;
use prost::Message;
use rusqlite::{Connection, params};
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    history::Worker,
    protocol::{self, Rule, ui_client::UiClient},
    rules::{
        self, Query,
        catalogue_policy::{self as policy, Context, Origin, Preset, RunId, Settings, Status},
        transfer::{self, ExportScope},
    },
};
use std::{collections::BTreeSet, path::Path, sync::atomic::AtomicBool, time::Duration};
use tonic::transport::{Channel, Endpoint};
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
fn rule(name: &str, duration: &str) -> Rule {
    Rule {
        name: name.into(),
        duration: duration.into(),
        created: now() - 60,
        enabled: true,
        action: "allow".into(),
        description: "owned policy fixture".into(),
        ..Default::default()
    }
}
fn options(path: &Path) -> Config {
    Config {
        transport: Default::default(),
        socket: path.join("ui.sock"),
        database: path.join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    }
}
async fn query(history: &rustleblower::history::History, query: Query) -> Result<rules::Page> {
    history.flush().await?;
    history
        .query_rules(query)
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)
}
async fn client(path: &Path, rules: Vec<Rule>) -> Result<UiClient<Channel>> {
    let path = path.to_path_buf();
    let channel = Endpoint::try_from("http://localhost")?
        .connect_with_connector(tower::service_fn(move |_| {
            let path = path.clone();
            async move {
                tokio::net::UnixStream::connect(path)
                    .await
                    .map(TokioIo::new)
            }
        }))
        .await?;
    let mut client = UiClient::new(channel);
    client
        .subscribe(protocol::ClientConfig {
            name: "catalogue-owned".into(),
            rules,
            ..Default::default()
        })
        .await?;
    Ok(client)
}
fn context(settings: Settings, run: Option<RunId>) -> Context {
    Context {
        settings,
        run,
        now_unix: now(),
        revision: None,
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn schema10_migration_preserves_wire_and_receipt_then_actual_rpc_observation_restores_origin()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = options(directory.path());
    let original = rule("retained-until-restart", "until restart");
    let mut wire = original.encode_to_vec();
    wire.extend_from_slice(&[0xa0, 0x06, 0x07]);
    let receipt = now() - 10;
    {
        let db = Connection::open(&config.database)?;
        db.execute_batch("CREATE TABLE application_rules(node TEXT NOT NULL,name TEXT NOT NULL,wire BLOB NOT NULL,process TEXT NOT NULL,description TEXT NOT NULL,duration TEXT NOT NULL,PRIMARY KEY(node,name));CREATE TABLE prompt_rules(node TEXT NOT NULL,name TEXT NOT NULL,wire BLOB NOT NULL,process TEXT NOT NULL,description TEXT NOT NULL,duration TEXT NOT NULL,PRIMARY KEY(node,name));CREATE TABLE rule_observations(node TEXT NOT NULL,name TEXT NOT NULL,received INTEGER NOT NULL,PRIMARY KEY(node,name));PRAGMA user_version=10;")?;
        db.execute(
            "INSERT INTO application_rules VALUES('catalogue-owned',?1,?2,'',?3,?4)",
            params![original.name, wire, original.description, original.duration],
        )?;
        db.execute(
            "INSERT INTO rule_observations VALUES('catalogue-owned',?1,?2)",
            params![original.name, receipt],
        )?;
    }
    let backend = Backend::start(config.clone())?;
    let frozen = context(Settings::default(), backend.handle.catalogue_run);
    assert!(frozen.run.is_some());
    let before = query(
        &backend.handle.history,
        Query {
            node: "catalogue-owned".into(),
            policy: Some(frozen),
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(before.rules.as_slice(), std::slice::from_ref(&original));
    assert_eq!(before.received[&original.name], receipt);
    assert_eq!(before.origins[&original.name], Origin::Restored);
    assert_eq!(
        before.cache_status[&original.name].status,
        Status::ExpiredAtUiRestart
    );
    assert!(before.rules[0].enabled);
    {
        let db = Connection::open(&config.database)?;
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
            11
        );
        assert_eq!(
            db.query_row("SELECT wire FROM application_rules", [], |r| r
                .get::<_, Vec<u8>>(0))?,
            wire
        );
        assert!(
            db.query_row("SELECT ui_run IS NULL FROM rule_observations", [], |r| r
                .get::<_, bool>(
                0
            ))?
        );
    }
    let mut client = client(&config.socket, vec![original.clone()]).await?;
    backend.handle.history.flush().await?;
    let fresh = query(
        &backend.handle.history,
        Query {
            node: "catalogue-owned".into(),
            policy: Some(context(Settings::default(), backend.handle.catalogue_run)),
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(fresh.rules, [original]);
    assert!(matches!(
        fresh.origins.values().next(),
        Some(Origin::ObservedThisUiRun { .. })
    ));
    assert_eq!(
        fresh.cache_status.values().next().unwrap().status,
        Status::Active
    );
    let mut asking = client.clone();
    let request = tokio::spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                protocol: "tcp".into(),
                dst_host: "policy-owned.example".into(),
                dst_ip: "203.0.113.7".into(),
                dst_port: 443,
                process_path: "/owned/catalogue-policy".into(),
                ..Default::default()
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while backend.handle.prompts.snapshot().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert!(
        backend
            .handle
            .prompts
            .choose(
                backend.handle.prompts.snapshot()[0].id,
                Decision::Deny,
                &rustleblower::prompts::choice::Choice {
                    duration: "5m".into(),
                    ..Default::default()
                },
            )
            .map_err(anyhow::Error::msg)?
    );
    let reply = request.await??.into_inner();
    let pending = query(
        &backend.handle.history,
        Query {
            node: "catalogue-owned".into(),
            policy: Some(context(Settings::default(), backend.handle.catalogue_run)),
            ..Default::default()
        },
    )
    .await?;
    assert!(pending.awaiting.contains(&reply.name));
    assert_eq!(pending.origins[&reply.name], Origin::Unknown);
    assert_eq!(pending.cache_status[&reply.name].status, Status::Unknown);
    assert_eq!(
        pending.rules.iter().find(|r| r.name == reply.name),
        Some(&reply)
    );
    client.ping(protocol::PingRequest::default()).await?;
    drop(client);
    backend.join();
    Ok(())
}
#[test]
fn explicit_run_marker_not_equal_receipt_seconds_controls_restart_and_random_failure() {
    let now = now();
    let rule = rule("marker", "until restart");
    let current = RunId([1; 16]);
    let other = RunId([2; 16]);
    let context = Context {
        now_unix: now,
        run: Some(current),
        settings: Settings::default(),
        revision: None,
    };
    assert_eq!(
        context.origin(Some(&other.0), Some(now), false),
        Origin::Restored
    );
    assert_eq!(context.origin(None, Some(now), false), Origin::Restored);
    assert_eq!(context.origin(None, Some(now), true), Origin::Unknown);
    assert_eq!(
        context.origin(Some(&current.0), Some(now), false),
        Origin::ObservedThisUiRun { received_unix: now }
    );
    assert_eq!(
        context.origin(Some(&[1, 2]), Some(now), false),
        Origin::Unknown
    );
    let failed = Context {
        run: None,
        ..context
    };
    let origin = failed.origin(Some(&current.0), Some(now), false);
    assert_eq!(origin, Origin::Unknown);
    assert_eq!(
        policy::derive(failed.settings, &rule, origin, now).status,
        Status::Unknown
    );
    assert!(rule.enabled);
}
fn insert(db: &Connection, node: &str, rule: &Rule) -> Result<()> {
    db.execute("INSERT INTO application_rules(node,name,wire,process,description,duration) VALUES(?1,?2,?3,'',?4,?5)",params![node,rule.name,rule.encode_to_vec(),rule.description,rule.duration])?;
    Ok(())
}
#[tokio::test]
async fn frozen_policy_applies_to_all_pages_selected_full_and_multi_node_exports() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("history.sqlite3");
    let worker = Worker::start(&database, 7, 16)?;
    worker.history.flush().await?;
    let settings = Settings {
        ignore_temporary: true,
        threshold: Preset::ThirtyMinutes,
    };
    let preference_worker = policy::Worker::start(policy::path_for(&database));
    let initial = preference_worker
        .handle()
        .request(policy::Command::Reload)
        .await?
        .map_err(anyhow::Error::msg)?;
    let saved = preference_worker
        .handle()
        .request(policy::Command::Save {
            expected_revision: initial.revision,
            settings,
        })
        .await?
        .map_err(anyhow::Error::msg)?;
    let frozen = Context::capture(&saved, policy::process_run_id(), now()).unwrap();
    let durations = [
        "once",
        "30s",
        "5m",
        "15m",
        "30m",
        "1h",
        "12h",
        "until restart",
        "always",
        "2m",
    ];
    {
        let db = Connection::open(&database)?;
        for node in ["alpha", "beta"] {
            for index in 0..650 {
                insert(
                    &db,
                    node,
                    &rule(&format!("rule-{index:04}"), durations[index % 10]),
                )?;
            }
        }
    }
    let expected = (0..650)
        .filter(|index| settings.includes(durations[index % 10]))
        .map(|index| format!("rule-{index:04}"))
        .collect::<Vec<_>>();
    assert!(expected.len() > 200);
    let query_scope = Query {
        node: "alpha".into(),
        search: "rule-".into(),
        policy: Some(frozen),
        ..Default::default()
    };
    let mut cursor = query_scope.clone();
    let mut names = Vec::new();
    loop {
        let page = query(&worker.history, cursor.clone()).await?;
        assert!(page.rules.iter().all(|r| r.enabled));
        names.extend(page.rules.iter().map(|r| r.name.clone()));
        if !page.has_more {
            break;
        }
        cursor.after = page.rules.last().unwrap().name.clone();
    }
    assert_eq!(names, expected);
    let scope = ExportScope {
        query: query_scope.clone(),
        names: None,
    };
    let (_, count) = transfer::export_text(&database, &scope)?;
    assert_eq!(count, expected.len());
    let selected = transfer::load_rules(
        &database,
        &ExportScope {
            query: query_scope.clone(),
            names: Some(BTreeSet::from([expected[0].clone(), expected[210].clone()])),
        },
    )?;
    assert_eq!(selected.len(), 2);
    assert!(selected.iter().all(|r| r.enabled));
    assert!(
        transfer::load_rules(
            &database,
            &ExportScope {
                query: query_scope.clone(),
                names: Some(BTreeSet::from(["rule-0000".into()]))
            }
        )
        .is_err()
    );
    preference_worker
        .handle()
        .request(policy::Command::Save {
            expected_revision: saved.revision,
            settings: Settings::default(),
        })
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(transfer::export_text(&database, &scope)?.1, expected.len());
    assert_eq!(scope.query.policy, Some(frozen));
    let stop = AtomicBool::new(false);
    let review = rules::export_nodes::capture(
        &database,
        &["alpha".into(), "beta".into()],
        &query_scope,
        &stop,
    )?;
    assert_eq!(review.rules, expected.len() * 2);
    let output = directory.path().join("exports");
    std::fs::create_dir(&output)?;
    let published = rules::export_nodes::publish(&review, &output, &stop)?;
    assert!(published.is_dir());
    let latest = preference_worker.handle().snapshot();
    let mut fresh = query_scope;
    fresh.policy = Context::capture(&latest, policy::process_run_id(), now());
    assert_eq!(
        transfer::export_text(
            &database,
            &ExportScope {
                query: fresh,
                names: None
            }
        )?
        .1,
        650
    );
    preference_worker.join();
    worker.join();
    Ok(())
}
