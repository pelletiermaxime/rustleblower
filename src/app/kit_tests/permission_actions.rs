use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let mut config = options(dir.path());
    config.prompt_timeout = 30;
    let backend = Backend::start(config.clone()).unwrap();
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
        let mut client = protocol::ui_client::UiClient::new(channel);
        client
            .subscribe(protocol::ClientConfig {
                name: "quick-allow-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        client
    });
    let mut cx = context(test_cx);
    let mut window = None;
    assert_eq!(backend.handle.preferences.snapshot().target, "process.path");
    let checkpoints = [
        "01-app-forever.json",
        "02-app-12-hours.json",
        "03-exact-hostname.json",
        "04-ip-keyboard.json",
        "05-saved-custom-rule.json",
        "06-unknown-app.json",
    ];
    for (case, checkpoint_name) in checkpoints.iter().enumerate() {
        let mut con = protocol::Connection {
            process_path: "/usr/bin/example".into(),
            process_args: vec!["example".into(), "--fixture".into()],
            dst_host: "api.example.test".into(),
            dst_ip: "192.0.2.42".into(),
            dst_port: 443,
            protocol: "tcp".into(),
            user_id: 1000,
            ..Default::default()
        };
        let mut defaults = crate::preferences::Popup {
            duration: "5m".into(),
            priority: true,
            logging: false,
            ..Default::default()
        };
        match case {
            1 => {
                defaults.target = "connection".into();
                defaults.constraints = vec!["dest.port".into()];
            }
            3 => con.dst_host.clear(),
            4 => {
                defaults.target = "process.command".into();
                defaults.constraints = vec!["user.id".into(), "dest.port".into()];
                defaults.show_advanced = true;
            }
            5 => {
                con.process_path.clear();
                con.process_args.clear();
                con.dst_host.clear();
                defaults.duration = "once".into();
            }
            _ => {}
        }
        backend.handle.preferences.save(defaults.clone()).unwrap();
        let mut asking = daemon.clone();
        let reply = runtime.spawn(async move { asking.ask_rule(con).await.unwrap().into_inner() });
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                while backend.handle.prompts.snapshot().is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        });
        let prompt = *window.get_or_insert_with(|| {
            cx.update(|cx| {
                crate::app::prompt_window::open(backend.handle.clone(), 1., false, cx).unwrap()
            })
        });
        flush(&mut cx);
        if case == 1 || case == 2 {
            cx.update_window(prompt, |_, window, cx| {
                window
                    .within("prompt-scope")
                    .click(if case == 1 { 0 } else { 1 }, cx);
            })
            .unwrap();
            flush(&mut cx);
        } else if case == 3 {
            // Both scope choices are actual Kit keyboard controls.
            cx.update_window(prompt, |_, window, cx| window.press("tab", cx))
                .unwrap();
            flush(&mut cx);
            cx.update_window(prompt, |_, window, cx| window.press("tab", cx))
                .unwrap();
            flush(&mut cx);
            cx.update_window(prompt, |_, window, cx| {
                window.press("space", cx);
            })
            .unwrap();
            flush(&mut cx);
        } else if case == 5 {
            cx.update_window(prompt, |_, window, cx| {
                window.within("prompt-scope").click(0, cx);
            })
            .unwrap();
        }
        checkpoint(&mut cx, prompt, &evidence.join(checkpoint_name));
        let (button, duration, action) = match case {
            0 | 2 => ("allow-forever", "always", "allow"),
            5 => ("deny-once", "once", "deny"),
            _ => ("allow-12h", "12h", "allow"),
        };
        click(&mut cx, prompt, button);
        let rule = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), reply)
                .await
                .unwrap()
                .unwrap()
        });
        assert_eq!(rule.action, action);
        assert_eq!(rule.duration, duration);
        assert!(rule.precedence && rule.nolog);
        let op = rule.operator.unwrap();
        let conditions = if op.r#type == "list" {
            op.list
        } else {
            vec![op]
        };
        let fields = conditions
            .iter()
            .map(|op| op.operand.as_str())
            .collect::<Vec<_>>();
        if case < 5 {
            assert!(
                conditions
                    .iter()
                    .any(|op| op.operand == "process.path" && op.data == "/usr/bin/example")
            );
        }
        match case {
            0 | 1 => assert_eq!(
                fields,
                vec!["process.path"],
                "simple app scope must remove hidden destination and port conditions"
            ),
            2 => {
                assert_eq!(fields, vec!["process.path", "dest.host"]);
                assert_eq!(conditions[1].r#type, "simple");
                assert_eq!(conditions[1].data, "api.example.test");
            }
            3 => {
                assert_eq!(fields, vec!["process.path", "dest.ip"]);
                assert_eq!(conditions[1].data, "192.0.2.42");
            }
            4 => {
                for field in ["process.path", "process.command", "user.id", "dest.port"] {
                    assert!(
                        fields.contains(&field),
                        "saved custom condition {field} must survive"
                    );
                }
            }
            5 => assert!(!fields.contains(&"process.path")),
            _ => unreachable!(),
        }
        assert_eq!(
            backend.handle.preferences.snapshot(),
            defaults,
            "scope choices do not overwrite saved defaults"
        );
        flush(&mut cx);
    }
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let db = rusqlite::Connection::open(&config.database).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM events WHERE reason='decision'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        6
    );
    drop(db);
    cx.update_window(window.unwrap(), |_, window, _| window.remove_window())
        .unwrap();
    shutdown(cx);
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("actions.json"), serde_json::to_vec_pretty(&[
        "Default whole-app Allow forever; choose whole-app scope to clear saved address/port restrictions and Allow for 12 hours",
        "Pointer exact-host scope and keyboard IP scope: require executable + exact destination, no port or wildcard",
        "Preserve saved custom conditions under Advanced; unknown executable disables both simple choices",
        "Require actual gRPC durations, metadata, unchanged preferences, six SQLite decisions and owned cleanup"
    ]).unwrap()).unwrap();
    std::fs::write(evidence.join("headless.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "status": "passed", "checkpoints": checkpoints, "framework": "gpui-kit", "real_gpu": false, "scoped_socket_removed": true,
        "proof_boundary": "Production Kit scope radio pointer/keyboard input, actual isolated gRPC replies and persisted decisions. No system daemon changes."
    })).unwrap()).unwrap();
}
