use super::*;
// User-input regression for the two main browsing pages, using owned fixtures.

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    fn drag(cx: &mut TestAppContext, main: AnyWindowHandle, viewport: &'static str, down: bool) {
        cx.update_window(main, |_, w, cx| {
            w.render_frame(cx);
            let bounds = w.find(viewport).bounds();
            let inset = if viewport == "history-table-viewport" {
                gpui_kit::component::Size::default().table_row_height()
            } else {
                px(0.)
            };
            let top = gpui::point(bounds.right() - px(6.), bounds.top() + inset + px(12.));
            let bottom = gpui::point(top.x, bounds.bottom() - px(12.));
            let (from, to) = if down { (top, bottom) } else { (bottom, top) };
            w.drag(from, to, cx);
        })
        .unwrap();
        flush(cx);
    }
    fn bottom_row(
        cx: &mut TestAppContext,
        main: AnyWindowHandle,
        viewport: &'static str,
        row: &'static str,
    ) {
        cx.update_window(main, |_, w, cx| {
            w.render_frame(cx);
            let bounds = w.find(viewport).bounds();
            let last = w.find((row, 199usize)).bounds();
            assert!(
                last.center().y >= bounds.top() && last.center().y <= bounds.bottom(),
                "drag must reveal row 200"
            );
        })
        .unwrap();
    }
    fn history(cx: &mut TestAppContext, view: &gpui::Entity<Dashboard>, node: &str, count: usize) {
        wait_for(cx, |cx| {
            cx.read_entity(view, |v, _| {
                v.history.page.rows.len() == count
                    && (node.is_empty() || v.history.page.rows.iter().all(|row| row.node == node))
            })
        });
    }
    fn rules(cx: &mut TestAppContext, view: &gpui::Entity<Dashboard>, action: &str, count: usize) {
        wait_for(cx, |cx| {
            cx.read_entity(view, |v, _| {
                v.rules.page.rules.len() == count
                    && (action.is_empty()
                        || v.rules.page.rules.iter().all(|rule| rule.action == action))
            })
        });
    }
    let evidence = evidence();
    let owned = tempfile::tempdir().unwrap();
    let config = options(owned.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (daemon, sender, stream) = runtime.block_on(async {
        use hyper_util::rt::TokioIo;
        let socket = config.socket.clone();
        let channel = tonic::transport::Endpoint::try_from("http://localhost")
            .unwrap()
            .connect_with_connector(tower::service_fn(move |_| {
                let socket = socket.clone();
                async move {
                    tokio::net::UnixStream::connect(socket)
                        .await
                        .map(TokioIo::new)
                }
            }))
            .await
            .unwrap();
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "node-a".into(),
                rules: (0..660)
                    .map(|n| protocol::Rule {
                        name: format!("owned.app.{n:04}"),
                        enabled: true,
                        created: 1_700_000_000 + n,
                        action: ["allow", "deny", "reject"][n as usize % 3].into(),
                        duration: if n % 2 == 0 { "always" } else { "5m" }.into(),
                        operator: Some(protocol::Operator {
                            r#type: "simple".into(),
                            operand: "process.path".into(),
                            data: format!("/owned/app-{n}"),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
            .await
            .unwrap();
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        let stream = daemon
            .notifications(tokio_stream::wrappers::ReceiverStream::new(receiver))
            .await
            .unwrap()
            .into_inner();
        (daemon, sender, stream)
    });
    let base = crate::history::now_nanos();
    for (node, offset) in [("node-a", 0), ("node-b-offline", 1000)] {
        backend.handle.history.ingest(
            node.into(),
            (0..260)
                .map(|n| protocol::Event {
                    unixnano: base + offset + n,
                    connection: Some(protocol::Connection {
                        process_path: format!("/owned/app-{n:03}"),
                        dst_host: "owned-browse.test".into(),
                        protocol: "tcp".into(),
                        dst_port: 443,
                        ..Default::default()
                    }),
                    rule: Some(protocol::Rule {
                        action: if n % 2 == 0 { "allow" } else { "deny" }.into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .collect(),
            "owned-browse",
        );
    }
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context(test_cx);
    let mut view = None;
    let main = cx
        .open_test_window(size(px(1200.), px(900.)), |w, cx| {
            let dashboard =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(dashboard.clone());
            cx.new(|cx| gpui_kit::base::Root::new(dashboard, w, cx))
        })
        .unwrap()
        .into();
    let view = view.unwrap();
    history(&mut cx, &view, "", 200);
    let mut frames = Vec::new();
    let mut snap = |cx: &mut TestAppContext, name: &str| {
        checkpoint(cx, main, &evidence.join(name));
        frames.push(name.to_owned());
    };
    cx.update_window(main, |_, w, cx| {
        w.render_frame(cx);
        let tabs = w.find("main-navigation").bounds();
        let node = w.find("history-node").bounds();
        assert!(
            node.top() - tabs.bottom() >= px(12.),
            "toolbar needs space below navigation"
        );
    })
    .unwrap();
    snap(&mut cx, "01-history-spacing-and-all-nodes.json");
    pick(&mut cx, main, "history-node", "node-b-offline");
    history(&mut cx, &view, "node-b-offline", 200);
    click(&mut cx, main, "deny");
    history(&mut cx, &view, "node-b-offline", 130);
    snap(&mut cx, "02-offline-node-and-action.json");
    click(&mut cx, main, "all");
    history(&mut cx, &view, "node-b-offline", 200);
    drag(&mut cx, main, "history-list-viewport", true);
    bottom_row(&mut cx, main, "history-list-viewport", "history-row");
    snap(&mut cx, "03-history-scrollbar-dragged.json");
    drag(&mut cx, main, "history-list-viewport", false);
    pick(&mut cx, main, "history-node", "All nodes");
    assert!(cx.read_entity(&view, |v, _| v.history.query.node.is_empty()));
    pick(&mut cx, main, "history-node", "node-b-offline");
    history(&mut cx, &view, "node-b-offline", 200);
    cx.update_window(main, |_, w, cx| w.press("ctrl-2", cx))
        .unwrap();
    rules(&mut cx, &view, "", 200);
    snap(&mut cx, "04-rules-spacing-and-action-filters.json");
    for (id, action) in [
        ("rules-action-allow", "allow"),
        ("rules-action-deny", "deny"),
        ("rules-action-reject", "reject"),
    ] {
        click(&mut cx, main, id);
        rules(&mut cx, &view, action, 200);
        assert!(cx.read_entity(&view, |v, _| v.rules.page.has_more));
        click(&mut cx, main, "rules-next");
        rules(&mut cx, &view, action, 20);
        assert!(!cx.read_entity(&view, |v, _| v.rules.page.has_more));
    }
    click(&mut cx, main, "rules-permanent");
    rules(&mut cx, &view, "reject", 110);
    assert!(cx.read_entity(&view, |v, _| {
        v.rules
            .page
            .rules
            .iter()
            .all(|rule| rule.duration == "always")
    }));
    snap(&mut cx, "05-rule-action-and-duration.json");
    click(&mut cx, main, "rules-all");
    click(&mut cx, main, "rules-action-all");
    rules(&mut cx, &view, "", 200);
    drag(&mut cx, main, "rules-list-viewport", true);
    bottom_row(&mut cx, main, "rules-list-viewport", "rule-row");
    snap(&mut cx, "06-rules-scrollbar-dragged.json");
    cx.update_window(main, |_, w, cx| w.press("ctrl-1", cx))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, main, "history-tools");
    flush(&mut cx);
    let tool = cx.update(|cx| cx.windows().into_iter().find(|w| *w != main).unwrap());
    cx.update_window(tool, |_, w, cx| {
        w.within("history-tools-tabs").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, tool, "history-table-mode");
    // Save is below the column list: reach it with actual wheel input.
    for _ in 0..20 {
        let visible = cx
            .update_window(tool, |_, w, _| {
                let panel = w.find("history-tools-panel").bounds();
                let button = w.find("history-columns-save").bounds();
                button.center().y <= panel.bottom() - px(8.)
            })
            .unwrap();
        if visible {
            break;
        }
        cx.update_window(tool, |_, w, cx| {
            w.scroll(
                "history-tools-panel",
                gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-220.))),
                cx,
            )
        })
        .unwrap();
        flush(&mut cx);
    }
    click(&mut cx, tool, "history-columns-save");
    flush(&mut cx);
    click(&mut cx, tool, "history-tools-close");
    history(&mut cx, &view, "node-b-offline", 200);
    let before = cx.read_entity(&view, |v, cx| v.history.test_table_offset(cx));
    drag(&mut cx, main, "history-table-viewport", true);
    let after = cx.read_entity(&view, |v, cx| v.history.test_table_offset(cx));
    assert!(
        after.y < before.y - px(100.),
        "table scrollbar must scroll actual rows: {before:?} -> {after:?}"
    );
    snap(&mut cx, "07-history-table-scrollbar-dragged.json");
    // The production Quit action flushes pending presentation saves.
    click(&mut cx, main, "settings-menu-trigger");
    flush(&mut cx);
    click(&mut cx, main, "quit");
    wait_for(&mut cx, |_cx| backend.handle.cancel.is_cancelled());
    let saved = crate::history::settings::Store::open(&config.database)
        .unwrap()
        .snapshot();
    assert_eq!(saved.presentation.node, "node-b-offline");
    assert!(saved.table);
    drop(stream);
    drop(sender);
    drop(daemon);
    shutdown(cx);
    backend.handle.shutdown();
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "status":"passed", "framework":"gpui-kit","real_gpu":false, "scoped_socket_removed":true, "checkpoints":frames,
        "proof_boundary":"Actual node/action/duration buttons, offline history, whole-catalogue paging, spacing, pointer-drag list/table scrollbars and persisted node/table choices through production Quit. Owned daemon/database fixtures; no real daemon or packaged/compositor proof."
    })).unwrap()).unwrap();
}
