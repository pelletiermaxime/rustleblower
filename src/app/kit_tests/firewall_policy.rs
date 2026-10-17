use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::firewall::{self, document::Document};
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mut initial = protocol::SysFirewall {
        enabled: true,
        version: 1,
        system_rules: vec![protocol::FwChains {
            rule: Some(protocol::FwRule {
                parameters: "retained wrapper".into(),
                position: u64::MAX,
                ..Default::default()
            }),
            chains: vec![protocol::FwChain {
                name: "input".into(),
                table: "opensnitch".into(),
                family: "inet".into(),
                r#type: "filter".into(),
                hook: "input".into(),
                priority: "-10".into(),
                policy: "drop".into(),
                rules: vec![protocol::FwRule {
                    uuid: "owned-user-rule".into(),
                    enabled: true,
                    position: u64::MAX,
                    description: "Retain this user rule".into(),
                    target: "accept".into(),
                    expressions: vec![protocol::Expressions {
                        statement: Some(protocol::Statement {
                            name: "tcp".into(),
                            op: "==".into(),
                            values: vec![protocol::StatementValues {
                                key: "dport".into(),
                                value: "443".into(),
                            }],
                        }),
                    }],
                    ..Default::default()
                }],
            }],
        }],
    };
    for hook in ["output", "forward"] {
        initial.system_rules[0].chains.push(protocol::FwChain {
            name: hook.into(),
            table: "opensnitch".into(),
            family: "inet".into(),
            r#type: "mangle".into(),
            hook: hook.into(),
            policy: "drop".into(),
            ..Default::default()
        });
    }
    initial.system_rules[0].chains.push(protocol::FwChain {
        name: "regular".into(),
        table: "custom".into(),
        family: "inet".into(),
        policy: "drop".into(),
        ..Default::default()
    });
    let (daemon, replies, mut commands) = runtime.block_on(async {
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
                name: "policy-fixture".into(),
                version: "1.9.0-synthetic".into(),
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
    let open_editor = |cx: &mut TestAppContext| {
        let before = cx.update(|cx| cx.windows());
        click(cx, window, "nodes-firewall");
        flush(cx);
        cx.update(|cx| cx.windows())
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    };
    let editor = open_editor(&mut cx);
    cx.update_window(editor, |_, window, cx| window.press("alt-p", cx))
        .unwrap();
    flush(&mut cx);
    let select = |cx: &mut TestAppContext, index: u64| {
        cx.update_window(editor, |_, window, cx| {
            window.click(("fw-policy-chain", index), cx)
        })
        .unwrap();
        flush(cx);
    };
    let copy = |cx: &mut TestAppContext| {
        click(cx, editor, "firewall-copy");
        Document::parse(&cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())).unwrap()
    };
    checkpoint(&mut cx, editor, &evidence.join("01-policy-controls.json"));
    click(&mut cx, editor, "fw-policy-accept");
    flush(&mut cx);
    let doc = copy(&mut cx);
    assert_eq!(doc.chain(0, 0).unwrap().policy, "accept");
    assert_eq!(doc.chain(0, 0).unwrap().rules.len(), 1);
    click(&mut cx, editor, "fw-policy-drop");
    flush(&mut cx);
    let input = copy(&mut cx);
    assert_eq!(input.chain(0, 0).unwrap().rules.len(), 3);
    assert_eq!(
        input.chain(0, 0).unwrap().rules[0],
        Document::from(initial.clone()).chain(0, 0).unwrap().rules[0]
    );
    click(&mut cx, editor, "fw-policy-drop");
    flush(&mut cx);
    assert_eq!(copy(&mut cx), input);
    checkpoint(&mut cx, editor, &evidence.join("02-input-preset.json"));
    select(&mut cx, 1);
    click(&mut cx, editor, "fw-policy-drop");
    flush(&mut cx);
    assert_eq!(copy(&mut cx).chain(0, 1).unwrap().rules.len(), 1);
    click(&mut cx, editor, "fw-policy-accept");
    flush(&mut cx);
    assert!(copy(&mut cx).chain(0, 1).unwrap().rules.is_empty());
    click(&mut cx, editor, "fw-policy-drop");
    flush(&mut cx);
    select(&mut cx, 2);
    click(&mut cx, editor, "fw-policy-accept");
    flush(&mut cx);
    assert_eq!(copy(&mut cx).chain(0, 2).unwrap().policy, "accept");
    click(&mut cx, editor, "fw-policy-drop");
    flush(&mut cx);
    let before = copy(&mut cx);
    select(&mut cx, 3);
    click(&mut cx, editor, "fw-policy-drop");
    flush(&mut cx);
    assert_eq!(copy(&mut cx), before);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("03-unsupported-preset-retained.json"),
    );
    select(&mut cx, 0);
    click(&mut cx, editor, "fw-system-enabled");
    flush(&mut cx);
    let disabled = copy(&mut cx);
    assert!(!disabled.enabled);
    assert!(
        disabled.system_rules[0].chains[..3]
            .iter()
            .all(|c| c.policy == "accept")
    );
    assert_eq!(disabled.chain(0, 3).unwrap().policy, "drop");
    assert_eq!(
        firewall::read(&config.database, "policy-fixture")
            .unwrap()
            .config,
        initial
    );
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(25), commands.message()).await
            })
            .is_err()
    );
    checkpoint(&mut cx, editor, &evidence.join("04-staged-disable.json"));
    click(&mut cx, editor, "fw-policy-review");
    flush(&mut cx);
    checkpoint(&mut cx, editor, &evidence.join("05-two-stage-review.json"));
    click(&mut cx, editor, "firewall-back");
    flush(&mut cx);
    assert_eq!(copy(&mut cx), disabled);
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, editor, "firewall-confirm");
    flush(&mut cx);
    let first = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    let mut preparation = initial.clone();
    for chain in &mut preparation.system_rules[0].chains[..3] {
        chain.policy = "accept".into();
    }
    assert_eq!(first.r#type, 5);
    assert_eq!(first.sys_firewall, Some(preparation.clone()));
    assert!(first.data.is_empty() && first.rules.is_empty());
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(25), commands.message()).await
            })
            .is_err()
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("06-preparation-awaits-ack.json"),
    );
    let mut asking = daemon.clone();
    let ask = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/policy-permission".into(),
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
        &evidence.join("07-permission-under-preparation.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(ask).unwrap().duration, "once");
    // Continue the next stage even if its originating editor is already gone.
    click(&mut cx, editor, "firewall-close");
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: first.id,
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx);
    let second = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(second.sys_firewall, Some(disabled.clone().into()));
    assert_eq!(
        firewall::read(&config.database, "policy-fixture")
            .unwrap()
            .config,
        preparation
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("08-disable-awaits-ack.json"),
    );
    // The editor was closed before the preparation ACK; stage 2 still arrived.
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: second.id,
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "policy-fixture")
            .unwrap()
            .config,
        disabled.clone().into()
    );
    let editor = open_editor(&mut cx);
    cx.update_window(editor, |_, window, cx| {
        window.within("firewall-tabs").click(3usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("09-closed-editor-operation-completed.json"),
    );
    cx.update_window(editor, |_, window, cx| window.press("ctrl-shift-e", cx))
        .unwrap();
    flush(&mut cx);
    let mut enabled = disabled.clone();
    enabled.enabled = true;
    click(&mut cx, editor, "fw-policy-review");
    flush(&mut cx);
    checkpoint(&mut cx, editor, &evidence.join("10-enable-review.json"));
    click(&mut cx, editor, "firewall-confirm");
    flush(&mut cx);
    let enable = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(enable.sys_firewall, Some(enabled.clone().into()));
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: enable.id,
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "policy-fixture")
            .unwrap()
            .config,
        enabled.clone().into()
    );
    wait_window(&mut cx, editor, |window, cx| {
        crate::app::firewall_view::test_idle(window, cx)
    });
    // A single enabled policy reload can fail without discarding its local draft.
    click(&mut cx, editor, "fw-policy-drop");
    flush(&mut cx);
    click(&mut cx, editor, "fw-policy-review");
    flush(&mut cx);
    click(&mut cx, editor, "firewall-confirm");
    flush(&mut cx);
    let rejected = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(
        rejected.sys_firewall.as_ref().unwrap().system_rules[0].chains[0].policy,
        "drop"
    );
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: rejected.id,
            code: 1,
            data: "Synthetic policy rejection".into(),
        }))
        .unwrap();
    flush(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "policy-fixture")
            .unwrap()
            .config,
        enabled.clone().into()
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("11-rejected-policy-draft-retained.json"),
    );
    wait_window(&mut cx, editor, |window, cx| {
        crate::app::firewall_view::test_idle(window, cx)
    });
    click(&mut cx, editor, "firewall-copy");
    let retained =
        Document::parse(&cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()))
            .unwrap();
    assert_eq!(retained.chain(0, 0).unwrap().policy, "drop");
    assert!(retained.enabled);
    click(&mut cx, editor, "firewall-close");
    drop(commands);
    drop(replies);
    drop(daemon);
    drop(view);
    shutdown(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-policy-controls.json","02-input-preset.json","03-unsupported-preset-retained.json","04-staged-disable.json","05-two-stage-review.json","06-preparation-awaits-ack.json","07-permission-under-preparation.json","08-disable-awaits-ack.json","09-closed-editor-operation-completed.json","10-enable-review.json","11-rejected-policy-draft-retained.json"],"proof_boundary":"Actual Kit policy presets and enabled checkbox, staged changes, exact two-stage complete reloads with held ACK and independent permission, no early continuation, detached completion after editor close, reopening/enabling and rejected-draft preservation. Synthetic daemon only; native compositor and kernel enforcement remain separate."})).unwrap()).unwrap();
}
