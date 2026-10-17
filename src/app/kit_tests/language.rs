use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::appearance::{ColorMode, DensityScale, Store};
    use crate::i18n::LanguageChoice;
    use gpui::InputEvent as _;
    fn finish(cx: &mut TestAppContext) {
        wait_for(cx, |cx| !(crate::app::appearance::test_state(cx).0));
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
        let visible = cx
            .update_window(window, |_, w, _| {
                let target = w.find(id);
                eprintln!(
                    "Language action {id}: bounds={:?}, visible={}, viewport={:?}",
                    target.bounds(),
                    target.visible(),
                    w.viewport_size()
                );
                target.visible()
            })
            .unwrap();
        if !visible {
            checkpoint(
                cx,
                window,
                &evidence().join(format!("unreachable-{id}.json")),
            );
        }
        click(cx, window, id);
        if id != "appearance-close" && id != "appearance-reload" {
            flush(cx);
            click(cx, window, "appearance-save");
        }
    }
    fn reveal_body(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        body_id: &'static str,
        id: gpui::ElementId,
    ) {
        for _ in 0..32 {
            let direction = cx
                .update_window(window, |_, w, cx| {
                    w.render_frame(cx);
                    let body = w.find(body_id).bounds();
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
                let body = w.find(body_id).bounds();
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

    fn condition(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        body: &'static str,
        field: &str,
        op: &str,
        value: &str,
    ) {
        let (field_id, comparison_id, value_id, add) = if body == "rule-filter-body" {
            (
                "rule-filter-field",
                "rule-filter-comparison",
                "rule-filter-value",
                "rule-filter-add",
            )
        } else {
            (
                "history-filter-field",
                "history-filter-comparison",
                "history-filter-value",
                "history-filter-add",
            )
        };
        reveal_body(cx, w, body, field_id.into());
        pick(cx, w, field_id, field);
        reveal_body(cx, w, body, comparison_id.into());
        pick(cx, w, comparison_id, op);
        reveal_body(cx, w, body, value_id.into());
        fill(cx, w, value_id, value);
        reveal_body(cx, w, body, add.into());
        click(cx, w, add);
        flush(cx);
    }
    fn open(
        cx: &mut TestAppContext,
        backend: &Backend,
        config: &Config,
    ) -> (AnyWindowHandle, gpui::Entity<Dashboard>) {
        cx.update(|cx| crate::app::appearance::init(&config.database, cx));
        let (w, v) = cx.update(|cx| {
            gpui_kit::open_window(crate::app::dashboard_options(), cx, |w, cx| {
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx))
            })
            .unwrap()
        });
        finish(cx);
        (w, v)
    }
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let store = Store::new(&config.database);
    let saved = store
        .save_preferences(
            &store.load().unwrap(),
            ColorMode::Dark,
            DensityScale::new(2).unwrap(),
        )
        .unwrap();
    store
        .save_language(&saved, LanguageChoice::try_from("en".to_string()).unwrap())
        .unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (daemon, _notification_sender, _notification_stream) = runtime.block_on(async {
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
                name: "language-fixture".into(),
                rules: (0..3)
                    .map(|i| protocol::Rule {
                        name: format!("language.{i}"),
                        enabled: i != 1,
                        action: if i == 2 { "allow" } else { "deny" }.into(),
                        duration: "always".into(),
                        operator: Some(protocol::Operator {
                            r#type: "simple".into(),
                            operand: "process.path".into(),
                            data: format!("/owned/language-{i}"),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let stream = client
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        (client, tx, stream)
    });
    backend.handle.history.ingest(
        "language-fixture".into(),
        (0..3)
            .map(|i| protocol::Event {
                unixnano: crate::history::now_nanos() + i,
                connection: Some(protocol::Connection {
                    process_path: format!("/owned/language-{i}"),
                    process_cwd: if i == 2 { "/other" } else { "/work" }.into(),
                    dst_port: if i == 1 { 80 } else { 443 },
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
    let (main, view) = open(&mut cx, &backend, &config);
    assert_eq!(crate::i18n::current().locale.as_str(), "en");
    cx.update_window(main, |_, w, cx| w.within("main-tabs").click(1usize, cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(main, |_, w, cx| w.click(("rule-row", 0usize), cx))
        .unwrap();
    click(&mut cx, main, "rule-edit");
    flush(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    fill(&mut cx, editor, "editor-name", "language-unsaved");
    fill(
        &mut cx,
        editor,
        "editor-description",
        "Preserve while changing language",
    );
    click(&mut cx, main, "appearance-preferences");
    let settings = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    let before = store.load().unwrap();
    appearance_click(&mut cx, settings, "appearance-language-fr");
    finish(&mut cx);
    let saved = store.load().unwrap();
    assert_eq!(
        saved.language,
        LanguageChoice::try_from("fr".to_string()).unwrap()
    );
    assert_eq!(saved.mode, before.mode);
    assert_eq!(saved.density, before.density);
    assert_eq!(saved.palette, before.palette);
    assert_eq!(crate::i18n::current().locale.as_str(), "en");
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("01-language-save-requires-restart.json"),
    );
    assert_eq!(
        cx.update_window(editor, |_, w, _| w
            .find("editor-name")
            .value()
            .unwrap()
            .to_string())
            .unwrap(),
        "language-unsaved"
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("02-language-preserves-unsaved-editor.json"),
    );
    drop(view);
    shutdown(cx);
    let mut cx = context(test_cx);
    let (main, view) = open(&mut cx, &backend, &config);
    assert_eq!(crate::i18n::current().locale.as_str(), "fr");
    assert_eq!(
        gpui_kit::component::_rust_i18n_translate("fr", "Input.Copy").to_string(),
        "Copier"
    );
    assert!(
        cx.update_window(main, |_, w, _| w
            .within("main-tabs")
            .find(0usize)
            .label()
            .unwrap()
            .contains("Historique"))
            .unwrap()
    );
    click(&mut cx, main, "appearance-preferences");
    let settings = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    appearance_click(&mut cx, settings, "appearance-language-fr");
    finish(&mut cx);
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("03-fresh-context-french-appearance.json"),
    );
    click(&mut cx, settings, "appearance-close");
    cx.update_window(main, |_, w, cx| w.within("main-tabs").click(1usize, cx))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, main, "rule-filter-open");
    flush(&mut cx);
    let filters = cx.read_entity(&view, |v, _| v.rules.filter_window.unwrap());
    condition(
        &mut cx,
        filters,
        "rule-filter-body",
        "Activée",
        "Est égal à",
        "1",
    );
    condition(
        &mut cx,
        filters,
        "rule-filter-body",
        "Action",
        "Est égal à",
        "deny",
    );
    click(&mut cx, filters, "rule-filter-apply");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.rules.page.rules.len() == 1 && v.rules.page.rules[0].name == "language.0"
        })
    });
    assert_eq!(
        cx.read_entity(&view, |v, _| v
            .rules
            .page
            .rules
            .iter()
            .map(|r| r.name.clone())
            .collect::<Vec<_>>()),
        vec!["language.0"]
    );
    checkpoint(
        &mut cx,
        filters,
        &evidence.join("04-french-rule-field-enums.json"),
    );
    click(&mut cx, filters, "rule-filter-close");
    cx.update_window(main, |_, w, cx| w.within("main-tabs").click(0usize, cx))
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
    condition(
        &mut cx,
        tools,
        "history-tools-panel",
        "Dossier de travail",
        "Contient",
        "/work",
    );
    condition(
        &mut cx,
        tools,
        "history-tools-panel",
        "Port de destination",
        "Au moins / à partir de",
        "443",
    );
    reveal_body(
        &mut cx,
        tools,
        "history-tools-panel",
        "history-filter-apply".into(),
    );
    click(&mut cx, tools, "history-filter-apply");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.history.page.rows.len() == 1 && v.history.page.rows[0].process == "/owned/language-0"
        })
    });
    assert_eq!(
        cx.read_entity(&view, |v, _| v
            .history
            .page
            .rows
            .iter()
            .map(|r| r.process.clone())
            .collect::<Vec<_>>()),
        vec!["/owned/language-0"]
    );
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("05-french-history-field-enums.json"),
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("06-french-filtered-dashboard.json"),
    );
    drop(view);
    shutdown(cx);
    let saved = store.load().unwrap();
    store
        .save_language(&saved, LanguageChoice::try_from("zz".to_string()).unwrap())
        .unwrap();
    let missing_bytes = std::fs::read(&store.path).unwrap();
    let mut cx = context(test_cx);
    let (main, view) = open(&mut cx, &backend, &config);
    assert_eq!(crate::i18n::current().locale.as_str(), "en");
    assert!(crate::i18n::current().warning.is_some());
    assert_eq!(std::fs::read(&store.path).unwrap(), missing_bytes);
    click(&mut cx, main, "appearance-preferences");
    let settings = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    reveal(&mut cx, settings, "appearance-language-system");
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("07-missing-language-fallback.json"),
    );
    appearance_click(&mut cx, settings, "appearance-language-system");
    finish(&mut cx);
    assert_eq!(store.load().unwrap().language, LanguageChoice::System);
    checkpoint(
        &mut cx,
        settings,
        &evidence.join("08-missing-language-explicit-system-repair.json"),
    );
    // Select each completed catalogue through the real saved-language control,
    // then let production startup activate it in a fresh GPUI context.
    let mut view = view;
    let mut settings = settings;
    for (id, button, frame) in [
        ("de", "appearance-language-de", "09-german-catalogue.json"),
        ("es", "appearance-language-es", "10-spanish-catalogue.json"),
        ("ja", "appearance-language-ja", "11-japanese-catalogue.json"),
    ] {
        let previous = crate::i18n::current().locale.clone();
        appearance_click(&mut cx, settings, button);
        finish(&mut cx);
        assert_eq!(
            store.load().unwrap().language,
            LanguageChoice::try_from(id.to_owned()).unwrap()
        );
        assert_eq!(crate::i18n::current().locale, previous);
        drop(view);
        shutdown(cx);
        cx = context(test_cx);
        let (main, next_view) = open(&mut cx, &backend, &config);
        view = next_view;
        assert_eq!(crate::i18n::current().locale.as_str(), id);
        let (translated, total) = crate::i18n::current().coverage();
        assert_eq!(translated, total);
        let history_title = crate::i18n::tr("History");
        assert_ne!(history_title, "History");
        assert!(
            cx.update_window(main, |_, w, _| w
                .within("main-tabs")
                .find(0usize)
                .label()
                .unwrap()
                .contains(history_title.as_str()))
                .unwrap()
        );
        checkpoint(&mut cx, main, &evidence.join(frame));
        click(&mut cx, main, "appearance-preferences");
        settings = cx.read_entity(&view, |v, _| v.settings_window.unwrap());
    }
    drop(view);
    shutdown(cx);
    drop(_notification_stream);
    drop(_notification_sender);
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-language-save-requires-restart.json","02-language-preserves-unsaved-editor.json","03-fresh-context-french-appearance.json","04-french-rule-field-enums.json","05-french-history-field-enums.json","06-french-filtered-dashboard.json","07-missing-language-fallback.json","08-missing-language-explicit-system-repair.json","09-german-catalogue.json","10-spanish-catalogue.json","11-japanese-catalogue.json"],"proof_boundary":"Actual language choice persists without changing active session locale or unsaved rule editor; fresh GPUI context executes startup appearance load and activates French/Kit Copy; actual translated rule and history Selects preserve stable numeric/text field enums and exact filtered rows; missing saved language falls back without file rewrite until explicit System repair. Actual saved German/Spanish/Japanese choices and fresh GPUI startup activate complete catalogues and paint translated navigation; original French draft/typed field and missing-language fallback assertions remain. Owned backend/database; no real daemon, OS process restart, external catalogue install or whole-application visual/native-speaker review proof."})).unwrap()).unwrap();
}
