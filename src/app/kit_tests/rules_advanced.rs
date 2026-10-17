use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use gpui::InputEvent as _;
    fn reveal(cx: &mut TestAppContext, window: AnyWindowHandle, id: gpui::ElementId) {
        for _ in 0..32 {
            let direction = cx
                .update_window(window, |_, w, cx| {
                    w.render_frame(cx);
                    let body = w.find("rule-filter-body").bounds();
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
                let body = w.find("rule-filter-body").bounds();
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
        panic!("Rule control {id:?} must be reachable by wheel input");
    }
    fn control(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<gpui::ElementId>) {
        let id = id.into();
        if !matches!(&id, gpui::ElementId::Name(name) if matches!(name.as_ref(), "rule-filter-apply" | "rule-filter-clear" | "rule-filter-close"))
        {
            reveal(cx, w, id.clone());
        }
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
        reveal(cx, w, "rule-filter-field".into());
        pick(cx, w, "rule-filter-field", field);
        reveal(cx, w, "rule-filter-comparison".into());
        pick(cx, w, "rule-filter-comparison", comparison);
        reveal(cx, w, "rule-filter-value".into());
        fill(cx, w, "rule-filter-value", value);
        control(cx, w, "rule-filter-add");
    }

    fn rows(cx: &mut TestAppContext, view: &gpui::Entity<Dashboard>, wanted: &[usize]) {
        let wanted = wanted
            .iter()
            .map(|i| format!("advanced.{i}"))
            .collect::<std::collections::BTreeSet<_>>();
        wait_for(cx, |cx| {
            cx.read_entity(view, |v, _| {
                v.rules
                    .page
                    .rules
                    .iter()
                    .map(|r| r.name.clone())
                    .collect::<std::collections::BTreeSet<_>>()
            }) == wanted
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
    let mut fixtures = (0..6)
        .map(|i| protocol::Rule {
            name: format!("advanced.{i}"),
            created: 100 + i,
            description: if i % 2 == 0 { "team blue" } else { "team red" }.into(),
            enabled: i != 1,
            action: if i < 3 { "deny" } else { "allow" }.into(),
            duration: "always".into(),
            operator: Some(protocol::Operator {
                r#type: "simple".into(),
                operand: "process.path".into(),
                data: format!("/owned/advanced-{i}"),
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    fixtures[4].operator = Some(protocol::Operator {
        r#type: "list".into(),
        operand: "list".into(),
        list: vec![protocol::Operator {
            r#type: "list".into(),
            list: vec![
                protocol::Operator {
                    operand: "dest.host".into(),
                    data: "leaf.deep.test".into(),
                    ..Default::default()
                },
                protocol::Operator {
                    operand: "dest.port".into(),
                    data: "8443".into(),
                    ..Default::default()
                },
                protocol::Operator {
                    r#type: "simple".into(),
                    operand: "process.path".into(),
                    data: "/owned/advanced-4".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    });
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
                name: "rule-search-fixture".into(),
                rules: fixtures.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        client
    });
    let mut cx = context(test_cx);
    // Use the Ubuntu package runner's font for layout and control reachability.
    cx.update(|cx| Theme::global_mut(cx).font_family = "DejaVu Sans".into());
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
    flush(&mut cx);
    cx.update_window(main, |_, w, cx| w.within("main-tabs").click(1usize, cx))
        .unwrap();
    rows(&mut cx, &view, &[0, 1, 2, 3, 4, 5]);
    // Header input must change the background global order, not just visible cells.
    for (button, expected, frame) in [
        (
            "rules-sort-added",
            vec![105, 104, 103, 102, 101, 100],
            "15-added-newest-first.json",
        ),
        (
            "rules-sort-added",
            vec![100, 101, 102, 103, 104, 105],
            "16-added-oldest-first.json",
        ),
        (
            "rules-sort-name",
            vec![100, 101, 102, 103, 104, 105],
            "17-added-name-order.json",
        ),
    ] {
        click(&mut cx, main, button);
        wait_for(&mut cx, |cx| {
            cx.read_entity(&view, |v, _| {
                v.rules
                    .page
                    .rules
                    .iter()
                    .map(|rule| rule.created)
                    .collect::<Vec<_>>()
                    == expected
            })
        });
        assert_eq!(
            cx.read_entity(&view, |v, _| v
                .rules
                .page
                .rules
                .iter()
                .map(|rule| rule.created)
                .collect::<Vec<_>>()),
            expected
        );
        cx.update_window(main, |_, w, _| {
            let header = w.find("rules-sort-added").bounds();
            let cell = w.find(("rule-added", 0usize)).bounds();
            assert!(cell.size.width > px(100.));
            assert!(
                (cell.left() - header.left()).abs() < px(2.),
                "Added header {header:?}, cell {cell:?}"
            );
            assert!(
                header.right() < w.viewport_size().width - px(200.),
                "Action/status column must follow Added"
            );
            assert!(cell.right() <= w.viewport_size().width);
        })
        .unwrap();
        checkpoint(&mut cx, main, &evidence.join(frame));
    }
    click(&mut cx, main, "rule-filter-open");
    flush(&mut cx);
    let tools = cx.read_entity(&view, |v, _| v.rules.filter_window.unwrap());
    condition(&mut cx, tools, "Enabled", "Exact match", "2");
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("01-invalid-boolean-refused.json"),
    );
    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[0, 1, 2, 3, 4, 5]);
    assert!(
        cx.update_window(tools, |_, w, _| w
            .try_find(("rule-filter-remove", 0usize))
            .is_none())
            .unwrap()
    );
    control(&mut cx, tools, "rule-filter-root-mode");
    condition(&mut cx, tools, "Action", "Exact match", "deny");
    condition(&mut cx, tools, "Enabled", "Exact match", "1");
    control(&mut cx, tools, "rule-filter-new-group");
    condition(&mut cx, tools, "Description", "Ends with", "blue");
    condition(
        &mut cx,
        tools,
        "Created (UTC)",
        "At least / at or after",
        "104",
    );
    checkpoint(&mut cx, tools, &evidence.join("02-two-rule-groups.json"));
    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[0, 2, 4]);
    checkpoint(&mut cx, main, &evidence.join("03-exact-rule-matches.json"));
    control(&mut cx, tools, ("rule-filter-group-mode", 1usize));
    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[0, 2, 4, 5]);
    checkpoint(
        &mut cx,
        main,
        &evidence.join("04-inner-any-rule-matches.json"),
    );
    control(&mut cx, tools, ("rule-filter-group-mode", 1usize));
    control(&mut cx, tools, "rule-filter-root-mode");
    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[]);
    checkpoint(&mut cx, main, &evidence.join("05-root-all-empty.json"));
    control(&mut cx, tools, "rule-filter-root-mode");
    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[0, 2, 4]);
    for invalid in [
        "1 OR 1 OR 2 OR 3 OR 4",
        "1 AND 2 AND 3 AND 9",
        "1 OR 2 OR 3 OR 4; DROP TABLE rules",
    ] {
        reveal(&mut cx, tools, "rule-filter-formula".into());
        fill(&mut cx, tools, "rule-filter-formula", invalid);
        control(&mut cx, tools, "rule-filter-apply");
        rows(&mut cx, &view, &[0, 2, 4]);
    }
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("09-invalid-rule-formulas-refused.json"),
    );
    reveal(&mut cx, tools, "rule-filter-formula".into());
    fill(
        &mut cx,
        tools,
        "rule-filter-formula",
        "4 OR (1 AND (3 OR 2))",
    );
    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[0, 2, 4, 5]);
    reveal(&mut cx, tools, "rule-filter-root-mode".into());
    let group_label = cx
        .update_window(tools, |_, w, _| {
            w.find("rule-filter-root-mode").label().map(str::to_owned)
        })
        .unwrap();
    control(&mut cx, tools, "rule-filter-root-mode");
    assert_eq!(
        cx.update_window(tools, |_, w, _| w
            .find("rule-filter-root-mode")
            .label()
            .map(str::to_owned))
            .unwrap(),
        group_label
    );

    checkpoint(&mut cx, main, &evidence.join("10-nested-rule-formula.json"));
    click(&mut cx, tools, "rule-filter-close");
    flush(&mut cx);
    click(&mut cx, main, "rule-filter-open");
    flush(&mut cx);
    let tools = cx.read_entity(&view, |v, _| v.rules.filter_window.unwrap());
    reveal(&mut cx, tools, "rule-filter-formula".into());
    control(&mut cx, tools, "rule-filter-formula");
    cx.update_window(tools, |_, w, cx| {
        w.press("ctrl-a", cx);
        w.press("ctrl-c", cx);
    })
    .unwrap();
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        "4 OR (1 AND (3 OR 2))"
    );
    reveal(&mut cx, tools, "rule-filter-root-mode".into());
    let group_label = cx
        .update_window(tools, |_, w, _| {
            w.find("rule-filter-root-mode").label().map(str::to_owned)
        })
        .unwrap();
    control(&mut cx, tools, "rule-filter-root-mode");
    assert_eq!(
        cx.update_window(tools, |_, w, _| w
            .find("rule-filter-root-mode")
            .label()
            .map(str::to_owned))
            .unwrap(),
        group_label
    );

    checkpoint(
        &mut cx,
        tools,
        &evidence.join("06-reopened-rule-groups.json"),
    );
    condition(&mut cx, tools, "Rule name", "Exact match", "never-apply");
    reveal(&mut cx, tools, "rule-filter-root-mode".into());
    let group_label = cx
        .update_window(tools, |_, w, _| {
            w.find("rule-filter-root-mode").label().map(str::to_owned)
        })
        .unwrap();
    control(&mut cx, tools, "rule-filter-root-mode");
    assert_ne!(
        cx.update_window(tools, |_, w, _| w
            .find("rule-filter-root-mode")
            .label()
            .map(str::to_owned))
            .unwrap(),
        group_label
    );
    control(&mut cx, tools, "rule-filter-root-mode");

    click(&mut cx, tools, "rule-filter-close");
    flush(&mut cx);

    rows(&mut cx, &view, &[0, 2, 4, 5]);
    click(&mut cx, main, "rules-export");
    flush(&mut cx);
    let export_window = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    let output = evidence.join("matching-rules.json");
    fill(
        &mut cx,
        export_window,
        "transfer-path",
        output.to_str().unwrap(),
    );
    click(&mut cx, export_window, "transfer-export-file");
    wait_for(&mut cx, |_cx| output.exists());
    let mut got: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    got.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    assert_eq!(
        got,
        [0, 2, 4, 5]
            .into_iter()
            .map(|i| crate::rules::document::rule_json(&fixtures[i]))
            .collect::<Vec<_>>()
    );
    assert!(
        crate::rules::transfer::Import::from_path(&output).is_err(),
        "Read-only export retains nested wire; flat daemon mutation admission remains separately refused"
    );
    checkpoint(
        &mut cx,
        export_window,
        &evidence.join("07-filtered-rule-export.json"),
    );
    click(&mut cx, export_window, "transfer-close");
    flush(&mut cx);
    click(&mut cx, main, "rule-filter-open");
    flush(&mut cx);
    let tools = cx.read_entity(&view, |v, _| v.rules.filter_window.unwrap());
    control(&mut cx, tools, ("rule-filter-remove", 2usize));
    reveal(&mut cx, tools, "rule-filter-root-mode".into());
    let group_label = cx
        .update_window(tools, |_, w, _| {
            w.find("rule-filter-root-mode").label().map(str::to_owned)
        })
        .unwrap();
    control(&mut cx, tools, "rule-filter-root-mode");
    assert_ne!(
        cx.update_window(tools, |_, w, _| w
            .find("rule-filter-root-mode")
            .label()
            .map(str::to_owned))
            .unwrap(),
        group_label
    );
    control(&mut cx, tools, "rule-filter-root-mode");

    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[0, 2, 4, 5]);
    control(&mut cx, tools, "rule-filter-clear");
    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[0, 1, 2, 3, 4, 5]);
    assert!(
        cx.update_window(tools, |_, w, _| w
            .try_find(("rule-filter-remove", 0usize))
            .is_none())
            .unwrap()
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("08-cleared-rule-conditions.json"),
    );
    condition(
        &mut cx,
        tools,
        "Last received by UI (UTC)",
        "At least / at or after",
        "not-a-date",
    );
    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[0, 1, 2, 3, 4, 5]);
    assert!(
        cx.update_window(tools, |_, w, _| w
            .try_find(("rule-filter-remove", 0usize))
            .is_none())
            .unwrap()
    );
    condition(
        &mut cx,
        tools,
        "Last received by UI (UTC)",
        "At least / at or after",
        "0",
    );
    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[0, 1, 2, 3, 4, 5]);
    checkpoint(
        &mut cx,
        main,
        &evidence.join("11-received-catalogue-filter.json"),
    );
    condition(
        &mut cx,
        tools,
        "Any operator operand",
        "Exact match",
        "dest.host",
    );
    condition(&mut cx, tools, "Any operator data", "Contains", "leaf.deep");
    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[4]);
    checkpoint(
        &mut cx,
        main,
        &evidence.join("12-deep-operator-leaf-filter.json"),
    );
    click(&mut cx, main, "rules-export");
    flush(&mut cx);
    let export_window = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    let output = evidence.join("deep-tree-rules.json");
    fill(
        &mut cx,
        export_window,
        "transfer-path",
        output.to_str().unwrap(),
    );
    click(&mut cx, export_window, "transfer-export-file");
    wait_for(&mut cx, |_cx| output.exists());
    let tree_export: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(
        tree_export,
        vec![crate::rules::document::rule_json(&fixtures[4])]
    );
    checkpoint(
        &mut cx,
        export_window,
        &evidence.join("13-deep-operator-whole-wire-export.json"),
    );
    click(&mut cx, export_window, "transfer-close");
    flush(&mut cx);
    control(&mut cx, tools, "rule-filter-clear");
    condition(
        &mut cx,
        tools,
        "Any operator data",
        "Does not contain",
        "leaf.deep",
    );
    control(&mut cx, tools, "rule-filter-apply");
    rows(&mut cx, &view, &[0, 1, 2, 3, 5]);
    checkpoint(
        &mut cx,
        main,
        &evidence.join("14-negative-tree-excludes-whole-rule.json"),
    );
    click(&mut cx, tools, "rule-filter-close");
    drop(view);
    shutdown(cx);
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-invalid-boolean-refused.json","02-two-rule-groups.json","03-exact-rule-matches.json","04-inner-any-rule-matches.json","05-root-all-empty.json","06-reopened-rule-groups.json","07-filtered-rule-export.json","08-cleared-rule-conditions.json","09-invalid-rule-formulas-refused.json","10-nested-rule-formula.json","11-received-catalogue-filter.json","12-deep-operator-leaf-filter.json","13-deep-operator-whole-wire-export.json","14-negative-tree-excludes-whole-rule.json","15-added-newest-first.json","16-added-oldest-first.json","17-added-name-order.json"],"proof_boundary":"Actual Added UTC column/header input and newest/oldest/name sorting, native Kit rule filter controls, invalid bool refusal, two-level root/inner groups, invalid duplicate/out-of-range/SQL-like formula refusal, reordered nested formula with disabled groups, exact cached rule identities, staged Close preservation, reopen, matching whole-wire JSON export, remove partition and clear; actual received timestamp/date refusal, deep Any operand/data matching, exact whole nested protobuf export, negative sibling exclusion. Owned fixture/database and background GPU; no policy writes, real system daemon, kernel enforcement or native compositor proof."})).unwrap()).unwrap();
}
