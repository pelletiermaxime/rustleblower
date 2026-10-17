use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::rules::{distribute::Status, transfer};
    type Client = protocol::ui_client::UiClient<tonic::transport::Channel>;
    fn rule(name: &str) -> protocol::Rule {
        protocol::Rule {
            name: name.into(),
            created: 321,
            description: "Owned original description".into(),
            enabled: true,
            action: "deny".into(),
            duration: "always".into(),
            operator: Some(protocol::Operator {
                r#type: "simple".into(),
                operand: "process.path".into(),
                data: "/owned/draft-app".into(),
                sensitive: true,
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    async fn connect(path: PathBuf, name: &str) -> Client {
        use hyper_util::rt::TokioIo;
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
        let mut daemon = Client::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: name.into(),
                config: "{\"Future\":true}".into(),
                rules: vec![rule("draft-old")],
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    }
    async fn responder(
        client: &mut Client,
        hold: Arc<AtomicBool>,
        observed: Arc<Mutex<Vec<protocol::Notification>>>,
    ) -> tokio::task::JoinHandle<()> {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let mut stream = client
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        tokio::spawn(async move {
            while let Ok(Some(command)) = stream.message().await {
                let id = command.id;
                {
                    let mut events = observed.lock().unwrap();
                    assert!(events.len() < 32);
                    events.push(command);
                }
                while hold.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                if tx
                    .send(protocol::NotificationReply {
                        id,
                        code: protocol::NotificationReplyCode::Ok as i32,
                        ..Default::default()
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        })
    }
    fn state(cx: &mut impl AppContext, window: AnyWindowHandle) -> (bool, usize, Vec<Status>) {
        cx.update_window(window, |_, window, cx| {
            gpui_kit::base::Root::read(window, cx)
                .view()
                .clone()
                .downcast::<crate::app::rule_distribution::Copies>()
                .unwrap()
                .read(cx)
                .test_state()
        })
        .unwrap()
    }
    fn finish(cx: &mut TestAppContext, window: AnyWindowHandle) {
        wait_for(cx, |cx| !state(cx, window).0);
    }
    fn opened(cx: &mut TestAppContext, before: &[AnyWindowHandle]) -> AnyWindowHandle {
        cx.update(|cx| {
            cx.windows()
                .into_iter()
                .find(|w| !before.contains(w))
                .unwrap()
        })
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
    let hold = Arc::new(AtomicBool::new(true));
    let ao = Arc::new(Mutex::new(Vec::new()));
    let bo = Arc::new(Mutex::new(Vec::new()));
    let (alpha, beta, at, bt) = runtime.block_on(async {
        let mut a = connect(config.socket.clone(), "alpha").await;
        let mut b = connect(config.socket.clone(), "beta").await;
        let at = responder(&mut a, hold.clone(), ao.clone()).await;
        let bt = responder(&mut b, Arc::new(AtomicBool::new(false)), bo.clone()).await;
        backend.handle.history.flush().await.unwrap();
        (a, b, at, bt)
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
    flush(&mut cx);
    cx.update_window(main, |_, window, cx| window.press("ctrl-2", cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(main, |_, window, cx| window.click(("rule-row", 0usize), cx))
        .unwrap();
    click(&mut cx, main, "rule-edit");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    fill(&mut cx, editor, "editor-name", "draft-renamed");
    fill(
        &mut cx,
        editor,
        "editor-description",
        "Edited before multi-node review",
    );
    click(&mut cx, editor, "editor-priority");
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("01-unsaved-editor-draft.json"),
    );
    assert!(
        transfer::find_confirmed_rule(&config.database, "alpha", "draft-renamed")
            .unwrap()
            .is_none()
    );
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, editor, "editor-nodes");
    let copy = opened(&mut cx, &before);
    click(&mut cx, copy, "copy-select-all");
    checkpoint(
        &mut cx,
        copy,
        &evidence.join("02-draft-node-selection.json"),
    );
    click(&mut cx, copy, "copy-close");
    flush(&mut cx);
    assert!(ao.lock().unwrap().is_empty() && bo.lock().unwrap().is_empty());
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("03-cancel-preserves-editor-draft.json"),
    );
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, editor, "editor-nodes");
    let copy = opened(&mut cx, &before);
    click(&mut cx, copy, "copy-select-all");
    click(&mut cx, copy, "copy-review-button");
    finish(&mut cx, copy);
    assert_eq!(state(&mut cx, copy).1, 1);
    checkpoint(
        &mut cx,
        copy,
        &evidence.join("04-two-node-rename-review.json"),
    );
    click(&mut cx, copy, "copy-apply");
    flush(&mut cx);
    assert_eq!(ao.lock().unwrap().len(), 1);
    assert_eq!(
        ao.lock().unwrap()[0].r#type,
        protocol::Action::ChangeRule as i32
    );
    assert!(
        transfer::find_confirmed_rule(&config.database, "alpha", "draft-renamed")
            .unwrap()
            .is_none()
    );
    checkpoint(&mut cx, copy, &evidence.join("05-new-name-awaits-ack.json"));
    let mut asking = beta.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/rename-independent".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    let prompt = permission_window(&mut cx, &view);
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("06-permission-during-rename.json"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    click(&mut cx, copy, "copy-stop");
    hold.store(false, Ordering::Release);
    finish(&mut cx, copy);
    assert_eq!(state(&mut cx, copy).2[0], Status::Stopped);
    assert!(
        transfer::find_confirmed_rule(&config.database, "alpha", "draft-old")
            .unwrap()
            .is_some()
    );
    assert!(
        transfer::find_confirmed_rule(&config.database, "alpha", "draft-renamed")
            .unwrap()
            .is_some()
    );
    assert!(bo.lock().unwrap().is_empty());
    checkpoint(
        &mut cx,
        copy,
        &evidence.join("07-stopped-before-old-name-removal.json"),
    );
    click(&mut cx, copy, "copy-apply");
    finish(&mut cx, copy);
    assert!(
        state(&mut cx, copy)
            .2
            .iter()
            .all(|s| *s == Status::Complete)
    );
    for node in ["alpha", "beta"] {
        assert!(
            transfer::find_confirmed_rule(&config.database, node, "draft-old")
                .unwrap()
                .is_none()
        );
        let saved = transfer::find_confirmed_rule(&config.database, node, "draft-renamed")
            .unwrap()
            .unwrap();
        assert_eq!(saved.description, "Edited before multi-node review");
        assert!(saved.precedence);
    }
    assert_eq!(
        ao.lock()
            .unwrap()
            .iter()
            .map(|n| n.r#type)
            .collect::<Vec<_>>(),
        vec![10, 9]
    );
    assert_eq!(
        bo.lock()
            .unwrap()
            .iter()
            .map(|n| n.r#type)
            .collect::<Vec<_>>(),
        vec![10, 9]
    );
    click(&mut cx, copy, "copy-results");
    let results = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(
        results
            .lines()
            .all(|line| line.contains("1 originals removed"))
    );
    checkpoint(
        &mut cx,
        copy,
        &evidence.join("08-rename-completed-on-both-nodes.json"),
    );
    click(&mut cx, copy, "copy-close");
    flush(&mut cx);
    click(&mut cx, editor, "editor-cancel");
    flush(&mut cx);
    let input = evidence.join("owned-import.json");
    std::fs::write(&input, "not json").unwrap();
    click(&mut cx, main, "rules-import-nodes");
    let import = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    fill(&mut cx, import, "copy-import-path", input.to_str().unwrap());
    click(&mut cx, import, "copy-select-all");
    click(&mut cx, import, "copy-review-button");
    finish(&mut cx, import);
    assert_eq!(state(&mut cx, import).1, 0);
    assert_eq!(ao.lock().unwrap().len(), 2);
    checkpoint(
        &mut cx,
        import,
        &evidence.join("09-invalid-multi-node-import.json"),
    );
    let imported = [rule("import-one"), rule("import-two")];
    std::fs::write(
        &input,
        serde_json::to_vec(
            &imported
                .iter()
                .map(crate::rules::document::rule_json)
                .collect::<Vec<_>>(),
        )
        .unwrap(),
    )
    .unwrap();
    click(&mut cx, import, "copy-review-button");
    finish(&mut cx, import);
    assert_eq!(state(&mut cx, import).1, 2);
    checkpoint(
        &mut cx,
        import,
        &evidence.join("10-file-import-review.json"),
    );
    std::fs::write(
        &input,
        serde_json::to_vec(&crate::rules::document::rule_json(&rule("file-changed"))).unwrap(),
    )
    .unwrap();
    click(&mut cx, import, "copy-apply");
    finish(&mut cx, import);
    for node in ["alpha", "beta"] {
        for rule in &imported {
            assert_eq!(
                transfer::find_confirmed_rule(&config.database, node, &rule.name).unwrap(),
                Some(rule.clone())
            );
        }
        assert!(
            transfer::find_confirmed_rule(&config.database, node, "file-changed")
                .unwrap()
                .is_none()
        );
    }
    checkpoint(
        &mut cx,
        import,
        &evidence.join("11-captured-file-import-applied.json"),
    );
    click(&mut cx, import, "copy-back");
    cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("not JSON".to_string())));
    click(&mut cx, import, "copy-import-clipboard");
    click(&mut cx, import, "copy-review-button");
    finish(&mut cx, import);
    assert_eq!(state(&mut cx, import).1, 0);
    assert_eq!(ao.lock().unwrap().len(), 4);
    checkpoint(
        &mut cx,
        import,
        &evidence.join("12-invalid-clipboard-has-no-file-fallback.json"),
    );
    let captured = rule("clipboard-captured");
    cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            serde_json::to_string(&crate::rules::document::rule_json(&captured)).unwrap(),
        ))
    });
    click(&mut cx, import, "copy-import-clipboard");
    click(&mut cx, import, "copy-review-button");
    finish(&mut cx, import);
    assert_eq!(state(&mut cx, import).1, 1);
    checkpoint(
        &mut cx,
        import,
        &evidence.join("13-captured-clipboard-review.json"),
    );
    cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            serde_json::to_string(&crate::rules::document::rule_json(&rule(
                "clipboard-changed",
            )))
            .unwrap(),
        ))
    });
    click(&mut cx, import, "copy-apply");
    finish(&mut cx, import);
    for node in ["alpha", "beta"] {
        assert_eq!(
            transfer::find_confirmed_rule(&config.database, node, &captured.name).unwrap(),
            Some(captured.clone())
        );
        assert!(
            transfer::find_confirmed_rule(&config.database, node, "clipboard-changed")
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(ao.lock().unwrap().len(), 5);
    assert_eq!(bo.lock().unwrap().len(), 5);
    checkpoint(
        &mut cx,
        import,
        &evidence.join("14-captured-clipboard-applied.json"),
    );
    click(&mut cx, import, "copy-close");
    drop(view);
    shutdown(cx);
    at.abort();
    bt.abort();
    drop(alpha);
    drop(beta);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let frames = [
        "unsaved-editor-draft",
        "draft-node-selection",
        "cancel-preserves-editor-draft",
        "two-node-rename-review",
        "new-name-awaits-ack",
        "permission-during-rename",
        "stopped-before-old-name-removal",
        "rename-completed-on-both-nodes",
        "invalid-multi-node-import",
        "file-import-review",
        "captured-file-import-applied",
        "invalid-clipboard-has-no-file-fallback",
        "captured-clipboard-review",
        "captured-clipboard-applied",
    ]
    .iter()
    .enumerate()
    .map(|(i, name)| format!("{:02}-{name}.json", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":frames,"proof_boundary":"Actual editor draft/name/metadata and parent-preserving Cancel, destination selection, rename review, held ACK/permission/Stop/continue, durable per-node removal, typed file import and captured clipboard input. Owned fixture writes change file/clipboard after review to prove immutable source; two synthetic gRPC daemons only. No real daemon, native compositor or kernel enforcement proof."})).unwrap()).unwrap();
}
