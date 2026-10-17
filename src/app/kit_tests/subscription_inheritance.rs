use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::subscriptions::{
        Reply, Request,
        limits::{Override, Unit},
        model::*,
    };
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let request = |r| {
        runtime.block_on(async {
            backend
                .handle
                .subscriptions
                .request(r)
                .unwrap()
                .await
                .unwrap()
                .unwrap()
        })
    };
    let rows = || match request(Request::Page(Query::default())) {
        Reply::Page(p) => p,
        _ => panic!(),
    };
    let legacy = match request(Request::Save {
        previous: None,
        spec: Spec {
            name: "Legacy absolute".into(),
            url: "http://127.0.0.1:9/owned-legacy".into(),
            interval: Some(Override::Absolute(60)),
            timeout: Some(Override::Absolute(2)),
            max_bytes: Some(Override::Absolute(1024)),
            ..Default::default()
        },
    }) {
        Reply::Saved(id) => id,
        _ => panic!(),
    };
    let mut cx = context(test_cx);
    let mut view = None;
    let window = cx
        .open_test_window(size(px(1200.), px(1000.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let view = view.unwrap();
    let window: AnyWindowHandle = window.into();
    flush(&mut cx);
    cx.update_window(window, |_, w, cx| w.press("ctrl-7", cx))
        .unwrap();
    flush(&mut cx);
    let select = |cx: &mut TestAppContext, id| {
        cx.update_window(window, |_, w, cx| w.click(("subscription-row", id), cx))
            .unwrap();
        flush(cx);
    };
    click(&mut cx, window, "subs-defaults");
    flush(&mut cx);
    fill(&mut cx, window, "subs-default-interval", "2");
    pick(&mut cx, window, "subs-default-interval-unit", "hours");
    fill(&mut cx, window, "subs-default-timeout", "2");
    pick(&mut cx, window, "subs-default-timeout-unit", "minutes");
    fill(&mut cx, window, "subs-default-size", "3");
    pick(&mut cx, window, "subs-default-size-unit", "MB");
    checkpoint(
        &mut cx,
        window,
        &evidence.join("01-default-count-unit-controls.json"),
    );
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    click(&mut cx, window, "subs-add");
    fill(&mut cx, window, "subs-name", "Independent inherited limits");
    fill(
        &mut cx,
        window,
        "subs-url",
        "http://127.0.0.1:9/owned-inheritance",
    );
    pick(&mut cx, window, "subs-interval-unit", "minutes");
    pick(&mut cx, window, "subs-timeout-unit", "seconds");
    pick(&mut cx, window, "subs-size-unit", "KB");
    flush(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert_eq!(state.0, vec!["", "", ""]);
    assert_eq!(state.1, vec!["minutes", "seconds", "KB"]);
    assert!(
        state.2.contains("120 seconds between")
            && state.2.contains("2 seconds timeout")
            && state.2.contains("3072 bytes")
    );
    checkpoint(&mut cx, window, &evidence.join("02-unit-only-draft.json"));
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    let id = rows()
        .rows
        .iter()
        .find(|r| r.spec.name == "Independent inherited limits")
        .unwrap()
        .id;
    select(&mut cx, id);
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert_eq!(state.0, vec!["", "", ""]);
    assert_eq!(state.1, vec!["minutes", "seconds", "KB"]);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("03-unit-only-reopened.json"),
    );
    click(&mut cx, window, "subs-cancel");
    flush(&mut cx);
    click(&mut cx, window, "subs-defaults");
    flush(&mut cx);
    fill(&mut cx, window, "subs-default-interval", "3");
    fill(&mut cx, window, "subs-default-timeout", "4");
    fill(&mut cx, window, "subs-default-size", "5");
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    select(&mut cx, id);
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert!(
        state.2.contains("180 seconds between")
            && state.2.contains("4 seconds timeout")
            && state.2.contains("5120 bytes")
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("04-default-counts-recompute-unit-only.json"),
    );
    click(&mut cx, window, "subs-cancel");
    flush(&mut cx);
    click(&mut cx, window, "subs-defaults");
    flush(&mut cx);
    fill(&mut cx, window, "subs-default-interval", "4");
    pick(&mut cx, window, "subs-default-interval-unit", "minutes");
    fill(&mut cx, window, "subs-default-timeout", "10");
    pick(&mut cx, window, "subs-default-timeout-unit", "seconds");
    fill(&mut cx, window, "subs-default-size", "10");
    pick(&mut cx, window, "subs-default-size-unit", "KB");
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    select(&mut cx, id);
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert!(
        state.2.contains("240 seconds between")
            && state.2.contains("10 seconds timeout")
            && state.2.contains("10240 bytes")
    );
    checkpoint(&mut cx, window, &evidence.join("05-new-default-units.json"));
    fill(&mut cx, window, "subs-interval", "3");
    fill(&mut cx, window, "subs-timeout", "1");
    fill(&mut cx, window, "subs-size", "2");
    for id in ["subs-interval-unit", "subs-timeout-unit", "subs-size-unit"] {
        pick(&mut cx, window, id, "Inherit default");
    }
    flush(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert_eq!(state.1, vec!["Inherit default"; 3]);
    assert!(
        state.2.contains("180 seconds between")
            && state.2.contains("1 seconds timeout")
            && state.2.contains("2048 bytes")
    );
    checkpoint(&mut cx, window, &evidence.join("06-count-only-draft.json"));
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    click(&mut cx, window, "subs-defaults");
    flush(&mut cx);
    fill(&mut cx, window, "subs-default-interval", "60");
    pick(&mut cx, window, "subs-default-interval-unit", "seconds");
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    assert_eq!(rows().settings.1.interval, 240);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("07-invalid-dependent-default-refused.json"),
    );
    let mut daemon = runtime.block_on(async {
        use hyper_util::rt::TokioIo;
        let socket = config.socket.clone();
        let channel = tonic::transport::Endpoint::try_from("http://localhost")
            .unwrap()
            .connect_with_connector(tower::service_fn(move |_| {
                let path = socket.clone();
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
            name: "inheritance-fixture".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        c
    });
    let reply = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/inheritance-prompt".into(),
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
        &evidence.join("08-permission-with-default-editor.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    click(&mut cx, window, "subs-cancel");
    flush(&mut cx);
    select(&mut cx, id);
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    let page = rows();
    let mut changed = page.settings.1;
    changed.interval = 3600;
    changed.units.as_mut().unwrap().interval = Unit::Hours;
    request(Request::SaveSettings {
        revision: page.settings.0,
        settings: changed,
    });
    fill(&mut cx, window, "subs-name", "Stale draft must not apply");
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    assert!(
        rows()
            .rows
            .iter()
            .any(|r| r.id == id && r.spec.name == "Independent inherited limits")
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("09-stale-defaults-editor-refused.json"),
    );
    click(&mut cx, window, "subs-cancel");
    flush(&mut cx);
    select(&mut cx, id);
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert_eq!(state.0, vec!["3", "1", "2"]);
    assert!(state.2.contains("10800 seconds between"));
    checkpoint(
        &mut cx,
        window,
        &evidence.join("10-reopened-current-defaults.json"),
    );
    click(&mut cx, window, "subs-cancel");
    flush(&mut cx);
    select(&mut cx, legacy);
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert_eq!(state.0, vec!["60", "2", "1"]);
    assert_eq!(state.1, vec!["seconds", "seconds", "KB"]);
    assert!(state.2.contains("60 seconds between") && state.2.contains("1024 bytes"));
    checkpoint(
        &mut cx,
        window,
        &evidence.join("11-legacy-absolute-preserved.json"),
    );
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    let final_rows = rows();
    assert_eq!(final_rows.total, 2);
    assert!(
        final_rows
            .rows
            .iter()
            .all(|r| r.bytes == 0 && r.checked == 0)
    );
    let old = final_rows.rows.iter().find(|r| r.id == legacy).unwrap();
    assert_eq!(
        effective(&old.spec, &final_rows.settings.1).unwrap(),
        (60, 2, 1024)
    );
    let current = final_rows.rows.iter().find(|r| r.id == id).unwrap();
    assert_eq!(current.spec.interval, Override::parts(Some(3), None));
    checkpoint(
        &mut cx,
        window,
        &evidence.join("12-saved-independent-and-legacy-limits.json"),
    );
    click(&mut cx, window, "quit");
    drop(view);
    shutdown(cx);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let names = [
        "default-count-unit-controls",
        "unit-only-draft",
        "unit-only-reopened",
        "default-counts-recompute-unit-only",
        "new-default-units",
        "count-only-draft",
        "invalid-dependent-default-refused",
        "permission-with-default-editor",
        "stale-defaults-editor-refused",
        "reopened-current-defaults",
        "legacy-absolute-preserved",
        "saved-independent-and-legacy-limits",
    ];
    let frames = names
        .iter()
        .enumerate()
        .map(|(i, n)| format!("{:02}-{n}.json", i + 1))
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":frames,"proof_boundary":"Actual Kit count/Input/unit Select/Save/Cancel controls, reopened raw inheritance, recomputed effective limits, invalid-dependent defaults rollback, stale-defaults save refusal and current reload, legacy absolute compatibility and independent real AskRule. Metadata-only fixture with paused downloads; no external HTTP or real daemon policy changes. Native unit selectors and real bounded-download behavior are separate evidence."})).unwrap()).unwrap();
}
