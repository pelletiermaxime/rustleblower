use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::rules::{checksums, transfer};
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let con = protocol::Connection {
        process_path: "/usr/bin/checksum-review-fixture".into(),
        dst_ip: "192.0.2.42".into(),
        dst_port: 443,
        protocol: "tcp".into(),
        process_checksums: [
            ("process.hash.md5".into(), "a".repeat(32)),
            ("process.hash.sha1".into(), "b".repeat(40)),
        ]
        .into(),
        ..Default::default()
    };
    let hash_context = checksums::Context::from_connection(&con).unwrap();
    let originals: Vec<_> = (0..35)
        .map(|index| protocol::Rule {
            name: format!("checksum.fixture.{index:03}"),
            created: 123456,
            description: "Preserve all metadata".into(),
            enabled: true,
            precedence: true,
            nolog: true,
            action: "reject".into(),
            duration: "5m".into(),
            operator: Some(protocol::Operator {
                r#type: "list".into(),
                operand: "list".into(),
                sensitive: true,
                list: vec![
                    protocol::Operator {
                        r#type: "simple".into(),
                        operand: "process.path".into(),
                        data: con.process_path.clone(),
                        sensitive: true,
                        ..Default::default()
                    },
                    protocol::Operator {
                        r#type: "simple".into(),
                        operand: "process.hash.md5".into(),
                        data: "c".repeat(32),
                        ..Default::default()
                    },
                    protocol::Operator {
                        r#type: "simple".into(),
                        operand: "process.hash.sha1".into(),
                        data: "d".repeat(40),
                        ..Default::default()
                    },
                    protocol::Operator {
                        r#type: "regexp".into(),
                        operand: "dest.host".into(),
                        data: "^updates\\.example\\.test$".into(),
                        sensitive: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
        })
        .collect();
    let daemon_rules = Arc::new(Mutex::new(
        originals
            .iter()
            .map(|rule| (rule.name.clone(), rule.clone()))
            .collect::<std::collections::BTreeMap<_, _>>(),
    ));
    let observed = Arc::new(Mutex::new(Vec::<protocol::Notification>::new()));
    let hold = Arc::new(AtomicBool::new(false));
    let accept = Arc::new(AtomicBool::new(true));
    let (daemon, notifications) = runtime.block_on(async {
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
                name: "checksum-fixture".into(),
                rules: originals.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let mut stream = client
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        let observed = observed.clone();
        let hold = hold.clone();
        let accept = accept.clone();
        let daemon_rules = daemon_rules.clone();
        let task = tokio::spawn(async move {
            while let Ok(Some(command)) = stream.message().await {
                observed.lock().unwrap().push(command.clone());
                while hold.load(Ordering::Relaxed) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let ok = accept.load(Ordering::Relaxed);
                if ok {
                    for rule in &command.rules {
                        daemon_rules
                            .lock()
                            .unwrap()
                            .insert(rule.name.clone(), rule.clone());
                    }
                }
                if tx
                    .send(protocol::NotificationReply {
                        id: command.id,
                        code: if ok {
                            protocol::NotificationReplyCode::Ok
                        } else {
                            protocol::NotificationReplyCode::Error
                        } as i32,
                        data: if ok {
                            String::new()
                        } else {
                            "Synthetic checksum failure".into()
                        },
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        (client, task)
    });
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut asking = daemon.clone();
    let connection = con.clone();
    let reply =
        runtime.spawn(async move { asking.ask_rule(connection).await.unwrap().into_inner() });
    runtime.block_on(async {
        while backend.handle.prompts.snapshot().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    let mut cx = context(test_cx);
    // Keep the configured family used on release runners; Kit uses test text metrics.
    cx.update(|cx| Theme::global_mut(cx).font_family = "DejaVu Sans".into());
    let prompt = cx.update(|cx| {
        crate::app::prompt_window::open(backend.handle.clone(), 1., false, cx).unwrap()
    });
    flush(&mut cx);
    checkpoint(&mut cx, prompt, &evidence.join("01-checksum-warning.json"));
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, prompt, "prompt-checksums");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("checksum-after-first-click.json"),
    );
    let review = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|window| !before.contains(window))
            .unwrap()
    });
    checkpoint(&mut cx, review, &evidence.join("02-checksum-review.json"));
    wait_window(&mut cx, review, |window, cx| {
        crate::app::prompt_checksums::test_idle(window, cx)
    });
    click(&mut cx, review, "checksum-next");
    wait_window(&mut cx, review, |window, cx| {
        crate::app::prompt_checksums::test_idle(window, cx)
    });
    checkpoint(
        &mut cx,
        review,
        &evidence.join("03-checksum-next-page.json"),
    );
    click(&mut cx, review, "checksum-reload");
    wait_window(&mut cx, review, |window, cx| {
        crate::app::prompt_checksums::test_idle(window, cx)
    });
    click(&mut cx, review, "checksum-update-selected");
    checkpoint(&mut cx, review, &evidence.join("04-checksum-confirm.json"));
    click(&mut cx, review, "checksum-cancel");
    assert!(observed.lock().unwrap().is_empty());
    hold.store(true, Ordering::Relaxed);
    click(&mut cx, review, "checksum-update-selected");
    click(&mut cx, review, "checksum-confirm");
    wait_for(&mut cx, |_| observed.lock().unwrap().len() == 1);
    assert_eq!(observed.lock().unwrap().len(), 1);
    let saved =
        transfer::find_confirmed_rule(&config.database, "checksum-fixture", &originals[0].name)
            .unwrap()
            .unwrap();
    assert_eq!(
        saved, originals[0],
        "no optimistic cache update before daemon ACK"
    );
    checkpoint(
        &mut cx,
        review,
        &evidence.join("05-checksum-awaiting-ack.json"),
    );
    let started = Instant::now();
    click(&mut cx, prompt, "allow-forever");
    let decision = runtime.block_on(reply).unwrap();
    assert_eq!(
        (decision.action.as_str(), decision.duration.as_str()),
        ("allow", "always")
    );
    assert!(started.elapsed() < Duration::from_millis(500));
    hold.store(false, Ordering::Relaxed);
    wait_for(&mut cx, |_cx| {
        checksums::page(&config.database, "checksum-fixture", &hash_context, "")
            .unwrap()
            .changes
            .first()
            .is_some_and(|change| change.rule.name == originals[1].name)
    });
    flush(&mut cx);
    assert!(
        checksums::differences(
            &transfer::find_confirmed_rule(
                &config.database,
                "checksum-fixture",
                &originals[0].name
            )
            .unwrap()
            .unwrap(),
            &hash_context
        )
        .is_empty()
    );
    wait_window(&mut cx, review, |window, cx| {
        crate::app::prompt_checksums::test_idle(window, cx)
    });
    hold.store(true, Ordering::Relaxed);
    click(&mut cx, review, "checksum-update-all");
    click(&mut cx, review, "checksum-confirm");
    wait_for(&mut cx, |_| observed.lock().unwrap().len() == 2);
    assert_eq!(observed.lock().unwrap().len(), 2);
    click(&mut cx, review, "checksum-stop");
    hold.store(false, Ordering::Relaxed);
    wait_window(&mut cx, review, |window, cx| {
        crate::app::prompt_checksums::test_idle(window, cx)
    });
    assert_eq!(
        observed.lock().unwrap().len(),
        2,
        "Stop must prevent unsent updates"
    );
    checkpoint(&mut cx, review, &evidence.join("06-checksum-stopped.json"));
    accept.store(false, Ordering::Relaxed);
    click(&mut cx, review, "checksum-update-selected");
    click(&mut cx, review, "checksum-confirm");
    wait_window(&mut cx, review, |window, cx| {
        crate::app::prompt_checksums::test_idle(window, cx)
    });
    assert_eq!(observed.lock().unwrap().len(), 3);
    assert!(
        backend
            .handle
            .history
            .health
            .catalogues
            .uncertain(backend.handle.status().session.unwrap().id)
    );
    assert_eq!(
        transfer::find_confirmed_rule(&config.database, "checksum-fixture", &originals[2].name)
            .unwrap()
            .unwrap(),
        originals[2]
    );
    checkpoint(&mut cx, review, &evidence.join("07-checksum-error.json"));
    click(&mut cx, review, "checksum-close");
    notifications.abort();
    accept.store(true, Ordering::Relaxed);
    let (daemon, notifications) = runtime.block_on(async {
        let mut client = daemon.clone();
        let rules = daemon_rules.lock().unwrap().values().cloned().collect();
        client
            .subscribe(protocol::ClientConfig {
                name: "checksum-fixture".into(),
                rules,
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let mut stream = client
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        let observed = observed.clone();
        let daemon_rules = daemon_rules.clone();
        let task = tokio::spawn(async move {
            while let Ok(Some(command)) = stream.message().await {
                observed.lock().unwrap().push(command.clone());
                for rule in &command.rules {
                    daemon_rules
                        .lock()
                        .unwrap()
                        .insert(rule.name.clone(), rule.clone());
                }
                if tx
                    .send(protocol::NotificationReply {
                        id: command.id,
                        code: protocol::NotificationReplyCode::Ok as i32,
                        ..Default::default()
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        (client, task)
    });
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut asking = daemon.clone();
    let reply = runtime.spawn(async move { asking.ask_rule(con).await.unwrap().into_inner() });
    flush(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, prompt, "prompt-checksums");
    flush(&mut cx);
    let review = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|window| !before.contains(window))
            .unwrap()
    });
    wait_window(&mut cx, review, |window, cx| {
        crate::app::prompt_checksums::test_idle(window, cx)
    });
    click(&mut cx, review, "checksum-update-all");
    click(&mut cx, review, "checksum-confirm");
    wait_for(&mut cx, |_cx| {
        checksums::page(&config.database, "checksum-fixture", &hash_context, "")
            .unwrap()
            .changes
            .is_empty()
    });
    flush(&mut cx);
    assert!(
        checksums::page(&config.database, "checksum-fixture", &hash_context, "")
            .unwrap()
            .changes
            .is_empty()
    );
    assert_eq!(observed.lock().unwrap().len(), originals.len() + 1);
    for original in &originals {
        let saved =
            transfer::find_confirmed_rule(&config.database, "checksum-fixture", &original.name)
                .unwrap()
                .unwrap();
        assert_eq!(saved, checksums::updated(original, &hash_context).unwrap());
    }
    checkpoint(
        &mut cx,
        review,
        &evidence.join("08-checksum-all-confirmed.json"),
    );
    checkpoint(&mut cx, prompt, &evidence.join("09-warning-cleared.json"));
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    click(&mut cx, review, "checksum-close");
    cx.update_window(prompt, |_, window, _| window.remove_window())
        .unwrap();
    shutdown(cx);
    notifications.abort();
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","checkpoints":["01-checksum-warning.json","02-checksum-review.json","03-checksum-next-page.json","04-checksum-confirm.json","05-checksum-awaiting-ack.json","06-checksum-stopped.json","07-checksum-error.json","08-checksum-all-confirmed.json","09-warning-cleared.json"],"framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"proof_boundary":"Actual GPUI checksum warning/review input, paged reads, confirmed rule changes over isolated gRPC, failure/stop/reconnect/all-page workflow and permission reply while command ACK is held; real daemon enforcement and native window input remain separate"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Open the reported-hash warning, inspect MD5/SHA1 and next page, cancel then confirm selected update", "Hold daemon ACK; require unchanged cache and immediate independent Allow forever; release ACK, stop all after one command, and reject the next update with uncertainty feedback", "Reconnect a fresh daemon inventory, reopen the review, update all remaining 33 rules across pages, require preserved metadata/conditions and cleared warning, then Deny once"]).unwrap()).unwrap();
}
