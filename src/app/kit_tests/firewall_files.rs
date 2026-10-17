use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::firewall::{
        self,
        document::{Chain, Document, Rule},
        transfer,
    };
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let files = evidence.join("files");
    std::fs::create_dir(&files).unwrap();
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
                position: u64::MAX,
                parameters: "legacy wrapper".into(),
                ..Default::default()
            }),
            chains: vec![protocol::FwChain {
                name: "input".into(),
                table: "filter".into(),
                family: "inet".into(),
                r#type: "filter".into(),
                hook: "input".into(),
                priority: "-10".into(),
                policy: "accept".into(),
                rules: vec![protocol::FwRule {
                    uuid: "original".into(),
                    description: "Original retained rule".into(),
                    enabled: true,
                    position: u64::MAX,
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
    let original = Document::from(initial.clone());
    let chain = original.chain(0, 0).unwrap();
    let profile = Chain {
        name: "different profile name".into(),
        table: "FILTER".into(),
        family: "INET".into(),
        kind: "FILTER".into(),
        hook: "INPUT".into(),
        policy: "drop".into(),
        priority: "999".into(),
        rules: vec![
            Rule {
                uuid: "same-content-other-ID".into(),
                ..chain.rules[0].clone()
            },
            Rule {
                uuid: "owned-profile-rule".into(),
                description: "Imported port rule".into(),
                target: "reject".into(),
                target_parameters: "tcp reset".into(),
                ..chain.rules[0].clone()
            },
        ],
    };
    let profile_text = transfer::profile_json(&profile).unwrap();
    let profile_path = files.join("000.profile");
    for index in 0..205 {
        std::fs::write(files.join(format!("{index:03}.profile")), &profile_text).unwrap();
    }
    let initial_path = files.join("initial.json");
    std::fs::write(&initial_path, original.json().unwrap()).unwrap();
    let invalid_path = files.join("invalid.json");
    std::fs::write(&invalid_path, "{\"Unknown\":true}").unwrap();
    let enabled_path = files.join("enabled-mismatch.json");
    let mut mismatch = original.clone();
    mismatch.enabled = false;
    std::fs::write(&enabled_path, mismatch.json().unwrap()).unwrap();
    let wrong_path = files.join("wrong-chain.profile");
    let mut wrong = profile.clone();
    wrong.hook = "forward".into();
    std::fs::write(&wrong_path, transfer::profile_json(&wrong).unwrap()).unwrap();
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
                name: "files-fixture".into(),
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
    let tab = |cx: &mut TestAppContext, index: usize| {
        cx.update_window(editor, |_, window, cx| {
            window.within("firewall-tabs").click(index, cx)
        })
        .unwrap();
        flush(cx);
    };
    let path = |cx: &mut TestAppContext, p: &std::path::Path| {
        fill(cx, editor, "fw-file-path", p.to_str().unwrap())
    };
    let copy = |cx: &mut TestAppContext| {
        click(cx, editor, "firewall-copy");
        Document::parse(&cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())).unwrap()
    };
    tab(&mut cx, 2);
    path(&mut cx, &files);
    click(&mut cx, editor, "fw-profiles-list");
    flush(&mut cx);
    checkpoint(&mut cx, editor, &evidence.join("01-profile-files.json"));
    click(&mut cx, editor, "fw-profiles-next");
    flush(&mut cx);
    cx.update_window(editor, |_, window, cx| {
        window.click(("fw-profile-file", 200usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    checkpoint(&mut cx, editor, &evidence.join("02-profile-page-two.json"));
    click(&mut cx, editor, "fw-profiles-previous");
    flush(&mut cx);
    cx.update_window(editor, |_, window, cx| {
        window.click(("fw-profile-file", 0usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, editor, "fw-profile-merge");
    flush(&mut cx);
    let mut expected = original.clone();
    expected.chain_mut(0, 0).unwrap().policy = "drop".into();
    expected
        .chain_mut(0, 0)
        .unwrap()
        .rules
        .push(profile.rules[1].clone());
    assert_eq!(copy(&mut cx), expected);
    click(&mut cx, editor, "fw-profile-merge");
    flush(&mut cx);
    assert_eq!(
        copy(&mut cx),
        expected,
        "repeat merge skips same-content and same-ID rules"
    );
    checkpoint(&mut cx, editor, &evidence.join("03-staged-profile.json"));
    path(&mut cx, &wrong_path);
    click(&mut cx, editor, "fw-profile-merge");
    flush(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    path(&mut cx, &invalid_path);
    click(&mut cx, editor, "fw-file-load");
    flush(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("04-invalid-import-retained.json"),
    );
    path(&mut cx, &enabled_path);
    click(&mut cx, editor, "fw-file-load");
    flush(&mut cx);
    assert_eq!(
        copy(&mut cx),
        mismatch,
        "Enabled imports stay local until transition review"
    );
    path(&mut cx, &initial_path);
    click(&mut cx, editor, "fw-file-load");
    flush(&mut cx);
    path(&mut cx, &profile_path);
    click(&mut cx, editor, "fw-profile-merge");
    flush(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    path(&mut cx, &profile_path);
    click(&mut cx, editor, "fw-profile-remove");
    flush(&mut cx);
    let mut removed = original.clone();
    removed.chain_mut(0, 0).unwrap().policy = "drop".into();
    assert_eq!(
        copy(&mut cx),
        removed,
        "remove only matching IDs; retain original semantic duplicate and policy"
    );
    click(&mut cx, editor, "fw-profile-merge");
    flush(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    let exported = files.join("exported.json");
    path(&mut cx, &exported);
    click(&mut cx, editor, "fw-file-export");
    flush(&mut cx);
    assert_eq!(
        Document::parse(&std::fs::read_to_string(&exported).unwrap()).unwrap(),
        expected
    );
    let preserved = std::fs::read(&exported).unwrap();
    click(&mut cx, editor, "fw-file-export");
    flush(&mut cx);
    assert_eq!(std::fs::read(&exported).unwrap(), preserved);
    let exported_profile = files.join("exported.profile");
    path(&mut cx, &exported_profile);
    click(&mut cx, editor, "fw-profile-export");
    flush(&mut cx);
    assert_eq!(
        &transfer::parse_profile(&std::fs::read_to_string(&exported_profile).unwrap()).unwrap(),
        expected.chain(0, 0).unwrap()
    );
    let envelope: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&exported_profile).unwrap()).unwrap();
    assert_eq!(envelope["Name"], "input");
    assert!(envelope["Profile"]["Rules"].is_array());
    click(&mut cx, editor, "fw-profile-copy");
    wait_for(&mut cx, |cx| {
        (&transfer::parse_profile(&(cx.read_from_clipboard().unwrap().text().unwrap())).unwrap())
            == (expected.chain(0, 0).unwrap())
    });

    assert_eq!(
        &transfer::parse_profile(
            &cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
        )
        .unwrap(),
        expected.chain(0, 0).unwrap()
    );
    checkpoint(&mut cx, editor, &evidence.join("05-new-file-export.json"));
    path(&mut cx, &initial_path);
    click(&mut cx, editor, "fw-file-load");
    flush(&mut cx);
    assert_eq!(copy(&mut cx), original);
    path(&mut cx, &exported);
    click(&mut cx, editor, "fw-file-load");
    flush(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    assert_eq!(
        firewall::read(&config.database, "files-fixture")
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
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("06-imported-complete-draft.json"),
    );
    click(&mut cx, editor, "fw-files-review");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("07-complete-file-review.json"),
    );
    click(&mut cx, editor, "firewall-back");
    flush(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
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
    assert_eq!(command.sys_firewall, Some(expected.clone().into()));
    assert!(command.data.is_empty());
    assert!(command.rules.is_empty());
    assert_eq!(
        firewall::read(&config.database, "files-fixture")
            .unwrap()
            .config,
        initial
    );
    let mut asking = daemon.clone();
    let asking = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/file-permission".into(),
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
        &evidence.join("08-permission-during-file-ack.json"),
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
    let saved = firewall::read(&config.database, "files-fixture").unwrap();
    assert_eq!(saved.config, expected.clone().into());
    assert!(!saved.reported);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("09-file-reload-accepted.json"),
    );
    // A raw JSON edit must be parsed before selecting/exporting a profile.
    tab(&mut cx, 1);
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("{invalid", cx);
    })
    .unwrap();
    flush(&mut cx);
    tab(&mut cx, 2);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("10-invalid-json-cannot-export.json"),
    );
    // Return to a valid complete document through actual textarea input.
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input(&expected.json().unwrap(), cx);
    })
    .unwrap();
    flush(&mut cx);
    tab(&mut cx, 2);
    click(&mut cx, editor, "fw-profile-copy");
    wait_for(&mut cx, |cx| {
        (&transfer::parse_profile(&(cx.read_from_clipboard().unwrap().text().unwrap())).unwrap())
            == (expected.chain(0, 0).unwrap())
    });

    assert_eq!(
        &transfer::parse_profile(
            &cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
        )
        .unwrap(),
        expected.chain(0, 0).unwrap()
    );
    // Read-only recovery still exports after the captured node disconnects.
    drop(commands);
    drop(replies);
    drop(daemon);
    flush(&mut cx);
    assert!(
        backend
            .handle
            .node("files-fixture")
            .is_none_or(|node| !node.notifications),
        "notification stream is actually disconnected before offline export"
    );
    let offline = files.join("offline.json");
    path(&mut cx, &offline);
    click(&mut cx, editor, "fw-file-export");
    flush(&mut cx);
    assert_eq!(
        Document::parse(&std::fs::read_to_string(offline).unwrap()).unwrap(),
        expected
    );
    checkpoint(&mut cx, editor, &evidence.join("11-offline-export.json"));
    click(&mut cx, editor, "firewall-close");
    drop(view);
    shutdown(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-profile-files.json","02-profile-page-two.json","03-staged-profile.json","04-invalid-import-retained.json","05-new-file-export.json","06-imported-complete-draft.json","07-complete-file-review.json","08-permission-during-file-ack.json","09-file-reload-accepted.json","10-invalid-json-cannot-export.json","11-offline-export.json"],"proof_boundary":"Actual Kit path input, two profile-list pages, merge/dedup/remove, import validation and complete/chain file/clipboard export; no overwrite; local-only staging then exact complete gRPC reload, independent permission under ACK, accepted persistence and offline export. Native OS picker interaction, built-in policy presets, enable-disable and enforcement remain separate."})).unwrap()).unwrap();
}
