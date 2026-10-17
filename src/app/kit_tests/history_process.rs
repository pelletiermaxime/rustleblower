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
    let connect = |name: &str| {
        runtime.block_on(async {
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
            let mut client = protocol::ui_client::UiClient::new(channel);
            client
                .subscribe(protocol::ClientConfig {
                    name: name.into(),
                    ..Default::default()
                })
                .await
                .unwrap();
            let (tx, rx) = tokio::sync::mpsc::channel(8);
            let stream = client
                .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
                .await
                .unwrap()
                .into_inner();
            (client, tx, stream)
        })
    };
    let (mut alpha, replies, mut commands) = connect("history-alpha");
    let (mut beta, _beta_replies, mut beta_commands) = connect("selected-beta");
    let alpha_id = backend.handle.node("history-alpha").unwrap().id;
    backend
        .handle
        .select_node(backend.handle.node("selected-beta").unwrap().id)
        .unwrap();
    for (node, path, pid) in [
        ("history-alpha", "/usr/bin/recorded-valid", 42),
        ("history-alpha", "/usr/bin/recorded-unknown", 0),
        ("history-alpha", "/usr/bin/recorded-invalid", u32::MAX),
        ("offline-node", "/usr/bin/recorded-offline", 77),
    ] {
        backend.handle.history.ingest(
            node.into(),
            vec![protocol::Event {
                unixnano: crate::history::now_nanos(),
                connection: Some(protocol::Connection {
                    process_path: path.into(),
                    process_id: pid,
                    ..Default::default()
                }),
                ..Default::default()
            }],
            "observed",
        );
    }
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context(test_cx);
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(crate::app::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    let open_detail = |cx: &mut TestAppContext, search: &str| {
        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-1", cx);
            window.press("ctrl-f", cx);
            window.press("ctrl-a", cx);
            window.input(search, cx);
        })
        .unwrap();
        flush(cx);
        cx.read_entity(&view, |v, _| assert_eq!(v.history.page.rows.len(), 1));
        cx.update_window(window, |_, window, cx| {
            window.click(("history-row", 0usize), cx)
        })
        .unwrap();
        click(cx, window, "history-details");
        flush(cx);
        cx.read_entity(&view, |v, _| v.history.detail_window.unwrap())
    };
    let details = open_detail(&mut cx, "recorded-valid");
    checkpoint(&mut cx, details, &evidence.join("01-recorded-process.json"));
    keyboard_button(&mut cx, details, "connection-inspect-process");
    flush(&mut cx);
    cx.read_entity(&view, |v, _| assert!(v.screen == crate::app::Screen::Live));
    assert_eq!(backend.handle.status().session.unwrap().id, alpha_id);
    let command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(command.r#type, 13);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&command.data).unwrap(),
        serde_json::json!({"name":"pid-monitor","data":{"pid":"42","interval":"5s"}})
    );
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), beta_commands.message()).await
            })
            .is_err(),
        "the selected unrelated node must receive nothing"
    );
    let request = runtime.spawn(async move {
        beta.ask_rule(protocol::Connection {
            process_path: "/usr/bin/independent-permission".into(),
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
        &evidence.join("02-permission-during-process-query.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(request).unwrap().duration, "once");
    runtime.block_on(replies.send(protocol::NotificationReply {
        id:command.id, data:serde_json::json!({"PID":42,"Path":"/usr/bin/reused-current-process","Status":"Name:\treused-current\nPid:\t42","Env":{"CURRENT_ONLY":"not historical"}}).to_string(), ..Default::default()
    })).unwrap();
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("03-current-process-on-event-node.json"),
    );
    click(&mut cx, window, "live-copy-json");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("reused-current-process")
    );
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: command.id,
            code: 1,
            data: "Synthetic process exited".into(),
        }))
        .unwrap();
    flush(&mut cx);
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| !v.live.snapshot.running)
    });
    cx.read_entity(&view, |v, _| {
        assert!(!v.live.snapshot.running);
        assert!(v.live.snapshot.error.as_ref().unwrap().contains("exited"));
        assert!(v.live.parsed.is_some());
    });
    checkpoint(&mut cx, window, &evidence.join("04-process-exited.json"));
    click(&mut cx, details, "connection-close");
    for query in ["recorded-unknown", "recorded-invalid", "recorded-offline"] {
        let details = open_detail(&mut cx, query);
        click(&mut cx, details, "connection-inspect-process");
        flush(&mut cx);
        cx.read_entity(&view, |v, _| {
            assert!(v.screen == crate::app::Screen::History)
        });
        checkpoint(&mut cx, details, &evidence.join(format!("05-{query}.json")));
        click(&mut cx, details, "connection-close");
    }
    // Capture an old session at input time, then replace it before the main timer
    // drains the action. The new connection must never inherit a stale request.
    let details = open_detail(&mut cx, "recorded-valid");
    click(&mut cx, details, "connection-inspect-process");
    runtime
        .block_on(alpha.subscribe(protocol::ClientConfig {
            name: "history-alpha".into(),
            ..Default::default()
        }))
        .unwrap();
    assert_ne!(backend.handle.node("history-alpha").unwrap().id, alpha_id);
    flush(&mut cx);
    cx.read_entity(&view, |v, _| {
        assert!(v.screen == crate::app::Screen::History)
    });
    checkpoint(
        &mut cx,
        details,
        &evidence.join("06-reconnected-request-rejected.json"),
    );
    click(&mut cx, details, "connection-close");
    // Only one start was issued. Stopping on ERROR is intentionally unacknowledged.
    let stop = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
    });
    if let Ok(Some(stop)) = stop {
        assert_eq!(stop.r#type, 14);
    }
    drop(replies);
    drop(commands);
    drop(beta_commands);
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    shutdown(cx);
    backend.handle.shutdown();
    backend.join();
    assert!(!config.socket.exists());
    let checkpoints = [
        "01-recorded-process.json",
        "02-permission-during-process-query.json",
        "03-current-process-on-event-node.json",
        "04-process-exited.json",
        "05-recorded-unknown.json",
        "05-recorded-invalid.json",
        "05-recorded-offline.json",
        "06-reconnected-request-rejected.json",
    ];
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":checkpoints,"proof_boundary":"Actual connection-details keyboard/button input; current PID routed to the historical event's node rather than the selected node; separate real permission while task update is held; current data copy, process error retention, invalid/missing/offline PID refusal and queued stale-session rejection. Synthetic daemon only; no real process visibility proof."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Select history row and open details; Tab/Space Inspect process","Require exact PID42/5s task only on history-alpha while selected-beta receives no command","Hold update, answer real independent permission, then receive current data with a different executable and copy complete JSON","Report process exited; keep last data with error","Unknown/invalid PID and offline node cannot start inspection","Queue inspection then reconnect; reject stale session rather than target replacement"]).unwrap()).unwrap();
}
