use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let mut cx = context(test_cx);
    let window = cx
        .update(|cx| crate::app::preferences_view::open(backend.handle.clone(), cx))
        .unwrap();
    flush(&mut cx);
    pick(&mut cx, window, "preferences-duration", "12 hours");
    click(&mut cx, window, "preferences-target");
    flush(&mut cx);
    checkpoint(&mut cx, window, &evidence.join("01-target-open.json"));
    cx.update_window(window, |_, window, cx| window.input("This executable", cx))
        .unwrap();
    flush(&mut cx);
    checkpoint(&mut cx, window, &evidence.join("02-target-query.json"));
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, window, "preferences-priority");
    click(&mut cx, window, "preferences-logging");
    click(&mut cx, window, "preferences-save");
    flush(&mut cx);
    checkpoint(&mut cx, window, &evidence.join("03-saved.json"));
    let value = backend.handle.preferences.snapshot();
    assert_eq!(value.target, "process.path");
    assert_eq!(value.duration, "12h");
    assert!(value.priority && !value.logging);
    assert_eq!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot(),
        value
    );
    advanced_defaults(&mut cx, window);
    fill(&mut cx, window, "preferences-timeout", "0");
    click(&mut cx, window, "preferences-save");
    wait_for(&mut cx, |_cx| {
        (backend.handle.preferences.snapshot()) == (value)
    });

    assert_eq!(backend.handle.preferences.snapshot(), value);
    checkpoint(&mut cx, window, &evidence.join("04-invalid-timeout.json"));
    fill(&mut cx, window, "preferences-timeout", "5");
    pick(&mut cx, window, "preferences-fallback", "Reject once");
    click(&mut cx, window, "preferences-save");
    flush(&mut cx);
    let advanced = backend.handle.preferences.snapshot();
    assert!(advanced.show_advanced && advanced.constraints.len() == 4);
    assert_eq!(advanced.timeout, Some(5));
    assert_eq!(advanced.fallback, Some(Decision::Reject));
    assert_eq!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot(),
        advanced
    );
    checkpoint(&mut cx, window, &evidence.join("05-advanced-defaults.json"));
    cx.update_window(window, |_, window, cx| {
        window.within("preferences-tabs").click(2usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    pick(&mut cx, window, "preferences-position", "Bottom right");
    click(&mut cx, window, "preferences-save");
    wait_for(&mut cx, |_cx| {
        (backend.handle.preferences.snapshot().position)
            == (crate::prompts::placement::Position::BottomRight)
    });

    assert_eq!(
        backend.handle.preferences.snapshot().position,
        crate::prompts::placement::Position::BottomRight
    );
    assert_eq!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .position,
        crate::prompts::placement::Position::BottomRight
    );
    checkpoint(&mut cx, window, &evidence.join("06-popup-position.json"));
    for (index, (key, _)) in crate::prompts::placement::POSITIONS.iter().enumerate() {
        let position = crate::prompts::placement::Position::parse(key).unwrap();
        let (_, mut reply) = backend
            .handle
            .prompts
            .insert_with_defaults(
                1,
                protocol::Connection {
                    process_path: "/usr/bin/placement-test".into(),
                    dst_ip: "192.0.2.1".into(),
                    ..Default::default()
                },
                Instant::now() + Duration::from_secs(30),
                crate::prompts::Initial {
                    position,
                    ..Default::default()
                },
            )
            .unwrap();
        let popup = cx
            .update(|cx| crate::app::prompt_window::open(backend.handle.clone(), 1., true, cx))
            .unwrap();
        cx.run_until_parked();
        let actual = cx
            .update_window(popup, |_, window, _| window.bounds())
            .unwrap();
        let screen = cx.update(|cx| cx.primary_display().unwrap().visible_bounds());
        let expected_x = match *key {
            "top-left" | "bottom-left" => screen.origin.x + px(16.),
            "top-right" | "bottom-right" => {
                screen.origin.x + screen.size.width - actual.size.width - px(16.)
            }
            _ => screen.origin.x + (screen.size.width - actual.size.width) / 2.,
        };
        let expected_y = match *key {
            "top-left" | "top-right" => screen.origin.y + px(16.),
            "bottom-left" | "bottom-right" => {
                screen.origin.y + screen.size.height - actual.size.height - px(16.)
            }
            _ => screen.origin.y + (screen.size.height - actual.size.height) / 2.,
        };
        assert_eq!(actual.origin, gpui::point(expected_x, expected_y));
        checkpoint(
            &mut cx,
            popup,
            &evidence.join(format!("{:02}-position-{key}.json", index + 7)),
        );
        if *key == "center" {
            keyboard_button(&mut cx, popup, "deny-once");
        } else {
            click(&mut cx, popup, "deny-once");
        }
        assert_eq!(reply.try_recv().unwrap().decision, Decision::Deny);
        cx.update_window(popup, |_, window, _| window.remove_window())
            .unwrap();
    }
    let mut dashboard_view = None;
    let dashboard = cx
        .open_test_window(size(px(1100.), px(780.)), |window, cx| {
            let view =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            dashboard_view = Some(view.clone());
            cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
        })
        .unwrap();
    let dashboard_view = dashboard_view.unwrap();
    let mut replies = Vec::new();
    for position in [
        crate::prompts::placement::Position::TopRight,
        crate::prompts::placement::Position::Center,
    ] {
        let (_, reply) = backend
            .handle
            .prompts
            .insert_with_defaults(
                1,
                protocol::Connection {
                    process_path: "/usr/bin/queued-placement".into(),
                    ..Default::default()
                },
                Instant::now() + Duration::from_secs(30),
                crate::prompts::Initial {
                    position,
                    ..Default::default()
                },
            )
            .unwrap();
        replies.push(reply);
    }
    flush(&mut cx);
    let first = permission_window(&mut cx, &dashboard_view);
    checkpoint(&mut cx, first, &evidence.join("12-queued-corner.json"));
    click(&mut cx, first, "deny-once");
    assert_eq!(replies[0].try_recv().unwrap().decision, Decision::Deny);
    flush(&mut cx);
    assert!(
        cx.update_window(first, |_, _, _| ()).is_err(),
        "queued position change must replace the previous popup"
    );
    let second = permission_window(&mut cx, &dashboard_view);
    let actual = cx
        .update_window(second, |_, window, _| window.bounds())
        .unwrap();
    let screen = cx.update(|cx| cx.primary_display().unwrap().visible_bounds());
    assert_eq!(
        actual.origin,
        gpui::point(
            screen.origin.x + (screen.size.width - actual.size.width) / 2.,
            screen.origin.y + (screen.size.height - actual.size.height) / 2.
        )
    );
    checkpoint(&mut cx, second, &evidence.join("13-queued-center.json"));
    click(&mut cx, second, "deny-once");
    assert_eq!(replies[1].try_recv().unwrap().decision, Decision::Deny);
    flush(&mut cx);
    cx.update_window(dashboard.into(), |_, window, _| window.remove_window())
        .unwrap();
    drop(dashboard_view);
    click(&mut cx, window, "preferences-close");
    shutdown(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","checkpoints":["01-target-open.json","02-target-query.json","03-saved.json","04-invalid-timeout.json","05-advanced-defaults.json","06-popup-position.json","07-position-center.json","08-position-top-right.json","09-position-bottom-right.json","10-position-top-left.json","11-position-bottom-left.json","12-queued-corner.json","13-queued-center.json"],"framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"proof_boundary":"Real GPUI preference controls, timeout validation, file persistence and initial virtual-window geometry; native compositor placement and daemon enforcement require separate proof"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Select duration and match target, toggle priority/logging, save and reopen preferences from disk", "Open Conditions & timeout; select four default conditions and expanded options; reject zero seconds without publishing staged settings; save five seconds and Reject once; reopen the file with exact values", "Save Bottom Right placement and reopen preferences; open all five production popup positions; compare virtual geometry and answer by pointer or Tab/Space", "Queue Top Right then Center requests; answer the first; require its window to close and the next centered popup to return its own reply"]).unwrap()).unwrap();
}
