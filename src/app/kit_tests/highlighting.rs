use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use gpui::InputEvent as _;
    const GREEN: [u8; 3] = [17, 221, 119];
    const MAGENTA: [u8; 3] = [219, 23, 163];
    const CYAN: [u8; 3] = [9, 173, 213];
    fn background(
        handle: &crate::highlighting::Handle,
        view: crate::highlighting::View,
        value: &str,
        selected: bool,
    ) -> Option<[u8; 4]> {
        let key = if view == crate::highlighting::View::Rules {
            "name"
        } else {
            "process"
        };
        handle
            .snapshot()
            .style(view, key, value, &[(key, value)], selected)
            .and_then(|style| style.bgcolor)
    }

    fn row(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        id: impl Into<gpui::ElementId>,
    ) -> (gpui::Bounds<gpui::Pixels>, gpui::Size<gpui::Pixels>) {
        let id = id.into();
        cx.update_window(w, |_, w, cx| {
            w.render_frame(cx);
            (w.find(id).bounds(), w.viewport_size())
        })
        .unwrap()
    }
    fn wait(
        cx: &mut TestAppContext,
        h: &crate::highlighting::Handle,
        p: impl Fn(&crate::highlighting::Snapshot) -> bool,
    ) {
        wait_for(cx, |cx| {
            p(&h.snapshot()) && !(crate::app::highlighting::pending(cx))
        });
    }
    fn reveal(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) {
        for _ in 0..32 {
            let delta = cx
                .update_window(w, |_, w, cx| {
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
            let Some(delta) = delta else { return };
            cx.update_window(w, |_, w, cx| {
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
                        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(delta))),
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
        panic!("Highlight history control {id} unreachable")
    }
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
                name: "highlight-fixture".into(),
                rules: (0..2)
                    .map(|i| protocol::Rule {
                        name: format!("highlight.{i}"),
                        description: "Owned highlight rule".into(),
                        enabled: true,
                        action: "deny".into(),
                        duration: "always".into(),
                        operator: Some(protocol::Operator {
                            r#type: "simple".into(),
                            operand: "process.path".into(),
                            data: format!("/owned/highlight-rule-{i}"),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    backend.handle.history.ingest(
        "highlight-fixture".into(),
        ["cell", "row"]
            .into_iter()
            .enumerate()
            .map(|(i, kind)| protocol::Event {
                unixnano: crate::history::now_nanos() + i as i64,
                connection: Some(protocol::Connection {
                    process_path: format!("/owned/highlight-{kind}"),
                    dst_host: "owned-highlight.test".into(),
                    dst_port: 443,
                    protocol: "tcp".into(),
                    ..Default::default()
                }),
                rule: Some(protocol::Rule {
                    action: "deny".into(),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect(),
        "owned-highlight",
    );
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context(test_cx);
    let mut view = None;
    let main = cx
        .open_test_window(size(px(1240.), px(940.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = view.unwrap();
    wait(&mut cx, &backend.handle.highlighting, |s| s.ready);
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.history.page.rows.len()) == 2
    });
    let cell_index = cx.read_entity(&view, |v, _| {
        v.history
            .page
            .rows
            .iter()
            .position(|r| r.process == "/owned/highlight-cell")
            .unwrap()
    });
    let row_index = cx.read_entity(&view, |v, _| {
        v.history
            .page
            .rows
            .iter()
            .position(|r| r.process == "/owned/highlight-row")
            .unwrap()
    });
    use crate::highlighting::View;
    let cell = |selected| {
        background(
            &backend.handle.highlighting,
            View::History,
            "/owned/highlight-cell",
            selected,
        )
    };
    let row_style = || {
        background(
            &backend.handle.highlighting,
            View::History,
            "/owned/highlight-row",
            false,
        )
    };
    let rule_style = |selected| {
        background(
            &backend.handle.highlighting,
            View::Rules,
            "highlight.0",
            selected,
        )
    };
    let rgba = |[r, g, b]: [u8; 3]| Some([r, g, b, 255]);
    let baseline = evidence.join("01-default-no-custom-highlights.json");
    checkpoint(&mut cx, main, &baseline);
    assert!(cell(false).is_none() && row_style().is_none());
    let path = crate::highlighting::path_for(&config.database);
    let profile = serde_json::json!({"version":1,"settings":{"profiles":[{"view":"history","enabled":true,"cells":[{"cols":["process"],"text":["/owned/highlight-cell"],"color":"#000000","bgcolor":"#11dd77","alignment":["hcenter"],"margins":[2,1]}],"rows":[{"cols":["process"],"text":["highlight-row"],"bgcolor":"#1324ee"},{"cols":["process"],"text":["highlight-row"],"color":"#ffffff","bgcolor":"#db17a3"}]},{"view":"rules","enabled":true,"cells":[{"cols":["name"],"text":["highlight.0","highlight.1"],"color":"#000000","bgcolor":"#09add5"}],"rows":[]}]}});
    std::fs::write(&path, serde_json::to_vec_pretty(&profile).unwrap()).unwrap();
    click(&mut cx, main, "highlighting-reload");
    wait(&mut cx, &backend.handle.highlighting, |s| {
        s.ready && s.warning.is_none() && s.profiles.len() == 2
    });
    let active = evidence.join("02-history-cell-row-precedence.json");
    checkpoint(&mut cx, main, &active);
    let cell_area = row(&mut cx, main, ("history-row", cell_index));
    let _row_area = row(&mut cx, main, ("history-row", row_index));
    assert_eq!(cell(false), rgba(GREEN));
    assert_eq!(row_style(), rgba(MAGENTA));
    assert_eq!(
        row_style(),
        rgba(MAGENTA),
        "Later row rule replaces earlier background"
    );
    cx.update_window(main, |_, w, cx| w.click(("history-row", cell_index), cx))
        .unwrap();
    flush(&mut cx);
    assert_eq!(
        row(&mut cx, main, ("history-row", cell_index)),
        cell_area,
        "Selection does not move sampled history row"
    );
    let selected = evidence.join("03-history-selection-suppresses-style.json");
    checkpoint(&mut cx, main, &selected);
    assert!(cell(true).is_none());
    assert_eq!(row_style(), rgba(MAGENTA));
    cx.update_window(main, |_, w, cx| w.within("main-tabs").click(1usize, cx))
        .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.rules.page.rules.len()) == 2
    });
    let rules = evidence.join("04-rule-cells-highlighted.json");
    checkpoint(&mut cx, main, &rules);
    let rule_area = row(&mut cx, main, ("rule-row", 0usize));
    assert_eq!(rule_style(false), rgba(CYAN));
    cx.update_window(main, |_, w, cx| w.click(("rule-row", 0usize), cx))
        .unwrap();
    flush(&mut cx);
    // Rule selection exposes the action toolbar, so sample the row after layout.
    let selected_rule_area = row(&mut cx, main, ("rule-row", 0usize));
    let selected = evidence.join("05-rule-selection-suppresses-style.json");
    checkpoint(&mut cx, main, &selected);
    assert!(rule_style(true).is_none());
    assert_eq!(
        rule_style(false),
        rgba(CYAN),
        "Unselected rule keeps its custom style"
    );
    let revision = backend.handle.highlighting.snapshot().revision.clone();
    let invalid = b"{invalid owned highlighting file preserved}";
    std::fs::write(&path, invalid).unwrap();
    click(&mut cx, main, "highlighting-reload");
    wait(&mut cx, &backend.handle.highlighting, |s| {
        s.warning.is_some()
    });
    assert_eq!(std::fs::read(&path).unwrap(), invalid);
    assert_eq!(backend.handle.highlighting.snapshot().revision, revision);
    let last_good = evidence.join("06-invalid-reload-keeps-last-good.json");
    checkpoint(&mut cx, main, &last_good);
    assert_eq!(rule_style(false), rgba(CYAN));
    let ask = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/highlight-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    let mut popup = None;
    wait_for(&mut cx, |cx| {
        popup = cx.read_entity(&view, |v, _| v.prompt_window);
        popup.is_some()
    });
    let popup = popup.unwrap();
    checkpoint(
        &mut cx,
        popup,
        &evidence.join("07-permission-under-highlight-file-error.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(ask).unwrap().action, "deny");
    std::fs::write(&path, serde_json::to_vec_pretty(&profile).unwrap()).unwrap();
    click(&mut cx, main, "highlighting-reload");
    wait(&mut cx, &backend.handle.highlighting, |s| {
        s.warning.is_none()
    });
    cx.update_window(main, |_, w, cx| w.within("main-tabs").click(0usize, cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(main, |_, w, cx| {
        w.press("ctrl-f", cx);
        w.press("ctrl-a", cx);
        w.input("/owned/highlight", cx)
    })
    .unwrap();
    flush(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, main, "history-tools");
    flush(&mut cx);
    let tools = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    });
    cx.update_window(tools, |_, w, cx| {
        w.within("history-tools-tabs").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    reveal(&mut cx, tools, "history-table-mode");
    click(&mut cx, tools, "history-table-mode");
    reveal(&mut cx, tools, "history-columns-save");
    click(&mut cx, tools, "history-columns-save");
    flush(&mut cx);
    click(&mut cx, tools, "history-tools-close");
    flush(&mut cx);
    let table = evidence.join("08-pinned-table-profile-paint.json");
    checkpoint(&mut cx, main, &table);
    assert!(cell(false) == rgba(GREEN) && row_style() == rgba(MAGENTA));
    let mut disabled = profile.clone();
    for p in disabled["settings"]["profiles"].as_array_mut().unwrap() {
        p["enabled"] = false.into()
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&disabled).unwrap()).unwrap();
    click(&mut cx, main, "highlighting-reload");
    wait(&mut cx, &backend.handle.highlighting, |s| {
        s.warning.is_none() && s.settings.profiles.iter().all(|p| !p.enabled)
    });
    let cleared = evidence.join("09-disabled-profiles-clear-paint.json");
    checkpoint(&mut cx, main, &cleared);
    assert!(cell(false).is_none() && row_style().is_none());
    std::fs::write(
        evidence.join("highlight-style-proof.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "kind":"configured-style-state", "real_gpu":false, "selected_style_suppressed":true,
            "last_good_revision_preserved":true, "disabled_profiles_clear_style":true,
            "rule_bounds_before_selection":format!("{rule_area:?}"),
            "rule_bounds_after_selection":format!("{selected_rule_area:?}")
        }))
        .unwrap(),
    )
    .unwrap();
    drop(view);
    shutdown(cx);
    backend.join();
    drop(runtime);
    assert!(!config.socket.exists());
    let checkpoints = (1..=9)
        .map(|n| {
            std::fs::read_dir(&evidence)
                .unwrap()
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().to_string())
                .find(|f| f.starts_with(&format!("{n:02}-")) && f.ends_with(".json"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":checkpoints,"proof_boundary":"Owned file profile and Kit Reload input; configured cell/row style precedence, History/Rules selected-style suppression, invalid reload preserving last-good bytes/styles, independent permission input and disabled-profile clearing. Assertions inspect style state and control layout; no screenshot, painted-pixel or real-font validation. Other style view categories, integer-column import mapping, native compositor, real daemon enforcement and package/RSS remain separate."})).unwrap()).unwrap();
}
