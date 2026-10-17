use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::firewall::{
        self,
        document::{Document, Rule as FwRule},
    };
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let service_path = dir.path().join("owned-services");
    let interface_path = dir.path().join("owned-interfaces");
    std::fs::create_dir(&interface_path).unwrap();
    std::fs::create_dir(interface_path.join("owned0")).unwrap();
    std::fs::write(
        &service_path,
        b"owned-web 4443/tcp ownedalias\nowned-web 5553/udp ownedalias\n",
    )
    .unwrap();
    let _discovery = crate::firewall::suggestions::discovery_fixture(
        service_path.clone(),
        interface_path.clone(),
    );
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let initial = protocol::SysFirewall {
        enabled: true,
        version: 1,
        system_rules: vec![protocol::FwChains {
            rule: Some(protocol::FwRule {
                parameters: "preserved-wrapper".into(),
                position: u64::MAX,
                ..Default::default()
            }),
            chains: vec![protocol::FwChain {
                name: "input".into(),
                table: "filter".into(),
                family: "inet".into(),
                r#type: "filter".into(),
                hook: "input".into(),
                policy: "accept".into(),
                rules: vec![
                    protocol::FwRule {
                        uuid: "existing-rule".into(),
                        enabled: true,
                        target: "accept".into(),
                        description: "Keep this rule unchanged".into(),
                        ..Default::default()
                    },
                    protocol::FwRule {
                        uuid: "budget-rule".into(),
                        enabled: true,
                        target: "accept".into(),
                        description: "Large synthetic value".into(),
                        expressions: vec![protocol::Expressions {
                            statement: Some(protocol::Statement {
                                name: "fixture".into(),
                                values: vec![protocol::StatementValues {
                                    key: "blob".into(),
                                    value: "x".repeat(crate::nodes::FIREWALL_BYTES - 6000),
                                }],
                                ..Default::default()
                            }),
                        }],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
        }],
    };
    assert!(prost::Message::encoded_len(&initial) < crate::nodes::FIREWALL_BYTES);
    let (mut daemon, replies, mut commands) = runtime.block_on(async {
        let path = config.socket.clone();
        let channel = tonic::transport::Endpoint::try_from("http://localhost")
            .unwrap()
            .connect_with_connector(tower::service_fn(move |_| {
                let path = path.clone();
                async move {
                    tokio::net::UnixStream::connect(path)
                        .await
                        .map(hyper_util::rt::TokioIo::new)
                }
            }))
            .await
            .unwrap();
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "forms-fixture".into(),
                config: r#"{"Firewall":"nftables"}"#.into(),
                system_firewall: Some(initial.clone()),
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let commands = daemon
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        (daemon, tx, commands)
    });
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context(test_cx);
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(crate::app::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-4", cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 0usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window, "nodes-firewall");
    flush(&mut cx);
    let editor = cx
        .update(|cx| cx.windows())
        .into_iter()
        .find(|w| !before.contains(w))
        .unwrap();
    let copy = |cx: &mut TestAppContext| {
        click(cx, editor, "firewall-copy");
        Document::parse(&cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())).unwrap()
    };
    let row = |cx: &mut TestAppContext, id: &'static str, index: usize| {
        cx.update_window(editor, |_, window, cx| window.click((id, index), cx))
            .unwrap();
        cx.run_until_parked();
    };
    let chain = |cx: &mut TestAppContext, index: u64| {
        cx.update_window(editor, |_, window, cx| {
            window.click(("firewall-chain", index), cx)
        })
        .unwrap();
        cx.run_until_parked();
    };
    let rule_tab = |cx: &mut TestAppContext, index: usize| {
        cx.update_window(editor, |_, window, cx| {
            window.within("fw-rule-tabs").click(index, cx)
        })
        .unwrap();
        cx.run_until_parked();
    };

    click(&mut cx, editor, "fw-chain-edit");
    flush(&mut cx);
    fill(&mut cx, editor, "fw-chain-name", "input-reviewed");
    fill(&mut cx, editor, "fw-chain-policy", "drop");
    checkpoint(&mut cx, editor, &evidence.join("01-chain-form.json"));
    click(&mut cx, editor, "fw-form-save");
    flush(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "forms-fixture")
            .unwrap()
            .config,
        initial
    );
    assert_eq!(copy(&mut cx).chain(0, 0).unwrap().rules.len(), 2);
    click(&mut cx, editor, "fw-chain-new");
    flush(&mut cx);
    click(&mut cx, editor, "fw-form-save");
    flush(&mut cx); // Missing name is invalid and remains in the form.
    fill(&mut cx, editor, "fw-chain-name", "owned-output");
    fill(&mut cx, editor, "fw-chain-hook", "output");
    click(&mut cx, editor, "fw-form-save");
    flush(&mut cx);
    assert_eq!(copy(&mut cx).system_rules[0].chains.len(), 2);
    checkpoint(&mut cx, editor, &evidence.join("02-created-chain.json"));

    chain(&mut cx, 0);
    row(&mut cx, "firewall-rule", 1);
    click(&mut cx, editor, "fw-rule-edit");
    flush(&mut cx);
    rule_tab(&mut cx, 1);
    row(&mut cx, "fw-expression-row", 0);
    click(&mut cx, editor, "fw-expression-edit");
    click(&mut cx, editor, "fw-value-new");
    fill(&mut cx, editor, "fw-value-key", "extra");
    click(&mut cx, editor, "fw-value-value");
    cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("y".repeat(8192))));
    cx.update_window(editor, |_, window, cx| window.press("ctrl-v", cx))
        .unwrap();
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("03-value-budget-rejected.json"),
    );
    click(&mut cx, editor, "fw-value-cancel");
    click(&mut cx, editor, "fw-statement-cancel");
    click(&mut cx, editor, "fw-form-cancel");
    assert_eq!(
        copy(&mut cx).chain(0, 0).unwrap().rules[1],
        FwRule::from(initial.system_rules[0].chains[0].rules[1].clone())
    );

    chain(&mut cx, 1);
    click(&mut cx, editor, "fw-rule-new");
    flush(&mut cx);
    fill(&mut cx, editor, "fw-rule-id", "native-owned-rule");
    fill(
        &mut cx,
        editor,
        "fw-rule-description",
        "Reviewed native rule",
    );
    fill(&mut cx, editor, "fw-rule-target", "reject");
    fill(
        &mut cx,
        editor,
        "fw-rule-target-parameters",
        "with tcp reset",
    );
    fill(&mut cx, editor, "fw-rule-position", "18446744073709551615");
    click(&mut cx, editor, "fw-rule-enabled");
    rule_tab(&mut cx, 2);
    fill(&mut cx, editor, "fw-rule-table", "legacy-table");
    fill(&mut cx, editor, "fw-rule-chain", "legacy-chain");
    fill(&mut cx, editor, "fw-rule-parameters", "-p tcp");
    rule_tab(&mut cx, 1);
    click(&mut cx, editor, "fw-expression-new");
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    checkpoint(&mut cx, editor, &evidence.join("04-invalid-statement.json"));
    fill(&mut cx, editor, "fw-statement-name", "tcp");
    fill(&mut cx, editor, "fw-statement-op", "==");
    click(&mut cx, editor, "fw-value-new");
    fill(&mut cx, editor, "fw-value-key", "dport");
    fill(&mut cx, editor, "fw-value-value", "443");
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    click(&mut cx, editor, "fw-value-new");
    fill(&mut cx, editor, "fw-value-key", "sport");
    fill(&mut cx, editor, "fw-value-value", "1000-2000");
    click(&mut cx, editor, "fw-value-save");
    row(&mut cx, "fw-value-row", 0);
    click(&mut cx, editor, "fw-value-edit");
    fill(&mut cx, editor, "fw-value-value", "8443");
    click(&mut cx, editor, "fw-value-save");
    click(&mut cx, editor, "fw-value-down");
    click(&mut cx, editor, "fw-value-up");
    row(&mut cx, "fw-value-row", 1);
    click(&mut cx, editor, "fw-value-delete");
    click(&mut cx, editor, "fw-value-new");
    fill(&mut cx, editor, "fw-value-key", "cancelled");
    click(&mut cx, editor, "fw-value-cancel");
    checkpoint(&mut cx, editor, &evidence.join("05-statement-values.json"));
    click(&mut cx, editor, "fw-statement-save");
    click(&mut cx, editor, "fw-expression-new");
    fill(&mut cx, editor, "fw-statement-name", "counter");
    click(&mut cx, editor, "fw-statement-save");
    row(&mut cx, "fw-expression-row", 1);
    click(&mut cx, editor, "fw-expression-up");
    click(&mut cx, editor, "fw-expression-down");
    click(&mut cx, editor, "fw-expression-delete");
    click(&mut cx, editor, "fw-expression-new");
    fill(&mut cx, editor, "fw-statement-name", "cancelled");
    click(&mut cx, editor, "fw-statement-cancel");
    row(&mut cx, "fw-expression-row", 0);
    click(&mut cx, editor, "fw-expression-edit");
    fill(&mut cx, editor, "fw-statement-op", "!=");
    fill(&mut cx, editor, "fw-statement-op", "==");
    click(&mut cx, editor, "fw-statement-save");
    click(&mut cx, editor, "fw-expression-guided");
    flush(&mut cx);
    fill(&mut cx, editor, "fw-helper-value", "65536");
    click(&mut cx, editor, "fw-helper-keep");
    flush(&mut cx);
    assert!(
        cx.update_window(editor, |_, w, _| w.find("fw-helper-keep").visible())
            .unwrap()
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("14-guided-port-refused.json"),
    );
    pick(&mut cx, editor, "fw-helper-kind", "Destination port");
    pick(&mut cx, editor, "fw-helper-option", "tcp,udp");
    pick(&mut cx, editor, "fw-helper-operator", "!=");
    fill(&mut cx, editor, "fw-helper-value", "443,8443");
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("15-guided-dual-protocol.json"),
    );
    cx.update_window(editor, |_, w, cx| w.press("ctrl-enter", cx))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, editor, "fw-expression-guided");
    flush(&mut cx);
    pick(&mut cx, editor, "fw-helper-kind", "Destination address");
    pick(&mut cx, editor, "fw-helper-option", "ip6");
    fill(&mut cx, editor, "fw-helper-value", "2001:db8::/126");
    checkpoint(&mut cx, editor, &evidence.join("16-guided-ipv6-cidr.json"));
    click(&mut cx, editor, "fw-helper-keep");
    flush(&mut cx);
    click(&mut cx, editor, "fw-expression-guided");
    flush(&mut cx);
    pick(&mut cx, editor, "fw-helper-kind", "Named counter");
    fill(
        &mut cx,
        editor,
        "fw-helper-value",
        "cancelled-guided-counter",
    );
    click(&mut cx, editor, "fw-helper-cancel");
    click(&mut cx, editor, "fw-expression-guided");
    flush(&mut cx);
    pick(&mut cx, editor, "fw-helper-option", "tcp,udp");
    fill(&mut cx, editor, "fw-helper-value", "ownedalias");
    click(&mut cx, editor, "fw-helper-keep");
    flush(&mut cx);
    assert!(
        cx.update_window(editor, |_, w, _| w.try_find("fw-helper-value").is_some())
            .unwrap(),
        "Protocol-ambiguous service alias must retain helper"
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("19-service-protocol-ambiguity-refused.json"),
    );
    pick(&mut cx, editor, "fw-helper-service", "owned-web — 5553/udp");
    click(&mut cx, editor, "fw-helper-use-service");
    flush(&mut cx);
    assert_eq!(
        cx.update_window(editor, |_, w, _| w
            .find("fw-helper-value")
            .value()
            .unwrap()
            .to_string())
            .unwrap(),
        "5553"
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("20-service-exact-port-protocol.json"),
    );
    click(&mut cx, editor, "fw-helper-keep");
    flush(&mut cx);
    click(&mut cx, editor, "fw-expression-guided");
    flush(&mut cx);
    pick(&mut cx, editor, "fw-helper-kind", "Input interface");
    pick(&mut cx, editor, "fw-helper-interface", "owned0");
    click(&mut cx, editor, "fw-helper-use-interface");
    flush(&mut cx);
    assert_eq!(
        cx.update_window(editor, |_, w, _| w
            .find("fw-helper-value")
            .value()
            .unwrap()
            .to_string())
            .unwrap(),
        "owned0"
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("21-local-interface-suggestion.json"),
    );
    click(&mut cx, editor, "fw-helper-keep");
    flush(&mut cx);
    click(&mut cx, editor, "fw-expression-guided");
    flush(&mut cx);
    fill(&mut cx, editor, "fw-helper-value", "1234");
    std::fs::write(&service_path, b"owned-new 9443/tcp\n").unwrap();
    click(&mut cx, editor, "fw-helper-reload-suggestions");
    flush(&mut cx);
    assert_eq!(
        cx.update_window(editor, |_, w, _| w
            .find("fw-helper-value")
            .value()
            .unwrap()
            .to_string())
            .unwrap(),
        "1234",
        "Reload must preserve manually staged value"
    );
    for _ in 0..10 {
        pick(&mut cx, editor, "fw-helper-service", "owned-new — 9443/tcp");
        if cx
            .update_window(editor, |_, w, _| {
                w.find("fw-helper-use-service").disabled() != Some(true)
            })
            .unwrap()
        {
            break;
        }
        flush(&mut cx);
    }
    click(&mut cx, editor, "fw-helper-use-service");
    flush(&mut cx);
    assert_eq!(
        cx.update_window(editor, |_, w, _| w
            .find("fw-helper-value")
            .value()
            .unwrap()
            .to_string())
            .unwrap(),
        "9443"
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("22-reloaded-owned-suggestions.json"),
    );
    click(&mut cx, editor, "fw-helper-cancel");
    rule_tab(&mut cx, 0);
    click(&mut cx, editor, "fw-verdict-guided");
    flush(&mut cx);
    pick(&mut cx, editor, "fw-helper-verdict", "snat");
    fill(&mut cx, editor, "fw-helper-verdict-value", "192.0.2.1:53");
    click(&mut cx, editor, "fw-helper-keep");
    flush(&mut cx);
    assert!(
        cx.update_window(editor, |_, w, _| w.find("fw-helper-keep").visible())
            .unwrap(),
        "SNAT refuses the output hook and retains its helper"
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("17-guided-verdict-hook-refused.json"),
    );
    click(&mut cx, editor, "fw-helper-cancel");
    click(&mut cx, editor, "fw-verdict-guided");
    flush(&mut cx);
    pick(&mut cx, editor, "fw-helper-verdict", "queue");
    fill(&mut cx, editor, "fw-helper-verdict-value", "12");
    click(&mut cx, editor, "fw-helper-keep");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("18-guided-queue-draft.json"),
    );
    rule_tab(&mut cx, 1);
    click(&mut cx, editor, "fw-form-save");
    flush(&mut cx);
    let doc = copy(&mut cx);
    let native = &doc.chain(0, 1).unwrap().rules[0];
    assert_eq!(native.uuid, "native-owned-rule");
    assert_eq!(native.position, u64::MAX);
    assert!(!native.enabled);
    assert_eq!(
        native.expressions.len(),
        5,
        "Cancelled helper cannot add an expression"
    );
    assert_eq!(
        native.expressions[0].statement.as_ref().unwrap().values[0].value,
        "8443"
    );
    checkpoint(&mut cx, editor, &evidence.join("06-created-rule.json"));
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-toggle");
    flush(&mut cx);
    assert!(copy(&mut cx).chain(0, 1).unwrap().rules[0].enabled);
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-duplicate");
    flush(&mut cx);
    click(&mut cx, editor, "fw-form-cancel");
    assert_eq!(copy(&mut cx).chain(0, 1).unwrap().rules.len(), 1);
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-duplicate");
    flush(&mut cx);
    fill(&mut cx, editor, "fw-rule-description", "duplicate draft");
    click(&mut cx, editor, "fw-form-save");
    flush(&mut cx);
    let doc = copy(&mut cx);
    assert_ne!(
        doc.chain(0, 1).unwrap().rules[0].uuid,
        doc.chain(0, 1).unwrap().rules[1].uuid
    );
    row(&mut cx, "firewall-rule", 1);
    click(&mut cx, editor, "fw-rule-up");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("07-duplicate-and-order.json"),
    );
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-delete");
    click(&mut cx, editor, "fw-delete-cancel");
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-delete");
    checkpoint(&mut cx, editor, &evidence.join("08-removal-review.json"));
    click(&mut cx, editor, "fw-delete-confirm");
    flush(&mut cx);
    click(&mut cx, editor, "fw-chain-new");
    flush(&mut cx);
    fill(&mut cx, editor, "fw-chain-name", "temporary-chain");
    click(&mut cx, editor, "fw-form-save");
    flush(&mut cx);
    click(&mut cx, editor, "fw-rule-new");
    flush(&mut cx);
    click(&mut cx, editor, "fw-form-save");
    flush(&mut cx); // Enabled empty rules are not silently accepted.
    rule_tab(&mut cx, 1);
    click(&mut cx, editor, "fw-expression-new");
    fill(&mut cx, editor, "fw-statement-name", "tcp");
    fill(&mut cx, editor, "fw-statement-op", "==");
    click(&mut cx, editor, "fw-value-new");
    fill(&mut cx, editor, "fw-value-key", "dport");
    fill(&mut cx, editor, "fw-value-value", "80");
    click(&mut cx, editor, "fw-value-save");
    click(&mut cx, editor, "fw-statement-save");
    click(&mut cx, editor, "fw-form-save");
    flush(&mut cx);
    assert_eq!(
        copy(&mut cx).chain(0, 2).unwrap().rules[0].position,
        0,
        "new insertion metadata defaults to zero, not an array index"
    );
    click(&mut cx, editor, "fw-chain-delete");
    click(&mut cx, editor, "fw-delete-cancel");
    click(&mut cx, editor, "fw-chain-delete");
    click(&mut cx, editor, "fw-delete-confirm");
    flush(&mut cx);
    let draft = copy(&mut cx);
    assert_eq!(draft.system_rules[0].chains.len(), 2);
    assert_eq!(
        draft.system_rules[0].rule,
        Some(FwRule::from(initial.system_rules[0].rule.clone().unwrap()))
    );
    let mut expected = initial.clone();
    expected.system_rules[0].chains[0].name = "input-reviewed".into();
    expected.system_rules[0].chains[0].policy = "drop".into();
    expected.system_rules[0].chains.push(protocol::FwChain {
        name: "owned-output".into(),
        table: "filter".into(),
        family: "inet".into(),
        r#type: "filter".into(),
        hook: "output".into(),
        policy: "accept".into(),
        rules: vec![protocol::FwRule {
            uuid: "native-owned-rule".into(),
            description: "Reviewed native rule".into(),
            target: "reject".into(),
            target_parameters: "with tcp reset".into(),
            position: u64::MAX,
            enabled: true,
            table: "legacy-table".into(),
            chain: "legacy-chain".into(),
            parameters: "-p tcp".into(),
            expressions: vec![protocol::Expressions {
                statement: Some(protocol::Statement {
                    name: "tcp".into(),
                    op: "==".into(),
                    values: vec![protocol::StatementValues {
                        key: "dport".into(),
                        value: "8443".into(),
                    }],
                }),
            }],
        }],
        ..Default::default()
    });
    let guided_rule = &mut expected.system_rules[0].chains[1].rules[0];
    guided_rule.target = "queue".into();
    guided_rule.target_parameters = "num 12".into();
    guided_rule.expressions.extend([
        protocol::Expressions {
            statement: Some(protocol::Statement {
                name: "meta".into(),
                op: "!=".into(),
                values: vec![
                    protocol::StatementValues {
                        key: "l4proto".into(),
                        value: "tcp,udp".into(),
                    },
                    protocol::StatementValues {
                        key: "dport".into(),
                        value: "443,8443".into(),
                    },
                ],
            }),
        },
        protocol::Expressions {
            statement: Some(protocol::Statement {
                name: "ip6".into(),
                op: "==".into(),
                values: vec![protocol::StatementValues {
                    key: "daddr".into(),
                    value: "2001:db8::-2001:db8::3".into(),
                }],
            }),
        },
    ]);
    guided_rule.expressions.extend([
        protocol::Expressions {
            statement: Some(protocol::Statement {
                name: "udp".into(),
                op: "==".into(),
                values: vec![protocol::StatementValues {
                    key: "dport".into(),
                    value: "5553".into(),
                }],
            }),
        },
        protocol::Expressions {
            statement: Some(protocol::Statement {
                name: "iifname".into(),
                op: "==".into(),
                values: vec![protocol::StatementValues {
                    key: "owned0".into(),
                    value: String::new(),
                }],
            }),
        },
    ]);
    assert_eq!(protocol::SysFirewall::from(draft), expected);
    assert_eq!(
        firewall::read(&config.database, "forms-fixture")
            .unwrap()
            .config,
        initial
    );
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), commands.message()).await
            })
            .is_err()
    );
    checkpoint(&mut cx, editor, &evidence.join("09-staged-forms.json"));
    click(&mut cx, editor, "fw-draft-review");
    flush(&mut cx);
    checkpoint(&mut cx, editor, &evidence.join("10-complete-review.json"));
    click(&mut cx, editor, "firewall-confirm");
    flush(&mut cx);
    let command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(command.r#type, 5);
    assert_eq!(command.sys_firewall, Some(expected.clone()));
    let mut asking = daemon.clone();
    let asking = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/forms-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    let popup = permission_window(&mut cx, &view);
    checkpoint(
        &mut cx,
        popup,
        &evidence.join("11-permission-under-form-ack.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(asking).unwrap().duration, "once");
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: command.id,
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx);
    wait_for(&mut cx, |_| {
        firewall::read(&config.database, "forms-fixture")
            .unwrap()
            .config
            == expected
    });
    assert_eq!(
        firewall::read(&config.database, "forms-fixture")
            .unwrap()
            .config,
        expected
    );
    wait_window(&mut cx, editor, |window, cx| {
        crate::app::firewall_view::test_idle(window, cx)
    });
    checkpoint(&mut cx, editor, &evidence.join("12-accepted-forms.json"));
    chain(&mut cx, 1);
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-edit");
    flush(&mut cx);
    fill(
        &mut cx,
        editor,
        "fw-rule-description",
        "Kept after reconnect",
    );
    runtime
        .block_on(daemon.subscribe(protocol::ClientConfig {
            name: "forms-fixture".into(),
            config: r#"{"Firewall":"nftables"}"#.into(),
            system_firewall: Some(expected.clone()),
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, editor, "fw-form-save");
    flush(&mut cx);
    assert_eq!(
        copy(&mut cx).chain(0, 1).unwrap().rules[0].description,
        "Kept after reconnect"
    );
    wait_for(&mut cx, |_| {
        firewall::read(&config.database, "forms-fixture")
            .unwrap()
            .config
            == expected
    });
    assert_eq!(
        firewall::read(&config.database, "forms-fixture")
            .unwrap()
            .config,
        expected
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("13-stale-form-kept-locally.json"),
    );
    click(&mut cx, editor, "firewall-close");
    drop(commands);
    drop(replies);
    drop(daemon);
    drop(view);
    shutdown(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-chain-form.json","02-created-chain.json","03-value-budget-rejected.json","04-invalid-statement.json","05-statement-values.json","06-created-rule.json","07-duplicate-and-order.json","08-removal-review.json","09-staged-forms.json","10-complete-review.json","11-permission-under-form-ack.json","12-accepted-forms.json","13-stale-form-kept-locally.json","14-guided-port-refused.json","15-guided-dual-protocol.json","16-guided-ipv6-cidr.json","17-guided-verdict-hook-refused.json","18-guided-queue-draft.json","19-service-protocol-ambiguity-refused.json","20-service-exact-port-protocol.json","21-local-interface-suggestion.json","22-reloaded-owned-suggestions.json"],"proof_boundary":"Actual native Kit chain/rule/expression/value forms and guided statement/verdict controls, invalid-port and hook refusal, exact dual-protocol/IPv6 CIDR/queue wire values, helper cancellation, actual bounded local service/interface suggestion pick/use/reload from owned discovery files, service protocol ambiguity refusal and exact numeric UDP/interface wire, invalid/over-budget preservation, local staging, duplicate/order/toggle/removal and cancellation, exact full gRPC snapshot, permission under held ACK and stale-form local recovery after reconnect. Near-cap synthetic wire value; no real firewall enforcement or native compositor proof. This journey does not cover file/profile controls, built-in policy presets or enable/disable."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Edit chain metadata and policy; reject empty new chain then add output chain","Edit near-cap synthetic statement; reject additional 8 KiB value and cancel without changing draft","Create complete rule with uint64 max, target parameters, enabled checkbox and legacy fields","Add/edit/delete/reorder/cancel statements and values through bounded native controls","Toggle rule, cancel/keep duplicate, move it and cancel/confirm removal; create/remove temporary chain","Require no daemon command before global review; require exact full protobuf and independent permission under ACK","ACK persists accepted snapshot; reconnect during a form still allows local keep/copy without applying to replacement session"]).unwrap()).unwrap();
}
