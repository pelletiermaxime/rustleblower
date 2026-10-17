use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    fn done(cx: &mut TestAppContext, v: &gpui::Entity<Dashboard>) {
        wait_for(cx, |cx| {
            !cx.read_entity(v, |v, _| v.subscriptions.url_test_state().0)
        });
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
    let server = runtime.block_on(url_probe_fixture::Server::start());
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
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "url-probe-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    let mut cx = context(test_cx);
    let mut view = None;
    let main = cx
        .open_test_window(size(px(1200.), px(1100.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let view = view.unwrap();
    let main: AnyWindowHandle = main.into();
    flush(&mut cx);
    cx.update_window(main, |_, w, cx| w.press("ctrl-7", cx))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, main, "subs-add");
    flush(&mut cx);
    click(&mut cx, main, "subs-test-url");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().2)
    });

    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().2));
    assert!(server.requests.lock().unwrap().is_empty());
    checkpoint(&mut cx, main, &evidence.join("01-empty-url-refused.json"));
    fill(
        &mut cx,
        main,
        "subs-url",
        &format!("{}/redirect", server.url),
    );
    click(&mut cx, main, "subs-test-url");
    done(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| {
        v.subscriptions
            .url_test_state()
            .1
            .is_some_and(|m| m.contains("via redirect") && m.contains("resembles hosts"))
    }));
    checkpoint(
        &mut cx,
        main,
        &evidence.join("02-redirect-hosts-sample.json"),
    );
    fill(&mut cx, main, "subs-url", &format!("{}/bad", server.url));
    click(&mut cx, main, "subs-test-url");
    done(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().2));
    checkpoint(
        &mut cx,
        main,
        &evidence.join("03-invalid-hosts-sample.json"),
    );
    pick(&mut cx, main, "subs-format", "Domain names / patterns");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().1.is_none())
    });

    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().1.is_none()));
    let request_count = server.requests.lock().unwrap().len();
    click(&mut cx, main, "subs-test-url");
    done(&mut cx, &view);
    assert!(!cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().2));
    assert_eq!(
        &server.requests.lock().unwrap()[request_count..],
        &["HEAD /bad"]
    );
    checkpoint(&mut cx, main, &evidence.join("04-non-hosts-head-only.json"));
    pick(&mut cx, main, "subs-format", "Hosts file");
    fill(&mut cx, main, "subs-url", &format!("{}/slow", server.url));
    click(&mut cx, main, "subs-test-url");
    flush(&mut cx);
    wait_for(&mut cx, |_| server.held.load(Ordering::Acquire));
    assert!(server.held.load(Ordering::Acquire));
    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().0));
    checkpoint(&mut cx, main, &evidence.join("05-held-url-test.json"));
    let reply = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/url-probe-permission".into(),
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
        &evidence.join("06-permission-during-url-test.json"),
    );
    click(&mut cx, popup, "deny-once");
    let rule = runtime.block_on(reply).unwrap();
    assert_eq!(rule.action, "deny");
    assert_eq!(rule.duration, "once");
    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().0));
    click(&mut cx, main, "subs-test-cancel");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            !v.subscriptions.url_test_state().0
                && v.subscriptions.url_test_state().1 == Some("URL test cancelled")
        })
    });

    assert!(
        cx.read_entity(&view, |v, _| !v.subscriptions.url_test_state().0
            && v.subscriptions.url_test_state().1 == Some("URL test cancelled"))
    );
    checkpoint(&mut cx, main, &evidence.join("07-cancel-url-test.json"));
    click(&mut cx, main, "subs-test-url");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().0)
    });

    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().0));
    fill(&mut cx, main, "subs-url", &format!("{}/hosts", server.url));
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            !v.subscriptions.url_test_state().0 && v.subscriptions.url_test_state().1.is_none()
        })
    });

    assert!(
        cx.read_entity(&view, |v, _| !v.subscriptions.url_test_state().0
            && v.subscriptions.url_test_state().1.is_none())
    );
    click(&mut cx, main, "subs-test-url");
    done(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| {
        !v.subscriptions.url_test_state().2
            && v.subscriptions
                .url_test_state()
                .1
                .is_some_and(|m| m.contains("resembles hosts"))
    }));
    checkpoint(
        &mut cx,
        main,
        &evidence.join("08-edited-url-discards-old-result.json"),
    );
    fill(&mut cx, main, "subs-url", &format!("{}/slow", server.url));
    click(&mut cx, main, "subs-test-url");
    flush(&mut cx);
    click(&mut cx, main, "subs-cancel");
    flush(&mut cx);
    click(&mut cx, main, "subs-add");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            !v.subscriptions.url_test_state().0 && v.subscriptions.url_test_state().1.is_none()
        })
    });

    assert!(
        cx.read_entity(&view, |v, _| !v.subscriptions.url_test_state().0
            && v.subscriptions.url_test_state().1.is_none())
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("09-cancel-form-drops-test.json"),
    );
    fill(&mut cx, main, "subs-url", &format!("{}/slow", server.url));
    click(&mut cx, main, "subs-test-url");
    flush(&mut cx);
    cx.update_window(main, |_, w, cx| w.press("ctrl-1", cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(main, |_, w, cx| w.press("ctrl-7", cx))
        .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            !v.subscriptions.url_test_state().0
                && v.subscriptions.url_test_state().1 == Some("URL test cancelled")
        })
    });

    assert!(
        cx.read_entity(&view, |v, _| !v.subscriptions.url_test_state().0
            && v.subscriptions.url_test_state().1 == Some("URL test cancelled"))
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("10-leave-page-cancels-test.json"),
    );
    let db = rusqlite::Connection::open(
        backend
            .handle
            .subscriptions
            .root
            .join("subscriptions.sqlite3"),
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM subscriptions", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM logs", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        0
    );
    fill(&mut cx, main, "subs-url", &format!("{}/hosts", server.url));
    click(&mut cx, main, "subs-test-url");
    done(&mut cx, &view);
    fill(&mut cx, main, "subs-name", "Explicitly saved after test");
    click(&mut cx, main, "subs-save");
    flush(&mut cx);
    assert_eq!(
        db.query_row("SELECT count(*) FROM subscriptions", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT sum(bytes) FROM subscriptions", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    checkpoint(&mut cx, main, &evidence.join("11-explicit-save-only.json"));
    server.release();
    drop(db);
    click(&mut cx, main, "quit");
    drop(view);
    shutdown(cx);
    drop(server);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let names = [
        "empty-url-refused",
        "redirect-hosts-sample",
        "invalid-hosts-sample",
        "non-hosts-head-only",
        "held-url-test",
        "permission-during-url-test",
        "cancel-url-test",
        "edited-url-discards-old-result",
        "cancel-form-drops-test",
        "leave-page-cancels-test",
        "explicit-save-only",
    ];
    let frames = names
        .iter()
        .enumerate()
        .map(|(i, n)| format!("{:02}-{n}.json", i + 1))
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":frames,"proof_boundary":"Owned loopback HTTP and actual Kit URL/format/test/cancel/form/save/navigation controls; redirect and bounded hosts sample, non-host HEAD-only, stale-input result refusal, independent real AskRule/Deny once while GET is held, no subscription/log/data writes before explicit Save. Native compositor input, actual external endpoints and certificate/compression variants remain separate."})).unwrap()).unwrap();
}
