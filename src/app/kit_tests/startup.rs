use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::startup::{ENTRY, Launch, Manager, Paths};
    fn state(cx: &mut impl AppContext, w: AnyWindowHandle) -> (bool, bool, bool) {
        cx.update_window(w, |_, window, cx| {
            gpui_kit::base::Root::read(window, cx)
                .view()
                .clone()
                .downcast::<crate::app::settings::Settings>()
                .unwrap()
                .read(cx)
                .test_startup_state(cx)
        })
        .unwrap()
    }
    fn finish(cx: &mut TestAppContext, w: AnyWindowHandle) {
        wait_for(cx, |cx| !state(cx, w).0);
    }
    let evidence = evidence();
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        Some(evidence.join("xdg"))
    );
    assert_eq!(
        std::env::var_os("XDG_CONFIG_DIRS").map(PathBuf::from),
        Some(evidence.join("system-xdg"))
    );
    let entry = evidence.join("xdg/autostart").join(ENTRY);
    assert!(!entry.exists());
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
        let mut client = protocol::ui_client::UiClient::new(channel);
        client
            .subscribe(protocol::ClientConfig {
                name: "startup-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        client
    });
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
    let (tray, actions, _availability) = crate::app::tray::Client::fixture();
    cx.update_entity(&view, |v, _| v.tray = Some(tray));
    flush(&mut cx);
    click(&mut cx, main, "startup-preferences");
    let prefs = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    finish(&mut cx, prefs);
    assert_eq!(state(&mut cx, prefs), (false, false, false));
    checkpoint(&mut cx, prefs, &evidence.join("01-startup-disabled.json"));
    click(&mut cx, prefs, "startup-enabled");
    click(&mut cx, prefs, "startup-close");
    wait_for(&mut cx, |_cx| !entry.exists());

    assert!(!entry.exists());
    actions.try_send(crate::app::tray::Action::Startup).unwrap();
    flush(&mut cx);
    let prefs = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    finish(&mut cx, prefs);
    assert!(!state(&mut cx, prefs).1);
    checkpoint(
        &mut cx,
        prefs,
        &evidence.join("02-tray-opens-unsaved-preference.json"),
    );
    click(&mut cx, prefs, "startup-enabled");
    click(&mut cx, prefs, "startup-save");
    finish(&mut cx, prefs);
    let manager = Manager {
        paths: Paths::environment().unwrap(),
        launch: Launch::environment(&config).unwrap(),
    };
    assert!(manager.load().unwrap().enabled);
    assert!(
        std::fs::read_to_string(&entry)
            .unwrap()
            .contains("--socket")
    );
    checkpoint(
        &mut cx,
        prefs,
        &evidence.join("03-enabled-entry-saved.json"),
    );
    click(&mut cx, prefs, "startup-close");
    flush(&mut cx);
    click(&mut cx, main, "startup-preferences");
    let prefs = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    finish(&mut cx, prefs);
    assert!(state(&mut cx, prefs).1);
    let reply = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/startup-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    let prompt = permission_window(&mut cx, &view);
    click(&mut cx, prefs, "startup-enabled");
    click(&mut cx, prefs, "startup-save");
    finish(&mut cx, prefs);
    assert!(!manager.load().unwrap().enabled);
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("04-permission-during-startup-save.json"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    // External replacement after load must retain the staged checkbox and file.
    let external = std::fs::read_to_string(&entry).unwrap() + "# external edit\n";
    std::fs::write(&entry, &external).unwrap();
    click(&mut cx, prefs, "startup-enabled");
    click(&mut cx, prefs, "startup-save");
    finish(&mut cx, prefs);
    assert_eq!(state(&mut cx, prefs), (false, true, true));
    assert_eq!(std::fs::read_to_string(&entry).unwrap(), external);
    checkpoint(
        &mut cx,
        prefs,
        &evidence.join("05-stale-entry-preserves-selection.json"),
    );
    click(&mut cx, prefs, "startup-reload");
    finish(&mut cx, prefs);
    assert_eq!(state(&mut cx, prefs), (false, false, false));
    checkpoint(
        &mut cx,
        prefs,
        &evidence.join("06-reloaded-disabled-entry.json"),
    );
    click(&mut cx, prefs, "startup-close");
    drop(view);
    shutdown(cx);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let frames = [
        "startup-disabled",
        "tray-opens-unsaved-preference",
        "enabled-entry-saved",
        "permission-during-startup-save",
        "stale-entry-preserves-selection",
        "reloaded-disabled-entry",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| format!("{:02}-{n}.json", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":frames,"proof_boundary":"Actual startup checkbox/save/close/reload, isolated XDG file enable/disable/reopen, staged Cancel, external stale refusal and retained selection, synthetic tray Startup action and real independent AskRule. No real user autostart changes, real login scheduler, actual tray menu D-Bus callback or compositor proof."})).unwrap()).unwrap();
}
