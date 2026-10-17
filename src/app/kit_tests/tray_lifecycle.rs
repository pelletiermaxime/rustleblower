use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let mut cx = context(test_cx);
    cx.update(|cx| crate::app::appearance::init(&config.database, cx));
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(crate::app::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    let (tray, actions, available) = crate::app::tray::Client::fixture();
    cx.update_entity(&view, |view, _| view.tray = Some(tray));
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("ctrl-4", cx))
        .unwrap();
    flush(&mut cx);
    checkpoint(&mut cx, window, &evidence.join("01-before-close.json"));
    cx.update_window(window, |_, window, cx| {
        let close = view.update(cx, |view, _| view.should_close_dashboard(window));
        assert!(close);
        window.remove_window();
    })
    .unwrap();
    flush(&mut cx);
    assert!(cx.update_window(window, |_, _, _| ()).is_err());
    assert!(!backend.handle.cancel.is_cancelled());

    let (_, mut reply) = backend
        .handle
        .prompts
        .insert_with_defaults(
            1,
            protocol::Connection {
                process_path: "/usr/bin/tray-fixture".into(),
                dst_host: "tray.example".into(),
                dst_ip: "192.0.2.20".into(),
                ..Default::default()
            },
            Instant::now() + Duration::from_secs(30),
            crate::prompts::Initial::default(),
        )
        .unwrap();
    flush(&mut cx);
    let popup = permission_window(&mut cx, &view);
    checkpoint(
        &mut cx,
        popup,
        &evidence.join("02-permission-while-hidden.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(reply.try_recv().unwrap().decision, Decision::Deny);
    flush(&mut cx);

    actions.try_send(crate::app::tray::Action::Open).unwrap();
    flush(&mut cx);
    let reopened = cx.read_entity(&view, |view, _| {
        assert!(matches!(view.screen, crate::app::Screen::Nodes));
        view.main_window.unwrap()
    });
    checkpoint(&mut cx, reopened, &evidence.join("03-restored.json"));
    // Repeated activation uses the same window.
    actions.try_send(crate::app::tray::Action::Open).unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.main_window)) == (Some(reopened))
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.main_window),
        Some(reopened)
    );
    cx.update_window(reopened, |_, window, cx| {
        assert!(view.update(cx, |view, _| view.should_close_dashboard(window)));
        window.remove_window();
    })
    .unwrap();
    available.store(2, Ordering::Release);
    flush(&mut cx);
    let restored = cx.read_entity(&view, |view, _| view.main_window.unwrap());
    checkpoint(
        &mut cx,
        restored,
        &evidence.join("04-tray-unavailable.json"),
    );
    // The existing visible Quit control still shuts down the backend.
    click(&mut cx, restored, "quit");
    assert!(backend.handle.cancel.is_cancelled());
    drop(actions);
    drop(view);
    shutdown(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "status": "passed", "framework": "gpui-kit", "real_gpu": false, "scoped_socket_removed": true,
        "checkpoints": ["01-before-close.json", "02-permission-while-hidden.json", "03-restored.json", "04-tray-unavailable.json"],
        "proof_boundary": "Production close-handler boundary and actual GPUI permission/Quit input; retained dashboard state, tray-action restoration and host-loss recovery. Tray callbacks are synthetic; native compositor and D-Bus protocol require separate proof."
    })).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"), serde_json::to_vec_pretty(&[
        "Select Nodes through Ctrl+4, apply production close boundary, require destroyed dashboard and running backend",
        "Deliver permission while dashboard is absent; Deny once through actual button",
        "Reopen twice through bounded tray channel, require retained Nodes screen and one window",
        "Lose tray host while hidden, require automatic restoration; Quit through actual button"
    ]).unwrap()).unwrap();
}
