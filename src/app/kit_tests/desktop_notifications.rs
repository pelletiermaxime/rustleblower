use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    fn control_ready(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
        for _ in 0..50 {
            flush(cx);
            if cx
                .update_window(window, |_, w, _| {
                    w.try_find(id)
                        .is_some_and(|e| e.visible() && e.disabled() != Some(true))
                })
                .unwrap()
            {
                return;
            }
        }
        panic!("{id} did not become enabled after the asynchronous reply");
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
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "notice-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    let mut cx = context(test_cx);
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(crate::app::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    let (tray, _tray_actions, _availability) = crate::app::tray::Client::fixture();
    let (notices, actions) = crate::app::desktop_notifications::Client::fixture();
    cx.update_entity(&view, |view, _| {
        view.tray = Some(tray);
        view.desktop_notifications = Some(notices);
    });
    let preferences = cx
        .update(|cx| crate::app::preferences_view::open(backend.handle.clone(), cx))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, preferences, "preferences-notifications");
    click(&mut cx, preferences, "preferences-save");
    wait_for(&mut cx, |_cx| {
        !crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .desktop_notifications
    });

    assert!(
        !crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .desktop_notifications
    );
    assert!(backend.handle.preferences.snapshot().popups);
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("01-notifications-muted.json"),
    );
    runtime.block_on(async {
        daemon
            .post_alert(protocol::Alert {
                id: 1,
                action: 1,
                data: Some(protocol::alert::Data::Text(
                    "Archived while desktop muted".into(),
                )),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    assert_eq!(
        crate::alerts::page(&config.database, &Default::default())
            .unwrap()
            .rows
            .len(),
        1
    );
    click(&mut cx, preferences, "preferences-notifications");
    click(&mut cx, preferences, "preferences-save");
    wait_for(&mut cx, |_cx| {
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .desktop_notifications
    });

    assert!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .desktop_notifications
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("02-notifications-enabled.json"),
    );
    click(&mut cx, preferences, "preferences-close");
    let subscription_config = || {
        runtime.block_on(async {
            match backend
                .handle
                .subscriptions
                .request(crate::subscriptions::Request::NotificationSettings)
                .unwrap()
                .await
                .unwrap()
                .unwrap()
            {
                crate::subscriptions::Reply::NotificationSettings(revision, config) => {
                    (revision, config)
                }
                _ => panic!("Expected subscription notification settings"),
            }
        })
    };
    cx.update_window(window, |_, window, cx| window.press("ctrl-7", cx))
        .unwrap();
    flush(&mut cx);
    assert!(!subscription_config().1.enabled);
    control_ready(&mut cx, window, "subs-notifications");
    click(&mut cx, window, "subs-notifications");
    flush(&mut cx);
    click(&mut cx, window, "subs-notify-enabled");
    fill(
        &mut cx,
        window,
        "subs-notify-success",
        "Owned success message",
    );
    fill(
        &mut cx,
        window,
        "subs-notify-error",
        "Owned failure message",
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("05-subscription-notice-settings.json"),
    );
    click(&mut cx, window, "subs-notify-save");
    flush(&mut cx);
    let saved = subscription_config().1;
    assert!(saved.enabled);
    assert_eq!(saved.success_message, "Owned success message");
    assert_eq!(saved.error_message, "Owned failure message");
    control_ready(&mut cx, window, "subs-notifications");
    click(&mut cx, window, "subs-notifications");
    flush(&mut cx);
    fill(&mut cx, window, "subs-notify-success", "Unsaved back draft");
    click(&mut cx, window, "subs-notify-cancel");
    assert_eq!(
        subscription_config().1,
        saved,
        "Back preserves saved notification configuration"
    );
    control_ready(&mut cx, window, "subs-notifications");
    click(&mut cx, window, "subs-notifications");
    flush(&mut cx);
    fill(&mut cx, window, "subs-notify-success", "Stale local draft");
    let (revision, mut external) = subscription_config();
    external.success_message = "External saved success".into();
    runtime.block_on(async {
        backend
            .handle
            .subscriptions
            .request(crate::subscriptions::Request::SaveNotificationSettings {
                revision,
                config: external.clone(),
            })
            .unwrap()
            .await
            .unwrap()
            .unwrap();
    });
    click(&mut cx, window, "subs-notify-save");
    flush(&mut cx);
    assert_eq!(subscription_config().1, external);
    assert!(
        cx.update_window(window, |_, w, _| w.find("subs-notify-save").visible())
            .unwrap(),
        "Stale save retains the editable form"
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("06-subscription-notice-stale-refused.json"),
    );
    click(&mut cx, window, "subs-notify-reload");
    flush(&mut cx);
    click(&mut cx, window, "subs-notify-success");
    cx.update_window(window, |_, w, cx| {
        w.press("ctrl-a", cx);
        w.press("ctrl-c", cx);
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_from_clipboard().unwrap().text().unwrap()) == ("External saved success")
    });

    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        "External saved success"
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("07-subscription-notice-reloaded.json"),
    );
    cx.update_window(window, |_, w, cx| w.press("ctrl-enter", cx))
        .unwrap();
    control_ready(&mut cx, window, "subs-notifications");
    assert!(
        cx.update_window(window, |_, w, _| w.find("subs-notifications").visible())
            .unwrap(),
        "Ctrl+Enter returns to the catalogue after saving"
    );
    control_ready(&mut cx, window, "subs-notifications");
    click(&mut cx, window, "subs-notifications");
    flush(&mut cx);
    click(&mut cx, window, "subs-notify-enabled");
    checkpoint(
        &mut cx,
        window,
        &evidence.join("08-subscription-notice-disabled.json"),
    );
    click(&mut cx, window, "subs-notify-save");
    flush(&mut cx);
    assert_eq!(
        subscription_config().1,
        crate::subscriptions::notifications::Config::default()
    );
    control_ready(&mut cx, window, "subs-notifications");
    click(&mut cx, window, "subs-notifications");
    flush(&mut cx);
    click(&mut cx, window, "subs-notify-enabled");
    click(&mut cx, window, "subs-notify-save");
    flush(&mut cx);
    assert!(subscription_config().1.enabled);
    cx.update_window(window, |_, window, cx| window.press("ctrl-2", cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        assert!(view.update(cx, |view, _| view.should_close_dashboard(window)));
        window.remove_window();
    })
    .unwrap();
    flush(&mut cx);
    let mut asking = daemon.clone();
    let request = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/notice-permission".into(),
                dst_host: "notice.example".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    let popup = permission_window(&mut cx, &view);
    checkpoint(
        &mut cx,
        popup,
        &evidence.join("03-notice-permission-hidden.json"),
    );
    actions
        .try_send(crate::desktop_notifications::Target::Alerts)
        .unwrap();
    flush(&mut cx);
    let restored = cx.read_entity(&view, |view, _| {
        assert!(view.screen == crate::app::Screen::Alerts);
        view.main_window.unwrap()
    });
    assert_eq!(
        backend.handle.prompts.snapshot().len(),
        1,
        "Opening an alert never answers a permission request"
    );
    checkpoint(
        &mut cx,
        restored,
        &evidence.join("04-alert-action-restored.json"),
    );
    cx.update_window(restored, |_, window, cx| {
        assert!(view.update(cx, |view, _| view.should_close_dashboard(window)));
        window.remove_window();
    })
    .unwrap();
    flush(&mut cx);
    actions
        .try_send(crate::desktop_notifications::Target::Subscriptions)
        .unwrap();
    flush(&mut cx);
    let restored = cx.read_entity(&view, |view, _| {
        assert!(view.screen == crate::app::Screen::Subscriptions);
        view.main_window.unwrap()
    });
    assert_eq!(
        backend.handle.prompts.snapshot().len(),
        1,
        "Subscription action never answers permission"
    );
    checkpoint(
        &mut cx,
        restored,
        &evidence.join("09-subscription-action-restored.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(request).unwrap().action, "deny");
    let preferences = cx
        .update(|cx| crate::app::preferences_view::open(backend.handle.clone(), cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(preferences, |_, w, cx| {
        w.within("preferences-tabs").click(3usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    let current = crate::preferences::Store::open(&config.database)
        .unwrap()
        .snapshot();
    assert!(current.missed_popup_notifications);
    assert_eq!(
        current.missed_popup_template,
        crate::desktop_notifications::missed::DEFAULT_TEMPLATE
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("10-missed-default-template.json"),
    );
    let encoded = r"%rule.action%|%conn.process%\n%node.hostname%\t\\literal";
    let decoded = crate::desktop_notifications::missed::decode_input(encoded).unwrap();
    fill(&mut cx, preferences, "preferences-missed-template", encoded);
    click(&mut cx, preferences, "preferences-save");
    control_ready(&mut cx, preferences, "preferences-save");
    assert_eq!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .missed_popup_template,
        decoded
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("11-missed-custom-preview.json"),
    );
    click(&mut cx, preferences, "preferences-close");
    let preferences = cx
        .update(|cx| crate::app::preferences_view::open(backend.handle.clone(), cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(preferences, |_, w, cx| {
        w.within("preferences-tabs").click(3usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, preferences, "preferences-missed-template");
    cx.update_window(preferences, |_, w, cx| {
        w.press("ctrl-a", cx);
        w.press("ctrl-c", cx);
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_from_clipboard().unwrap().text().unwrap())
            == (crate::desktop_notifications::missed::encode_input(&decoded))
    });

    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        crate::desktop_notifications::missed::encode_input(&decoded)
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("12-missed-template-reopened.json"),
    );
    click(&mut cx, preferences, "preferences-missed-notifications");
    click(&mut cx, preferences, "preferences-save");
    control_ready(&mut cx, preferences, "preferences-save");
    let muted = crate::preferences::Store::open(&config.database)
        .unwrap()
        .snapshot();
    assert!(!muted.missed_popup_notifications);
    assert_eq!(muted.missed_popup_template, decoded);
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("13-missed-notices-muted.json"),
    );
    fill(
        &mut cx,
        preferences,
        "preferences-missed-template",
        r"unsupported\q",
    );
    click(&mut cx, preferences, "preferences-save");
    control_ready(&mut cx, preferences, "preferences-save");
    assert_eq!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot(),
        muted,
        "Malformed escape cannot publish staged preferences"
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("14-missed-invalid-escape-refused.json"),
    );
    click(&mut cx, preferences, "preferences-missed-reset");
    click(&mut cx, preferences, "preferences-save");
    control_ready(&mut cx, preferences, "preferences-save");
    let reset = crate::preferences::Store::open(&config.database)
        .unwrap()
        .snapshot();
    assert!(
        !reset.missed_popup_notifications,
        "Message reset preserves the mute choice"
    );
    assert_eq!(
        reset.missed_popup_template,
        crate::desktop_notifications::missed::DEFAULT_TEMPLATE
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("15-missed-default-reset.json"),
    );
    click(&mut cx, preferences, "preferences-missed-notifications");
    click(&mut cx, preferences, "preferences-save");
    control_ready(&mut cx, preferences, "preferences-save");
    assert!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .missed_popup_notifications
    );
    click(&mut cx, preferences, "preferences-close");
    let mut asking = daemon.clone();
    let request = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/missed-control-permission".into(),
                dst_host: "missed-control.example".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    let popup = permission_window(&mut cx, &view);
    cx.update_window(restored, |_, w, cx| {
        assert!(view.update(cx, |v, _| v.should_close_dashboard(w)));
        w.remove_window();
    })
    .unwrap();
    flush(&mut cx);
    actions
        .try_send(crate::desktop_notifications::Target::Rules)
        .unwrap();
    flush(&mut cx);
    let restored = cx.read_entity(&view, |v, _| {
        assert!(v.screen == crate::app::Screen::Rules);
        v.main_window.unwrap()
    });
    assert_eq!(
        backend.handle.prompts.snapshot().len(),
        1,
        "Opening rules from a missed-popup notice never answers another request"
    );
    checkpoint(
        &mut cx,
        restored,
        &evidence.join("16-missed-action-restores-rules.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(request).unwrap().action, "deny");
    drop(daemon);
    drop(actions);
    drop(view);
    shutdown(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-notifications-muted.json","02-notifications-enabled.json","03-notice-permission-hidden.json","04-alert-action-restored.json","05-subscription-notice-settings.json","06-subscription-notice-stale-refused.json","07-subscription-notice-reloaded.json","08-subscription-notice-disabled.json","09-subscription-action-restored.json","10-missed-default-template.json","11-missed-custom-preview.json","12-missed-template-reopened.json","13-missed-notices-muted.json","14-missed-invalid-escape-refused.json","15-missed-default-reset.json","16-missed-action-restores-rules.json"],"proof_boundary":"Actual general and subscription Kit toggles/save/reopen, custom fields, Back preservation, external metadata stale refusal, reload and Ctrl+Enter, disable-clears-source messages, archive while generally muted, missed-popup tab/default/custom escaped message/save/reopen/mute/invalid-escape refusal/reset, production close boundary and distinct Alerts/Blocklists/Rules callback restoration, two real gRPC permissions answered by GPUI input. Notification callbacks are synthetic; owned HTTP worker, actual missed-timeout RPC and private-bus tests prove production outcomes and transport separately. No host desktop banner proof."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Disable desktop notifications through actual checkbox and Save; independently reopen preferences and retain SHOW_ALERT","Enable and save; close preferences and retain dashboard through production close boundary","Deliver real AskRule while dashboard absent; notification callback restores Alerts without answering","Deny once through actual GPUI input; require real reply and owned cleanup"]).unwrap()).unwrap();
}
