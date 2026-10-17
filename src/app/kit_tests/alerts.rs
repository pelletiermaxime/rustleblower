use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::alerts;
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mut daemon = runtime.block_on(async {
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
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "alerts-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        for id in 0..205 {
            daemon
                .post_alert(protocol::Alert {
                    id,
                    action: 1,
                    r#type: (id % 3) as i32,
                    priority: 2,
                    data: Some(protocol::alert::Data::Text(format!(
                        "Recorded alert {id:03}"
                    ))),
                    ..Default::default()
                })
                .await
                .unwrap();
            // This fixture needs all 206 rows for paging, rather than a burst
            // that can intentionally overflow the bounded writer on slow disks.
            // RPC receipt alone does not acknowledge SQLite durability.
            backend.handle.history.flush().await.unwrap();
        }
        daemon
            .post_alert(protocol::Alert {
                id: 205,
                action: 2,
                r#type: 1,
                data: Some(protocol::alert::Data::Proc(protocol::Process {
                    pid: 987,
                    path: "/usr/bin/alert-fixture".into(),
                    args: vec!["two words".into()],
                    ..Default::default()
                })),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
        daemon
    });
    assert_eq!(
        backend
            .handle
            .history
            .health
            .alerts_skipped
            .load(Ordering::Relaxed),
        0
    );
    let mut cx = context(test_cx);
    let mut view = None;
    let window = cx
        .open_test_window(size(px(1180.), px(900.)), |window, cx| {
            let dashboard =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(dashboard.clone());
            cx.new(|cx| gpui_kit::base::Root::new(dashboard, window, cx))
        })
        .unwrap();
    let window: AnyWindowHandle = window.into();
    let view = view.unwrap();
    cx.update_window(window, |_, window, cx| window.press("ctrl-5", cx))
        .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.alerts.page.rows.len())) == (200)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        200
    );
    checkpoint(&mut cx, window, &evidence.join("01-alerts-latest.json"));
    click(&mut cx, window, "alerts-older");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.alerts.page.rows.len())) == (6)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        6
    );
    checkpoint(&mut cx, window, &evidence.join("02-alerts-older.json"));
    runtime.block_on(async {
        daemon
            .post_alert(protocol::Alert {
                id: 999,
                action: 1,
                r#type: 0,
                data: Some(protocol::alert::Data::Text("Later arrival".into())),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.alerts.page.rows.len())) == (6)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        6
    );
    click(&mut cx, window, "alerts-previous");
    flush(&mut cx);
    click(&mut cx, window, "alerts-latest");
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("alerts-severity", 1usize), cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.alerts.page.rows.len())) == (70)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        70
    );
    checkpoint(&mut cx, window, &evidence.join("03-alerts-errors.json"));
    cx.update_window(window, |_, window, cx| {
        window.click(("alerts-severity", 0usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    fill(&mut cx, window, "alerts-search", "/usr/bin/alert-fixture");
    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        1
    );
    let typed = alerts::page(
        &config.database,
        &alerts::Query {
            search: "/usr/bin/alert-fixture".into(),
            ..Default::default()
        },
    )
    .unwrap()
    .rows[0]
        .id;
    cx.update_window(window, |_, window, cx| {
        window.click(("alert-row", typed as u64), cx)
    })
    .unwrap();
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("04-alerts-typed-details.json"),
    );
    click(&mut cx, window, "alerts-copy-detail");
    let copied = cx
        .update(|cx| cx.read_from_clipboard())
        .unwrap()
        .text()
        .unwrap();
    let copied: serde_json::Value = serde_json::from_str(&copied).unwrap();
    assert_eq!(copied["payload"]["path"], "/usr/bin/alert-fixture");
    assert_eq!(copied["payload"]["args"][0], "two words");
    click(&mut cx, window, "alerts-read");
    flush(&mut cx);
    click(&mut cx, window, "alerts-unread");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.alerts.page.rows.is_empty())
    });

    assert!(cx.read_entity(&view, |view, _| view.alerts.page.rows.is_empty()));
    checkpoint(
        &mut cx,
        window,
        &evidence.join("05-alerts-read-filter.json"),
    );
    click(&mut cx, window, "alerts-unread");
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("alert-row", typed as u64), cx)
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, window, "alerts-delete");
    click(&mut cx, window, "alerts-delete-cancel");
    assert!(alerts::detail(&config.database, typed).unwrap().is_some());
    click(&mut cx, window, "alerts-delete");
    checkpoint(
        &mut cx,
        window,
        &evidence.join("06-alerts-delete-confirm.json"),
    );
    click(&mut cx, window, "alerts-delete-confirm");
    flush(&mut cx);
    assert!(alerts::detail(&config.database, typed).unwrap().is_none());
    fill(&mut cx, window, "alerts-search", "");
    let export = evidence.join("alerts.jsonl");
    fill(
        &mut cx,
        window,
        "alerts-export-path",
        export.to_str().unwrap(),
    );
    click(&mut cx, window, "alerts-export-json");
    flush(&mut cx);
    let original = std::fs::read_to_string(&export).unwrap();
    assert_eq!(original.lines().count(), 206);
    click(&mut cx, window, "alerts-export-csv");
    flush(&mut cx);
    assert_eq!(std::fs::read_to_string(&export).unwrap(), original);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("07-alerts-export-existing.json"),
    );
    click(&mut cx, window, "alerts-export-replace");
    click(&mut cx, window, "alerts-export-csv");
    flush(&mut cx);
    let csv = std::fs::read_to_string(&export).unwrap();
    assert!(csv.starts_with("\"Received UTC\""));
    assert!(csv.contains("Later arrival"));
    click(&mut cx, window, "alerts-copy-page");
    assert!(
        cx.update(|cx| cx.read_from_clipboard())
            .unwrap()
            .text()
            .unwrap()
            .contains("Later arrival")
    );
    let mut asking = daemon.clone();
    let request = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/bin/alert-permission".into(),
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
        &evidence.join("08-permission-from-alerts.json"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(request).unwrap().action, "deny");
    flush(&mut cx);
    fill(&mut cx, window, "alerts-search", "Recorded alert 00");
    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        10
    );
    click(&mut cx, window, "alerts-clear");
    click(&mut cx, window, "alerts-delete-cancel");
    assert_eq!(
        alerts::page(
            &config.database,
            &alerts::Query {
                search: "Recorded alert 00".into(),
                ..Default::default()
            }
        )
        .unwrap()
        .rows
        .len(),
        10
    );
    click(&mut cx, window, "alerts-clear");
    checkpoint(
        &mut cx,
        window,
        &evidence.join("09-alerts-clear-review.json"),
    );
    runtime.block_on(async {
        daemon
            .post_alert(protocol::Alert {
                id: 1001,
                action: 2,
                data: Some(protocol::alert::Data::Text(
                    "Recorded alert 005 after review".into(),
                )),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    click(&mut cx, window, "alerts-delete-confirm");
    flush(&mut cx);
    let remaining = alerts::page(
        &config.database,
        &alerts::Query {
            search: "Recorded alert 00".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(remaining.rows.len(), 1);
    assert_eq!(remaining.rows[0].summary, "Recorded alert 005 after review");
    fill(&mut cx, window, "alerts-search", "absent alert text");
    assert!(cx.read_entity(&view, |view, _| view.alerts.page.rows.is_empty()));
    checkpoint(
        &mut cx,
        window,
        &evidence.join("10-alerts-empty-search.json"),
    );
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    shutdown(cx);
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let checkpoints = [
        "01-alerts-latest.json",
        "02-alerts-older.json",
        "03-alerts-errors.json",
        "04-alerts-typed-details.json",
        "05-alerts-read-filter.json",
        "06-alerts-delete-confirm.json",
        "07-alerts-export-existing.json",
        "08-permission-from-alerts.json",
        "09-alerts-clear-review.json",
        "10-alerts-empty-search.json",
    ];
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":checkpoints,"boundary":"Actual Kit alert browse/filter/page/detail/copy/read/delete/export controls, independent gRPC permission and SQLite assertions; real daemon untouched"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Open Alerts with Ctrl+5; browse older frozen page while a newer alert arrives; Previous/Latest", "Filter errors and typed payload, inspect/copy complete JSON; mark read and unread-only empty state", "Cancel then confirm local deletion; export all retained matches as JSON, preserve existing file, explicitly replace as CSV and copy page", "Answer a real permission from the Alerts tab; search absent text to empty state"]).unwrap()).unwrap();
}
