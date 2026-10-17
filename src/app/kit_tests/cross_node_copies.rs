use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::rules::{
        distribute::Status,
        transfer::{self, ExportScope},
    };
    use std::collections::BTreeMap;
    type Client = protocol::ui_client::UiClient<tonic::transport::Channel>;
    type Observed = Arc<Mutex<BTreeMap<String, Vec<protocol::Notification>>>>;
    fn rule(i: usize) -> protocol::Rule {
        protocol::Rule {
            name: format!("copy-ui-{i:03}"),
            created: 321,
            enabled: true,
            action: "deny".into(),
            duration: "always".into(),
            operator: Some(protocol::Operator {
                r#type: "list".into(),
                operand: "list".into(),
                list: vec![
                    protocol::Operator {
                        r#type: "simple".into(),
                        operand: "process.path".into(),
                        data: format!("/owned/copy-ui-{i}"),
                        sensitive: true,
                        ..Default::default()
                    },
                    protocol::Operator {
                        r#type: "simple".into(),
                        operand: "user.id".into(),
                        data: "1001".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    async fn connect(path: PathBuf, name: &str, rules: Vec<protocol::Rule>) -> Client {
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
        let mut client = Client::new(channel);
        client
            .subscribe(protocol::ClientConfig {
                name: name.into(),
                config: "{\"Future\":true}".into(),
                rules,
                ..Default::default()
            })
            .await
            .unwrap();
        client
    }
    async fn responder(
        client: &mut Client,
        name: &str,
        hold: Arc<AtomicBool>,
        accept: Arc<AtomicBool>,
        observed: Observed,
    ) -> tokio::task::JoinHandle<()> {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let mut stream = client
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        let name = name.to_string();
        tokio::spawn(async move {
            while let Ok(Some(command)) = stream.message().await {
                let id = command.id;
                {
                    let mut all = observed.lock().unwrap();
                    let events = all.entry(name.clone()).or_default();
                    assert!(events.len() < 512);
                    events.push(command);
                }
                while hold.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let ok = accept.load(Ordering::Acquire);
                if tx
                    .send(protocol::NotificationReply {
                        id,
                        code: if ok {
                            protocol::NotificationReplyCode::Ok
                        } else {
                            protocol::NotificationReplyCode::Error
                        } as i32,
                        data: if ok { "" } else { "Owned copy rejection" }.into(),
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
    fn finished(cx: &mut TestAppContext, window: AnyWindowHandle) {
        wait_for(cx, |cx| !state(cx, window).0);
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
    let observed = Arc::new(Mutex::new(BTreeMap::new()));
    let hold = Arc::new(AtomicBool::new(true));
    let gamma_accept = Arc::new(AtomicBool::new(false));
    let originals = (0..230).map(rule).collect::<Vec<_>>();
    let mut conflict = rule(0);
    conflict.action = "allow".into();
    let (mut source, beta, mut gamma, st, bt, gt) = runtime.block_on(async {
        let mut s = connect(config.socket.clone(), "source", originals.clone()).await;
        let mut b = connect(config.socket.clone(), "beta", vec![conflict.clone()]).await;
        let mut g = connect(config.socket.clone(), "gamma", vec![]).await;
        let st = responder(
            &mut s,
            "source",
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(true)),
            observed.clone(),
        )
        .await;
        let bt = responder(
            &mut b,
            "beta",
            hold.clone(),
            Arc::new(AtomicBool::new(true)),
            observed.clone(),
        )
        .await;
        let gt = responder(
            &mut g,
            "gamma",
            Arc::new(AtomicBool::new(false)),
            gamma_accept.clone(),
            observed.clone(),
        )
        .await;
        backend.handle.history.flush().await.unwrap();
        (s, b, g, st, bt, gt)
    });
    let mut cx = context(test_cx);
    let mut view = None;
    let main = cx
        .open_test_window(size(px(1250.), px(940.)), |window, cx| {
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
    click(&mut cx, main, "rules-select-page");
    click(&mut cx, main, "rules-next");
    flush(&mut cx);
    cx.update_window(main, |_, window, cx| {
        window.click(("rule-mark", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, main, "rules-copy-nodes");
    let copy = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    checkpoint(&mut cx, copy, &evidence.join("01-copy-destinations.json"));
    click(&mut cx, copy, "copy-select-all");
    click(&mut cx, copy, "copy-review-button");
    finished(&mut cx, copy);
    assert_eq!(state(&mut cx, copy).1, 201);
    checkpoint(
        &mut cx,
        copy,
        &evidence.join("02-selected-cross-page-review.json"),
    );
    click(&mut cx, copy, "copy-rules-tab");
    cx.update_window(copy, |_, window, cx| {
        window.click(("copy-source-json", 0usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    let json = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(
        crate::rules::document::parse_document(&json).unwrap(),
        vec![rule(0)]
    );
    checkpoint(&mut cx, copy, &evidence.join("03-exact-source-json.json"));
    click(&mut cx, copy, "copy-back");
    click(&mut cx, copy, "copy-all-rules");
    click(&mut cx, copy, "copy-review-button");
    finished(&mut cx, copy);
    assert_eq!(state(&mut cx, copy).1, 230);
    click(&mut cx, copy, "copy-destinations-tab");
    checkpoint(&mut cx, copy, &evidence.join("04-full-filter-review.json"));
    assert!(observed.lock().unwrap().is_empty());
    click(&mut cx, copy, "copy-apply");
    flush(&mut cx);
    assert_eq!(observed.lock().unwrap()["beta"].len(), 1);
    assert!(
        transfer::find_confirmed_rule(&config.database, "beta", &rule(1).name)
            .unwrap()
            .is_none()
    );
    checkpoint(&mut cx, copy, &evidence.join("05-copy-awaits-ack.json"));
    let mut asking = gamma.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/copy-popup".into(),
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
        &evidence.join("06-permission-during-copy.json"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    click(&mut cx, copy, "copy-stop");
    hold.store(false, Ordering::Release);
    finished(&mut cx, copy);
    assert_eq!(state(&mut cx, copy).2[0], Status::Stopped);
    checkpoint(&mut cx, copy, &evidence.join("07-copy-stopped.json"));
    click(&mut cx, copy, "copy-apply");
    finished(&mut cx, copy);
    let statuses = state(&mut cx, copy).2;
    assert_eq!(statuses[0], Status::Complete);
    assert!(matches!(statuses[1], Status::Failed(_)));
    assert_eq!(observed.lock().unwrap()["beta"].len(), 229);
    checkpoint(
        &mut cx,
        copy,
        &evidence.join("08-partial-target-failure.json"),
    );
    click(&mut cx, copy, "copy-results");
    let results = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(
        results.contains("beta: Complete")
            && results.contains("gamma: Failed")
            && results.contains("Owned copy rejection")
    );
    click(&mut cx, copy, "copy-apply");
    finished(&mut cx, copy);
    assert_eq!(observed.lock().unwrap()["beta"].len(), 229);
    assert_eq!(observed.lock().unwrap()["gamma"].len(), 1);
    checkpoint(
        &mut cx,
        copy,
        &evidence.join("09-uncertain-retry-refused.json"),
    );
    gt.abort();
    gamma_accept.store(true, Ordering::Release);
    runtime.block_on(async {
        gamma
            .subscribe(protocol::ClientConfig {
                name: "gamma".into(),
                config: "{\"Future\":true}".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    let gt = runtime.block_on(responder(
        &mut gamma,
        "gamma",
        Arc::new(AtomicBool::new(false)),
        gamma_accept.clone(),
        observed.clone(),
    ));
    click(&mut cx, copy, "copy-back");
    click(&mut cx, copy, "copy-refresh-nodes");
    click(&mut cx, copy, "copy-review-button");
    finished(&mut cx, copy);
    checkpoint(&mut cx, copy, &evidence.join("10-fresh-target-review.json"));
    click(&mut cx, copy, "copy-apply");
    finished(&mut cx, copy);
    assert!(
        state(&mut cx, copy)
            .2
            .iter()
            .all(|s| *s == Status::Complete)
    );
    assert_eq!(observed.lock().unwrap()["beta"].len(), 229);
    assert_eq!(observed.lock().unwrap()["gamma"].len(), 231);
    checkpoint(&mut cx, copy, &evidence.join("11-targets-completed.json"));
    click(&mut cx, copy, "copy-back");
    click(&mut cx, copy, "copy-replace");
    click(&mut cx, copy, "copy-review-button");
    finished(&mut cx, copy);
    checkpoint(
        &mut cx,
        copy,
        &evidence.join("12-explicit-replacement-review.json"),
    );
    click(&mut cx, copy, "copy-apply");
    finished(&mut cx, copy);
    assert_eq!(
        transfer::find_confirmed_rule(&config.database, "beta", &rule(0).name).unwrap(),
        Some(rule(0))
    );
    checkpoint(
        &mut cx,
        copy,
        &evidence.join("13-replacement-confirmed.json"),
    );
    let actual = transfer::load_rules(
        &config.database,
        &ExportScope {
            query: crate::rules::Query {
                node: "source".into(),
                ..Default::default()
            },
            names: None,
        },
    )
    .unwrap();
    assert_eq!(actual, originals);
    assert!(!observed.lock().unwrap().contains_key("source"));
    let extended = (0..231).map(rule).collect::<Vec<_>>();
    runtime.block_on(async {
        source
            .subscribe(protocol::ClientConfig {
                name: "source".into(),
                config: "{\"Future\":true}".into(),
                rules: extended,
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    click(&mut cx, copy, "copy-back");
    click(&mut cx, copy, "copy-review-button");
    finished(&mut cx, copy);
    hold.store(true, Ordering::Release);
    click(&mut cx, copy, "copy-apply");
    flush(&mut cx);
    assert_eq!(observed.lock().unwrap()["beta"].len(), 231);
    cx.update_window(copy, |_, window, _| window.remove_window())
        .unwrap();
    flush(&mut cx);
    hold.store(false, Ordering::Release);
    flush(&mut cx);
    flush(&mut cx);
    assert!(
        transfer::find_confirmed_rule(&config.database, "beta", &rule(230).name)
            .unwrap()
            .is_some()
    );
    assert!(
        transfer::find_confirmed_rule(&config.database, "gamma", &rule(230).name)
            .unwrap()
            .is_none()
    );
    assert!(
        !backend
            .handle
            .control
            .node_busy(backend.handle.node("beta").unwrap().id)
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("14-closed-copy-stops-unsent-targets.json"),
    );
    drop(view);
    shutdown(cx);
    st.abort();
    bt.abort();
    gt.abort();
    drop(source);
    drop(beta);
    drop(gamma);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let frames = [
        "copy-destinations",
        "selected-cross-page-review",
        "exact-source-json",
        "full-filter-review",
        "copy-awaits-ack",
        "permission-during-copy",
        "copy-stopped",
        "partial-target-failure",
        "uncertain-retry-refused",
        "fresh-target-review",
        "targets-completed",
        "explicit-replacement-review",
        "replacement-confirmed",
        "closed-copy-stops-unsent-targets",
    ]
    .iter()
    .enumerate()
    .map(|(i, name)| format!("{:02}-{name}.json", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":frames,"proof_boundary":"Actual GPUI selection across pages, target checkboxes, source JSON, scope/replacement/rebuild/retry/stop/results input; exact synthetic gRPC ACK/error and SQLite persistence; independent permission and forced owned-window destruction while ACK held. Source inventories and synthetic reconnections enter at the daemon fixture boundary. No real daemon, native compositor or kernel enforcement proof."})).unwrap()).unwrap();
}
