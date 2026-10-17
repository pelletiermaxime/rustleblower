use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::window_state::{Mode, Page, Rect, State, Store};
    fn ready(cx: &mut TestAppContext, v: &gpui::Entity<Dashboard>) -> AnyWindowHandle {
        for _ in 0..50 {
            flush(cx);
            if let Some(w) =
                cx.read_entity(v, |v, _| v.window_ready.then_some(v.main_window).flatten())
            {
                return w;
            }
        }
        panic!("Dashboard restoration did not finish");
    }
    fn saved(cx: &mut TestAppContext, v: &gpui::Entity<Dashboard>) -> (u64, u64, u64) {
        for _ in 0..50 {
            flush(cx);
            let state = cx.read_entity(v, |v, _| v.memory.as_ref().unwrap().test_saved());
            if state.0 >= state.2 {
                return state;
            }
        }
        panic!("Layout write did not settle");
    }
    fn open(
        cx: &mut TestAppContext,
        backend: &Backend,
        config: &Config,
    ) -> gpui::Entity<Dashboard> {
        cx.update(|cx| {
            crate::app::appearance::init(&config.database, cx);
            crate::app::window_memory::open(backend.handle.clone(), config.clone(), cx).unwrap()
        })
    }
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let store = Store::new(&config.database);
    let seed = State {
        version: 1,
        bounds: Rect {
            x: 50000.,
            y: 50000.,
            width: 960.,
            height: 680.,
        },
        mode: Mode::Windowed,
        page: Page::Nodes,
    };
    store.save(&store.load().unwrap(), seed).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (mut daemon, notifications, stream) = runtime.block_on(async {
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
        let mut c = protocol::ui_client::UiClient::new(channel);
        c.subscribe(protocol::ClientConfig {
            name: "window-fixture".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let stream = c
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        backend.handle.history.flush().await.unwrap();
        (c, tx, stream)
    });
    let mut cx = context(test_cx);
    let view = open(&mut cx, &backend, &config);
    let main = ready(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.screen == crate::app::Screen::Nodes));
    let bounds = cx.update_window(main, |_, w, _| w.bounds()).unwrap();
    assert_eq!(bounds.size, size(px(960.), px(680.)));
    assert!(f32::from(bounds.origin.x) < 50000. && f32::from(bounds.origin.y) < 50000.);
    checkpoint(&mut cx, main, &evidence.join("01-restored-nodes-page.json"));
    cx.update_window(main, |_, w, cx| w.press("ctrl-2", cx))
        .unwrap();
    saved(&mut cx, &view);
    assert_eq!(store.load().unwrap().state.unwrap().page, Page::Rules);
    checkpoint(&mut cx, main, &evidence.join("02-rules-page-saved.json"));
    // Open the shared settings window from the minimum dashboard size.
    cx.update_window(main, |_, w, _| w.resize(size(px(780.), px(560.))))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, main, "settings-menu-trigger");
    flush(&mut cx);
    let preferences = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("12-compact-settings-menu.json"),
    );
    click(&mut cx, preferences, "settings-popup");
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("13-settings-opens-preferences.json"),
    );
    click(&mut cx, preferences, "preferences-close");
    flush(&mut cx);
    cx.update_window(main, |_, w, _| {
        assert!(
            w.try_find("settings-menu-trigger")
                .is_some_and(|target| target.visible())
        );
        w.resize(size(px(960.), px(680.)));
    })
    .unwrap();
    flush(&mut cx);
    let burst_before = saved(&mut cx, &view);
    for i in 0..25 {
        cx.update_window(main, |_, w, _| {
            w.resize(size(px(976. + i as f32), px(720.)))
        })
        .unwrap();
        sync_geometry(&mut cx);
        cx.advance_clock(Duration::from_millis(60));
        cx.run_until_parked();
    }
    let after = saved(&mut cx, &view);
    assert!(after.2 >= burst_before.2 + 5);
    assert!(
        after.1 - burst_before.1 <= 2,
        "Resize burst must be coalesced"
    );
    assert_eq!(store.load().unwrap().state.unwrap().bounds.width, 1000.);
    checkpoint(&mut cx, main, &evidence.join("03-resize-burst-saved.json"));
    let (tray, actions, _) = crate::app::tray::Client::fixture();
    cx.update_entity(&view, |v, _| v.tray = Some(tray));
    cx.update_window(main, |_, window, cx| {
        if view.update(cx, |v, _| v.should_close_dashboard(window)) {
            window.remove_window();
        }
    })
    .unwrap();
    saved(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.main_window.is_none()));
    let mut asking = daemon.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/hidden-layout".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    let prompt = permission_window(&mut cx, &view);
    checkpoint(&mut cx, prompt, &evidence.join("04-permission-hidden.json"));
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    actions.try_send(crate::app::tray::Action::Open).unwrap();
    let main = ready(&mut cx, &view);
    assert_eq!(
        cx.update_window(main, |_, w, _| w.bounds().size).unwrap(),
        size(px(1000.), px(720.))
    );
    assert!(cx.read_entity(&view, |v, _| v.screen == crate::app::Screen::Rules));
    checkpoint(&mut cx, main, &evidence.join("05-warm-reopen.json"));
    cx.update_window(main, |_, window, cx| {
        if view.update(cx, |v, _| v.should_close_dashboard(window)) {
            window.remove_window();
        }
    })
    .unwrap();
    saved(&mut cx, &view);
    drop(view);
    shutdown(cx);
    // Hold only load-result delivery; the production bounded UI wait must still
    // expose a default dashboard and answer a request without accepting late data.
    let mut cx = context(test_cx);
    let (deliver, loaded) = tokio::sync::oneshot::channel();
    let view = cx.update(|cx| {
        crate::app::appearance::init(&config.database, cx);
        crate::app::window_memory::open_delayed(backend.handle.clone(), config.clone(), loaded, cx)
            .unwrap()
    });
    let mut asking = daemon.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/loading-layout".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    for _ in 0..100 {
        if !backend.handle.prompts.snapshot().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    cx.run_until_parked();
    cx.advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    sync_geometry(&mut cx);
    let prompt = permission_window(&mut cx, &view);
    assert!(!cx.read_entity(&view, |v, _| v.window_ready));
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("06-permission-before-load-result.json"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    let main = ready(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.screen == crate::app::Screen::History));
    assert!(deliver.send(Ok(Some(seed))).is_err());
    assert_eq!(store.load().unwrap().state.unwrap().page, Page::Rules);
    checkpoint(
        &mut cx,
        main,
        &evidence.join("07-timeout-default-main.json"),
    );
    drop(view);
    shutdown(cx);
    let mut cx = context(test_cx);
    let view = open(&mut cx, &backend, &config);
    let main = ready(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.screen == crate::app::Screen::Rules));
    assert_eq!(
        cx.update_window(main, |_, w, _| w.bounds().size).unwrap(),
        size(px(1000.), px(720.))
    );
    checkpoint(&mut cx, main, &evidence.join("08-cold-restored-rules.json"));
    let bytes = std::fs::read(&store.path).unwrap();
    std::fs::write(&store.path, b"{external}").unwrap();
    cx.update_window(main, |_, w, cx| w.press("ctrl-4", cx))
        .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.memory.as_ref().unwrap().error().is_some())
    });
    assert!(cx.read_entity(&view, |v, _| v.memory.as_ref().unwrap().error().is_some()));
    let before = cx.read_entity(&view, |v, _| v.memory.as_ref().unwrap().test_saved().1);
    for key in ["ctrl-1", "ctrl-2", "ctrl-3"] {
        cx.update_window(main, |_, w, cx| w.press(key, cx)).unwrap();
        flush(&mut cx);
    }
    assert_eq!(
        cx.read_entity(&view, |v, _| v.memory.as_ref().unwrap().test_saved().1),
        before
    );
    assert_eq!(std::fs::read(&store.path).unwrap(), b"{external}");
    checkpoint(
        &mut cx,
        main,
        &evidence.join("09-stale-state-preserved.json"),
    );
    drop(view);
    shutdown(cx);
    std::fs::write(&store.path, bytes).unwrap();
    let mut cx = context(test_cx);
    let view = open(&mut cx, &backend, &config);
    let main = ready(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.screen == crate::app::Screen::Rules));
    checkpoint(
        &mut cx,
        main,
        &evidence.join("10-restarted-after-external-repair.json"),
    );
    let reply = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/quit-layout".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    cx.update_window(main, |_, w, cx| w.press("ctrl-3", cx))
        .unwrap();
    cx.run_until_parked();
    checkpoint(&mut cx, main, &evidence.join("11-quit-flush-state.json"));
    click(&mut cx, main, "quit");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    assert_eq!(store.load().unwrap().state.unwrap().page, Page::Statistics);
    drop(view);
    shutdown(cx);
    drop(notifications);
    drop(stream);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let frames = [
        "restored-nodes-page",
        "rules-page-saved",
        "resize-burst-saved",
        "permission-hidden",
        "warm-reopen",
        "permission-before-load-result",
        "timeout-default-main",
        "cold-restored-rules",
        "stale-state-preserved",
        "restarted-after-external-repair",
        "quit-flush-state",
        "compact-settings-menu",
        "settings-opens-preferences",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| format!("{:02}-{n}.json", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":frames,"resize_updates":after.2-burst_before.2,"resize_writes":after.1-burst_before.1,"proof_boundary":"Actual navigation/Quit and permission input with production bootstrap/worker; synthetic tray close/Open boundary and native GPUI resize requests plus documented compositor-callback shim. Held load-result delivery is not a deliberately stalled filesystem. Exact disk state and cold/warm GUI reopens, bounded coalescing/stale refusal and pending-request shutdown are proved. Actual compositor resize/maximize/fullscreen/position behavior and native focus need separate proof."})).unwrap()).unwrap();
}
