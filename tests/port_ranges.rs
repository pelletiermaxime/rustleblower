use anyhow::Result;
use hyper_util::rt::TokioIo;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    protocol::{self, Action, Operator, Rule, ui_client::UiClient},
    rules::{self, document},
};
use std::{path::Path, time::Duration};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};

fn options(path: &Path) -> Config {
    Config {
        socket: path.join("ui.sock"),
        database: path.join("history.sqlite3"),
        prompt_timeout: 5,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    }
}
fn rule() -> Rule {
    Rule {
        name: "owned-port-range".into(),
        created: 123,
        enabled: true,
        action: "deny".into(),
        duration: "always".into(),
        operator: Some(Operator {
            r#type: "list".into(),
            operand: "list".into(),
            list: vec![
                Operator {
                    r#type: "range".into(),
                    operand: "source.port".into(),
                    data: " 04000 - 04010 ".into(),
                    sensitive: true,
                    ..Default::default()
                },
                Operator {
                    r#type: "range".into(),
                    operand: "dest.port".into(),
                    data: "8000-8080".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    }
}
async fn connect(path: &Path) -> Result<UiClient<Channel>> {
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
    Ok(UiClient::new(channel))
}
async fn version(client: &mut UiClient<Channel>, version: &str) -> Result<()> {
    client
        .ping(protocol::PingRequest {
            stats: Some(protocol::Statistics {
                daemon_version: version.into(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await?;
    Ok(())
}
async fn subscribe(
    backend: &Backend,
    client: &mut UiClient<Channel>,
    rules: Vec<Rule>,
) -> Result<u64> {
    client
        .subscribe(protocol::ClientConfig {
            name: "range-fixture".into(),
            rules,
            ..Default::default()
        })
        .await?;
    backend.handle.history.flush().await?;
    Ok(backend.handle.status().session.unwrap().id)
}

#[test]
fn range_grammar_and_import_export_preserve_the_exact_operator() -> Result<()> {
    let cases: Vec<(&str, Option<(u64, u64)>)> = vec![
        ("0-0", Some((0, 0))),
        ("0-65535", Some((0, 65535))),
        (" 04000 - 04010 ", Some((4000, 4010))),
        ("8 0-8 1", Some((80, 81))),
        ("443-443", Some((443, 443))),
        ("65536-18446744073709551615", Some((65536, u64::MAX))),
        ("", None),
        ("80", None),
        ("90-80", None),
        ("-1-80", None),
        ("+1-2", None),
        ("1-+2", None),
        ("1-2-3", None),
        ("1.0-2", None),
        ("1-18446744073709551616", None),
        ("\t1-2", None),
        ("1-2\n", None),
        ("１-２", None),
        ("1–2", None),
        ("1-", None),
        ("-2", None),
    ];
    for (input, expected) in &cases {
        assert_eq!(document::port_range(input).ok(), *expected, "{input:?}");
    }
    let original = rule();
    assert_eq!(
        document::rule_from_json(&document::rule_json(&original)).unwrap(),
        original
    );
    let json = serde_json::to_string(&document::rule_json(&original))?;
    assert_eq!(
        document::parse_document(&json).unwrap(),
        vec![original.clone()]
    );
    for operand in ["source.port", "dest.port"] {
        for (input, expected) in &cases {
            let mut current = original.clone();
            current.operator = Some(Operator {
                r#type: "range".into(),
                operand: operand.into(),
                data: input.to_string(),
                ..Default::default()
            });
            assert_eq!(
                document::validate_edit(&current).is_ok(),
                expected.is_some()
            );
        }
    }
    let mut wrong = original;
    wrong.operator.as_mut().unwrap().list[0].operand = "dest.host".into();
    assert!(
        document::validate_edit(&wrong)
            .unwrap_err()
            .contains("port field")
    );
    if let Some(path) = std::env::var_os("RUSTLEBLOWER_RANGE_EVIDENCE") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path)?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join("grammar.json"))?;
        let fixtures = cases.iter().map(|(data,bounds)| serde_json::json!({"data":data,"bounds":bounds.map(|(min,max)|[min.to_string(),max.to_string()])})).collect::<Vec<_>>();
        f.write_all(serde_json::to_string_pretty(&fixtures)?.as_bytes())?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn range_commands_wait_for_ack_and_permissions_stay_independent() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut daemon = connect(&config.socket).await?;
    version(&mut daemon, "1.9.0 (owned fixture)").await?; // Ping may precede Subscribe.
    let session = subscribe(&backend, &mut daemon, vec![]).await?;
    assert!(backend.handle.control.range_supported(session));
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let mut stream = daemon
        .notifications(ReceiverStream::new(rx))
        .await?
        .into_inner();
    let original = rule();
    let reply = backend
        .handle
        .control
        .submit(session, Action::ChangeRule, original.clone())
        .map_err(anyhow::Error::msg)?;
    let command = tokio::time::timeout(Duration::from_secs(1), stream.message())
        .await?
        .map_err(anyhow::Error::msg)?
        .unwrap();
    assert_eq!(command.rules, vec![original.clone()]);
    assert!(
        backend
            .handle
            .history
            .query_rules(rules::Query {
                node: "range-fixture".into(),
                ..Default::default()
            })
            .unwrap()
            .await?
            .map_err(anyhow::Error::msg)?
            .rules
            .is_empty()
    );
    let mut asking = daemon.clone();
    let asked = tokio::spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/independent".into(),
                dst_port: 443,
                ..Default::default()
            })
            .await
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while backend.handle.prompts.snapshot().is_empty() {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let prompt = backend.handle.prompts.snapshot()[0].clone();
    assert!(backend.handle.prompts.decide(prompt.id, Decision::Allow));
    let decision = tokio::time::timeout(Duration::from_millis(500), asked)
        .await???
        .into_inner();
    assert_eq!(decision.action, "allow");
    tx.send(protocol::NotificationReply {
        id: command.id,
        code: protocol::NotificationReplyCode::Ok as i32,
        ..Default::default()
    })
    .await?;
    reply.await?.map_err(anyhow::Error::msg)?;
    let saved = backend
        .handle
        .history
        .query_rules(rules::Query {
            node: "range-fixture".into(),
            ..Default::default()
        })
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(saved.rules, vec![original.clone()]);
    let mut changed = original.clone();
    changed.action = "allow".into();
    let rejected = backend
        .handle
        .control
        .submit(session, Action::ChangeRule, changed)
        .map_err(anyhow::Error::msg)?;
    let command = stream.message().await?.unwrap();
    tx.send(protocol::NotificationReply {
        id: command.id,
        code: protocol::NotificationReplyCode::Error as i32,
        data: "owned rejection".into(),
    })
    .await?;
    assert!(rejected.await?.unwrap_err().contains("owned rejection"));
    let scope = rules::transfer::ExportScope {
        query: rules::Query {
            node: "range-fixture".into(),
            ..Default::default()
        },
        names: None,
    };
    let (text, count) = rules::transfer::export_text(&config.database, &scope)?;
    assert_eq!(count, 1);
    assert_eq!(document::parse_document(&text).unwrap(), vec![original]);
    drop(stream);
    drop(tx);
    drop(daemon);
    backend.join();
    assert!(!config.socket.exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn range_support_is_session_scoped_and_older_or_unknown_daemons_receive_nothing() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut daemon = connect(&config.socket).await?;
    for reported in ["", "1.7.2", "1.8.0.r224", "1.9.invalid", "1.9", "nonsense"] {
        let session = subscribe(&backend, &mut daemon, vec![]).await?;
        version(&mut daemon, reported).await?;
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let mut stream = daemon
            .notifications(ReceiverStream::new(rx))
            .await?
            .into_inner();
        assert!(
            !backend.handle.control.range_supported(session),
            "{reported}"
        );
        let error = backend
            .handle
            .control
            .submit(session, Action::ChangeRule, rule())
            .err()
            .unwrap();
        assert!(error.contains("not established"));
        assert!(
            tokio::time::timeout(Duration::from_millis(30), stream.message())
                .await
                .is_err()
        );
        drop(stream);
        drop(tx);
    }
    let session = subscribe(&backend, &mut daemon, vec![rule()]).await?;
    version(&mut daemon, "1.8.0 (range backport)").await?;
    assert!(
        backend.handle.control.range_supported(session),
        "an existing reported range establishes a backport"
    );
    let next = subscribe(&backend, &mut daemon, vec![]).await?;
    assert_ne!(session, next);
    assert!(!backend.handle.control.range_supported(session));
    assert!(!backend.handle.control.range_supported(next));
    for reported in ["1.9.0", "v1.9.0-rc1", "2.0.0", "1.10.3 (git)"] {
        version(&mut daemon, reported).await?;
        assert!(backend.handle.control.range_supported(next), "{reported}");
    }
    drop(daemon);
    backend.join();
    Ok(())
}
