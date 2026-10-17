//! Settings shell regression: real menu/control input and independent persistence/RPC assertions.
use super::*;
use crate::app::settings;

fn footer(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    close: &'static str,
    save: impl Into<Option<&'static str>>,
) {
    let save = save.into();
    cx.update_window(window, |_, window, _| {
        let close = window.find(close);
        assert_eq!(close.label(), Some("Close"));
        let last = if let Some(save) = save {
            let save = window.find(save);
            assert_eq!(save.label(), Some("Save"));
            assert_eq!(close.bounds().size, save.bounds().size);
            assert_eq!(close.bounds().top(), save.bounds().top());
            assert!(close.bounds().right() < save.bounds().left());
            save.bounds()
        } else {
            assert!(window.try_find("highlighting-save").is_none());
            close.bounds()
        };
        let edge = window.viewport_size().width - window.rem_size() * 1.25;
        assert!((last.right() - edge).abs() < px(1.));
        let bottom = window.viewport_size().height - window.rem_size() * 0.75;
        assert!((last.bottom() - bottom).abs() < px(1.));
    })
    .unwrap();
}

#[test]
fn headless_settings() {
    use gpui::prelude::*;
    let evidence = evidence();
    let owned = tempfile::tempdir().unwrap();
    let config = options(owned.path());
    let backend = Backend::start(config.clone()).unwrap();
    let mut cx = context();
    let mut dashboard = None;
    let main = cx
        .open_window(size(px(1240.), px(940.)), |window, cx| {
            let view =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            dashboard = Some(view.clone());
            cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
        })
        .unwrap()
        .into();
    settle(&mut cx);
    click(&mut cx, main, "settings-menu-trigger");
    settle(&mut cx);
    let settings = cx.update(|cx| settings::test_window(cx));
    let appearance = crate::appearance::Store::new(&config.database);
    let before = appearance.load().unwrap();
    let active = cx.update(|cx| super::super::appearance::test_state(cx).1);
    click(&mut cx, settings, "appearance-dark");
    settle(&mut cx);
    assert_eq!(appearance.load().unwrap().mode, before.mode);
    assert_eq!(
        cx.update(|cx| super::super::appearance::test_state(cx).1),
        active
    );
    footer(&mut cx, settings, "appearance-close", "appearance-save");
    click(&mut cx, settings, "appearance-save");
    settle(&mut cx);
    assert_eq!(
        appearance.load().unwrap().mode,
        crate::appearance::ColorMode::Dark
    );
    capture(&mut cx, settings, &evidence.join("01-appearance.png"));
    cx.update_window(settings, |_, window, cx| window.press("ctrl-tab", cx))
        .unwrap();
    settle(&mut cx);
    assert!(
        cx.update_window(settings, |_, window, _| window
            .try_find("startup-enabled")
            .is_some())
            .unwrap()
    );
    cx.update_window(settings, |_, window, cx| window.press("ctrl-shift-tab", cx))
        .unwrap();
    settle(&mut cx);
    let baseline_windows = cx.update(|cx| cx.windows().len());
    click(&mut cx, settings, "settings-popup");
    footer(&mut cx, settings, "preferences-close", "preferences-save");
    // Stage a change, leave the category, and return before saving it.
    let original = backend.handle.preferences.snapshot().priority;
    click(&mut cx, settings, "preferences-priority");
    click(&mut cx, settings, "settings-updates");
    footer(&mut cx, settings, "updates-close", "updates-save");
    capture(&mut cx, settings, &evidence.join("02-updates.png"));
    click(&mut cx, settings, "settings-popup");
    assert_eq!(backend.handle.preferences.snapshot().priority, original);
    click(&mut cx, settings, "preferences-save");
    for _ in 0..40 {
        settle(&mut cx);
        if backend.handle.preferences.snapshot().priority != original {
            break;
        }
    }
    assert_eq!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .priority,
        !original
    );
    capture(&mut cx, settings, &evidence.join("03-popup-saved.png"));
    let mut frames = vec![
        "01-appearance.png".to_owned(),
        "02-updates.png".into(),
        "03-popup-saved.png".into(),
    ];
    for (id, name) in [
        ("settings-startup", "04-startup.png"),
        ("settings-history", "05-history.png"),
        ("settings-catalogue", "06-catalogue.png"),
        ("settings-reputation", "07-reputation.png"),
        ("settings-downloads", "08-downloads.png"),
        ("settings-highlighting", "09-highlighting.png"),
    ] {
        click(&mut cx, settings, id);
        settle(&mut cx);
        let (close, save) = match id {
            "settings-startup" => ("startup-close", Some("startup-save")),
            "settings-history" => (
                "history-preferences-close",
                Some("history-retention-review"),
            ),
            "settings-catalogue" => ("catalogue-close", Some("catalogue-review")),
            "settings-reputation" => ("rep-close", Some("rep-save")),
            "settings-downloads" => ("raw-close", Some("raw-save")),
            "settings-highlighting" => ("settings-close", None),
            _ => unreachable!(),
        };
        footer(&mut cx, settings, close, save);
        capture(&mut cx, settings, &evidence.join(name));
        frames.push(name.into());
        assert_eq!(
            cx.update(|cx| cx.windows().len()),
            baseline_windows,
            "Category input must reuse the same window"
        );
    }
    // Reopening from the dashboard also reuses the window and retains drafts.
    click(&mut cx, main, "settings-menu-trigger");
    assert_eq!(cx.update(|cx| settings::test_window(cx)), settings);
    click(&mut cx, settings, "appearance-light");
    settle(&mut cx);
    assert_eq!(
        appearance.load().unwrap().mode,
        crate::appearance::ColorMode::Dark
    );
    capture(
        &mut cx,
        settings,
        &evidence.join("14-appearance-staged.png"),
    );
    frames.push("14-appearance-staged.png".into());
    click(&mut cx, main, "history-preferences");
    assert_eq!(cx.update(|cx| settings::test_window(cx)), settings);
    fill(&mut cx, settings, "history-days", "19");
    click(&mut cx, settings, "settings-popup");
    click(&mut cx, settings, "settings-history");
    click(&mut cx, settings, "history-retention-review");
    settle(&mut cx);
    footer(
        &mut cx,
        settings,
        "history-preferences-close",
        "history-retention-apply",
    );
    capture(
        &mut cx,
        settings,
        &evidence.join("10-retained-history-review.png"),
    );
    frames.push("10-retained-history-review.png".into());
    assert_eq!(
        backend.handle.history.storage.snapshot().config.days,
        config.history_days
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
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
                name: "settings-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    let mut asking = daemon.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/settings-fixture".into(),
                dst_host: "settings.example".into(),
                dst_ip: "192.0.2.10".into(),
                dst_port: 443,
                protocol: "tcp".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    for _ in 0..40 {
        settle(&mut cx);
        if !backend.handle.prompts.snapshot().is_empty() {
            break;
        }
    }
    let prompt = cx.read_entity(dashboard.as_ref().unwrap(), |view, _| {
        view.prompt_window.unwrap()
    });
    settle(&mut cx);
    capture(
        &mut cx,
        prompt,
        &evidence.join("11-independent-permission.png"),
    );
    frames.push("11-independent-permission.png".into());
    click(&mut cx, prompt, "deny-once");
    let rule = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), reply)
            .await
            .unwrap()
            .unwrap()
    });
    assert_eq!(
        (rule.action.as_str(), rule.duration.as_str()),
        ("deny", "once")
    );
    click(&mut cx, settings, "history-retention-back");
    click(&mut cx, settings, "history-preferences-close");
    settle(&mut cx);
    click(&mut cx, main, "settings-menu-trigger");
    let reopened = cx.update(|cx| settings::test_window(cx));
    settle(&mut cx);
    assert_eq!(
        appearance.load().unwrap().mode,
        crate::appearance::ColorMode::Dark
    );
    assert_eq!(
        cx.update(|cx| super::super::appearance::test_state(cx).1),
        crate::appearance::ColorMode::Dark
    );
    assert_ne!(settings, reopened);
    click(&mut cx, reopened, "settings-popup");
    capture(&mut cx, reopened, &evidence.join("12-reopened.png"));
    frames.push("12-reopened.png".into());
    // Also paint the same menu at its supported minimum size.
    cx.update_window(reopened, |_, window, _| {
        window.resize(size(px(900.), px(620.)))
    })
    .unwrap();
    settle(&mut cx);
    footer(&mut cx, reopened, "preferences-close", "preferences-save");
    capture(&mut cx, reopened, &evidence.join("13-minimum-size.png"));
    frames.push("13-minimum-size.png".into());
    cx.update_window(reopened, |_, window, cx| window.press("ctrl-w", cx))
        .unwrap();
    settle(&mut cx);
    assert!(cx.update_window(reopened, |_, _, _| ()).is_err());
    cx.update_window(prompt, |_, window, _| window.remove_window())
        .ok();
    cx.update_window(main, |_, window, _| window.remove_window())
        .unwrap();
    drop(dashboard);
    drop(cx);
    drop(daemon);
    backend.handle.shutdown();
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "status": "passed", "real_gpu": true, "scoped_socket_removed": true, "screenshots": frames,
        "proof_boundary": "Actual Kit sidebar and settings controls in one window, retained popup/history drafts, independent file reopening, minimum size, close/reopen and independent real AskRule reply with retention review open. Owned synthetic socket/database only; no real daemon, compositor or packaging proof."
    })).unwrap()).unwrap();
    std::fs::write(
        evidence.join("actions.json"),
        serde_json::to_vec_pretty(&[
            "Open Settings directly, visit every category without opening another window",
            "Stage popup priority, switch away/back, save and independently reopen preferences",
            "Retain history days draft across navigation, review without applying",
            "Answer independent AskRule with history review still open",
            "Close/reopen Settings, paint minimum size, clean owned resources",
        ])
        .unwrap(),
    )
    .unwrap();
}
