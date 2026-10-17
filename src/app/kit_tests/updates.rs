use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::updates::{Outcome, Target, Unit, Worker};
    use gpui::InputEvent as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn ready(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) {
        for _ in 0..60 {
            flush(cx);
            if cx
                .update_window(w, |_, w, _| {
                    w.try_find(id)
                        .is_some_and(|e| e.visible() && e.disabled() != Some(true))
                })
                .unwrap()
            {
                return;
            }
        }
        panic!("Updates control {id} did not become ready");
    }
    fn choose(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str, key: &str) {
        click(cx, w, id);
        flush(cx);
        cx.update_window(w, |_, w, cx| {
            w.press(key, cx);
            w.press("enter", cx)
        })
        .unwrap();
        flush(cx);
    }
    fn state(
        cx: &mut TestAppContext,
        h: &crate::updates::Handle,
        test: impl Fn(&crate::updates::State) -> bool,
    ) {
        wait_for(cx, |_cx| test(&h.state()));
    }
    fn reveal(cx: &mut TestAppContext, window: AnyWindowHandle, id: gpui::ElementId) {
        for _ in 0..32 {
            let direction = cx
                .update_window(window, |_, w, cx| {
                    w.render_frame(cx);
                    let body = w.find("updates-scroll").bounds();
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
                let body = w.find("updates-scroll").bounds();
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
        panic!("Updates control {id:?} must be reachable by wheel input");
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
    type Response = (u16, String, bool);
    let responses = Arc::new(Mutex::new(std::collections::VecDeque::<Response>::new()));
    let hits = Arc::new(AtomicUsize::new(0));
    let hold = Arc::new(AtomicBool::new(false));
    let paths = Arc::new(Mutex::new(Vec::<String>::new()));
    let (base,http)=runtime.block_on(async{
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let base=format!("http://{}/",listener.local_addr().unwrap());
        let (queue,hits,hold,paths)=(responses.clone(),hits.clone(),hold.clone(),paths.clone());
        let task=tokio::spawn(async move{while let Ok((mut socket,_))=listener.accept().await{
            let mut bytes=[0u8;8192];let count=socket.read(&mut bytes).await.unwrap_or(0);
            {let mut paths=paths.lock().unwrap();assert!(paths.len()<16);paths.push(String::from_utf8_lossy(&bytes[..count]).lines().next().unwrap_or("").to_string());}
            hits.fetch_add(1,Ordering::SeqCst);let (status,body,held)=queue.lock().unwrap().pop_front().expect("Unexpected fixture HTTP request");
            while held&&hold.load(Ordering::Acquire){tokio::time::sleep(Duration::from_millis(5)).await;}
            let wire=format!("HTTP/1.1 {status} owned\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());let _=socket.write_all(wire.as_bytes()).await;
        }});(base,task)
    });
    let notices = Arc::new(Mutex::new(Vec::<crate::updates::Notice>::new()));
    let out = notices.clone();
    let worker = Worker::start_fixture(
        crate::updates::path_for(&config.database),
        Some(Arc::new(move |notice| {
            let mut notices = out.lock().unwrap();
            assert!(notices.len() < 8);
            notices.push(notice);
        })),
        &base,
    )
    .unwrap();
    let updates = worker.handle();
    let mut handle = backend.handle.clone();
    handle.updates = updates.clone();
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
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "updates-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    let mut cx = context(test_cx);
    let (main, view) = cx.update(|cx| {
        gpui_kit::open_window(crate::app::dashboard_options(), cx, |w, cx| {
            cx.new(|cx| Dashboard::new(handle.clone(), config.clone(), w, cx))
        })
        .unwrap()
    });
    let (tray, _tray_actions, _availability) = crate::app::tray::Client::fixture();
    let (notifier, actions) = crate::app::desktop_notifications::Client::fixture();
    cx.update_entity(&view, |v, _| {
        v.tray = Some(tray);
        v.desktop_notifications = Some(notifier)
    });
    flush(&mut cx);
    click(&mut cx, main, "updates-preferences");
    flush(&mut cx);
    let w = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    ready(&mut cx, w, "updates-save");
    assert!(!updates.state().settings.enabled);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("01-update-defaults-no-network.json"),
    );
    reveal(&mut cx, w, "updates-interval".into());
    fill(&mut cx, w, "updates-interval", "0");
    click(&mut cx, w, "updates-save");
    wait_for(&mut cx, |_cx| {
        !crate::updates::path_for(&config.database).exists()
    });

    assert!(!crate::updates::path_for(&config.database).exists());
    checkpoint(
        &mut cx,
        w,
        &evidence.join("02-invalid-update-interval.json"),
    );
    fill(&mut cx, w, "updates-interval", "2");
    choose(&mut cx, w, "updates-unit", "up");
    choose(&mut cx, w, "updates-target", "down");
    click(&mut cx, w, "updates-on-start");
    click(&mut cx, w, "updates-prereleases");
    click(&mut cx, w, "updates-enabled");
    click(&mut cx, w, "updates-save");
    ready(&mut cx, w, "updates-save");
    assert!(updates.state().settings.enabled);
    assert!(!updates.state().settings.check_on_start);
    assert_eq!(updates.state().settings.unit, Unit::Minutes);
    assert_eq!(updates.state().settings.interval, 2);
    assert_eq!(updates.state().settings.target, Target::SelectedDaemon);
    assert!(updates.state().settings.include_prereleases);
    click(&mut cx, w, "updates-close");
    flush(&mut cx);
    click(&mut cx, main, "updates-preferences");
    flush(&mut cx);
    let w = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    ready(&mut cx, w, "updates-save");
    assert_eq!(
        cx.update_window(w, |_, w, _| w
            .find("updates-interval")
            .value()
            .unwrap()
            .to_string())
            .unwrap(),
        "2"
    );
    checkpoint(
        &mut cx,
        w,
        &evidence.join("03-update-preferences-reopened.json"),
    );
    click(&mut cx, w, "updates-check");
    state(&mut cx, &updates, |s| {
        s.outcome == Outcome::UnknownCurrentVersion
    });
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("04-unreported-daemon-no-network.json"),
    );
    reveal(&mut cx, w, "updates-target".into());
    choose(&mut cx, w, "updates-target", "up");
    click(&mut cx, w, "updates-prereleases");
    click(&mut cx, w, "updates-enabled");
    click(&mut cx, w, "updates-save");
    ready(&mut cx, w, "updates-save");
    responses
        .lock()
        .unwrap()
        .push_back((200, r#"{"tag_name":"v0.9.0"}"#.into(), false));
    click(&mut cx, w, "updates-check");
    state(&mut cx, &updates, |s| s.outcome == Outcome::Available);
    assert_eq!(notices.lock().unwrap().len(), 1);
    assert_eq!(updates.state().latest.as_deref(), Some("0.9.0"));
    assert!(
        !updates.state().settings.enabled,
        "Manual check works while automatic checks are off"
    );
    click(&mut cx, w, "updates-close");
    flush(&mut cx);
    click(&mut cx, main, "updates-preferences");
    flush(&mut cx);
    let w = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    ready(&mut cx, w, "updates-check");
    assert_eq!(updates.state().outcome, Outcome::Available);
    assert_eq!(updates.state().latest.as_deref(), Some("0.9.0"));
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "Reopen preserves delivered result without network"
    );
    checkpoint(&mut cx, w, &evidence.join("05-newer-official-release.json"));
    responses.lock().unwrap().push_back((
        200,
        format!(r#"{{"tag_name":"v{}"}}"#, env!("CARGO_PKG_VERSION")),
        false,
    ));
    ready(&mut cx, w, "updates-check");
    click(&mut cx, w, "updates-check");
    state(&mut cx, &updates, |s| s.outcome == Outcome::Current);
    checkpoint(&mut cx, w, &evidence.join("06-current-release.json"));
    responses
        .lock()
        .unwrap()
        .push_back((429, "{}".into(), false));
    ready(&mut cx, w, "updates-check");
    click(&mut cx, w, "updates-check");
    state(&mut cx, &updates, |s| s.outcome == Outcome::RateLimited);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("07-release-check-rate-limited.json"),
    );
    hold.store(true, Ordering::Release);
    responses
        .lock()
        .unwrap()
        .push_back((200, r#"{"tag_name":"v0.9.1"}"#.into(), true));
    ready(&mut cx, w, "updates-check");
    click(&mut cx, w, "updates-check");
    state(&mut cx, &updates, |s| s.busy);
    let mut asking = daemon.clone();
    let ask = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/updates-held-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    let popup = permission_window(&mut cx, &view);
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(ask).unwrap().action, "deny");
    assert!(updates.state().busy);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("08-permission-during-held-release.json"),
    );
    click(&mut cx, w, "updates-close");
    flush(&mut cx);
    cx.update_window(main, |_, w, cx| {
        assert!(view.update(cx, |v, _| v.should_close_dashboard(w)));
        w.remove_window()
    })
    .unwrap();
    flush(&mut cx);
    actions
        .try_send(crate::desktop_notifications::Target::Updates)
        .unwrap();
    flush(&mut cx);
    let w = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    assert!(
        updates.state().busy,
        "Opening a notice preserves in-flight release check"
    );
    actions
        .try_send(crate::desktop_notifications::Target::Updates)
        .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.settings_window.unwrap())) == (w)
    });

    assert_eq!(cx.read_entity(&view, |v, _| v.settings_window.unwrap()), w);
    ready(&mut cx, w, "updates-cancel");
    click(&mut cx, w, "updates-cancel");
    state(&mut cx, &updates, |s| s.outcome == Outcome::Cancelled);
    hold.store(false, Ordering::Release);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("09-cancel-and-notice-window-reuse.json"),
    );
    ready(&mut cx, w, "updates-save");
    let path = crate::updates::path_for(&config.database);
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    doc["settings"]["interval"] = 3.into();
    let external = serde_json::to_vec_pretty(&doc).unwrap();
    std::fs::write(&path, &external).unwrap();
    reveal(&mut cx, w, "updates-interval".into());
    fill(&mut cx, w, "updates-interval", "4");
    click(&mut cx, w, "updates-save");
    ready(&mut cx, w, "updates-save");
    assert_eq!(std::fs::read(&path).unwrap(), external);
    assert_eq!(updates.state().settings.interval, 2);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("10-stale-update-save-refused.json"),
    );
    click(&mut cx, w, "updates-reload");
    ready(&mut cx, w, "updates-save");
    assert_eq!(updates.state().settings.interval, 3);
    reveal(&mut cx, w, "updates-enabled".into());
    click(&mut cx, w, "updates-save");
    ready(&mut cx, w, "updates-save");
    assert!(!updates.state().settings.enabled);
    assert_eq!(hits.load(Ordering::SeqCst), 4);
    let saved = std::fs::read(&path).unwrap();
    std::fs::write(evidence.join("saved-update-preferences.json"), &saved).unwrap();
    assert!(
        paths
            .lock()
            .unwrap()
            .iter()
            .all(|p| p.starts_with("GET /releases/latest "))
    );
    checkpoint(&mut cx, w, &evidence.join("11-disabled-saved-updates.json"));
    click(&mut cx, w, "updates-close");
    drop(view);
    shutdown(cx);
    drop(daemon);
    worker.join();
    http.abort();
    drop(runtime);
    drop(handle);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-update-defaults-no-network.json","02-invalid-update-interval.json","03-update-preferences-reopened.json","04-unreported-daemon-no-network.json","05-newer-official-release.json","06-current-release.json","07-release-check-rate-limited.json","08-permission-during-held-release.json","09-cancel-and-notice-window-reuse.json","10-stale-update-save-refused.json","11-disabled-saved-updates.json"],"proof_boundary":"Actual Kit release preferences controls/save/reopen/invalid/stale/reload, owned network HTTP available/current/rate-limit/cancel, no traffic before opt-in/manual check or with unknown daemon version, real permission reply while HTTP held, synthetic Updates callback preserves in-flight job and reuses window, exact persisted preferences. Fixed loopback-only test endpoint; no official network, package download/install, release-page click or host banner/native compositor proof."})).unwrap()).unwrap();
}
