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
    let initial = protocol::SysFirewall {
        enabled: true,
        version: 1,
        system_rules: vec![protocol::FwChains {
            rule: Some(protocol::FwRule {
                parameters: "legacy-kept".into(),
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
                rules: (0..225)
                    .map(|i| protocol::FwRule {
                        uuid: format!("owned-rule-{i:04}"),
                        position: i,
                        enabled: true,
                        description: format!("Synthetic service {i}"),
                        target: "accept".into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
        }],
    };
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
                name: "firewall-fixture".into(),
                system_firewall: Some(initial.clone()),
                config: r#"{"Firewall":"nftables"}"#.into(),
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
    checkpoint(&mut cx, editor, &evidence.join("01-firewall-chains.json"));
    click(&mut cx, editor, "firewall-next");
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("02-firewall-rules-page-two.json"),
    );
    fill(&mut cx, editor, "firewall-search", "owned-rule-0224");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("03-firewall-rule-filter.json"),
    );
    cx.update_window(editor, |_, window, cx| {
        window.within("firewall-tabs").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("[]", cx);
    })
    .unwrap();
    click(&mut cx, editor, "firewall-review");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("04-firewall-invalid-json.json"),
    );
    let mut next = initial.clone();
    next.system_rules[0].chains[0].policy = "drop".into();
    next.system_rules[0].chains[0].rules[0].description = "Reviewed firewall edit".into();
    cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            " ".repeat(firewall::MAX_JSON + 1),
        ))
    });
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.press("ctrl-v", cx);
    })
    .unwrap();
    click(&mut cx, editor, "firewall-copy");
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        "[]",
        "oversized paste must preserve the previous draft"
    );
    cx.update_window(editor, |_, window, cx| {
        window.within("firewall-tabs").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    let text = Document::from(next.clone()).json().unwrap();
    cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)));
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.press("ctrl-v", cx);
    })
    .unwrap();
    click(&mut cx, editor, "firewall-review");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("05-firewall-before-after.json"),
    );
    click(&mut cx, editor, "firewall-back");
    flush(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "firewall-fixture")
            .unwrap()
            .config,
        initial
    );
    click(&mut cx, editor, "firewall-review");
    flush(&mut cx);
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
    assert_eq!(command.sys_firewall.as_ref(), Some(&next));
    assert!(command.data.is_empty() && command.rules.is_empty());
    assert_eq!(
        firewall::read(&config.database, "firewall-fixture")
            .unwrap()
            .config,
        initial
    );
    let mut asking = daemon.clone();
    let asking = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/firewall-prompt".into(),
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
        &evidence.join("06-permission-during-firewall-ack.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(asking).unwrap().action, "deny");
    let held_read = (0..100)
        .find_map(|_| {
            let permit = crate::nodes::read_permit();
            if permit.is_none() {
                std::thread::sleep(Duration::from_millis(2));
            }
            permit
        })
        .expect("fixture can own the single node read permit");
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: command.id,
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx); // Post-ACK load must wait without retaining a busy-reader error.
    drop(held_read);
    flush(&mut cx);
    let accepted = firewall::read(&config.database, "firewall-fixture").unwrap();
    assert_eq!(accepted.config, next);
    assert!(!accepted.reported);
    checkpoint(&mut cx, editor, &evidence.join("07-firewall-accepted.json"));
    click(&mut cx, editor, "firewall-copy");
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(
        protocol::SysFirewall::from(Document::parse(&copied).unwrap()),
        next
    );
    cx.update_window(editor, |_, window, cx| {
        window.within("firewall-tabs").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    let mut rejected = next.clone();
    rejected.system_rules[0].chains[0].policy = "accept".into();
    let text = Document::from(rejected).json().unwrap();
    cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)));
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.press("ctrl-v", cx);
    })
    .unwrap();
    click(&mut cx, editor, "firewall-review");
    flush(&mut cx);
    click(&mut cx, editor, "firewall-confirm");
    flush(&mut cx);
    let command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: command.id,
            code: 1,
            data: "Synthetic reload rejected".into(),
        }))
        .unwrap();
    flush(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "firewall-fixture")
            .unwrap()
            .config,
        next
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("08-firewall-rejected-draft-retained.json"),
    );
    click(&mut cx, editor, "firewall-close");
    drop(daemon);
    drop(commands);
    drop(replies);
    drop(view);
    shutdown(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-firewall-chains.json","02-firewall-rules-page-two.json","03-firewall-rule-filter.json","04-firewall-invalid-json.json","05-firewall-before-after.json","06-permission-during-firewall-ack.json","07-firewall-accepted.json","08-firewall-rejected-draft-retained.json"],"proof_boundary":"Actual Nodes/open/chain paging/search/complete JSON validation/review/back/apply/copy, exact complete RELOAD_FW_RULES protobuf, no optimistic cache update, independent permission under held ACK, accepted-vs-reported provenance and rejected draft/cache preservation. Synthetic daemon only. This journey does not cover dedicated forms, file/profile workflows, enable-disable sequencing or native firewall input."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Open selected node system firewall; browse 225 rules with paging and filter","Reject non-object JSON; edit policy/description in complete draft and review exact before/after","Back leaves configuration intact; confirm sends complete protobuf while cache remains reported old value","Answer independent real permission under held firewall ACK","ACK saves accepted snapshot and exact full clipboard roundtrip; ERROR preserves draft and last accepted state"]).unwrap()).unwrap();
}
