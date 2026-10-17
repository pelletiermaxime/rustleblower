use anyhow::Result;
use hyper_util::rt::TokioIo;
use prost::Message;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    firewall::{
        self,
        document::{Chain, Document, Rule},
    },
    nodes,
    protocol::{self, ui_client::UiClient},
};
use std::{path::Path, time::Duration};
use tonic::transport::{Channel, Endpoint};

fn config() -> protocol::SysFirewall {
    protocol::SysFirewall {
        version: 1,
        enabled: true,
        system_rules: vec![protocol::FwChains {
            rule: Some(protocol::FwRule {
                table: "mangle".into(),
                chain: "OUTPUT".into(),
                parameters: "-p icmp".into(),
                position: u64::MAX,
                target: "ACCEPT".into(),
                target_parameters: "legacy-target".into(),
                ..Default::default()
            }),
            chains: vec![protocol::FwChain {
                name: "input".into(),
                table: "filter".into(),
                family: "inet".into(),
                priority: "-10".into(),
                r#type: "filter".into(),
                hook: "input".into(),
                policy: "accept".into(),
                rules: vec![
                    protocol::FwRule {
                        uuid: "first".into(),
                        enabled: true,
                        position: u64::MAX,
                        description: "Unicode Ω".into(),
                        expressions: vec![protocol::Expressions {
                            statement: Some(protocol::Statement {
                                name: "ct".into(),
                                op: "in".into(),
                                values: vec![protocol::StatementValues {
                                    key: "state".into(),
                                    value: "established,related".into(),
                                }],
                            }),
                        }],
                        target: "accept".into(),
                        target_parameters: "unchanged".into(),
                        ..Default::default()
                    },
                    protocol::FwRule {
                        uuid: "second".into(),
                        position: 1,
                        target: "drop".into(),
                        ..Default::default()
                    },
                ],
            }],
        }],
    }
}
#[test]
fn complete_document_roundtrip_and_atomic_edits_preserve_unknown_values() -> Result<()> {
    let original = config();
    let mut doc = Document::from(original.clone());
    let text = doc.json().map_err(anyhow::Error::msg)?;
    assert!(text.contains("\"18446744073709551615\""));
    assert_eq!(
        protocol::SysFirewall::from(Document::parse(&text).map_err(anyhow::Error::msg)?),
        original
    );
    doc.move_rule(0, 0, 1, 0).map_err(anyhow::Error::msg)?;
    assert_eq!(doc.chain(0, 0).unwrap().rules[0].uuid, "second");
    assert_eq!(
        doc.chain(0, 0).unwrap().rules[1].position,
        u64::MAX,
        "array reordering preserves nftables insertion metadata"
    );
    assert_eq!(
        doc.system_rules[0].rule.as_ref().unwrap().position,
        u64::MAX,
        "legacy wrapper is retained exactly"
    );
    let chain = doc.chain(0, 0).unwrap().clone();
    doc.apply_profile(Chain {
        policy: "drop".into(),
        rules: vec![
            chain.rules[0].clone(),
            Rule {
                uuid: "profile-added".into(),
                target: "reject".into(),
                ..Default::default()
            },
        ],
        ..chain
    })
    .map_err(anyhow::Error::msg)?;
    assert_eq!(doc.chain(0, 0).unwrap().rules.len(), 3);
    assert_eq!(doc.chain(0, 0).unwrap().policy, "drop");
    let before = doc.clone();
    assert!(
        doc.edit(|d| {
            d.chain_mut(0, 0)?.name.clear();
            Ok(())
        })
        .is_err()
    );
    assert_eq!(doc, before);
    assert!(doc.move_rule(0, 0, 20, 0).is_err());
    assert_eq!(doc, before);
    let wire = protocol::SysFirewall::from(doc);
    firewall::validate(&wire).map_err(anyhow::Error::msg)?;
    assert_eq!(
        wire.system_rules[0].chains[0].rules[1].expressions,
        original.system_rules[0].chains[0].rules[0].expressions
    );
    Ok(())
}
#[test]
fn payload_and_container_limits_reject_before_typed_allocation_or_wire_send() {
    let mut doc = Document::from(config());
    doc.system_rules[0].chains[0].rules = vec![Rule::default(); firewall::MAX_RULES + 1];
    let text = serde_json::to_string(&doc).unwrap();
    assert!(
        Document::parse(&text)
            .unwrap_err()
            .contains("structural limits")
    );
    assert!(Document::parse("{\"Version\":1,\"Future\":[]}").is_err());
    assert!(Document::parse("{\"Version\":2}").is_err());
    assert!(
        Document::parse(
            "{\"Version\":1,\"SystemRules\":[{\"Rule\":{\"Position\":\"18446744073709551616\"}}]}"
        )
        .is_err()
    );
    let mut wire = config();
    wire.system_rules[0].chains[0].rules[0].description = "x".repeat(nodes::FIREWALL_BYTES);
    assert!(firewall::validate(&wire).is_err());
    assert!(Document::parse(&" ".repeat(firewall::MAX_JSON + 1)).is_err());
}

#[test]
fn form_commands_preserve_unedited_fields_and_stage_atomic_crud() -> Result<()> {
    use firewall::document::Command;
    let original = Document::from(config());
    let mut doc = original.clone();
    doc.move_rule(0, 0, 0, 0).map_err(anyhow::Error::msg)?;
    assert_eq!(
        doc, original,
        "a no-op move preserves explicit daemon positions"
    );
    let mut chain = doc.chain(0, 0).unwrap().clone();
    chain.name = "renamed-input".into();
    chain.policy = "drop".into();
    chain.rules.clear();
    doc.command(Command::Chain {
        group: 0,
        index: Some(0),
        fields: chain,
    })
    .map_err(anyhow::Error::msg)?;
    assert_eq!(
        doc.chain(0, 0).unwrap().rules,
        original.chain(0, 0).unwrap().rules,
        "chain metadata edits cannot erase its rules"
    );
    let before = doc.clone();
    let chain = doc.chain(0, 0).unwrap().clone();
    assert!(
        doc.command(Command::Chain {
            group: 0,
            index: None,
            fields: chain
        })
        .is_err()
    );
    assert_eq!(
        doc, before,
        "duplicate chain creation rolls back completely"
    );
    let chain = Chain {
        name: "owned-output".into(),
        table: "filter".into(),
        family: "inet".into(),
        kind: "filter".into(),
        hook: "output".into(),
        policy: "accept".into(),
        ..Default::default()
    };
    doc.command(Command::Chain {
        group: 0,
        index: None,
        fields: chain,
    })
    .map_err(anyhow::Error::msg)?;
    let mut rule = Rule {
        uuid: "new-rule".into(),
        enabled: true,
        target: "reject".into(),
        description: "new from form".into(),
        position: u64::MAX,
        ..Default::default()
    };
    let before = doc.clone();
    assert!(
        doc.command(Command::Rule {
            group: 0,
            chain: 1,
            index: None,
            rule: rule.clone()
        })
        .is_err(),
        "the daemon ignores enabled rules with no expressions"
    );
    assert_eq!(doc, before);
    rule.expressions = original.chain(0, 0).unwrap().rules[0].expressions.clone();
    doc.command(Command::Rule {
        group: 0,
        chain: 1,
        index: None,
        rule: rule.clone(),
    })
    .map_err(anyhow::Error::msg)?;
    let before = doc.clone();
    assert!(
        doc.command(Command::Rule {
            group: 0,
            chain: 0,
            index: None,
            rule
        })
        .is_err()
    );
    assert_eq!(
        doc, before,
        "new nonempty IDs cannot overwrite another chain's rule"
    );
    let mut edit = doc.chain(0, 0).unwrap().rules[0].clone();
    edit.description = "edited independently".into();
    doc.command(Command::Rule {
        group: 0,
        chain: 0,
        index: Some(0),
        rule: edit,
    })
    .map_err(anyhow::Error::msg)?;
    assert_eq!(
        doc.chain(0, 0).unwrap().rules[0].expressions,
        original.chain(0, 0).unwrap().rules[0].expressions
    );
    doc.command(Command::ToggleRule {
        group: 0,
        chain: 0,
        rule: 0,
    })
    .map_err(anyhow::Error::msg)?;
    assert!(!doc.chain(0, 0).unwrap().rules[0].enabled);
    doc.command(Command::MoveRule {
        group: 0,
        chain: 0,
        from: 0,
        to: 1,
    })
    .map_err(anyhow::Error::msg)?;
    assert_eq!(doc.chain(0, 0).unwrap().rules[1].uuid, "first");
    doc.command(Command::DeleteRule {
        group: 0,
        chain: 1,
        rule: 0,
    })
    .map_err(anyhow::Error::msg)?;
    assert!(doc.chain(0, 1).unwrap().rules.is_empty());
    doc.command(Command::DeleteChain { group: 0, chain: 1 })
        .map_err(anyhow::Error::msg)?;
    assert_eq!(doc.system_rules[0].chains.len(), 1);
    assert_eq!(
        doc.system_rules[0].rule, original.system_rules[0].rule,
        "legacy wrapper remains unchanged throughout"
    );
    let mut empty = Document {
        version: 1,
        ..Default::default()
    };
    empty
        .command(Command::Chain {
            group: 0,
            index: None,
            fields: Chain {
                name: "first-chain".into(),
                table: "filter".into(),
                family: "inet".into(),
                ..Default::default()
            },
        })
        .map_err(anyhow::Error::msg)?;
    assert_eq!(empty.system_rules.len(), 1);
    let before = Document {
        version: 1,
        ..Default::default()
    };
    let (review, _) = before.differences(&empty).map_err(anyhow::Error::msg)?;
    assert!(
        review
            .iter()
            .any(|line| line.contains("/Chains/0/Name: absent") && line.contains("first-chain")),
        "new chains are reviewed field by field"
    );
    assert!(
        review
            .iter()
            .any(|line| line.contains("/Chains/0/Rules: absent → []")),
        "empty collections remain visible in the review"
    );
    Ok(())
}
async fn client(path: &Path) -> Result<UiClient<Channel>> {
    let path = path.to_owned();
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
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_reload_routing_ack_cache_rollback_reconnect_and_prompt_isolation() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let options = Config {
        transport: Default::default(),
        socket: dir.path().join("ui.sock"),
        database: dir.path().join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    };
    let backend = Backend::start(options.clone())?;
    let initial = config();
    let mut alpha = client(&options.socket).await?;
    alpha
        .subscribe(protocol::ClientConfig {
            name: "alpha".into(),
            system_firewall: Some(initial.clone()),
            config: r#"{"Firewall":"nftables"}"#.into(),
            ..Default::default()
        })
        .await?;
    let id = backend.handle.node("alpha").unwrap().id;
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let mut commands = alpha
        .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await?
        .into_inner();
    let mut beta = client(&options.socket).await?;
    beta.subscribe(protocol::ClientConfig {
        name: "beta".into(),
        system_firewall: Some(initial.clone()),
        config: r#"{"Firewall":"nftables"}"#.into(),
        ..Default::default()
    })
    .await?;
    let (btx, brx) = tokio::sync::mpsc::channel(8);
    let mut bcommands = beta
        .notifications(tokio_stream::wrappers::ReceiverStream::new(brx))
        .await?
        .into_inner();
    backend.handle.history.flush().await?;
    let read = firewall::read(&options.database, "alpha")?;
    assert!(read.reported);
    assert_eq!(read.session, id);
    let mut changed = initial.clone();
    changed.system_rules[0].chains[0].policy = "drop".into();
    let change =
        firewall::Change::new(read.fingerprint, changed.clone()).map_err(anyhow::Error::msg)?;
    let mut invalid = change.clone();
    invalid.expected = [0; 32];
    assert!(
        backend
            .handle
            .control
            .submit_node(id, nodes::Change::Firewall(invalid))
            .is_err()
    );
    let mut pending = backend
        .handle
        .control
        .submit_node(id, nodes::Change::Firewall(change.clone()))
        .map_err(anyhow::Error::msg)?;
    let command = tokio::time::timeout(Duration::from_secs(1), commands.message())
        .await??
        .unwrap();
    assert_eq!(command.r#type, 5);
    assert!(command.data.is_empty() && command.rules.is_empty());
    assert_eq!(command.sys_firewall, Some(changed.clone()));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), bcommands.message())
            .await
            .is_err()
    );
    assert_eq!(
        firewall::read(&options.database, "alpha")?.config,
        initial,
        "no optimistic cache mutation"
    );
    btx.send(protocol::NotificationReply {
        id: command.id,
        ..Default::default()
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(matches!(
        pending.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    tokio::time::timeout(
        Duration::from_millis(500),
        alpha.ping(protocol::PingRequest {
            id: 7,
            ..Default::default()
        }),
    )
    .await??;
    let mut asking = beta.clone();
    let ask = tokio::spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/firewall-permission".into(),
                ..Default::default()
            })
            .await
    });
    tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            if let Some(p) = backend.handle.prompts.snapshot().first() {
                backend.handle.decide(p.id, Decision::Allow);
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await?;
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(500), ask)
            .await???
            .into_inner()
            .action,
        "allow"
    );
    tx.send(protocol::NotificationReply {
        id: command.id,
        ..Default::default()
    })
    .await?;
    tokio::time::timeout(Duration::from_secs(1), pending)
        .await??
        .map_err(anyhow::Error::msg)?;
    let accepted = firewall::read(&options.database, "alpha")?;
    assert!(!accepted.reported);
    assert_eq!(accepted.config, changed);
    assert!(
        backend
            .handle
            .control
            .submit_node(id, nodes::Change::Firewall(change))
            .is_err(),
        "another editor cannot replay an old snapshot"
    );
    let db = rusqlite::Connection::open(&options.database)?;
    db.execute_batch("CREATE TRIGGER fail_firewall_marker BEFORE UPDATE ON firewall_accepted BEGIN SELECT RAISE(ABORT,'synthetic cache failure'); END;")?;
    let mut next = changed.clone();
    next.system_rules[0].chains[0].rules[0].description = "new draft".into();
    let pending = backend
        .handle
        .control
        .submit_node(
            id,
            nodes::Change::Firewall(
                firewall::Change::new(accepted.fingerprint, next).map_err(anyhow::Error::msg)?,
            ),
        )
        .map_err(anyhow::Error::msg)?;
    let command = commands.message().await?.unwrap();
    tx.send(protocol::NotificationReply {
        id: command.id,
        ..Default::default()
    })
    .await?;
    assert!(pending.await?.is_err());
    assert_eq!(
        firewall::read(&options.database, "alpha")?.config,
        changed,
        "cache update and accepted marker roll back together"
    );
    assert!(backend.handle.history.health.catalogues.uncertain(id));
    alpha
        .subscribe(protocol::ClientConfig {
            name: "alpha".into(),
            system_firewall: Some(changed.clone()),
            config: r#"{"Firewall":"nftables"}"#.into(),
            ..Default::default()
        })
        .await?;
    backend.handle.history.flush().await?;
    let fresh = firewall::read(&options.database, "alpha")?;
    assert!(fresh.reported);
    assert_ne!(fresh.session, id);
    assert_eq!(fresh.config, changed);
    assert!(
        backend
            .handle
            .control
            .submit_node(
                id,
                nodes::Change::Firewall(
                    firewall::Change::new(fresh.fingerprint, initial.clone())
                        .map_err(anyhow::Error::msg)?
                )
            )
            .is_err()
    );
    let mut future = initial;
    future.version = 2;
    beta.subscribe(protocol::ClientConfig {
        name: "beta".into(),
        system_firewall: Some(future.clone()),
        config: r#"{"Firewall":"nftables"}"#.into(),
        ..Default::default()
    })
    .await?;
    let expected = firewall::fingerprint(&future.encode_to_vec());
    let mut replacement = future;
    replacement.version = 1;
    assert!(
        backend
            .handle
            .control
            .submit_node(
                backend.handle.node("beta").unwrap().id,
                nodes::Change::Firewall(
                    firewall::Change::new(expected, replacement).map_err(anyhow::Error::msg)?
                )
            )
            .is_err()
    );
    let legacy = protocol::SysFirewall {
        version: 1,
        ..Default::default()
    };
    for engine in ["iptables", "future-backend", ""] {
        beta.subscribe(protocol::ClientConfig {
            name: "beta".into(),
            config: serde_json::json!({"Firewall":engine}).to_string(),
            system_firewall: Some(legacy.clone()),
            ..Default::default()
        })
        .await?;
        let session = backend.handle.node("beta").unwrap().id;
        let digest = firewall::fingerprint(&legacy.encode_to_vec());
        assert!(!backend.handle.control.firewall_current(session, digest));
        let error = backend
            .handle
            .control
            .submit_node(
                session,
                nodes::Change::Firewall(
                    firewall::Change::new(digest, legacy.clone()).map_err(anyhow::Error::msg)?,
                ),
            )
            .err()
            .unwrap();
        assert!(error.contains("nftables"), "{error}");
    }
    let (ctx, crx) = tokio::sync::mpsc::channel(4);
    let mut fresh_commands = alpha
        .notifications(tokio_stream::wrappers::ReceiverStream::new(crx))
        .await?
        .into_inner();
    assert!(
        backend
            .handle
            .control
            .firewall_current(fresh.session, fresh.fingerprint)
    );
    let pending = backend
        .handle
        .control
        .submit_node(fresh.session, nodes::Change::Configuration("{}".into()))
        .map_err(anyhow::Error::msg)?;
    let command = fresh_commands.message().await?.unwrap();
    assert_eq!(command.r#type, 6);
    ctx.send(protocol::NotificationReply {
        id: command.id,
        ..Default::default()
    })
    .await?;
    pending.await?.map_err(anyhow::Error::msg)?;
    assert!(
        !backend
            .handle
            .control
            .firewall_current(fresh.session, fresh.fingerprint),
        "daemon configuration can replace the firewall source and requires a fresh report"
    );
    assert!(
        backend
            .handle
            .control
            .submit_node(
                fresh.session,
                nodes::Change::Firewall(
                    firewall::Change::new(fresh.fingerprint, fresh.config.clone())
                        .map_err(anyhow::Error::msg)?
                )
            )
            .is_err()
    );
    drop(fresh_commands);
    drop(ctx);
    drop(commands);
    drop(bcommands);
    drop(tx);
    drop(btx);
    backend.join();
    assert!(!options.socket.exists());
    Ok(())
}

#[test]
fn profile_files_roundtrip_deduplicate_and_remove_without_policy_guessing() -> Result<()> {
    use rustleblower::firewall::transfer;
    let directory = tempfile::tempdir()?;
    let mut doc = Document::from(config());
    let original = doc.clone();
    let mut profile = doc.chain(0, 0).unwrap().clone();
    profile.name = "profile display name; not the existing chain name".into();
    profile.table = "FILTER".into();
    profile.family = "INET".into();
    profile.kind = "FILTER".into();
    profile.hook = "INPUT".into();
    profile.policy = "drop".into();
    profile.priority = "999".into(); // A profile must not replace chain metadata.
    let duplicate = Rule {
        uuid: "different-id-same-complete-content".into(),
        ..profile.rules[0].clone()
    };
    let added = Rule {
        uuid: "profile-new".into(),
        description: "Owned profile rule".into(),
        ..profile.rules[0].clone()
    };
    profile.rules = vec![duplicate, added.clone()];
    let text = transfer::profile_json(&profile).map_err(anyhow::Error::msg)?;
    let envelope: serde_json::Value = serde_json::from_str(&text)?;
    assert_eq!(envelope["Name"], profile.name);
    assert!(
        envelope["Profile"].is_object(),
        "OpenSnitch predefined profile envelope"
    );
    assert_eq!(
        transfer::parse_profile(&serde_json::to_string(&profile)?).map_err(anyhow::Error::msg)?,
        profile,
        "bare FwChain remains accepted"
    );
    let with_metadata = serde_json::json!({"Name":"Display name", "Description":"Optional outer metadata", "Profile":profile});
    assert_eq!(
        transfer::parse_profile(&with_metadata.to_string()).map_err(anyhow::Error::msg)?,
        profile
    );

    let path = directory.path().join("owned.profile");
    transfer::write_new(&path, &text).map_err(anyhow::Error::msg)?;
    let bytes = std::fs::read(&path)?;
    assert!(transfer::write_new(&path, "replace existing").is_err());
    assert_eq!(std::fs::read(&path)?, bytes);
    let decoded = transfer::parse_profile(&transfer::read_text(&path).map_err(anyhow::Error::msg)?)
        .map_err(anyhow::Error::msg)?;
    assert_eq!(decoded, profile);
    doc.apply_profile(decoded.clone())
        .map_err(anyhow::Error::msg)?;
    let mut expected = original.clone();
    expected.chain_mut(0, 0).unwrap().policy = "drop".into();
    expected.chain_mut(0, 0).unwrap().rules.push(added);
    assert_eq!(
        doc, expected,
        "case-insensitive matching preserves chain name/priority/wrapper and skips semantic duplicates"
    );
    doc.apply_profile(decoded.clone())
        .map_err(anyhow::Error::msg)?;
    assert_eq!(doc, expected, "reapplying is idempotent");
    doc.remove_profile(&decoded).map_err(anyhow::Error::msg)?;
    expected.chain_mut(0, 0).unwrap().rules.pop();
    assert_eq!(
        doc, expected,
        "remove exact profile IDs, preserve policy and unrelated rule with another ID"
    );
    let unchanged = doc.clone();
    let mut wrong = decoded.clone();
    wrong.hook = "forward".into();
    assert!(doc.apply_profile(wrong.clone()).is_err());
    assert!(doc.remove_profile(&wrong).is_err());
    assert_eq!(doc, unchanged);
    let mut anonymous = decoded;
    anonymous.rules[0].uuid.clear();
    assert!(doc.remove_profile(&anonymous).is_err());
    assert_eq!(doc, unchanged);
    let config_path = directory.path().join("configuration.json");
    transfer::write_new(&config_path, &doc.json().map_err(anyhow::Error::msg)?)
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        Document::parse(&transfer::read_text(&config_path).map_err(anyhow::Error::msg)?)
            .map_err(anyhow::Error::msg)?,
        doc
    );
    assert_eq!(
        transfer::profiles(directory.path()).map_err(anyhow::Error::msg)?,
        vec![path]
    );
    Ok(())
}

#[test]
fn profile_and_file_limits_are_checked_before_draft_changes() -> Result<()> {
    use rustleblower::firewall::transfer;
    let dir = tempfile::tempdir()?;
    let oversized = dir.path().join("huge.json");
    std::fs::write(&oversized, "x".repeat(firewall::MAX_JSON + 1))?;
    assert!(transfer::read_text(&oversized).is_err());
    assert!(transfer::read_text(dir.path()).is_err());
    assert!(transfer::read_text(Path::new("nul\0path")).is_err());
    {
        use std::os::unix::ffi::OsStringExt;
        let invalid = std::ffi::OsString::from_vec(vec![0xff]);
        assert!(transfer::read_text(Path::new(&invalid)).is_err());
    }

    assert!(transfer::read_text(&dir.path().join("absent")).is_err());
    assert!(transfer::parse_profile("{\"Name\":\"x\",\"Unknown\":true}").is_err());
    assert!(
        transfer::parse_profile("{\"Name\":\"p\",\"Profile\":{\"Name\":\"x\",\"Unknown\":true}}")
            .is_err()
    );

    assert!(
        transfer::parse_profile(&format!(
            "{{\"Name\":\"x\",\"Table\":\"filter\",\"Family\":\"inet\",\"Rules\":[{}]}}",
            vec!["{}"; firewall::MAX_RULES + 1].join(",")
        ))
        .is_err()
    );
    let mut doc = Document::from(config());
    let before = doc.clone();
    let mut bad = doc.chain(0, 0).unwrap().clone();
    bad.policy = "drop".into();
    bad.rules = vec![Rule {
        uuid: "new-active-empty".into(),
        enabled: true,
        ..Default::default()
    }];
    assert!(doc.apply_profile(bad).is_err());
    assert_eq!(
        doc, before,
        "failed profile merge rolls back its policy too"
    );
    for i in 0..transfer::MAX_PROFILES {
        std::fs::write(dir.path().join(format!("{i:03}.profile")), "{}")?;
    }
    let list = transfer::profiles(dir.path()).map_err(anyhow::Error::msg)?;
    assert_eq!(list.len(), 256);
    assert!(list.windows(2).all(|v| v[0] < v[1]));
    std::fs::write(dir.path().join("overflow.profile"), "{}")?;
    assert!(transfer::profiles(dir.path()).is_err());
    Ok(())
}

#[test]
fn built_in_policies_preserve_selected_chain_metadata_and_prepare_exact_steps() -> Result<()> {
    use firewall::policy::{self, Plan, Policy};
    for table in ["filter", "opensnitch"] {
        let mut doc = Document::from(config());
        doc.chain_mut(0, 0).unwrap().table = table.into();
        let original = doc.clone();
        let original_chain = doc.chain(0, 0).unwrap().clone();
        policy::preset(&mut doc, 0, 0, Policy::Drop).map_err(anyhow::Error::msg)?;
        let chain = doc.chain(0, 0).unwrap();
        assert_eq!(
            (&chain.name, &chain.table, &chain.family, &chain.priority),
            (
                &original_chain.name,
                &original_chain.table,
                &original_chain.family,
                &original_chain.priority
            )
        );
        assert_eq!(&chain.rules[..2], &original_chain.rules);
        assert_eq!(chain.rules.len(), 4);
        assert_eq!(
            chain.rules[2].expressions[0]
                .statement
                .as_ref()
                .unwrap()
                .name,
            "iifname"
        );
        assert_eq!(
            chain.rules[3].expressions[0]
                .statement
                .as_ref()
                .unwrap()
                .values
                .iter()
                .map(|v| v.value.as_str())
                .collect::<Vec<_>>(),
            ["related", "established"]
        );
        let once = doc.clone();
        policy::preset(&mut doc, 0, 0, Policy::Drop).map_err(anyhow::Error::msg)?;
        assert_eq!(doc, once);
        policy::preset(&mut doc, 0, 0, Policy::Accept).map_err(anyhow::Error::msg)?;
        assert_eq!(doc, original);
    }
    let mut before = config();
    before.system_rules[0].chains[0].policy = "drop".into();
    for hook in ["output", "forward"] {
        let mut chain = before.system_rules[0].chains[0].clone();
        chain.name = hook.into();
        chain.table = "opensnitch".into();
        chain.hook = hook.into();
        chain.r#type = "mangle".into();
        chain.rules.clear();
        before.system_rules[0].chains.push(chain);
    }
    let mut regular = before.system_rules[0].chains[0].clone();
    regular.name = "regular".into();
    regular.hook.clear();
    regular.r#type.clear();
    before.system_rules[0].chains.push(regular);
    let mut doc = Document::from(before.clone());
    policy::preset(&mut doc, 0, 1, Policy::Drop).map_err(anyhow::Error::msg)?;
    assert_eq!(doc.chain(0, 1).unwrap().rules.len(), 1);
    assert_eq!(
        doc.chain(0, 1).unwrap().rules[0].expressions[0]
            .statement
            .as_ref()
            .unwrap()
            .values[0]
            .value,
        "established,related"
    );
    policy::preset(&mut doc, 0, 2, Policy::Drop).map_err(anyhow::Error::msg)?;
    assert!(doc.chain(0, 2).unwrap().rules.is_empty());
    let unchanged = doc.clone();
    assert!(policy::preset(&mut doc, 0, 3, Policy::Drop).is_err());
    assert_eq!(doc, unchanged);
    policy::enabled(&mut doc, false).map_err(anyhow::Error::msg)?;
    assert!(!doc.enabled);
    assert!(
        doc.system_rules[0].chains[..3]
            .iter()
            .all(|c| c.policy == "accept")
    );
    assert_eq!(doc.chain(0, 3).unwrap().policy, "drop");
    let expected = firewall::fingerprint(&before.encode_to_vec());
    let after = protocol::SysFirewall::from(doc.clone());
    let plan = Plan::new(expected, &before, after.clone()).map_err(anyhow::Error::msg)?;
    let steps = plan.configs().collect::<Vec<_>>();
    assert_eq!(steps.len(), 2);
    assert!(steps[0].enabled);
    assert!(!steps[1].enabled);
    assert_eq!(
        steps[0].system_rules[0].chains[0].rules, before.system_rules[0].chains[0].rules,
        "preparation never activates new unsaved rules"
    );
    assert!(steps[0].system_rules[0].chains[1].rules.is_empty());
    assert_eq!(steps[1], &after);
    let mut unsafe_after = before.clone();
    unsafe_after.enabled = false;
    assert!(Plan::new(expected, &before, unsafe_after).is_err());
    assert!(Plan::new([0; 32], &before, after.clone()).is_err());
    let mut enabling = after.clone();
    enabling.enabled = true;
    assert_eq!(
        Plan::new(
            firewall::fingerprint(&after.encode_to_vec()),
            &after,
            enabling
        )
        .map_err(anyhow::Error::msg)?
        .configs()
        .count(),
        1
    );
    let mut unknown = doc.clone();
    unknown.enabled = true;
    unknown.chain_mut(0, 0).unwrap().policy = "future-policy".into();
    let unchanged = unknown.clone();
    assert!(policy::enabled(&mut unknown, false).is_err());
    assert_eq!(unknown, unchanged);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sequenced_disable_waits_for_ack_and_cache_stops_on_failures_and_releases_its_lease()
-> Result<()> {
    use firewall::policy::{self, Plan};
    for scenario in [
        "success",
        "reject-preparation",
        "reject-disable",
        "cache-failure",
        "reconnect",
        "timeout",
    ] {
        let dir = tempfile::tempdir()?;
        let options = Config {
            transport: Default::default(),
            socket: dir.path().join("ui.sock"),
            database: dir.path().join("history.sqlite3"),
            prompt_timeout: 30,
            timeout_action: Decision::Deny,
            history_days: 7,
            history_mib: 16,
            headless: true,
        };
        let backend = Backend::start(options.clone())?;
        let mut initial = config();
        initial.system_rules[0].chains[0].policy = "drop".into();
        let mut daemon = client(&options.socket).await?;
        daemon
            .subscribe(protocol::ClientConfig {
                name: "sequence".into(),
                config: r#"{"Firewall":"nftables"}"#.into(),
                system_firewall: Some(initial.clone()),
                ..Default::default()
            })
            .await?;
        let session = backend.handle.node("sequence").unwrap().id;
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let mut commands = daemon
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await?
            .into_inner();
        backend.handle.history.flush().await?;
        let source = firewall::read(&options.database, "sequence")?;
        let mut draft = Document::from(initial.clone());
        policy::enabled(&mut draft, false).map_err(anyhow::Error::msg)?;
        let after = protocol::SysFirewall::from(draft);
        let make_plan =
            || Plan::new(source.fingerprint, &initial, after.clone()).map_err(anyhow::Error::msg);
        let operation = make_plan()?
            .begin(&backend.handle.control, session)
            .map_err(anyhow::Error::msg)?;
        let progress = operation.progress();
        let running = tokio::spawn(operation.run());
        let first = tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await??
            .unwrap();
        let mut prepared = initial.clone();
        prepared.system_rules[0].chains[0].policy = "accept".into();
        assert_eq!(first.sys_firewall, Some(prepared.clone()));
        assert_eq!(first.r#type, 5);
        assert!(first.data.is_empty() && first.rules.is_empty());
        assert!(
            make_plan()?
                .begin(&backend.handle.control, session)
                .is_err()
        );
        assert!(
            backend
                .handle
                .control
                .submit_node(session, nodes::Change::Interception(false))
                .is_err()
        );
        assert!(backend.handle.refresh_rules(session).is_err());
        assert!(
            tokio::time::timeout(Duration::from_millis(25), commands.message())
                .await
                .is_err()
        );
        assert_eq!(
            firewall::read(&options.database, "sequence")?.config,
            initial
        );
        let mut asking = daemon.clone();
        let ask = tokio::spawn(async move {
            asking
                .ask_rule(protocol::Connection {
                    process_path: "/usr/bin/sequence-permission".into(),
                    ..Default::default()
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(p) = backend.handle.prompts.snapshot().first() {
                    backend.handle.decide(p.id, Decision::Allow);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await?;
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), ask)
                .await???
                .into_inner()
                .action,
            "allow"
        );
        if scenario == "cache-failure" {
            let db = rusqlite::Connection::open(&options.database)?;
            db.execute_batch("CREATE TRIGGER fail_owned_stage BEFORE UPDATE OF firewall ON node_info BEGIN SELECT RAISE(FAIL,'owned stage cache failure'); END;")?;
        }
        if scenario == "reconnect" {
            let mut replacement = client(&options.socket).await?;
            replacement
                .subscribe(protocol::ClientConfig {
                    name: "sequence".into(),
                    config: r#"{"Firewall":"nftables"}"#.into(),
                    system_firewall: Some(initial.clone()),
                    ..Default::default()
                })
                .await?;
        } else if scenario != "timeout" {
            tx.send(protocol::NotificationReply {
                id: first.id,
                code: if scenario == "reject-preparation" {
                    1
                } else {
                    0
                },
                data: if scenario == "reject-preparation" {
                    "fixture preparation rejection".into()
                } else {
                    String::new()
                },
            })
            .await?;
        }
        if matches!(scenario, "success" | "reject-disable") {
            let second = tokio::time::timeout(Duration::from_secs(1), commands.message())
                .await??
                .unwrap();
            assert_eq!(second.r#type, 5);
            assert_eq!(second.sys_firewall, Some(after.clone()));
            assert_eq!(
                firewall::read(&options.database, "sequence")?.config,
                prepared
            );
            assert_eq!(progress.lock().unwrap().completed, 1);
            assert!(backend.handle.control.firewall_busy(session));
            tx.send(protocol::NotificationReply {
                id: second.id,
                code: if scenario == "reject-disable" { 1 } else { 0 },
                data: if scenario == "reject-disable" {
                    "fixture disable rejection".into()
                } else {
                    String::new()
                },
            })
            .await?;
        }
        tokio::time::timeout(Duration::from_secs(13), running).await??;
        let done = progress.lock().unwrap().clone();
        assert!(done.outcome.is_some());
        assert!(!backend.handle.control.firewall_busy(session));
        if scenario == "success" {
            assert_eq!(done.outcome, Some(Ok(2)));
            assert_eq!(firewall::read(&options.database, "sequence")?.config, after);
            let mut enabled = after.clone();
            enabled.enabled = true;
            let plan = Plan::new(
                firewall::fingerprint(&after.encode_to_vec()),
                &after,
                enabled.clone(),
            )
            .map_err(anyhow::Error::msg)?;
            let op = plan
                .begin(&backend.handle.control, session)
                .map_err(anyhow::Error::msg)?;
            let progress = op.progress();
            let running = tokio::spawn(op.run());
            let command = tokio::time::timeout(Duration::from_secs(1), commands.message())
                .await??
                .unwrap();
            assert_eq!(command.sys_firewall, Some(enabled.clone()));
            tx.send(protocol::NotificationReply {
                id: command.id,
                ..Default::default()
            })
            .await?;
            running.await?;
            assert_eq!(progress.lock().unwrap().outcome, Some(Ok(1)));
            assert_eq!(
                firewall::read(&options.database, "sequence")?.config,
                enabled
            );
        } else {
            assert!(done.outcome.unwrap().is_err());
            assert_eq!(done.completed, usize::from(scenario == "reject-disable"));
            assert_eq!(
                firewall::read(&options.database, "sequence")?.config,
                if scenario == "reject-disable" {
                    prepared
                } else {
                    initial
                }
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(25), commands.message())
                    .await
                    .is_err(),
                "no unreviewed continuation after {scenario}"
            );
        }
        drop(commands);
        drop(tx);
        drop(daemon);
        backend.join();
        assert!(!options.socket.exists());
    }
    Ok(())
}
