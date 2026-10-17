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
    let make_event = |i: i64| protocol::Event {
        unixnano: crate::history::now_nanos() + i,
        connection: Some(protocol::Connection {
            process_path: format!("/usr/bin/retention-{i:04}"),
            dst_host: "history.example".into(),
            dst_ip: "192.0.2.10".into(),
            dst_port: 443,
            ..Default::default()
        }),
        ..Default::default()
    };
    backend.handle.history.ingest(
        "retention-fixture".into(),
        (0..430).map(make_event).collect(),
        "observed",
    );
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let db = rusqlite::Connection::open(&config.database).unwrap();
    db.execute(
        "UPDATE events SET received=? WHERE id<=180",
        [crate::history::now_seconds() - 2 * 86400],
    )
    .unwrap();
    let mut cx = context(test_cx);
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(crate::app::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    flush(&mut cx);
    click(&mut cx, window, "history-preferences");
    flush(&mut cx);
    let preferences = cx.read_entity(&view, |view, _| view.history.preferences_window.unwrap());
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("01-history-preferences.json"),
    );
    fill(&mut cx, preferences, "history-days", "0");
    click(&mut cx, preferences, "history-retention-review");
    wait_for(&mut cx, |_cx| {
        (backend.handle.history.storage.snapshot().config.days) == (7)
    });

    assert_eq!(backend.handle.history.storage.snapshot().config.days, 7);
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("02-invalid-retention.json"),
    );
    fill(&mut cx, preferences, "history-days", "1");
    fill(&mut cx, preferences, "history-mib", "32");
    fill(&mut cx, preferences, "history-cleanup", "1");
    click(&mut cx, preferences, "history-retention-review");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("03-retention-review.json"),
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        430
    );
    assert_eq!(backend.handle.history.storage.snapshot().config.days, 7);
    click(&mut cx, preferences, "history-retention-back");
    wait_for(&mut cx, |_cx| {
        (backend.handle.history.storage.snapshot().config.days) == (7)
    });

    assert_eq!(backend.handle.history.storage.snapshot().config.days, 7);
    click(&mut cx, preferences, "history-retention-review");
    flush(&mut cx);
    // Hold SQLite while Apply waits, then prove a real permission can still be answered.
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    click(&mut cx, preferences, "history-retention-apply");
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
                name: "retention-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    let asking = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/history-save-permission".into(),
                dst_host: "history-save.example".into(),
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
        &evidence.join("04-permission-during-retention.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(asking).unwrap().action, "deny");
    db.execute_batch("ROLLBACK").unwrap();
    flush(&mut cx);
    // A writer busy timeout is an honest failure, never a partial preference save.
    if backend.handle.history.storage.snapshot().config.days == 7 {
        click(&mut cx, preferences, "history-retention-review");
        flush(&mut cx);
        click(&mut cx, preferences, "history-retention-apply");
        flush(&mut cx);
    }
    let saved = backend.handle.history.storage.snapshot();
    assert_eq!(
        (
            saved.config.days,
            saved.config.mib,
            saved.config.cleanup_minutes
        ),
        (1, 32, 1)
    );
    assert_eq!(
        db.query_row("SELECT days FROM history_preferences", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        251
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("05-retention-applied.json"),
    );
    cx.update_window(preferences, |_, window, cx| {
        window.within("history-preferences-tabs").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    fill(&mut cx, preferences, "history-refresh-seconds", "61");
    click(&mut cx, preferences, "history-refresh-save");
    wait_for(&mut cx, |_cx| {
        (backend.handle.history.settings.snapshot().refresh_seconds) == (1)
    });

    assert_eq!(
        backend.handle.history.settings.snapshot().refresh_seconds,
        1
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("06-invalid-refresh.json"),
    );
    fill(&mut cx, preferences, "history-refresh-seconds", "0");
    click(&mut cx, preferences, "history-refresh-save");
    wait_for(&mut cx, |_cx| {
        (crate::history::settings::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .refresh_seconds)
            == (0)
    });

    assert_eq!(
        crate::history::settings::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .refresh_seconds,
        0
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("07-manual-refresh-saved.json"),
    );
    click(&mut cx, preferences, "history-preferences-close");
    flush(&mut cx);
    let previous = cx.read_entity(&view, |view, _| view.history.page.rows[0].id);
    backend.handle.history.ingest(
        "retention-fixture".into(),
        (1000..1003).map(make_event).collect(),
        "observed",
    );
    runtime.block_on(backend.handle.history.flush()).unwrap();
    flush(&mut cx);
    cx.read_entity(&view, |view, _| {
        assert_eq!(view.history.page.rows[0].id, previous)
    });
    assert_eq!(
        db.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        254
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("08-manual-recording-continues.json"),
    );
    click(&mut cx, window, "history-refresh");
    flush(&mut cx);
    cx.read_entity(&view, |view, _| {
        assert!(view.history.page.rows[0].id > previous)
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("09-refreshed-on-demand.json"),
    );
    click(&mut cx, window, "history-preferences");
    flush(&mut cx);
    let preferences = cx.read_entity(&view, |view, _| view.history.preferences_window.unwrap());
    cx.update_window(preferences, |_, window, cx| {
        window.within("history-preferences-tabs").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    fill(&mut cx, preferences, "history-refresh-seconds", "2");
    click(&mut cx, preferences, "history-refresh-save");
    flush(&mut cx);
    click(&mut cx, preferences, "history-preferences-copy-path");
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        config.database.display().to_string()
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("10-auto-refresh-restored.json"),
    );
    click(&mut cx, preferences, "history-preferences-close");
    flush(&mut cx);
    let last = cx.read_entity(&view, |view, _| view.history.page.rows[0].id);
    backend.handle.history.ingest(
        "retention-fixture".into(),
        vec![make_event(2000)],
        "observed",
    );
    std::thread::sleep(Duration::from_millis(2100));
    runtime.block_on(backend.handle.history.flush()).unwrap();
    flush(&mut cx);
    cx.read_entity(&view, |view, _| {
        assert!(view.history.page.rows[0].id > last)
    });
    drop(db);
    drop(view);
    shutdown(cx);
    backend.join();
    assert!(!config.socket.exists());
    let reopened = Backend::start(config.clone()).unwrap();
    assert_eq!(
        reopened.handle.history.storage.snapshot().config,
        saved.config
    );
    assert_eq!(
        reopened.handle.history.settings.snapshot().refresh_seconds,
        2
    );
    reopened.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-history-preferences.json","02-invalid-retention.json","03-retention-review.json","04-permission-during-retention.json","05-retention-applied.json","06-invalid-refresh.json","07-manual-refresh-saved.json","08-manual-recording-continues.json","09-refreshed-on-demand.json","10-auto-refresh-restored.json"],"proof_boundary":"Actual Kit retention validation/review/back/apply and saved display refresh; independent SQLite/file/clipboard/reopen assertions, real permission while SQLite is locked, manual display with continuing ingestion and explicit/automatic refresh. Synthetic owned fixtures only; real idle timer and rollback/CLI/physical compaction tested separately."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Open history preferences; reject zero days; review 1 day/32 MiB/1-minute cleanup; Back changes nothing","Hold writer during actual Apply, answer a real permission by input, release lock and retry if save honestly timed out","Reopen saved retention; reject refresh61, save0/manual, prove new history persists without display changes","Refresh now fetches new rows; reopen/save2 seconds, copy database path and prove automatic display resumes","Restart isolated backend and independently reopen retention and view settings; require scoped cleanup"]).unwrap()).unwrap();
}
