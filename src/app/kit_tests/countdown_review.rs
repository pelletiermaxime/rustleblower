use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let mut config = options(dir.path());
    config.prompt_timeout = 2;
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let daemon = runtime.block_on(async {
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
                name: "countdown-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        client
    });
    let connection = protocol::Connection {
        process_path: "/usr/bin/review-fixture".into(),
        dst_ip: "192.0.2.42".into(),
        dst_port: 443,
        protocol: "tcp".into(),
        ..Default::default()
    };
    let mut asking = daemon.clone();
    let con = connection.clone();
    let first = runtime.spawn(async move { asking.ask_rule(con).await.unwrap().into_inner() });
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), async {
            while backend.handle.prompts.snapshot().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    });
    let mut cx = context(test_cx);
    let window = cx.update(|cx| {
        crate::app::prompt_window::open(backend.handle.clone(), 1., false, cx).unwrap()
    });
    wait_for(&mut cx, |_cx| {
        backend
            .handle
            .prompts
            .snapshot()
            .first()
            .is_some_and(|prompt| !prompt.reviewing)
    });

    assert!(
        backend
            .handle
            .prompts
            .snapshot()
            .first()
            .is_some_and(|prompt| !prompt.reviewing)
    );
    checkpoint(&mut cx, window, &evidence.join("01-normal-countdown.json"));
    click(&mut cx, window, "prompt-more");
    assert!(
        backend.handle.prompts.snapshot()[0].reviewing,
        "pointer input pauses countdown before child handling"
    );
    std::thread::sleep(Duration::from_millis(2200));
    wait_for(&mut cx, |_cx| !first.is_finished());

    assert!(
        !first.is_finished(),
        "review survives the ordinary two-second countdown"
    );
    checkpoint(&mut cx, window, &evidence.join("02-pointer-review.json"));
    click(&mut cx, window, "allow-selected");
    let rule = runtime.block_on(first).unwrap();
    assert_eq!(
        (rule.action.as_str(), rule.duration.as_str()),
        ("allow", "once")
    );
    let mut asking = daemon.clone();
    let second =
        runtime.spawn(async move { asking.ask_rule(connection).await.unwrap().into_inner() });
    wait_for(&mut cx, |_cx| {
        backend
            .handle
            .prompts
            .snapshot()
            .first()
            .is_some_and(|prompt| !prompt.reviewing)
    });

    assert!(
        backend
            .handle
            .prompts
            .snapshot()
            .first()
            .is_some_and(|prompt| !prompt.reviewing),
        "next request gets a fresh countdown"
    );
    checkpoint(&mut cx, window, &evidence.join("03-next-countdown.json"));
    cx.update_window(window, |_, window, cx| window.press("tab", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        backend.handle.prompts.snapshot()[0].reviewing,
        "keyboard input pauses countdown"
    );
    std::thread::sleep(Duration::from_millis(2200));
    wait_for(&mut cx, |_cx| !second.is_finished());

    assert!(!second.is_finished());
    checkpoint(&mut cx, window, &evidence.join("04-keyboard-review.json"));
    keyboard_button(&mut cx, window, "deny-once");
    let rule = runtime.block_on(second).unwrap();
    assert_eq!(
        (rule.action.as_str(), rule.duration.as_str()),
        ("deny", "once")
    );
    assert_eq!(backend.handle.status().timed_out, 0);
    assert_eq!(backend.handle.status().answered, 2);
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let db = rusqlite::Connection::open(&config.database).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM events WHERE reason='decision'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    drop(db);
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    shutdown(cx);
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","checkpoints":["01-normal-countdown.json","02-pointer-review.json","03-next-countdown.json","04-keyboard-review.json"],"framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"proof_boundary":"Production GPUI pointer and keyboard review, survival beyond ordinary countdown, next-request reset, actual gRPC replies and persisted decisions; real daemon enforcement and native compositor input require separate proof"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"), serde_json::to_vec_pretty(&["Open a real AskRule request with two-second countdown; click Rule options and wait beyond two seconds; require pending review then Allow once", "Open the next request; require normal countdown; Tab to Deny, wait beyond two seconds, then Space; require Deny once and two persisted input decisions"]).unwrap()).unwrap();
}
