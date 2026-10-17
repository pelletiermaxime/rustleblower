use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    // Filter controls can be below the viewport with the Ubuntu release fonts.
    // Scroll through real input before clicking, including after Clear reflows it.
    fn control(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
        use gpui::InputEvent as _;
        for _ in 0..32 {
            let direction = cx
                .update_window(window, |_, w, cx| {
                    w.render_frame(cx);
                    let body = w.find("history-tools-panel").bounds();
                    let target = w.find(id).bounds();
                    if target.center().y < body.top() + px(8.) {
                        Some(120.)
                    } else if target.center().y > body.bottom() - px(8.) {
                        Some(-120.)
                    } else {
                        None
                    }
                })
                .unwrap();
            let Some(direction) = direction else {
                click(cx, window, id);
                flush(cx);
                return;
            };
            cx.update_window(window, |_, w, cx| {
                let position = w.find("history-tools-panel").bounds().center();
                w.dispatch_event(
                    gpui::MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                w.render_frame(cx);
                w.dispatch_event(
                    gpui::ScrollWheelEvent {
                        position,
                        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(direction))),
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                w.render_frame(cx);
            })
            .unwrap();
            flush(cx);
        }
        panic!("History control {id} must be reachable by wheel input");
    }

    fn search(cx: &mut TestAppContext, w: AnyWindowHandle, value: &str) {
        cx.update_window(w, |_, w, cx| w.press("ctrl-f", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(w, |_, w, cx| {
            w.press("ctrl-a", cx);
            w.input(value, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    use crate::history::query::Field;
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let events = (0..430usize)
        .map(|index| protocol::Event {
            unixnano: crate::history::now_nanos() + index as i64,
            connection: Some(protocol::Connection {
                process_path: format!("/usr/bin/history-app-{}", index % 7),
                process_args: vec![
                    "tool".into(),
                    format!("--session blue{}", index % 4),
                    "comma, \"argument\"".into(),
                ],
                src_ip: "192.0.2.10".into(),
                src_port: 40000,
                dst_ip: "198.51.100.20".into(),
                dst_host: "api.example.test".into(),
                dst_port: 443,
                protocol: "tcp".into(),
                user_id: if index.is_multiple_of(2) { 0 } else { 1001 },
                process_id: 100 + index as u32,
                ..Default::default()
            }),
            rule: Some(protocol::Rule {
                name: "history-filter-rule".into(),
                action: if index.is_multiple_of(3) {
                    "reject"
                } else {
                    "allow"
                }
                .into(),
                duration: "5m".into(),
                precedence: true,
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    backend
        .handle
        .history
        .ingest("history-tools-fixture".into(), events, "observed");
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context(test_cx);
    // Use the Ubuntu package runner's font for layout and control reachability.
    cx.update(|cx| Theme::global_mut(cx).font_family = "DejaVu Sans".into());
    let mut view = None;
    let window = cx
        .open_test_window(size(px(1100.), px(780.)), |window, cx| {
            let dashboard =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(dashboard.clone());
            cx.new(|cx| gpui_kit::base::Root::new(dashboard, window, cx))
        })
        .unwrap();
    let view = view.unwrap();
    flush(&mut cx);
    click(&mut cx, window.into(), "reject");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.history.page.rows.len())) == (144)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows.len()),
        144
    );
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("01-rejected-history.json"),
    );
    click(&mut cx, window.into(), "all");
    flush(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window.into(), "history-tools");
    flush(&mut cx);
    let tools = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|handle| !before.contains(handle))
            .unwrap()
    });
    pick(&mut cx, tools, "history-filter-field", "UID");
    pick(&mut cx, tools, "history-filter-comparison", "Equals");
    fill(&mut cx, tools, "history-filter-value", "1001");
    control(&mut cx, tools, "history-filter-add");
    pick(&mut cx, tools, "history-filter-field", "Command line");
    pick(&mut cx, tools, "history-filter-comparison", "Contains");
    fill(&mut cx, tools, "history-filter-value", "blue1");
    control(&mut cx, tools, "history-filter-add");
    control(&mut cx, tools, "history-filter-apply");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.history.page.rows.len())) == (108)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows.len()),
        108
    );
    assert!(
        cx.read_entity(&view, |view, _| view.history.page.rows.iter().all(
            |row| row.metadata.user == Some(1001) && row.metadata.command.contains("blue1")
        ))
    );
    checkpoint(&mut cx, tools, &evidence.join("02-history-conditions.json"));
    pick(&mut cx, tools, "history-filter-field", "Destination port");
    pick(&mut cx, tools, "history-filter-comparison", "Equals");
    fill(&mut cx, tools, "history-filter-value", "65536");
    control(&mut cx, tools, "history-filter-add");
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("03-invalid-history-port.json"),
    );
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.query.filters.len()),
        2
    );
    cx.update_window(tools, |_, window, cx| {
        window.within("history-tools-tabs").click(1usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, tools, "history-table-mode");
    cx.update_window(tools, |_, window, cx| {
        window.scroll(
            "history-table-mode",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-300.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, tools, "history-column-user");
    click(&mut cx, tools, "history-column-command");
    cx.update_window(tools, |_, window, cx| {
        window.scroll(
            "history-column-command",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-1000.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, tools, "history-columns-save");
    flush(&mut cx);
    let saved = crate::history::settings::Store::open(&config.database)
        .unwrap()
        .snapshot();
    assert!(
        saved.table
            && saved.columns.contains(&Field::User)
            && saved.columns.contains(&Field::Command)
    );
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("04-saved-history-columns.json"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.within("table").click(("icon-sort", 0usize), cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.history.query.sort)) == (Some((Field::Time, false)))
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.query.sort),
        Some((Field::Time, false))
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.within("table").click(("icon-sort", 0usize), cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.history.query.sort)) == (Some((Field::Time, true)))
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.query.sort),
        Some((Field::Time, true))
    );
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows[0].metadata.pid),
        Some(101)
    );
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("05-sorted-history-table.json"),
    );
    cx.update_window(tools, |_, window, cx| {
        window.within("history-tools-tabs").click(2usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    let csv = evidence.join("filtered-history.csv");
    fill(&mut cx, tools, "history-export-path", csv.to_str().unwrap());
    click(&mut cx, tools, "history-export-save");
    wait_for(&mut cx, |_cx| csv.exists());
    flush(&mut cx);
    let original = std::fs::read_to_string(&csv).unwrap();
    assert_eq!(original.lines().count(), 109);
    assert!(original.contains("Command line") && original.contains("\"\"argument\"\""));
    checkpoint(&mut cx, tools, &evidence.join("06-history-csv-export.json"));
    click(&mut cx, tools, "history-export-save");
    flush(&mut cx);
    assert_eq!(std::fs::read_to_string(&csv).unwrap(), original);
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("07-history-existing-file.json"),
    );
    let json = evidence.join("filtered-history.jsonl");
    fill(
        &mut cx,
        tools,
        "history-export-path",
        json.to_str().unwrap(),
    );
    pick(&mut cx, tools, "history-export-format", "JSON Lines");
    click(&mut cx, tools, "history-export-save");
    wait_for(&mut cx, |_cx| json.exists());
    flush(&mut cx);
    let text = std::fs::read_to_string(&json).unwrap();
    assert_eq!(text.lines().count(), 108);
    assert!(text.lines().all(|line| {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        value["user"] == "1001" && value["command"].as_str().unwrap().contains("blue1")
    }));
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("08-history-json-export.json"),
    );
    click(&mut cx, tools, "history-copy-page");
    flush(&mut cx);
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(copied.lines().count(), 109);
    assert!(copied.contains("blue1"));
    checkpoint(&mut cx, tools, &evidence.join("09-history-clipboard.json"));
    cx.update_window(tools, |_, window, cx| {
        window.within("history-tools-tabs").click(0usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    control(&mut cx, tools, "history-filter-clear");
    control(&mut cx, tools, "history-filter-apply");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.history.page.rows.len())) == (200)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows.len()),
        200
    );
    click(&mut cx, window.into(), "older");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.history.page.rows.len())) == (200)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows.len()),
        200
    );
    assert!(cx.read_entity(&view, |view, _| view.history.query.cursor.is_some()));
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("10-history-sorted-next-page.json"),
    );
    // Persist actual user choices, including the Kit resize gesture.
    pick(&mut cx, tools, "history-filter-field", "UID");
    pick(&mut cx, tools, "history-filter-comparison", "Equals");
    fill(&mut cx, tools, "history-filter-value", "1001");
    control(&mut cx, tools, "history-filter-add");
    control(&mut cx, tools, "history-filter-apply");
    click(&mut cx, window.into(), "reject");
    search(&mut cx, window.into(), "history-app");
    flush(&mut cx);
    let before_width = cx
        .update_window(window.into(), |_, w, _| {
            w.within("table")
                .find(("col-header", 0usize))
                .bounds()
                .size
                .width
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        let header = w.within("table").find(("col-header", 0usize)).bounds();
        let from = gpui::point(header.right() - px(2.), header.center().y);
        w.drag(from, gpui::point(from.x + px(80.), from.y), cx);
    })
    .unwrap();
    flush(&mut cx);
    let after_width = cx
        .update_window(window.into(), |_, w, _| {
            w.within("table")
                .find(("col-header", 0usize))
                .bounds()
                .size
                .width
        })
        .unwrap();
    assert!(
        after_width > before_width + px(30.),
        "Actual Kit resize must change the visible header width"
    );
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.history.memory.settled())
    });
    let saved = crate::history::settings::Store::open(&config.database)
        .unwrap()
        .snapshot();
    assert_eq!(saved.presentation.search, "history-app");
    assert_eq!(saved.presentation.action, "reject");
    assert_eq!(saved.presentation.filters.len(), 1);
    assert_eq!(saved.presentation.sort, Some((Field::Time, true)));
    assert!(
        saved
            .presentation
            .widths
            .iter()
            .any(|(f, w)| *f == Field::Time && *w > 270.)
    );
    assert_eq!(cx.read_entity(&view, |v, _| v.history.page.rows.len()), 72);
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("11-history-remembered-controls.json"),
    );
    click(&mut cx, tools, "history-tools-close");
    // Quit must flush the final edit even before the ordinary debounce expires.
    search(&mut cx, window.into(), "history-app-");
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("12-history-before-quit.json"),
    );
    click(&mut cx, window.into(), "quit");
    drop(view);
    shutdown(cx);
    backend.join();
    let settings_path = config.database.with_extension("sqlite3.history.json");
    let durable = std::fs::read(&settings_path).unwrap();
    let saved = crate::history::settings::Store::open(&config.database)
        .unwrap()
        .snapshot();
    assert_eq!(saved.presentation.search, "history-app-");
    std::fs::write(evidence.join("saved-history-view.json"), &durable).unwrap();
    let backend = Backend::start(config.clone()).unwrap();
    let mut cx = context(test_cx);
    let mut view = None;
    let window = cx
        .open_test_window(size(px(1100.), px(780.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let view = view.unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.history.query.search.clone()))
            == (saved.presentation.search)
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.search.clone()),
        saved.presentation.search
    );
    assert_eq!(
        cx.read_entity(&view, |v, cx| v.history.input.read(cx).value().to_string()),
        saved.presentation.search
    );
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.action.clone()),
        "reject"
    );
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.filters.clone()),
        saved.presentation.filters
    );
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.sort),
        Some((Field::Time, true))
    );
    assert!(
        cx.read_entity(&view, |v, _| v.history.query.before.is_none()
            && v.history.query.cursor.is_none()
            && v.history.page.rows.len() == 72)
    );
    let restored_width = cx
        .update_window(window.into(), |_, w, _| {
            w.within("table")
                .find(("col-header", 0usize))
                .bounds()
                .size
                .width
        })
        .unwrap();
    assert!((f32::from(restored_width) - f32::from(after_width)).abs() < 2.);
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("13-history-cold-restored.json"),
    );
    // An external edit stops automatic writes, without blocking actual RPC prompts.
    let external = b"{invalid external edit preserved}";
    std::fs::write(&settings_path, external).unwrap();
    search(&mut cx, window.into(), "no-such-history-app");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.history.memory.error().is_some())
    });

    assert!(cx.read_entity(&view, |v, _| v.history.memory.error().is_some()));
    assert_eq!(std::fs::read(&settings_path).unwrap(), external);
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("14-history-session-only.json"),
    );
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
        let mut client = protocol::ui_client::UiClient::new(channel);
        client
            .subscribe(protocol::ClientConfig {
                name: "history-view-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        client
    });
    let reply = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/history-view-permission".into(),
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
        &evidence.join("15-history-save-error-permission.json"),
    );
    click(&mut cx, popup, "deny-once");
    let decision = runtime.block_on(reply).unwrap();
    assert_eq!(decision.action, "deny");
    assert_eq!(decision.duration, "once");
    click(&mut cx, window.into(), "allow");
    flush(&mut cx);
    assert_eq!(std::fs::read(&settings_path).unwrap(), external);
    click(&mut cx, window.into(), "quit");
    drop(view);
    shutdown(cx);
    backend.join();
    // Repair only this fixture, then prove a fresh process can restore again.
    std::fs::write(&settings_path, &durable).unwrap();
    let backend = Backend::start(config.clone()).unwrap();
    let mut cx = context(test_cx);
    let mut view = None;
    let window = cx
        .open_test_window(size(px(1100.), px(780.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let view = view.unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.history.query.search.clone()))
            == (saved.presentation.search)
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.search.clone()),
        saved.presentation.search
    );
    assert!(cx.read_entity(&view, |v, _| v.history.memory.error().is_none()));
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("16-history-repaired-restart.json"),
    );
    click(&mut cx, window.into(), "quit");
    drop(view);
    shutdown(cx);
    backend.join();
    drop(runtime);
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","checkpoints":["01-rejected-history.json","02-history-conditions.json","03-invalid-history-port.json","04-saved-history-columns.json","05-sorted-history-table.json","06-history-csv-export.json","07-history-existing-file.json","08-history-json-export.json","09-history-clipboard.json","10-history-sorted-next-page.json","11-history-remembered-controls.json","12-history-before-quit.json","13-history-cold-restored.json","14-history-session-only.json","15-history-save-error-permission.json","16-history-repaired-restart.json"],"framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"proof_boundary":"Real GPUI history conditions/column/header sort/resize/export input; actual cold backend and GUI restore of search/action/conditions/sort/widths, fresh pagination, immediate Quit flush, external-edit preservation and independent AskRule under save error, explicit fixture repair/restart; native compositor tools/pickers/width dragging and real daemon enforcement remain separate"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Filter Rejected; combine UID equals 1001 and command contains blue1; reject invalid port 65536 without changing conditions", "Enable pinned Kit table, UID and command columns; save/reopen history settings; sort all matching history by time through the actual header", "Export all 108 matches to CSV and JSON Lines, preserve existing destination on refusal, copy a matching page; clear conditions and page sorted results with a frozen ceiling"]).unwrap()).unwrap();
}
