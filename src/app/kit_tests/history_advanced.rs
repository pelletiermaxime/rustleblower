use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::history::{
        query::{Group, Logic, Match},
        settings::Store,
    };
    use gpui::InputEvent as _;
    fn reveal(cx: &mut TestAppContext, window: AnyWindowHandle, id: gpui::ElementId) {
        for _ in 0..32 {
            let direction = cx
                .update_window(window, |_, w, cx| {
                    w.render_frame(cx);
                    let body = w.find("history-tools-panel").bounds();
                    let target = w.find(id.clone()).bounds();
                    if target.center().y < body.top() + px(8.) {
                        Some(120.)
                    } else if target.center().y > body.bottom() - px(8.) {
                        Some(-120.)
                    } else {
                        None
                    }
                })
                .unwrap();
            let Some(direction) = direction else { return };
            cx.update_window(window, |_, w, cx| {
                let body = w.find("history-tools-panel").bounds();
                w.dispatch_event(
                    gpui::MouseMoveEvent {
                        position: body.center(),
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                w.render_frame(cx);
                w.dispatch_event(
                    gpui::ScrollWheelEvent {
                        position: body.center(),
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
        panic!("History control {id:?} must be reachable by wheel input");
    }
    fn control(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<gpui::ElementId>) {
        let id = id.into();
        reveal(cx, w, id.clone());
        cx.update_window(w, |_, w, cx| w.click(id, cx)).unwrap();
        flush(cx);
    }
    fn condition(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        field: &str,
        comparison: &str,
        value: &str,
    ) {
        reveal(cx, w, "history-filter-field".into());
        pick(cx, w, "history-filter-field", field);
        reveal(cx, w, "history-filter-comparison".into());
        pick(cx, w, "history-filter-comparison", comparison);
        reveal(cx, w, "history-filter-value".into());
        fill(cx, w, "history-filter-value", value);
        control(cx, w, "history-filter-add");
    }
    fn rows(cx: &mut TestAppContext, v: &gpui::Entity<Dashboard>, expected: &[usize]) {
        let wanted = expected
            .iter()
            .map(|i| format!("/owned/history-advanced-{i}"))
            .collect::<std::collections::BTreeSet<_>>();
        wait_for(cx, |cx| {
            let actual = cx.read_entity(v, |v, _| {
                v.history
                    .page
                    .rows
                    .iter()
                    .map(|r| r.process.clone())
                    .collect::<std::collections::BTreeSet<_>>()
            });
            actual == wanted
        });
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
    let fixtures = [
        ("/work/a", 443, "zero.demo"),
        ("/work/a", 80, "one.test"),
        ("/work/b", 80, "two.test"),
        ("/work/b", 80, "three.demo"),
        ("/work/a", 444, "four.test"),
        ("/work/b", 443, "five.test"),
    ];
    backend.handle.history.ingest(
        "advanced-history-fixture".into(),
        fixtures
            .iter()
            .enumerate()
            .map(|(i, (cwd, port, host))| protocol::Event {
                unixnano: crate::history::now_nanos() + i as i64,
                connection: Some(protocol::Connection {
                    process_path: format!("/owned/history-advanced-{i}"),
                    process_cwd: (*cwd).into(),
                    dst_port: *port,
                    dst_host: (*host).into(),
                    protocol: "tcp".into(),
                    ..Default::default()
                }),
                rule: Some(protocol::Rule {
                    action: "allow".into(),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect(),
        "observed",
    );
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context(test_cx);
    let mut view = None;
    let main = cx
        .open_test_window(size(px(1240.), px(940.)), |window, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = view.unwrap();
    rows(&mut cx, &view, &[0, 1, 2, 3, 4, 5]);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, main, "history-tools");
    flush(&mut cx);
    let tools = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    });
    control(&mut cx, tools, "history-filter-root-mode");
    condition(
        &mut cx,
        tools,
        "Working directory",
        "Starts with",
        "/work/a",
    );
    condition(
        &mut cx,
        tools,
        "Destination port",
        "At least / at or after",
        "443",
    );
    control(&mut cx, tools, "history-filter-new-group");
    condition(
        &mut cx,
        tools,
        "Working directory",
        "Does not contain",
        "/work/a",
    );
    condition(&mut cx, tools, "Destination host", "Ends with", ".test");
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("01-two-group-expression.json"),
    );
    control(&mut cx, tools, "history-filter-apply");
    rows(&mut cx, &view, &[0, 2, 4, 5]);
    let mut expected = Logic {
        mode: Match::Any,
        groups: vec![
            Group {
                count: 2,
                mode: Match::All,
            },
            Group {
                count: 2,
                mode: Match::All,
            },
        ],
        ..Default::default()
    };
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.logic.clone()),
        expected
    );
    assert!(
        cx.read_entity(&view, |v, _| v
            .history
            .page
            .rows
            .iter()
            .any(|r| r.port == 443 && r.metadata.cwd == "/work/a")),
        "Inclusive boundary is present"
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("02-exact-grouped-history.json"),
    );
    control(&mut cx, tools, ("history-filter-group-mode", 1usize));
    control(&mut cx, tools, "history-filter-apply");
    rows(&mut cx, &view, &[0, 1, 2, 3, 4, 5]);
    checkpoint(
        &mut cx,
        main,
        &evidence.join("03-inner-any-expands-results.json"),
    );
    control(&mut cx, tools, ("history-filter-group-mode", 1usize));
    control(&mut cx, tools, "history-filter-root-mode");
    control(&mut cx, tools, "history-filter-apply");
    rows(&mut cx, &view, &[]);
    checkpoint(&mut cx, main, &evidence.join("04-root-all-empty.json"));
    control(&mut cx, tools, "history-filter-root-mode");
    control(&mut cx, tools, "history-filter-apply");
    rows(&mut cx, &view, &[0, 2, 4, 5]);
    for invalid in [
        "1 OR 1 OR 2 OR 3 OR 4",
        "1 AND 2 AND 3 AND 9",
        "1 OR 2 OR 3 OR 4; DROP TABLE events",
    ] {
        reveal(&mut cx, tools, "history-filter-formula".into());
        fill(&mut cx, tools, "history-filter-formula", invalid);
        control(&mut cx, tools, "history-filter-apply");
        rows(&mut cx, &view, &[0, 2, 4, 5]);
        assert_eq!(
            cx.read_entity(&view, |v, _| v.history.query.logic.clone()),
            expected
        );
    }
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("08-invalid-history-formulas-refused.json"),
    );
    reveal(&mut cx, tools, "history-filter-formula".into());
    fill(
        &mut cx,
        tools,
        "history-filter-formula",
        "4 OR (1 AND (3 OR 2))",
    );
    control(&mut cx, tools, "history-filter-apply");
    rows(&mut cx, &view, &[0, 1, 2, 4, 5]);
    expected.formula = "4 OR (1 AND (3 OR 2))".into();
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.logic.clone()),
        expected
    );
    reveal(&mut cx, tools, "history-filter-root-mode".into());
    let group_label = cx
        .update_window(tools, |_, w, _| {
            w.find("history-filter-root-mode")
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    control(&mut cx, tools, "history-filter-root-mode");
    assert_eq!(
        cx.update_window(tools, |_, w, _| w
            .find("history-filter-root-mode")
            .label()
            .map(str::to_owned))
            .unwrap(),
        group_label
    );

    checkpoint(
        &mut cx,
        main,
        &evidence.join("09-nested-history-formula.json"),
    );
    cx.update_window(tools, |_, w, cx| {
        w.within("history-tools-tabs").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    control(&mut cx, tools, "history-column-cwd");
    control(&mut cx, tools, "history-columns-save");
    cx.update_window(tools, |_, w, cx| {
        w.within("history-tools-tabs").click(2usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    let export = evidence.join("advanced-history.jsonl");
    reveal(&mut cx, tools, "history-export-path".into());
    fill(
        &mut cx,
        tools,
        "history-export-path",
        export.to_str().unwrap(),
    );
    reveal(&mut cx, tools, "history-export-format".into());
    pick(&mut cx, tools, "history-export-format", "JSON Lines");
    control(&mut cx, tools, "history-export-save");
    wait_for(&mut cx, |_cx| export.exists());
    let values = std::fs::read_to_string(&export)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 5);
    assert_eq!(
        values
            .iter()
            .map(|v| v["process"].as_str().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>(),
        [0, 1, 2, 4, 5]
            .into_iter()
            .map(|i| format!("/owned/history-advanced-{i}"))
            .collect()
    );
    assert!(
        values
            .iter()
            .all(|v| matches!(v["cwd"].as_str(), Some("/work/a" | "/work/b")))
    );
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("05-grouped-json-export.json"),
    );
    let mut saved = None;
    for _ in 0..50 {
        flush(&mut cx);
        let value = Store::open(&config.database).unwrap().snapshot();
        if value.presentation.logic == expected && value.presentation.filters.len() == 4 {
            saved = Some(value);
            break;
        }
    }
    let saved = saved.expect("Independent saved expression reopen");
    std::fs::write(
        evidence.join("saved-history-options.json"),
        serde_json::to_vec_pretty(&saved).unwrap(),
    )
    .unwrap();
    click(&mut cx, tools, "history-tools-close");
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
    reveal(&mut cx, tools, "history-filter-formula".into());
    control(&mut cx, tools, "history-filter-formula");
    cx.update_window(tools, |_, w, cx| {
        w.press("ctrl-a", cx);
        w.press("ctrl-c", cx);
    })
    .unwrap();
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        "4 OR (1 AND (3 OR 2))"
    );
    reveal(&mut cx, tools, "history-filter-root-mode".into());
    let group_label = cx
        .update_window(tools, |_, w, _| {
            w.find("history-filter-root-mode")
                .label()
                .map(str::to_owned)
        })
        .unwrap();
    control(&mut cx, tools, "history-filter-root-mode");
    assert_eq!(
        cx.update_window(tools, |_, w, _| w
            .find("history-filter-root-mode")
            .label()
            .map(str::to_owned))
            .unwrap(),
        group_label
    );

    checkpoint(
        &mut cx,
        tools,
        &evidence.join("06-reopened-grouped-expression.json"),
    );
    control(&mut cx, tools, ("history-filter-remove", 2usize));
    control(&mut cx, tools, "history-filter-apply");
    rows(&mut cx, &view, &[0, 1, 2, 4, 5]);
    assert!(cx.read_entity(&view, |v, _| v.history.query.logic.formula.is_empty()));
    assert_eq!(
        cx.read_entity(&view, |v, _| v
            .history
            .query
            .logic
            .groups
            .iter()
            .map(|g| g.count)
            .collect::<Vec<_>>()),
        vec![2, 1]
    );
    control(&mut cx, tools, "history-filter-clear");
    control(&mut cx, tools, "history-filter-apply");
    rows(&mut cx, &view, &[0, 1, 2, 3, 4, 5]);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.logic.clone()),
        Logic::default()
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("07-clear-restores-history.json"),
    );
    click(&mut cx, tools, "history-tools-close");
    drop(view);
    shutdown(cx);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-two-group-expression.json","02-exact-grouped-history.json","03-inner-any-expands-results.json","04-root-all-empty.json","05-grouped-json-export.json","06-reopened-grouped-expression.json","07-clear-restores-history.json","08-invalid-history-formulas-refused.json","09-nested-history-formula.json"],"proof_boundary":"Actual Kit CWD starts/not-contains, inclusive port and hostname suffix fields, new-group checkbox, root/inner All/Any controls, invalid duplicate/out-of-range/SQL-like formula refusal, reordered nested formula with disabled groups, exact production history row identities, independently reopened saved expression, selected CWD JSON export, removal partition and clear. Owned backend/database and background GPU; native compositor, old native Tab paths and kernel enforcement remain separate."})).unwrap()).unwrap();
}
