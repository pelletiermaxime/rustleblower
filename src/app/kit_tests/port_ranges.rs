use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let hold = Arc::new(AtomicBool::new(false));
    let accept = Arc::new(AtomicBool::new(true));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let original = protocol::Rule {
        name: "owned-port-rule".into(),
        enabled: true,
        action: "deny".into(),
        duration: "always".into(),
        operator: Some(protocol::Operator {
            r#type: "list".into(),
            operand: "list".into(),
            list: vec![
                protocol::Operator {
                    r#type: "simple".into(),
                    operand: "source.port".into(),
                    data: "4000".into(),
                    ..Default::default()
                },
                protocol::Operator {
                    r#type: "simple".into(),
                    operand: "dest.port".into(),
                    data: "8000".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    let (mut daemon, notifications) = runtime.block_on(async {
        use hyper_util::rt::TokioIo;
        let path = config.socket.clone();
        let channel = tonic::transport::Endpoint::try_from("http://localhost")
            .unwrap()
            .connect_with_connector(tower::service_fn(move |_| {
                let path = path.clone();
                async move {
                    tokio::net::UnixStream::connect(path)
                        .await
                        .map(TokioIo::new)
                }
            }))
            .await
            .unwrap();
        let mut client = protocol::ui_client::UiClient::new(channel);
        client
            .subscribe(protocol::ClientConfig {
                name: "port-range-fixture".into(),
                rules: vec![original.clone()],
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let mut stream = client
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        let hold = hold.clone();
        let accept = accept.clone();
        let observed = observed.clone();
        let task = tokio::spawn(async move {
            while let Ok(Some(command)) = stream.message().await {
                assert!(observed.lock().unwrap().len() < 8);
                observed.lock().unwrap().push(command.clone());
                while hold.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let ok = accept.load(Ordering::Acquire);
                if tx
                    .send(protocol::NotificationReply {
                        id: command.id,
                        code: if ok {
                            protocol::NotificationReplyCode::Ok
                        } else {
                            protocol::NotificationReplyCode::Error
                        } as i32,
                        data: if ok { "" } else { "owned range rejection" }.into(),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        (client, task)
    });
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context(test_cx);
    let window = cx.update(|cx| {
        cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::centered(
                    None,
                    size(px(1152.), px(850.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
            },
        )
        .unwrap()
    });
    let view = cx.update(|cx| window.entity(cx).unwrap());
    flush(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.within("main-tabs").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-edit");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    fn select(cx: &mut TestAppContext, editor: AnyWindowHandle, id: u64, value: &str) {
        cx.update_window(editor, |_, window, cx| {
            window.click(("editor-match", id), cx)
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(editor, |_, window, cx| window.input(value, cx))
            .unwrap();
        flush(cx);
        cx.update_window(editor, |_, window, cx| window.press("enter", cx))
            .unwrap();
        flush(cx);
    }
    fn value(cx: &mut TestAppContext, editor: AnyWindowHandle, id: u64, value: &str) {
        cx.update_window(editor, |_, window, cx| {
            window.scroll(
                "rule-editor-body",
                gpui::ScrollDelta::Pixels(gpui::point(
                    px(0.),
                    px(if id == 0 { 1000. } else { -400. }),
                )),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(editor, |_, window, cx| {
            window.click(("editor-value", id), cx);
            window.press("ctrl-a", cx);
            window.input(value, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    fn error(cx: &mut TestAppContext, editor: AnyWindowHandle) -> String {
        cx.update_window(editor, |_, window, cx| {
            gpui_kit::base::Root::read(window, cx)
                .view()
                .clone()
                .downcast::<crate::app::rule_editor::RuleEditor>()
                .unwrap()
                .read(cx)
                .test_error()
                .unwrap()
        })
        .unwrap()
    }
    select(&mut cx, editor, 0, "Port range");
    select(&mut cx, editor, 1, "Port range");
    value(&mut cx, editor, 0, "4000-4010");
    value(&mut cx, editor, 1, "8080-8000");
    click(&mut cx, editor, "editor-save");
    flush(&mut cx);
    assert!(error(&mut cx, editor).contains("min <= max"));
    assert!(observed.lock().unwrap().is_empty());
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("01-invalid-port-range.json"),
    );
    value(&mut cx, editor, 1, " 8000 - 8080 ");
    click(&mut cx, editor, "editor-save");
    flush(&mut cx);
    assert!(error(&mut cx, editor).contains("not established"));
    assert!(observed.lock().unwrap().is_empty());
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("02-unknown-daemon-refused.json"),
    );
    runtime
        .block_on(daemon.ping(protocol::PingRequest {
            stats: Some(protocol::Statistics {
                daemon_version: "1.9.0 (owned range fixture)".into(),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .unwrap();
    cx.update_window(editor, |_, window, cx| {
        window.scroll(
            "rule-editor-body",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(1000.))),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, editor, "editor-json-toggle");
    checkpoint(&mut cx, editor, &evidence.join("03-range-match-json.json"));
    click(&mut cx, editor, "editor-json-toggle");
    hold.store(true, Ordering::Release);
    click(&mut cx, editor, "editor-save");
    flush(&mut cx);
    assert_eq!(observed.lock().unwrap().len(), 1);
    assert_eq!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "port-range-fixture",
            &original.name
        )
        .unwrap(),
        Some(original.clone())
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("04-range-awaiting-ack.json"),
    );
    let mut asking = daemon.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/range-independent-prompt".into(),
                dst_port: 443,
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    let prompt = permission_window(&mut cx, &view);
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("05-permission-during-range-save.json"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    hold.store(false, Ordering::Release);
    flush(&mut cx);
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.rules.editor_window.is_none())
    });

    assert!(cx.read_entity(&view, |v, _| v.rules.editor_window.is_none()));
    let saved = crate::rules::transfer::find_confirmed_rule(
        &config.database,
        "port-range-fixture",
        &original.name,
    )
    .unwrap()
    .unwrap();
    let ops = &saved.operator.as_ref().unwrap().list;
    assert_eq!(ops[0].r#type, "range");
    assert_eq!(ops[0].data, "4000-4010");
    assert_eq!(ops[1].r#type, "range");
    assert_eq!(ops[1].data, " 8000 - 8080 ");
    assert_eq!(observed.lock().unwrap()[0].rules, vec![saved.clone()]);
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("06-range-confirmed.json"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-edit");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    value(&mut cx, editor, 1, "9000-9010");
    accept.store(false, Ordering::Release);
    click(&mut cx, editor, "editor-save");
    flush(&mut cx);
    assert!(error(&mut cx, editor).contains("owned range rejection"));
    assert_eq!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "port-range-fixture",
            &original.name
        )
        .unwrap(),
        Some(saved.clone())
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("07-rejected-range-draft-retained.json"),
    );
    click(&mut cx, editor, "editor-cancel");
    flush(&mut cx);
    click(&mut cx, window.into(), "rules-export");
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, transfer, "transfer-copy");
    flush(&mut cx);
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(
        crate::rules::document::parse_document(&copied).unwrap(),
        vec![saved]
    );
    checkpoint(
        &mut cx,
        transfer,
        &evidence.join("08-range-export-roundtrip.json"),
    );
    click(&mut cx, transfer, "transfer-close");
    assert_eq!(observed.lock().unwrap().len(), 2);
    drop(view);
    shutdown(cx);
    notifications.abort();
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-invalid-port-range.json","02-unknown-daemon-refused.json","03-range-match-json.json","04-range-awaiting-ack.json","05-permission-during-range-save.json","06-range-confirmed.json","07-rejected-range-draft-retained.json","08-range-export-roundtrip.json"],"proof_boundary":"Actual rule-tab/editor Select/Input/JSON/save/cancel/clipboard input; exact source and destination range wire values; unknown capability refusal, held ACK with independent permission, durable success and rejected-draft preservation. Synthetic owned daemon; no enforcement or native compositor proof."})).unwrap()).unwrap();
}
