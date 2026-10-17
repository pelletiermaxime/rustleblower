use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let accept_changes = Arc::new(AtomicBool::new(true));
    let reject_at = Arc::new(AtomicUsize::new(usize::MAX));
    let hold_reply = Arc::new(AtomicBool::new(false));
    let observed_commands = Arc::new(Mutex::new(Vec::new()));
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
        let rules = (0usize..230)
            .map(|i| protocol::Rule {
                name: format!("fixture.application.{i:03}"),
                description: format!("Synthetic application {i}"),
                enabled: true,
                action: "deny".into(),
                duration: if i.is_multiple_of(2) { "always" } else { "5m" }.into(),
                operator: Some(protocol::Operator {
                    r#type: "simple".into(),
                    operand: "process.path".into(),
                    data: format!("/usr/bin/synthetic-app-{i}"),
                    sensitive: true,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        client
            .subscribe(protocol::ClientConfig {
                name: "fixture".into(),
                rules,
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let mut stream = client
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        let accept = accept_changes.clone();
        let observed = observed_commands.clone();
        let reject = reject_at.clone();
        let hold = hold_reply.clone();
        let task = tokio::spawn(async move {
            while let Some(command) = stream.message().await.unwrap() {
                let id = command.id;
                let number = {
                    let mut records = observed.lock().unwrap();
                    assert!(records.len() < 512);
                    records.push(command);
                    records.len()
                };
                while hold.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let ok = accept.load(Ordering::Relaxed) && reject.load(Ordering::Relaxed) != number;
                tx.send(protocol::NotificationReply {
                    id,
                    code: if ok {
                        protocol::NotificationReplyCode::Ok
                    } else {
                        protocol::NotificationReplyCode::Error
                    } as i32,
                    data: if ok {
                        String::new()
                    } else {
                        "Synthetic rule rejection".into()
                    },
                })
                .await
                .unwrap();
            }
        });
        (client, task)
    });
    let events = (0..430)
        .map(|i| protocol::Event {
            unixnano: crate::history::now_nanos() + i,
            connection: Some(protocol::Connection {
                process_path: if i % 2 == 0 {
                    "/usr/bin/firefox"
                } else {
                    "/usr/bin/curl"
                }
                .into(),
                dst_host: "api.example.test".into(),
                dst_ip: "192.0.2.42".into(),
                dst_port: 443,
                protocol: "tcp".into(),
                src_ip: "192.0.2.10".into(),
                src_port: 50000,
                user_id: 1000,
                process_id: 4567,
                process_cwd: "/tmp/synthetic working directory".into(),
                process_args: vec![
                    if i % 2 == 0 {
                        "/usr/bin/firefox"
                    } else {
                        "/usr/bin/curl"
                    }
                    .into(),
                    "--profile".into(),
                    "profile with spaces".into(),
                ],
                ..Default::default()
            }),
            rule: Some(protocol::Rule {
                action: if i % 2 == 0 { "deny" } else { "allow" }.into(),
                name: "fixture.application.001".into(),
                duration: "always".into(),
                enabled: true,
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    backend
        .handle
        .history
        .ingest("fixture".into(), events, "observed");
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context(test_cx);
    cx.update(|cx| crate::app::appearance::init(&config.database, cx));
    let mut view = None;
    let window = cx
        .open_test_window(size(px(1100.), px(780.)), |window, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config, window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    let view = view.unwrap();
    cx.run_until_parked();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.history.page.rows.len())) == (200)
    });

    assert_eq!(cx.read_entity(&view, |v, _| v.history.page.rows.len()), 200);
    checkpoint(&mut cx, window.into(), &evidence.join("01-history.json"));
    click(&mut cx, window.into(), "older");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.history.query.cursor.is_some())
    });

    assert!(cx.read_entity(&view, |v, _| v.history.query.cursor.is_some()));
    assert_eq!(cx.read_entity(&view, |v, _| v.history.page.rows.len()), 200);
    checkpoint(&mut cx, window.into(), &evidence.join("02-older.json"));
    click(&mut cx, window.into(), "latest");
    flush(&mut cx);
    click(&mut cx, window.into(), "deny");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.history.page.rows.iter().all(|r| r.action == "deny")
        })
    });

    assert!(cx.read_entity(&view, |v, _| {
        v.history.page.rows.iter().all(|r| r.action == "deny")
    }));
    cx.update_window(window.into(), |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("does-not-exist", cx);
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.history.query.search.clone())) == ("does-not-exist")
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.search.clone()),
        "does-not-exist"
    );
    assert!(cx.read_entity(&view, |v, _| v.history.page.rows.is_empty()));
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("03-empty-search.json"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("firefox", cx);
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            !v.history.page.rows.is_empty()
                && v.history
                    .page
                    .rows
                    .iter()
                    .all(|r| r.process.contains("firefox") && r.action == "deny")
        })
    });

    assert!(cx.read_entity(&view, |v, _| {
        !v.history.page.rows.is_empty()
            && v.history
                .page
                .rows
                .iter()
                .all(|r| r.process.contains("firefox") && r.action == "deny")
    }));
    checkpoint(&mut cx, window.into(), &evidence.join("04-filtered.json"));
    assert!(backend.handle.history.health.warning().is_none());
    backend.handle.history.ingest(
        "fixture".into(),
        vec![protocol::Event::default(); 4097],
        "observed",
    );
    flush(&mut cx);
    assert!(
        backend
            .handle
            .history
            .health
            .warning()
            .unwrap()
            .contains("4097 that exceeded event limits")
    );
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("07-history-warning.json"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.within("main-tabs").click(1usize, cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.rules.page.rules.len())) == (200)
    });

    assert_eq!(cx.read_entity(&view, |v, _| v.rules.page.rules.len()), 200);
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("08-application-rules.json"),
    );
    click(&mut cx, window.into(), "rules-next");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.rules.page.rules.len())) == (30)
    });

    assert_eq!(cx.read_entity(&view, |v, _| v.rules.page.rules.len()), 30);
    click(&mut cx, window.into(), "rules-first");
    flush(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-allow");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.rules.page.rules[0].action.clone())) == ("allow")
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.rules.page.rules[0].action.clone()),
        "allow"
    );
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("09-rule-allowed.json"),
    );
    click(&mut cx, window.into(), "new-rule");
    cx.update_window(window.into(), |_, window, cx| {
        window.input("/usr/bin/new-application", cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "create-rule-allow");
    flush(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-a", cx);
        window.input("/usr/bin/new-application", cx);
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.rules.page.rules.len() == 1
                && v.rules.page.rules[0].duration == "always"
                && v.rules.page.rules[0].action == "allow"
        })
    });

    assert!(cx.read_entity(&view, |v, _| v.rules.page.rules.len() == 1
        && v.rules.page.rules[0].duration == "always"
        && v.rules.page.rules[0].action == "allow"));
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("10-rule-created.json"),
    );
    accept_changes.store(false, Ordering::Relaxed);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-deny");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.rules.page.rules[0].action.clone())) == ("allow")
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.rules.page.rules[0].action.clone()),
        "allow"
    );
    assert!(cx.read_entity(&view, |v, _| {
        v.rules
            .error
            .as_deref()
            .is_some_and(|e| e.contains("Synthetic rule rejection"))
    }));
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("11-rule-rejected.json"),
    );
    accept_changes.store(true, Ordering::Relaxed);
    click(&mut cx, window.into(), "rule-delete");
    click(&mut cx, window.into(), "rule-delete-cancel");
    assert_eq!(observed_commands.lock().unwrap().len(), 3);
    click(&mut cx, window.into(), "rule-delete");
    click(&mut cx, window.into(), "rule-delete-confirm");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.rules.page.rules.is_empty())
    });

    assert!(cx.read_entity(&view, |v, _| v.rules.page.rules.is_empty()));
    assert_eq!(observed_commands.lock().unwrap().len(), 4);
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("12-rule-deleted.json"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-a", cx);
        window.input("/usr/bin/synthetic-app-0", cx);
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-edit");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    checkpoint(&mut cx, editor, &evidence.join("13-rule-editor.json"));
    cx.update_window(editor, |_, window, cx| {
        window.click("editor-name", cx);
        window.press("ctrl-a", cx);
        window.input("fixture.renamed", cx);
        window.click("editor-description", cx);
        window.press("ctrl-a", cx);
        window.input("Edited using native Kit controls", cx);
        window.click("editor-priority", cx);
        window.click("editor-logging", cx);
    })
    .unwrap();
    click(&mut cx, editor, "editor-duration");
    cx.update_window(editor, |_, window, cx| {
        window.input("Until daemon restart", cx)
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(editor, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    click(&mut cx, editor, "editor-add-condition");
    cx.update_window(editor, |_, window, cx| {
        window.click(("editor-match", 1u64), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(editor, |_, window, cx| {
        window.input("Regular expression", cx)
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(editor, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(editor, |_, window, cx| {
        window.click(("editor-value", 1u64), cx);
        window.input(".*\\.example\\.test", cx);
        window.click(("editor-case", 1u64), cx);
    })
    .unwrap();
    checkpoint(&mut cx, editor, &evidence.join("14-rule-edited-draft.json"));
    click(&mut cx, editor, "editor-save");
    flush(&mut cx);
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.rules.editor_window.is_none())
    });
    assert!(cx.read_entity(&view, |v, _| v.rules.editor_window.is_none()));
    let renamed = cx.read_entity(&view, |v, _| v.rules.page.rules[0].clone());
    assert_eq!(renamed.name, "fixture.renamed");
    assert_eq!(renamed.description, "Edited using native Kit controls");
    assert!(renamed.precedence && renamed.nolog && renamed.enabled);
    assert_eq!(renamed.duration, "until restart");
    let op = renamed.operator.as_ref().unwrap();
    assert_eq!(op.r#type, "list");
    assert_eq!(op.list.len(), 2);
    assert_eq!(op.list[0].operand, "process.path");
    assert_eq!(op.list[1].r#type, "regexp");
    assert_eq!(op.list[1].data, ".*\\.example\\.test");
    assert!(!op.list[1].sensitive);
    {
        let commands = observed_commands.lock().unwrap();
        assert_eq!(commands.len(), 6);
        assert_eq!(commands[4].r#type, protocol::Action::ChangeRule as i32);
        assert_eq!(commands[5].r#type, protocol::Action::DeleteRule as i32);
    }
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-duplicate");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, editor, "editor-cancel");
    flush(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 6);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-duplicate");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    checkpoint(&mut cx, editor, &evidence.join("15-rule-duplicate.json"));
    click(&mut cx, editor, "editor-json-toggle");
    checkpoint(&mut cx, editor, &evidence.join("16-rule-match-json.json"));
    click(&mut cx, editor, "editor-json-toggle");
    click(&mut cx, editor, "editor-custom-duration-enabled");
    cx.update_window(editor, |_, window, cx| {
        window.click("editor-custom-duration", cx);
        window.press("ctrl-a", cx);
        window.input("tomorrow", cx);
    })
    .unwrap();
    click(&mut cx, editor, "editor-save");
    flush(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 6);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("17-rule-invalid-duration.json"),
    );
    cx.update_window(editor, |_, window, cx| {
        window.click("editor-custom-duration", cx);
        window.press("ctrl-a", cx);
        window.input("90s", cx);
    })
    .unwrap();
    click(&mut cx, editor, "editor-save");
    flush(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 7);
    assert!(cx.read_entity(&view, |v, _| {
        v.rules.page.rules.len() == 2
            && v.rules
                .page
                .rules
                .iter()
                .any(|r| r.name == "fixture.renamed" && r.duration == "until restart")
            && v.rules.page.rules.iter().any(|r| {
                r.name.starts_with("fixture.renamed.copy.")
                    && r.duration == "90s"
                    && r.operator == renamed.operator
            })
    }));
    let export_path = dir.path().join("exported-rules.json");
    click(&mut cx, window.into(), "rules-export");
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    cx.update_window(transfer, |_, window, cx| {
        window.input(export_path.to_str().unwrap(), cx)
    })
    .unwrap();
    click(&mut cx, transfer, "transfer-export-file");
    flush(&mut cx);
    assert_eq!(
        crate::rules::transfer::Import::from_path(&export_path)
            .unwrap()
            .rules
            .len(),
        2
    );
    click(&mut cx, transfer, "transfer-copy");
    flush(&mut cx);
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(
        crate::rules::transfer::Import::from_text(&copied)
            .unwrap()
            .rules
            .len(),
        2
    );
    checkpoint(&mut cx, transfer, &evidence.join("18-rules-export.json"));
    click(&mut cx, transfer, "transfer-close");
    flush(&mut cx);
    click(&mut cx, window.into(), "rules-import");
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    cx.update_window(transfer, |_, window, cx| {
        window.input(export_path.to_str().unwrap(), cx)
    })
    .unwrap();
    click(&mut cx, transfer, "transfer-read");
    flush(&mut cx);
    click(&mut cx, transfer, "transfer-apply");
    flush(&mut cx);
    assert_eq!(
        observed_commands.lock().unwrap().len(),
        7,
        "existing names are skipped by default"
    );
    checkpoint(&mut cx, transfer, &evidence.join("19-import-skipped.json"));
    let imported = (1..=2)
        .map(|i| {
            let mut rule = renamed.clone();
            rule.name = format!("fixture.import.{i}");
            rule
        })
        .collect::<Vec<_>>();
    let text = imported
        .iter()
        .map(|rule| crate::rules::document::rule_json(rule).to_string())
        .collect::<Vec<_>>()
        .join("\n");
    cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)));
    click(&mut cx, transfer, "transfer-paste");
    flush(&mut cx);
    checkpoint(&mut cx, transfer, &evidence.join("20-import-review.json"));
    reject_at.store(9, Ordering::Relaxed);
    click(&mut cx, transfer, "transfer-apply");
    flush(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 9);
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.rules
                .page
                .rules
                .iter()
                .any(|r| r.name == "fixture.import.1")
                && !v
                    .rules
                    .page
                    .rules
                    .iter()
                    .any(|r| r.name == "fixture.import.2")
        })
    });
    assert!(cx.read_entity(&view, |v, _| {
        v.rules
            .page
            .rules
            .iter()
            .any(|r| r.name == "fixture.import.1")
            && !v
                .rules
                .page
                .rules
                .iter()
                .any(|r| r.name == "fixture.import.2")
    }));
    checkpoint(
        &mut cx,
        transfer,
        &evidence.join("21-import-partial-failure.json"),
    );
    reject_at.store(usize::MAX, Ordering::Relaxed);
    click(&mut cx, transfer, "transfer-apply");
    flush(&mut cx);
    assert_eq!(
        observed_commands.lock().unwrap().len(),
        10,
        "retry sends only the remaining rule"
    );
    click(&mut cx, transfer, "transfer-close");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.rules.page.rules.len())) == (4)
    });

    assert_eq!(cx.read_entity(&view, |v, _| v.rules.page.rules.len()), 4);
    // Select one rule through its Kit checkbox, then the whole bounded page.
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-mark", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, window.into(), "rules-clear-selection");
    click(&mut cx, window.into(), "rules-select-page");
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("22-bulk-selection.json"),
    );
    click(&mut cx, window.into(), "rules-bulk-disable");
    flush(&mut cx);
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    hold_reply.store(true, Ordering::Release);
    click(&mut cx, transfer, "transfer-apply");
    // The fixture holds the current acknowledgement while the real Stop control
    // is clicked. Confirmed work remains; subsequent rules must not be sent.
    for _ in 0..100 {
        if observed_commands.lock().unwrap().len() == 11 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(observed_commands.lock().unwrap().len(), 11);
    let (_, mut during_bulk) = backend
        .handle
        .prompts
        .insert(
            1,
            protocol::Connection {
                process_path: "/usr/bin/during-bulk".into(),
                dst_host: "prompt.example.test".into(),
                dst_port: 443,
                ..Default::default()
            },
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap();
    flush(&mut cx);
    let prompt = permission_window(&mut cx, &view);
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("25-prompt-during-bulk.json"),
    );
    click(&mut cx, prompt, "allow-forever");
    assert_eq!(during_bulk.try_recv().unwrap().decision, Decision::Allow);
    assert_eq!(
        observed_commands.lock().unwrap().len(),
        11,
        "prompt handling must not wait for the bulk acknowledgement"
    );
    click(&mut cx, transfer, "transfer-stop");
    hold_reply.store(false, Ordering::Release);
    flush(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 11);
    checkpoint(&mut cx, transfer, &evidence.join("23-bulk-stopped.json"));
    click(&mut cx, transfer, "transfer-apply");
    flush(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 14);
    click(&mut cx, transfer, "transfer-close");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.rules.page.rules.iter().all(|r| !r.enabled))
    });

    assert!(cx.read_entity(&view, |v, _| v.rules.page.rules.iter().all(|r| !r.enabled)));
    click(&mut cx, window.into(), "rules-select-page");
    click(&mut cx, window.into(), "rules-bulk-delete");
    flush(&mut cx);
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    checkpoint(
        &mut cx,
        transfer,
        &evidence.join("24-bulk-delete-review.json"),
    );
    click(&mut cx, transfer, "transfer-close");
    flush(&mut cx);
    assert_eq!(
        observed_commands.lock().unwrap().len(),
        14,
        "closing bulk delete review sends nothing"
    );
    click(&mut cx, window.into(), "rules-select-page");
    click(&mut cx, window.into(), "rules-bulk-delete");
    flush(&mut cx);
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, transfer, "transfer-apply");
    wait_window(&mut cx, transfer, |window, cx| {
        crate::app::rule_transfer::test_idle(window, cx)
    });
    assert_eq!(observed_commands.lock().unwrap().len(), 18);
    click(&mut cx, transfer, "transfer-close");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.rules.page.rules.is_empty())
    });

    assert!(cx.read_entity(&view, |v, _| v.rules.page.rules.is_empty()));
    // Replacing existing names is explicit and remains available after the
    // default skip path. Use a remaining fixture outside the current filter.
    let mut replacement = renamed.clone();
    replacement.name = "fixture.application.001".into();
    replacement.action = "reject".into();
    replacement.operator.as_mut().unwrap().list[0].data = "/usr/bin/synthetic-app-1".into();
    cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            crate::rules::document::rule_json(&replacement).to_string(),
        ))
    });
    click(&mut cx, window.into(), "rules-import");
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, transfer, "transfer-paste");
    flush(&mut cx);
    click(&mut cx, transfer, "transfer-replace");
    click(&mut cx, transfer, "transfer-apply");
    flush(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 19);
    let stored = runtime.block_on(async {
        backend
            .handle
            .history
            .query_rules(crate::rules::Query {
                node: "fixture".into(),
                search: "fixture.application.001".into(),
                ..Default::default()
            })
            .unwrap()
            .await
            .unwrap()
            .unwrap()
    });
    assert_eq!(stored.rules, vec![replacement]);
    click(&mut cx, transfer, "transfer-close");
    flush(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.rules.page.rules.len())) == (200)
    });

    assert_eq!(cx.read_entity(&view, |v, _| v.rules.page.rules.len()), 200);
    click(&mut cx, window.into(), "rules-select-page");
    click(&mut cx, window.into(), "rules-next");
    flush(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-mark", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, window.into(), "rules-first");
    flush(&mut cx);
    click(&mut cx, window.into(), "rules-export-selected");
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, transfer, "transfer-copy");
    flush(&mut cx);
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(
        crate::rules::transfer::Import::from_text(&copied)
            .unwrap()
            .rules
            .len(),
        201
    );
    click(&mut cx, transfer, "transfer-close");
    flush(&mut cx);
    click(&mut cx, window.into(), "rules-select-page");
    click(&mut cx, window.into(), "rules-bulk-disable");
    flush(&mut cx);
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, transfer, "transfer-all-matching");
    flush(&mut cx);
    checkpoint(&mut cx, transfer, &evidence.join("26-bulk-all-pages.json"));
    click(&mut cx, transfer, "transfer-apply");
    wait_for(&mut cx, |_cx| {
        observed_commands.lock().unwrap().len() == 248
    });
    assert_eq!(
        observed_commands.lock().unwrap().len(),
        248,
        "all 229 remaining rules, including the second page, must be sent"
    );
    let second = runtime.block_on(async {
        backend
            .handle
            .history
            .query_rules(crate::rules::Query {
                node: "fixture".into(),
                after: "fixture.application.200".into(),
                ..Default::default()
            })
            .unwrap()
            .await
            .unwrap()
            .unwrap()
    });
    assert!(!second.rules.is_empty() && second.rules.iter().all(|r| !r.enabled));
    click(&mut cx, transfer, "transfer-close");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.rules.page.rules.iter().all(|r| !r.enabled))
    });

    assert!(cx.read_entity(&view, |v, _| v.rules.page.rules.iter().all(|r| !r.enabled)));
    cx.update_window(window.into(), |_, window, cx| {
        window.within("main-tabs").click(0usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("history-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "history-details");
    flush(&mut cx);
    let details = cx.read_entity(&view, |v, _| v.history.detail_window.unwrap());
    checkpoint(
        &mut cx,
        details,
        &evidence.join("27-connection-overview.json"),
    );
    click(&mut cx, details, "connection-copy");
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(
        copied.contains("User ID: 1000")
            && copied.contains("Process ID: 4567")
            && copied.contains("Source: 192.0.2.10:50000")
            && copied.contains("/tmp/synthetic working directory")
    );
    cx.update_window(details, |_, window, cx| {
        window.within("connection-tabs").click(1usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, details, "connection-copy-command");
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        "/usr/bin/firefox --profile profile with spaces"
    );
    checkpoint(
        &mut cx,
        details,
        &evidence.join("28-connection-arguments.json"),
    );
    cx.update_window(details, |_, window, cx| {
        window.within("connection-tabs").click(2usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    checkpoint(
        &mut cx,
        details,
        &evidence.join("29-connection-matched-rule.json"),
    );
    click(&mut cx, details, "connection-edit-rule");
    flush(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("30-current-matched-rule.json"),
    );
    click(&mut cx, editor, "editor-cancel");
    flush(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 248);
    click(&mut cx, details, "connection-create-destination");
    flush(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    cx.update_window(editor, |_, window, cx| {
        window.click("editor-name", cx);
        window.press("ctrl-a", cx);
        window.input("fixture.history.from-event", cx);
    })
    .unwrap();
    checkpoint(&mut cx, editor, &evidence.join("31-rule-from-history.json"));
    click(&mut cx, editor, "editor-save");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.rules.editor_window.is_none())
    });

    assert!(cx.read_entity(&view, |v, _| v.rules.editor_window.is_none()));
    assert_eq!(observed_commands.lock().unwrap().len(), 249);
    let stored = runtime.block_on(async {
        backend
            .handle
            .history
            .query_rules(crate::rules::Query {
                node: "fixture".into(),
                search: "fixture.history.from-event".into(),
                ..Default::default()
            })
            .unwrap()
            .await
            .unwrap()
            .unwrap()
    });
    let created = &stored.rules[0];
    assert_eq!(created.action, "allow");
    assert_eq!(created.duration, "always");
    let conditions = &created.operator.as_ref().unwrap().list;
    assert_eq!(
        conditions
            .iter()
            .map(|op| op.operand.as_str())
            .collect::<Vec<_>>(),
        ["process.path", "dest.host", "dest.port", "protocol"]
    );
    assert_eq!(conditions[0].data, "/usr/bin/firefox");
    assert_eq!(conditions[1].data, "api.example.test");
    click(&mut cx, details, "connection-create-application");
    flush(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    cx.update_window(editor, |_, window, cx| {
        window.click("editor-name", cx);
        window.press("ctrl-a", cx);
        window.input("fixture.history.from-event", cx);
    })
    .unwrap();
    click(&mut cx, editor, "editor-save");
    flush(&mut cx);
    assert_eq!(
        observed_commands.lock().unwrap().len(),
        249,
        "a new rule cannot silently overwrite an existing name"
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("32-history-rule-name-collision.json"),
    );
    click(&mut cx, editor, "editor-cancel");
    flush(&mut cx);
    click(&mut cx, details, "connection-close");
    flush(&mut cx);
    backend.handle.history.ingest(
        "fixture".into(),
        vec![protocol::Event {
            unixnano: crate::history::now_nanos(),
            connection: Some(protocol::Connection {
                process_path: "/usr/bin/firefox".into(),
                process_args: vec!["long argument".repeat(200); 100],
                dst_host: "limits.example.test".into(),
                dst_port: 443,
                ..Default::default()
            }),
            rule: Some(protocol::Rule {
                action: "deny".into(),
                ..Default::default()
            }),
            ..Default::default()
        }],
        "observed",
    );
    runtime.block_on(backend.handle.history.flush()).unwrap();
    click(&mut cx, window.into(), "latest");
    flush(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("history-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "history-details");
    flush(&mut cx);
    let details = cx.read_entity(&view, |v, _| v.history.detail_window.unwrap());
    checkpoint(
        &mut cx,
        details,
        &evidence.join("33-limited-connection-details.json"),
    );
    click(&mut cx, details, "connection-copy");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("Limited fields: arguments")
    );
    click(&mut cx, details, "connection-close");
    flush(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.within("main-tabs").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, window.into(), "new-rule");
    cx.update_window(window.into(), |_, window, cx| {
        window.click("new-rule-path", cx);
        window.press("ctrl-a", cx);
        window.input("/usr/bin/other-application", cx);
        window.click("new-rule-name", cx);
        window.press("ctrl-a", cx);
        window.input("fixture.history.from-event", cx);
    })
    .unwrap();
    click(&mut cx, window.into(), "create-rule-allow");
    flush(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 249);
    assert!(cx.read_entity(&view, |v, _| {
        v.rules
            .error
            .as_ref()
            .is_some_and(|error| error.contains("already exists"))
    }));
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("34-quick-rule-name-collision.json"),
    );
    click(&mut cx, window.into(), "create-rule-cancel");
    drop(view);
    shutdown(cx);

    let connection = protocol::Connection {
        process_path: "/usr/bin/synthetic-app".into(),
        dst_ip: "192.0.2.42".into(),
        dst_host: "permission.example.test".into(),
        dst_port: 443,
        protocol: "tcp".into(),
        user_id: 1000,
        process_id: 5000,
        ..Default::default()
    };
    let (_, mut allow) = backend
        .handle
        .prompts
        .insert(
            1,
            connection.clone(),
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap();
    let (_, mut deny) = backend
        .handle
        .prompts
        .insert(1, connection, Instant::now() + Duration::from_secs(30))
        .unwrap();
    let mut cx = context(test_cx);
    let mut view = None;
    let window = cx
        .open_test_window(size(px(650.), px(490.)), |window, cx| {
            let v = cx.new(|cx| PromptWindow::new(backend.handle.clone(), window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    checkpoint(&mut cx, window.into(), &evidence.join("05-permission.json"));
    click(&mut cx, window.into(), "allow-forever");
    let allowed = allow.try_recv().unwrap();
    assert_eq!(allowed.decision, Decision::Allow);
    assert_eq!(allowed.rule.unwrap().duration, "always");
    // Traverse the actual scope controls and activate Block with Tab/Space.
    keyboard_button(&mut cx, window.into(), "deny-once");
    assert_eq!(deny.try_recv().unwrap().decision, Decision::Deny);
    assert!(backend.handle.prompts.snapshot().is_empty());
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("06-permissions-complete.json"),
    );
    // Drive an actual AskRule RPC with lasting options, then independently
    // report the daemon's observation. The reply itself must not confirm cache.
    let popup_connection = protocol::Connection {
        process_path: "/usr/bin/popup-example".into(),
        process_args: vec!["popup-example".into(), "argument with spaces".into()],
        process_id: 6000,
        user_id: 1000,
        src_ip: "192.0.2.10".into(),
        src_port: 50000,
        dst_ip: "198.51.100.20".into(),
        dst_host: "api.example.test".into(),
        dst_port: 443,
        protocol: "tcp".into(),
        process_cwd: "/tmp/popup fixture".into(),
        process_env: [("POPUP_TEST".into(), "live-only-value".into())].into(),
        process_checksums: [("process.hash.md5".into(), "f".repeat(32))].into(),
        ..Default::default()
    };
    let con = popup_connection.clone();
    let mut asking = daemon.clone();
    let reply = runtime.spawn(async move { asking.ask_rule(con).await.unwrap().into_inner() });
    flush(&mut cx);
    click(&mut cx, window.into(), "prompt-more");
    pick(&mut cx, window.into(), "prompt-duration", "Forever");
    pick(&mut cx, window.into(), "prompt-target", "This command line");
    let targets = crate::prompts::choice::targets(&popup_connection);
    let uid = targets
        .iter()
        .position(|target| target.key == "user.id")
        .unwrap();
    let port = targets
        .iter()
        .position(|target| target.key == "dest.port")
        .unwrap();
    let checksum = targets
        .iter()
        .position(|target| target.key == "process.hash.md5")
        .unwrap();
    for index in [uid, port, checksum] {
        click_prompt_option(&mut cx, window.into(), ("prompt-constraint", index).into());
    }
    click_prompt_option(&mut cx, window.into(), "prompt-priority".into());
    click_prompt_option(&mut cx, window.into(), "prompt-logging".into());
    cx.update_window(window.into(), |_, window, cx| {
        window.scroll(
            "prompt-options-body",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(2000.))),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("35-prompt-lasting-options.json"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.within("prompt-tabs").click(1usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, window.into(), "prompt-copy-details");
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(copied.contains("POPUP_TEST: live-only-value") && copied.contains("process.hash.md5"));
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("36-prompt-live-details.json"),
    );
    click(&mut cx, window.into(), "reject-once");
    let lasting = runtime.block_on(reply).unwrap();
    assert_eq!(lasting.action, "reject");
    assert_eq!(lasting.duration, "always");
    assert!(lasting.precedence && lasting.nolog);
    let operators = &lasting.operator.as_ref().unwrap().list;
    for field in [
        "process.command",
        "process.path",
        "user.id",
        "dest.port",
        "process.hash.md5",
    ] {
        assert!(operators.iter().any(|op| op.operand == field), "{field}");
    }
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut catalogue_view = None;
    let catalogue = cx
        .open_test_window(size(px(1100.), px(780.)), |window, cx| {
            let view = cx
                .new(|cx| Dashboard::new(backend.handle.clone(), options(dir.path()), window, cx));
            catalogue_view = Some(view.clone());
            cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
        })
        .unwrap();
    let catalogue_view = catalogue_view.unwrap();
    cx.update_window(catalogue.into(), |_, window, cx| {
        window.within("main-tabs").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(catalogue.into(), |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input(&lasting.name, cx);
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&catalogue_view, |view, _| {
            view.rules.page.awaiting.contains(&lasting.name)
        })
    });

    assert!(cx.read_entity(&catalogue_view, |view, _| {
        view.rules.page.awaiting.contains(&lasting.name)
    }));
    cx.update_window(catalogue.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    checkpoint(
        &mut cx,
        catalogue.into(),
        &evidence.join("37-popup-rule-awaiting.json"),
    );
    let row = runtime.block_on(async {
        backend
            .handle
            .history
            .query(crate::history::Query {
                search: "popup-example".into(),
                ..Default::default()
            })
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .rows
            .first()
            .unwrap()
            .clone()
    });
    let detail = runtime.block_on(async {
        backend
            .handle
            .history
            .detail(row.id)
            .unwrap()
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert!(
        detail.connection().unwrap().process_env.is_empty()
            && detail.connection().unwrap().process_checksums.is_empty()
    );
    let mut observed = lasting.clone();
    observed.description = "Observed daemon popup rule".into();
    observed.operator.as_mut().unwrap().data.clear();
    let mut reporting = daemon.clone();
    let con = popup_connection.clone();
    let rule = observed.clone();
    runtime.block_on(async {
        reporting
            .ping(protocol::PingRequest {
                id: 9001,
                stats: Some(protocol::Statistics {
                    daemon_version: "1.7.2".into(),
                    rules: 231,
                    events: vec![protocol::Event {
                        unixnano: crate::history::now_nanos(),
                        connection: Some(con),
                        rule: Some(rule),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    wait_for(&mut cx, |cx| {
        cx.read_entity(&catalogue_view, |view, _| {
            view.rules.page.awaiting.is_empty() && view.rules.page.rules[0] == observed
        })
    });

    assert!(cx.read_entity(
        &catalogue_view,
        |view, _| view.rules.page.awaiting.is_empty() && view.rules.page.rules[0] == observed
    ));
    checkpoint(
        &mut cx,
        catalogue.into(),
        &evidence.join("38-popup-rule-observed.json"),
    );
    cx.update_window(catalogue.into(), |_, window, _| window.remove_window())
        .unwrap();
    drop(catalogue_view);
    cx.run_until_parked();
    let con = protocol::Connection {
        process_path: "/usr/bin/next-popup".into(),
        dst_ip: "192.168.1.20".into(),
        dst_port: 443,
        protocol: "tcp".into(),
        ..Default::default()
    };
    let mut asking = daemon.clone();
    let next = runtime.spawn(async move { asking.ask_rule(con).await.unwrap().into_inner() });
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("39-prompt-reset.json"),
    );
    click(&mut cx, window.into(), "deny-once");
    let next = runtime.block_on(next).unwrap();
    assert_eq!(next.duration, "once");
    assert_eq!(next.action, "deny");
    assert!(!next.precedence && !next.nolog);
    let con = protocol::Connection {
        process_path: "/usr/bin/network-popup".into(),
        dst_ip: "192.168.1.20".into(),
        dst_port: 443,
        protocol: "tcp".into(),
        ..Default::default()
    };
    let mut asking = daemon.clone();
    let temporary = runtime.spawn(async move { asking.ask_rule(con).await.unwrap().into_inner() });
    flush(&mut cx);
    click(&mut cx, window.into(), "prompt-more");
    pick(&mut cx, window.into(), "prompt-duration", "5 minutes");
    pick(
        &mut cx,
        window.into(),
        "prompt-target",
        "Destination network: 192.168.1.0/24",
    );
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("40-prompt-temporary-network.json"),
    );
    click(&mut cx, window.into(), "allow-selected");
    let temporary = runtime.block_on(temporary).unwrap();
    assert_eq!(temporary.action, "allow");
    assert_eq!(temporary.duration, "5m");
    assert_eq!(temporary.operator.as_ref().unwrap().r#type, "network");
    assert_eq!(temporary.operator.unwrap().data, "192.168.1.0/24");
    let mut settings_owner = None;
    let dashboard = cx
        .open_test_window(size(px(1100.), px(780.)), |window, cx| {
            let entity = cx
                .new(|cx| Dashboard::new(backend.handle.clone(), options(dir.path()), window, cx));
            settings_owner = Some(entity.clone());
            cx.new(|cx| gpui_kit::base::Root::new(entity, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    click(&mut cx, dashboard.into(), "preferences");
    let owner = settings_owner.unwrap();
    let preferences = cx.read_entity(&owner, |view, _| view.settings_window.unwrap());
    cx.update_window(dashboard.into(), |_, window, _| window.remove_window())
        .unwrap();
    drop(owner);
    pick(&mut cx, preferences, "preferences-duration", "12 hours");
    pick(
        &mut cx,
        preferences,
        "preferences-target",
        "This executable",
    );
    click(&mut cx, preferences, "preferences-priority");
    click(&mut cx, preferences, "preferences-logging");
    click(&mut cx, preferences, "preferences-save");
    flush(&mut cx);
    let saved = backend.handle.preferences.snapshot();
    assert_eq!(saved.duration, "12h");
    assert_eq!(saved.target, "process.path");
    assert!(saved.priority && !saved.logging && saved.popups);
    assert_eq!(
        crate::preferences::Store::open(&options(dir.path()).database)
            .unwrap()
            .snapshot(),
        saved
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("41-popup-preferences.json"),
    );
    let mut asking = daemon.clone();
    let configured = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/configured-popup".into(),
                dst_ip: "192.0.2.17".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("42-configured-popup.json"),
    );
    click(&mut cx, window.into(), "prompt-more");
    click(&mut cx, window.into(), "reject-once");
    let configured = runtime.block_on(configured).unwrap();
    assert_eq!(configured.duration, "12h");
    assert!(configured.precedence && configured.nolog);
    assert_eq!(configured.operator.unwrap().operand, "process.path");
    click(&mut cx, preferences, "preferences-popups");
    click(&mut cx, preferences, "preferences-save");
    wait_for(&mut cx, |_cx| !backend.handle.preferences.snapshot().popups);

    assert!(!backend.handle.preferences.snapshot().popups);
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("43-disabled-popups.json"),
    );
    let mut asking = daemon.clone();
    let automatic = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(1),
            asking.ask_rule(protocol::Connection {
                process_path: "/usr/bin/disabled-popup".into(),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .unwrap()
        .into_inner()
    });
    assert_eq!(automatic.action, "deny");
    assert_eq!(automatic.duration, "once");
    assert!(backend.handle.prompts.snapshot().is_empty());
    click(&mut cx, preferences, "preferences-popups");
    advanced_defaults(&mut cx, preferences);
    fill(&mut cx, preferences, "preferences-timeout", "0");
    click(&mut cx, preferences, "preferences-save");
    wait_for(&mut cx, |_cx| !backend.handle.preferences.snapshot().popups);

    assert!(
        !backend.handle.preferences.snapshot().popups,
        "invalid timeout must not save staged changes"
    );
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("44-invalid-popup-timeout.json"),
    );
    fill(&mut cx, preferences, "preferences-timeout", "5");
    pick(&mut cx, preferences, "preferences-fallback", "Reject once");
    click(&mut cx, preferences, "preferences-save");
    flush(&mut cx);
    let advanced = backend.handle.preferences.snapshot();
    assert!(advanced.popups && advanced.show_advanced && advanced.constraints.len() == 4);
    assert_eq!(advanced.timeout, Some(5));
    assert_eq!(advanced.fallback, Some(Decision::Reject));
    checkpoint(
        &mut cx,
        preferences,
        &evidence.join("45-saved-advanced-conditions.json"),
    );
    let mut asking = daemon.clone();
    let con = popup_connection.clone();
    let configured = runtime.spawn(async move { asking.ask_rule(con).await.unwrap().into_inner() });
    wait_for(&mut cx, |_cx| {
        backend.handle.prompts.snapshot()[0].reviewing
    });

    assert!(
        backend.handle.prompts.snapshot()[0].reviewing,
        "initial advanced options pause the ordinary countdown"
    );
    assert_eq!(
        backend.handle.prompts.snapshot()[0]
            .initial
            .choice
            .constraints
            .len(),
        4
    );
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("46-advanced-default-prompt.json"),
    );
    click(&mut cx, window.into(), "allow-selected");
    let configured = runtime.block_on(configured).unwrap();
    assert_eq!(configured.duration, "12h");
    assert_eq!(configured.action, "allow");
    assert!(configured.precedence && configured.nolog);
    let operators = configured.operator.unwrap().list;
    for field in [
        "process.path",
        "dest.ip",
        "dest.port",
        "user.id",
        "process.hash.md5",
    ] {
        assert!(operators.iter().any(|op| op.operand == field), "{field}");
    }
    let mut asking = daemon.clone();
    let missing = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/missing-default-fields".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    wait_for(&mut cx, |_cx| {
        (backend.handle.prompts.snapshot()[0]
            .initial
            .unavailable
            .len())
            == (3)
    });

    assert_eq!(
        backend.handle.prompts.snapshot()[0]
            .initial
            .unavailable
            .len(),
        3
    );
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("47-unavailable-default-conditions.json"),
    );
    click(&mut cx, window.into(), "deny-once");
    assert_eq!(runtime.block_on(missing).unwrap().action, "deny");
    cx.update_window(preferences, |_, window, cx| {
        window.scroll(
            "preferences-timeout",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(1000.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, preferences, "preferences-expanded");
    cx.update_window(preferences, |_, window, cx| {
        window.scroll(
            "preferences-expanded",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-240.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    fill(&mut cx, preferences, "preferences-timeout", "1");
    click(&mut cx, preferences, "preferences-save");
    wait_for(&mut cx, |_cx| {
        (backend.handle.preferences.snapshot().timeout) == (Some(1))
    });

    assert_eq!(backend.handle.preferences.snapshot().timeout, Some(1));
    let timed_before = backend.handle.status().timed_out;
    let mut asking = daemon.clone();
    let con = popup_connection.clone();
    let timed = runtime.spawn(async move { asking.ask_rule(con).await.unwrap().into_inner() });
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window.into(),
        &evidence.join("48-configured-fallback-countdown.json"),
    );
    let timed = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), timed)
            .await
            .unwrap()
            .unwrap()
    });
    assert_eq!(timed.action, "reject");
    assert_eq!(timed.duration, "once");
    assert_eq!(backend.handle.status().timed_out, timed_before + 1);
    click(&mut cx, preferences, "preferences-close");
    drop(view);
    shutdown(cx);
    notifications.abort();
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!dir.path().join("ui.sock").exists());
    let actions = [
        "History: 200-row first page",
        "Click Older",
        "Click Latest",
        "Click Denied",
        "Ctrl+F; type absent query",
        "Ctrl+A; type firefox",
        "Report an oversized event batch; inspect the specific history warning",
        "Application rules: Next/First, change deny to allow, create permanent application rule, filter by executable, reject a daemon command without optimistic mutation, cancel then confirm deletion",
        "Full rule editor: rename with acknowledged create/delete, edit description/priority/logging/duration, add a case-insensitive regex condition; cancel a duplicate then save a duplicate without deleting its source",
        "Rule transfer: export file/clipboard; import file with default duplicate skipping; import adjacent clipboard JSON; reject second command and retry only the remaining rule; explicitly replace an existing name",
        "Bulk: mark a row and select page; stop after a held acknowledgement; answer a separate permission prompt while the batch waits; resume remaining rules; cancel and confirm deletion; apply all matching rules across multiple pages",
        "Connection details: select a history row, copy metadata/arguments, inspect the historical rule and open the current rule without saving; create and confirm a permanent application+destination rule; reject a duplicate new name without a daemon command",
        "Click Allow forever",
        "Tab; Space: Deny once",
        "Popup options: permanent Reject with command/user/port/checksum, priority and disabled logging; scroll constraints and copy live environment/checksum details; require real gRPC reply",
        "Popup catalogue: awaiting observation with edits disabled; daemon Ping promotes reported values; local decision history does not confirm or persist live environment/checksums",
        "Next request resets to once/default options; a later request allows five-minute destination-network matching",
        "Popup preferences: save 12-hour executable default with priority/no logging through Kit controls; reopen disk state and require defaults in next actual gRPC reply; disable popups and require immediate deny/once without a pending prompt",
        "Advanced defaults: reject invalid timeout without publishing staged changes; save four reported AND fields, open-options preference and Reject fallback; verify next RPC conditions, warn about unavailable fields, and expire a one-second prompt to Reject once despite a lasting selected duration",
    ];
    std::fs::write(
        evidence.join("actions.json"),
        serde_json::to_vec_pretty(&actions).unwrap(),
    )
    .unwrap();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","checkpoints":["01-history.json","02-older.json","03-empty-search.json","04-filtered.json","05-permission.json","06-permissions-complete.json","07-history-warning.json","08-application-rules.json","09-rule-allowed.json","10-rule-created.json","11-rule-rejected.json","12-rule-deleted.json","13-rule-editor.json","14-rule-edited-draft.json","15-rule-duplicate.json","16-rule-match-json.json","17-rule-invalid-duration.json","18-rules-export.json","19-import-skipped.json","20-import-review.json","21-import-partial-failure.json","22-bulk-selection.json","23-bulk-stopped.json","24-bulk-delete-review.json","25-prompt-during-bulk.json","26-bulk-all-pages.json","27-connection-overview.json","28-connection-arguments.json","29-connection-matched-rule.json","30-current-matched-rule.json","31-rule-from-history.json","32-history-rule-name-collision.json","33-limited-connection-details.json","34-quick-rule-name-collision.json","35-prompt-lasting-options.json","36-prompt-live-details.json","37-popup-rule-awaiting.json","38-popup-rule-observed.json","39-prompt-reset.json","40-prompt-temporary-network.json","41-popup-preferences.json","42-configured-popup.json","43-disabled-popups.json","44-invalid-popup-timeout.json","45-saved-advanced-conditions.json","46-advanced-default-prompt.json","47-unavailable-default-conditions.json","48-configured-fallback-countdown.json"],"framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"proof_boundary":"Real GPUI views/input and SQLite; rules use an isolated gRPC daemon fixture with acknowledgements; synthetic history and prompts enter at the backend boundary."})).unwrap()).unwrap();
}
