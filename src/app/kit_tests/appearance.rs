use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::appearance::{ColorMode, DensityScale, Store};
    fn finish(cx: &mut TestAppContext) {
        wait_for(cx, |cx| !(crate::app::appearance::test_state(cx).0));
    }
    fn mode(cx: &mut TestAppContext) -> (ColorMode, bool) {
        cx.update(|cx| {
            (
                crate::app::appearance::test_state(cx).1,
                cx.global::<Theme>().is_dark(),
            )
        })
    }
    fn reveal(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
        use gpui::InputEvent as _;
        for _ in 0..32 {
            let direction = cx
                .update_window(window, |_, window, cx| {
                    window.render_frame(cx);
                    let clip = window.find("appearance-content").bounds();
                    let top = clip.top() + window.rem_size() * 4.;
                    let bottom = clip.bottom() - px(8.);
                    match window.try_find(id) {
                        Some(target)
                            if target.bounds().center().y >= top
                                && target.bounds().center().y <= bottom =>
                        {
                            None
                        }
                        Some(target) if target.bounds().center().y < top => Some(120.),
                        _ => Some(-120.),
                    }
                })
                .unwrap();
            let Some(direction) = direction else {
                return;
            };
            cx.update_window(window, |_, window, cx| {
                let viewport = window.viewport_size();
                let position = gpui::point(viewport.width * 0.75, viewport.height * 0.50);
                window.dispatch_event(
                    gpui::MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                window.render_frame(cx);
                window.dispatch_event(
                    gpui::ScrollWheelEvent {
                        position,
                        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(direction))),
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                window.render_frame(cx);
            })
            .unwrap();
            flush(cx);
        }
        panic!("{id} must be reachable inside Settings by wheel input");
    }
    fn appearance_click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
        if id != "appearance-close" {
            reveal(cx, window, id);
        }
        click(cx, window, id);
        if id != "appearance-close" && id != "appearance-reload" {
            flush(cx);
            click(cx, window, "appearance-save");
        }
    }
    fn density(cx: &mut TestAppContext) -> DensityScale {
        cx.update(|cx| crate::app::appearance::test_density(cx))
    }
    fn metrics(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
    ) -> (gpui::Pixels, gpui::Pixels, gpui::Pixels) {
        reveal(cx, window, "appearance-density-up");
        cx.update_window(window, |_, window, cx| {
            (
                window.rem_size(),
                cx.global::<Theme>().font_size,
                window.find("appearance-density-up").bounds().size.height,
            )
        })
        .unwrap()
    }
    fn field(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) -> String {
        click(cx, w, id);
        cx.update_window(w, |_, w, cx| {
            w.press("ctrl-a", cx);
            w.press("ctrl-c", cx);
        })
        .unwrap();
        flush(cx);
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
    }
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let store = Store::new(&config.database);
    std::fs::create_dir_all(store.themes_path()).unwrap();
    let palette_path = store.themes_path().join("ocean.json");
    let palette_bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "version":1,"name":"Owned Ocean","mode":"dark","colors":{
            "background":"#112233","foreground":"#F0F4F8",
            "primary":"#3388CC","primary_foreground":"#FFFFFF",
            "secondary":"#223344","secondary_foreground":"#E0E8F0",
            "muted":"#334455","muted_foreground":"#AABBCD",
            "accent":"#447799","accent_foreground":"#FFFFFF",
            "border":"#567890","danger":"#CC4455","danger_foreground":"#FFFFFF"
        }
    }))
    .unwrap();
    std::fs::write(&palette_path, &palette_bytes).unwrap();
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
        let mut c = protocol::ui_client::UiClient::new(channel);
        c.subscribe(protocol::ClientConfig {
            name: "appearance-fixture".into(),
            rules: vec![protocol::Rule {
                name: "theme-original".into(),
                enabled: true,
                action: "deny".into(),
                duration: "always".into(),
                operator: Some(protocol::Operator {
                    r#type: "simple".into(),
                    operand: "process.path".into(),
                    data: "/owned/theme-app".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
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
    cx.update(|cx| crate::app::appearance::init(&config.database, cx));
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
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::System, false));
    click(&mut cx, main, "appearance-preferences");
    let settings = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    checkpoint(&mut cx, settings, &evidence.join("01-system-settings.json"));
    assert_eq!(density(&mut cx), DensityScale::default());
    assert!(
        cx.update(|cx| crate::app::appearance::test_palette(cx))
            .is_none()
    );
    let baseline_metrics = metrics(&mut cx, settings);
    let baseline_bounds = cx.update_window(settings, |_, w, _| w.bounds()).unwrap();
    appearance_click(&mut cx, settings, "appearance-dark");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::Dark, true));
    checkpoint(&mut cx, main, &evidence.join("02-dark-dashboard.json"));
    cx.update_window(main, |_, w, cx| w.press("ctrl-2", cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(main, |_, w, cx| w.click(("rule-row", 0usize), cx))
        .unwrap();
    click(&mut cx, main, "rule-edit");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    fill(&mut cx, editor, "editor-name", "theme-unsaved");
    fill(
        &mut cx,
        editor,
        "editor-description",
        "Keep this while theme changes",
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("03-dark-rule-editor-draft.json"),
    );
    for _ in 0..5 {
        appearance_click(&mut cx, settings, "appearance-density-up");
        finish(&mut cx);
    }
    assert_eq!(density(&mut cx).value(), 5);
    assert_eq!(density(&mut cx).percent(), 110);
    let larger_metrics = metrics(&mut cx, settings);
    let device_scale = cx
        .update_window(settings, |_, w, _| w.scale_factor())
        .unwrap();
    std::fs::write(evidence.join("density-step-metrics.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "device_scale":device_scale,"requested_scale":1.10,
        "baseline":{"rem":f32::from(baseline_metrics.0),"font":f32::from(baseline_metrics.1),"button_height":f32::from(baseline_metrics.2)},
        "observed":{"rem":f32::from(larger_metrics.0),"font":f32::from(larger_metrics.1),"button_height":f32::from(larger_metrics.2)},
        "button_deviation_physical_pixels":(f32::from(larger_metrics.2)-f32::from(baseline_metrics.2)*1.10).abs()*device_scale
    })).unwrap()).unwrap();
    for (actual, baseline) in [
        (larger_metrics.0, baseline_metrics.0),
        (larger_metrics.1, baseline_metrics.1),
    ] {
        assert!(
            (f32::from(actual) - f32::from(baseline) * 1.10).abs() < 0.0001,
            "Exact density basis: actual {actual:?}, baseline {baseline:?}"
        );
    }
    // GPUI rounds the far and near layout edges to physical pixels. Their
    // difference may vary by one device pixel even for an exact rem height.
    assert!(
        (f32::from(larger_metrics.2) - f32::from(baseline_metrics.2) * 1.10).abs() * device_scale
            <= 1.0001,
        "Rendered Button height may round by at most one physical pixel: baseline {:?}, observed {:?}, device scale {device_scale}",
        baseline_metrics.2,
        larger_metrics.2
    );
    assert_eq!(
        cx.update_window(settings, |_, w, _| w.bounds()).unwrap(),
        baseline_bounds
    );
    assert_eq!(field(&mut cx, editor, "editor-name"), "theme-unsaved");
    assert_eq!(
        field(&mut cx, editor, "editor-description"),
        "Keep this while theme changes"
    );
    assert_eq!(
        Store::new(&config.database).load().unwrap().density.value(),
        5
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("12-density-retains-editor-draft.json"),
    );
    appearance_click(&mut cx, settings, "appearance-light");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::Light, false));
    assert_eq!(density(&mut cx).value(), 5);
    assert_eq!(metrics(&mut cx, settings), larger_metrics);
    assert_eq!(field(&mut cx, editor, "editor-name"), "theme-unsaved");
    assert_eq!(
        field(&mut cx, editor, "editor-description"),
        "Keep this while theme changes"
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("04-light-editor-retains-draft.json"),
    );
    click(&mut cx, editor, "editor-cancel");
    runtime.block_on(backend.handle.history.flush()).unwrap();
    assert!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "appearance-fixture",
            "theme-unsaved"
        )
        .unwrap()
        .is_none()
    );
    assert!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "appearance-fixture",
            "theme-original"
        )
        .unwrap()
        .is_some()
    );

    let reply = runtime.spawn(async move {
        daemon
            .0
            .ask_rule(protocol::Connection {
                process_path: "/owned/theme-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    let prompt = permission_window(&mut cx, &view);
    checkpoint(&mut cx, prompt, &evidence.join("05-light-permission.json"));
    appearance_click(&mut cx, settings, "appearance-dark");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::Dark, true));
    checkpoint(&mut cx, prompt, &evidence.join("06-dark-permission.json"));
    appearance_click(&mut cx, settings, "appearance-density-up");
    finish(&mut cx);
    assert_eq!(density(&mut cx).value(), 6);
    assert_eq!(
        cx.update_window(prompt, |_, w, _| w.rem_size()).unwrap(),
        baseline_metrics.0 * 1.12
    );
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("13-density-live-permission.json"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    std::fs::write(&store.path, b"{external invalid}").unwrap();
    appearance_click(&mut cx, settings, "appearance-light");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::Dark, true));
    assert_eq!(std::fs::read(&store.path).unwrap(), b"{external invalid}");
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("07-staged-save-error.json"),
    );
    appearance_click(&mut cx, settings, "appearance-reload");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::System, false));
    assert_eq!(density(&mut cx), DensityScale::default());
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("08-invalid-file-system-fallback.json"),
    );
    appearance_click(&mut cx, settings, "appearance-dark");
    finish(&mut cx);
    assert_eq!(store.load().unwrap().mode, ColorMode::Dark);
    for _ in 0..3 {
        appearance_click(&mut cx, settings, "appearance-density-down");
        finish(&mut cx);
    }
    assert_eq!(store.load().unwrap().density.value(), -3);
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("09-repaired-dark-preference.json"),
    );
    appearance_click(&mut cx, settings, "appearance-close");
    drop(view);
    shutdown(cx);
    let mut cx = context(test_cx);
    cx.update(|cx| crate::app::appearance::init(&config.database, cx));
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
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::Dark, true));
    assert_eq!(density(&mut cx).value(), -3);
    assert_eq!(
        cx.update_window(main, |_, w, _| w.rem_size()).unwrap(),
        baseline_metrics.0 * 0.94
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("10-restarted-dark-dashboard.json"),
    );
    click(&mut cx, main, "appearance-preferences");
    let settings = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    appearance_click(&mut cx, settings, "appearance-reset");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::System, false));
    assert_eq!(store.load().unwrap().mode, ColorMode::System);
    assert_eq!(
        density(&mut cx).value(),
        -3,
        "Color reset preserves density"
    );
    checkpoint(&mut cx, settings, &evidence.join("11-reset-system.json"));
    for _ in -20..-3 {
        appearance_click(&mut cx, settings, "appearance-density-down");
        finish(&mut cx);
    }
    assert_eq!(density(&mut cx).value(), -20);
    assert_eq!(density(&mut cx).percent(), 60);
    appearance_click(&mut cx, settings, "appearance-density-down");
    finish(&mut cx);
    assert_eq!(
        density(&mut cx).value(),
        -20,
        "Lower endpoint refuses more input"
    );
    let minimum_metrics = metrics(&mut cx, settings);
    checkpoint(&mut cx, settings, &evidence.join("14-density-minimum.json"));
    for _ in -20..20 {
        appearance_click(&mut cx, settings, "appearance-density-up");
        finish(&mut cx);
    }
    assert_eq!(density(&mut cx).value(), 20);
    assert_eq!(density(&mut cx).percent(), 140);
    appearance_click(&mut cx, settings, "appearance-density-up");
    finish(&mut cx);
    assert_eq!(
        density(&mut cx).value(),
        20,
        "Upper endpoint refuses more input"
    );
    let maximum_metrics = metrics(&mut cx, settings);
    assert!((f32::from(minimum_metrics.0) - f32::from(baseline_metrics.0) * 0.60).abs() < 0.05);
    assert!((f32::from(maximum_metrics.0) - f32::from(baseline_metrics.0) * 1.40).abs() < 0.05);
    assert!(maximum_metrics.2 > minimum_metrics.2);
    checkpoint(&mut cx, settings, &evidence.join("15-density-maximum.json"));
    appearance_click(&mut cx, settings, "appearance-density-reset");
    finish(&mut cx);
    assert_eq!(density(&mut cx), DensityScale::default());
    assert_eq!(mode(&mut cx), (ColorMode::System, false));
    assert_eq!(
        metrics(&mut cx, settings),
        baseline_metrics,
        "Reset derives from the original basis without accumulated scale drift"
    );
    checkpoint(&mut cx, settings, &evidence.join("16-density-reset.json"));
    let invalid = br#"{"version":1,"color_mode":"dark","density_scale":21}"#;
    std::fs::write(&store.path, invalid).unwrap();
    appearance_click(&mut cx, settings, "appearance-reload");
    finish(&mut cx);
    assert_eq!(density(&mut cx), DensityScale::default());
    assert_eq!(mode(&mut cx), (ColorMode::System, false));
    assert!(store.load().unwrap().warning.is_some());
    assert_eq!(std::fs::read(&store.path).unwrap(), invalid);
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("17-density-invalid-file-preserved.json"),
    );
    appearance_click(&mut cx, settings, "appearance-density-up");
    finish(&mut cx);
    assert_eq!(store.load().unwrap().density.value(), 1);
    assert!(store.load().unwrap().warning.is_none());
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("18-density-explicit-repair.json"),
    );
    appearance_click(&mut cx, settings, "appearance-density-reset");
    finish(&mut cx);
    assert_eq!(store.load().unwrap().density, DensityScale::default());
    std::fs::write(evidence.join("density-metrics.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "basis": {"rem":f32::from(baseline_metrics.0),"font":f32::from(baseline_metrics.1),"button_height":f32::from(baseline_metrics.2)},
        "scale_5": {"percent":110,"rem":f32::from(larger_metrics.0),"font":f32::from(larger_metrics.1),"button_height":f32::from(larger_metrics.2)},
        "scale_minimum": {"value":-20,"percent":60,"rem":f32::from(minimum_metrics.0),"button_height":f32::from(minimum_metrics.2)},
        "scale_maximum": {"value":20,"percent":140,"rem":f32::from(maximum_metrics.0),"button_height":f32::from(maximum_metrics.2)},
        "fixed_window_bounds_unchanged":true,"fresh_context_density":-3,"permission_reply":"deny"
    })).unwrap()).unwrap();
    appearance_click(&mut cx, settings, "appearance-palette-ocean");
    finish(&mut cx);
    let custom_background = gpui::Hsla::from(gpui::rgb(0x112233));
    assert_eq!(
        cx.update(|cx| crate::app::appearance::test_palette(cx)
            .unwrap()
            .as_str()
            .to_owned()),
        "ocean"
    );
    assert_eq!(
        cx.update(|cx| cx.global::<Theme>().background),
        custom_background
    );
    assert_eq!(store.load().unwrap().palette.unwrap().as_str(), "ocean");
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("19-native-palette-selected.json"),
    );
    appearance_click(&mut cx, settings, "appearance-density-up");
    finish(&mut cx);
    assert_eq!(density(&mut cx).value(), 1);
    assert_eq!(
        cx.update(|cx| crate::app::appearance::test_palette(cx)
            .unwrap()
            .as_str()
            .to_owned()),
        "ocean"
    );
    assert_eq!(
        cx.update(|cx| cx.global::<Theme>().background),
        custom_background
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("20-density-preserves-native-palette.json"),
    );
    appearance_click(&mut cx, settings, "appearance-light");
    finish(&mut cx);
    assert!(
        cx.update(|cx| crate::app::appearance::test_palette(cx))
            .is_none()
    );
    assert!(store.load().unwrap().palette.is_none());
    assert_ne!(
        cx.update(|cx| cx.global::<Theme>().background),
        custom_background
    );
    assert_eq!(density(&mut cx).value(), 1);
    checkpoint(
        &mut cx,
        main,
        &evidence.join("21-builtin-clears-native-palette.json"),
    );
    appearance_click(&mut cx, settings, "appearance-palette-ocean");
    finish(&mut cx);
    appearance_click(&mut cx, settings, "appearance-close");
    drop(view);
    shutdown(cx);
    let mut cx = context(test_cx);
    cx.update(|cx| crate::app::appearance::init(&config.database, cx));
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
    finish(&mut cx);
    assert_eq!(density(&mut cx).value(), 1);
    assert_eq!(
        cx.update(|cx| crate::app::appearance::test_palette(cx)
            .unwrap()
            .as_str()
            .to_owned()),
        "ocean"
    );
    assert_eq!(
        cx.update(|cx| cx.global::<Theme>().background),
        custom_background
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("22-native-palette-restarted.json"),
    );
    click(&mut cx, main, "appearance-preferences");
    let settings = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    let saved_bytes = std::fs::read(&store.path).unwrap();
    std::fs::remove_file(&palette_path).unwrap();
    appearance_click(&mut cx, settings, "appearance-reload");
    finish(&mut cx);
    assert_eq!(
        cx.update(|cx| crate::app::appearance::test_palette(cx)
            .unwrap()
            .as_str()
            .to_owned()),
        "ocean"
    );
    assert_ne!(
        cx.update(|cx| cx.global::<Theme>().background),
        custom_background
    );
    assert_eq!(
        std::fs::read(&store.path).unwrap(),
        saved_bytes,
        "Missing catalogue file cannot erase a saved palette choice"
    );
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("23-missing-native-palette-fallback.json"),
    );
    std::fs::write(&palette_path, &palette_bytes).unwrap();
    appearance_click(&mut cx, settings, "appearance-reload");
    finish(&mut cx);
    assert_eq!(
        cx.update(|cx| cx.global::<Theme>().background),
        custom_background
    );
    assert_eq!(
        std::fs::read(&palette_path).unwrap(),
        palette_bytes,
        "Palette catalogue is read-only to appearance preferences"
    );
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("24-native-palette-restored.json"),
    );
    appearance_click(&mut cx, settings, "appearance-system");
    finish(&mut cx);
    appearance_click(&mut cx, settings, "appearance-density-reset");
    finish(&mut cx);
    appearance_click(&mut cx, settings, "appearance-close");
    drop(view);
    shutdown(cx);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let frames = [
        "system-settings",
        "dark-dashboard",
        "dark-rule-editor-draft",
        "light-editor-retains-draft",
        "light-permission",
        "dark-permission",
        "staged-save-error",
        "invalid-file-system-fallback",
        "repaired-dark-preference",
        "restarted-dark-dashboard",
        "reset-system",
        "density-retains-editor-draft",
        "density-live-permission",
        "density-minimum",
        "density-maximum",
        "density-reset",
        "density-invalid-file-preserved",
        "density-explicit-repair",
        "native-palette-selected",
        "density-preserves-native-palette",
        "builtin-clears-native-palette",
        "native-palette-restarted",
        "missing-native-palette-fallback",
        "native-palette-restored",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| format!("{:02}-{n}.json", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":frames,"proof_boundary":"Actual Kit Settings color/density/reset/reload controls, exact 2%-per-step rem/font/button geometry, -20/20 endpoint refusal, fixed window bounds, dashboard/editor/popup repaint, copied unsaved editor fields, independent real AskRule while density changes, stale save preserves the applied theme and retains the staged choices, out-of-range density file preservation/explicit repair fresh GPUI context color/density persistence, actual bounded data-only native palette choice/token repaint, density preservation, built-in reset, restart, missing-file fallback and restored-file reload. Platform is fixed Light here: actual native system appearance-change events, compositor, Qt Material palette equivalence and whole-app visual pass remain separate."})).unwrap()).unwrap();
}
