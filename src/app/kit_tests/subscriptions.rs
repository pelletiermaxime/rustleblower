use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::subscriptions::{Reply, Request, model::*};
    use std::io::{Read, Write};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
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
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let stopping = stop.clone();
    let released = release.clone();
    let log = requests.clone();
    let http = std::thread::spawn(move || {
        while !stopping.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut buf = [0u8; 1024];
                    while request.len() < 8192 && !request.windows(4).any(|v| v == b"\r\n\r\n") {
                        let n = stream.read(&mut buf).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        request.extend_from_slice(&buf[..n]);
                    }
                    let text = String::from_utf8_lossy(&request).into_owned();
                    let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                    {
                        let mut log = log.lock().unwrap();
                        if log.len() < 32 {
                            log.push(text.clone());
                        }
                    }
                    if path == "/slow" {
                        let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n0.0.0.0 pending.example\n");
                        while !released.load(Ordering::Relaxed) && !stopping.load(Ordering::Relaxed)
                        {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        continue;
                    }
                    let response = if path == "/bad" {
                        let body = "<html>Not a list</html>\n";
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    } else if text.to_lowercase().contains("if-none-match: \"gpu-v1\"") {
                        "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".into()
                    } else {
                        let body = if path == "/ips" {
                            "192.0.2.1\n2001:db8::/32\n"
                        } else {
                            "0.0.0.0 blocked.example\n127.0.0.1 another.example\n"
                        };
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"gpu-v1\"\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    };
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => break,
            }
        }
    });
    let mut cx = context(test_cx);
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(crate::app::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-7", cx))
        .unwrap();
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("01-empty-subscriptions.json"),
    );
    click(&mut cx, window, "subs-collections");
    flush(&mut cx);
    cx.read_entity(&view, |v, _| {
        let page = v.subscriptions.collection_page().unwrap();
        assert_eq!(page.total, 5);
        assert!(
            page.rows
                .iter()
                .all(|r| r.members == 0 && r.group.is_none())
        );
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("21-empty-rule-collections.json"),
    );
    click(&mut cx, window, "subs-collection-back");
    flush(&mut cx);
    let query = |q| {
        let rx = backend
            .handle
            .subscriptions
            .request(Request::Page(q))
            .unwrap();
        match runtime.block_on(rx).unwrap().unwrap() {
            Reply::Page(page) => page,
            _ => panic!(),
        }
    };
    let row = |id| {
        let rx = backend
            .handle
            .subscriptions
            .request(Request::Get(id))
            .unwrap();
        match runtime.block_on(rx).unwrap().unwrap() {
            Reply::Row(row) => *row,
            _ => panic!(),
        }
    };
    let wait = |cx: &mut TestAppContext, id, status: &str| {
        wait_for(cx, |_cx| row(id).status == status);
    };
    let select = |cx: &mut TestAppContext, id: u64| {
        cx.update_window(window, |_, window, cx| {
            window.click(("subscription-row", id), cx)
        })
        .unwrap();
        flush(cx);
    };
    assert_eq!(query(Query::default()).total, 0);
    click(&mut cx, window, "subs-defaults");
    flush(&mut cx);
    pick(&mut cx, window, "subs-default-interval-unit", "seconds");
    pick(&mut cx, window, "subs-default-timeout-unit", "seconds");
    pick(&mut cx, window, "subs-default-size-unit", "KB");
    fill(&mut cx, window, "subs-default-interval", "60");
    fill(&mut cx, window, "subs-default-timeout", "2");
    fill(&mut cx, window, "subs-default-size", "1");
    fill(&mut cx, window, "subs-default-disk", "20");
    fill(
        &mut cx,
        window,
        "subs-default-agent",
        "Rustleblower GPU fixture",
    );
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    click(&mut cx, window, "subs-add");
    flush(&mut cx);
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("02-invalid-add-preserved.json"),
    );
    fill(&mut cx, window, "subs-name", "Alpha hosts");
    fill(&mut cx, window, "subs-url", &format!("{url}/list"));
    fill(&mut cx, window, "subs-filename", "alpha.txt");
    fill(&mut cx, window, "subs-groups", "Test, local");
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    assert_eq!(query(Query::default()).total, 1);
    click(&mut cx, window, "subs-add");
    flush(&mut cx);
    fill(&mut cx, window, "subs-name", "Cancelled draft");
    click(&mut cx, window, "subs-cancel");
    flush(&mut cx);
    assert_eq!(query(Query::default()).total, 1);
    cx.update_window(window, |_, window, cx| window.press("ctrl-n", cx))
        .unwrap();
    flush(&mut cx);
    fill(&mut cx, window, "subs-name", "Beta source");
    fill(&mut cx, window, "subs-url", &format!("{url}/bad"));
    fill(&mut cx, window, "subs-filename", "beta.txt");
    cx.update_window(window, |_, window, cx| window.press("ctrl-s", cx))
        .unwrap();
    flush(&mut cx);
    assert_eq!(query(Query::default()).total, 2);
    assert!(
        requests.lock().unwrap().is_empty(),
        "Paused runtime never downloads just because a row was saved"
    );
    select(&mut cx, 1);
    click(&mut cx, window, "subs-refresh");
    wait(&mut cx, 1, "updated");
    let first = row(1);
    let data = std::fs::read(first.file.as_ref().unwrap()).unwrap();
    assert!(data.starts_with(b"0.0.0.0"));
    assert_eq!(first.spec.groups, ["Test", "local"]);
    assert_eq!(first.spec.timeout, None);
    assert!(
        (60..=62).contains(&(first.next_due - first.checked)),
        "Schedule uses completion time plus the configured interval"
    );
    checkpoint(&mut cx, window, &evidence.join("03-downloaded-list.json"));
    click(&mut cx, window, "subs-preview");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("04-downloaded-text-preview.json"),
    );
    click(&mut cx, window, "subs-preview-copy");
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .as_bytes(),
        data
    );
    click(&mut cx, window, "subs-preview-back");
    flush(&mut cx);
    click(&mut cx, window, "subs-refresh");
    wait(&mut cx, 1, "unchanged");
    assert_eq!(row(1).updated, first.updated);
    assert!(
        requests.lock().unwrap()[1]
            .to_lowercase()
            .contains("if-none-match")
    );
    click(&mut cx, window, "subs-force");
    wait(&mut cx, 1, "updated");
    assert!(
        !requests.lock().unwrap()[2]
            .to_lowercase()
            .contains("if-none-match")
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("05-conditional-and-forced-refresh.json"),
    );
    select(&mut cx, 2);
    click(&mut cx, window, "subs-refresh");
    wait(&mut cx, 2, "error");
    assert!(row(2).file.is_none());
    assert!(row(2).error.contains("HTML"));
    checkpoint(&mut cx, window, &evidence.join("06-download-error.json"));
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    fill(&mut cx, window, "subs-url", &format!("{url}/slow"));
    fill(&mut cx, window, "subs-timeout", "30");
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    click(&mut cx, window, "subs-refresh");
    flush(&mut cx);
    assert_eq!(backend.handle.subscriptions.state().active, Some(2));
    assert!(
        backend
            .handle
            .subscriptions
            .state()
            .downloaded
            .load(Ordering::Relaxed)
            > 0
    );
    let default_rule = crate::subscriptions::attachment::draft(&row(1)).unwrap();
    let owner = default_rule.operator.as_ref().unwrap();
    assert_eq!(owner.r#type, "list");
    assert_eq!(owner.list.len(), 2);
    assert_eq!(owner.list[0].operand, "user.id");
    assert_eq!(
        owner.list[0].data,
        rustix::process::getuid().as_raw().to_string()
    );
    assert_eq!(owner.list[1].operand, "lists.domains");
    let mut fixture_rules = Vec::new();
    for index in 0..201 {
        fixture_rules.push(protocol::Rule {
            name: format!("unrelated-{index:03}"),
            enabled: true,
            action: "allow".into(),
            duration: "always".into(),
            operator: Some(protocol::Operator {
                r#type: "simple".into(),
                operand: "process.path".into(),
                data: format!("/usr/bin/unrelated-{index}"),
                sensitive: true,
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    for index in 0..18 {
        let mut rule = default_rule.clone();
        rule.name = format!("saved-list-{index:03}");
        if index % 2 == 0 {
            rule.operator
                .as_mut()
                .unwrap()
                .list
                .push(protocol::Operator {
                    r#type: "simple".into(),
                    operand: "dest.port".into(),
                    data: "443".into(),
                    sensitive: true,
                    ..Default::default()
                });
        }
        fixture_rules.push(rule);
    }
    let (daemon, mut rule_commands, rule_replies) = runtime.block_on(async {
        let path = config.socket.clone();
        let channel = tonic::transport::Endpoint::try_from("http://localhost")
            .unwrap()
            .connect_with_connector(tower::service_fn(move |_| {
                let path = path.clone();
                async move {
                    tokio::net::UnixStream::connect(path)
                        .await
                        .map(hyper_util::rt::TokioIo::new)
                }
            }))
            .await
            .unwrap();
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "subscription-gpu-node".into(),
                rules: fixture_rules.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let stream = daemon
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        (daemon, stream, tx)
    });
    let mut asking = daemon.clone();
    let ask = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/subscription-gpu-permission".into(),
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
        &evidence.join("07-permission-during-slow-http.json"),
    );
    assert_eq!(backend.handle.subscriptions.state().active, Some(2));
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(ask).unwrap().duration, "once");
    click(&mut cx, window, "subs-stop");
    wait(&mut cx, 2, "cancelled");
    release.store(true, Ordering::Relaxed);
    assert!(row(2).file.is_none());
    // The worker can publish Cancelled before the dashboard consumes its reply.
    // Wait for the actual Edit control to be enabled before dispatching input.
    for _ in 0..20 {
        flush(&mut cx);
        if cx
            .update_window(window, |_, window, _| {
                window.find("subs-edit").disabled() != Some(true)
            })
            .unwrap()
        {
            break;
        }
    }
    assert!(
        cx.update_window(window, |_, window, _| window.find("subs-edit").disabled()
            != Some(true))
            .unwrap()
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("08-cancelled-download.json"),
    );
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    fill(&mut cx, window, "subs-url", &format!("{url}/ips"));
    pick(&mut cx, window, "subs-format", "IP addresses and CIDRs");
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("22-format-membership-review.json"),
    );
    assert_eq!(row(2).spec.format, Format::Hosts);
    click(&mut cx, window, "subs-membership-confirm");
    flush(&mut cx);

    assert_eq!(row(2).spec.format, Format::Ips);
    click(&mut cx, window, "subs-refresh");
    wait(&mut cx, 2, "updated");
    assert_eq!(
        std::fs::read_to_string(row(2).file.unwrap()).unwrap(),
        "192.0.2.1\n2001:db8::/32\n"
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("09-edited-list-format.json"),
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-sort", 3usize), cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| {
            v.subscriptions.page.as_ref().unwrap().rows[0].id
        })) == (2)
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.page.as_ref().unwrap().rows[0]
            .id),
        2
    );
    click(&mut cx, window, "subs-descending");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| {
            v.subscriptions.page.as_ref().unwrap().rows[0].id
        })) == (1)
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.page.as_ref().unwrap().rows[0]
            .id),
        1
    );
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("absent source", cx);
    })
    .unwrap();
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("10-filtered-empty-state.json"),
    );
    assert_eq!(
        query(Query {
            search: "absent source".into(),
            ..Default::default()
        })
        .total,
        0
    );
    fill(&mut cx, window, "subs-search", "Beta");
    select(&mut cx, 2);
    click(&mut cx, window, "subs-remove");
    flush(&mut cx);
    click(&mut cx, window, "subs-remove-cancel");
    flush(&mut cx);
    assert!(!row(2).removed);
    let retained = row(2).file.unwrap();
    click(&mut cx, window, "subs-remove");
    flush(&mut cx);
    click(&mut cx, window, "subs-remove-confirm");
    flush(&mut cx);
    assert!(row(2).removed && std::path::Path::new(&retained).is_file());
    click(&mut cx, window, "subs-removed");
    flush(&mut cx);
    select(&mut cx, 2);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("11-removed-data-retained.json"),
    );
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("23-restored-membership-review.json"),
    );
    assert!(row(2).removed);
    click(&mut cx, window, "subs-membership-confirm");
    flush(&mut cx);
    click(&mut cx, window, "subs-removed");
    flush(&mut cx);
    select(&mut cx, 2);
    assert!(!row(2).removed);
    click(&mut cx, window, "subs-logs");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("12-restored-status-history.json"),
    );
    click(&mut cx, window, "subs-preview-back");
    flush(&mut cx);
    fill(&mut cx, window, "subs-search", "Alpha");
    select(&mut cx, 1);
    click(&mut cx, window, "subs-rule-create");
    flush(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("15-compatible-list-rule-review.json"),
    );
    click(&mut cx, editor, "editor-cancel");
    flush(&mut cx);
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), rule_commands.message()).await
            })
            .is_err(),
        "Cancelling a review must not send a command"
    );
    click(&mut cx, window, "subs-rule-create");
    flush(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, editor, "editor-save");
    flush(&mut cx);
    let command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), rule_commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(command.r#type, protocol::Action::ChangeRule as i32);
    assert_eq!(command.rules.len(), 1);
    assert_eq!(command.rules[0].operator, default_rule.operator);
    assert_eq!(command.rules[0].action, "deny");
    assert_eq!(command.rules[0].duration, "always");
    assert!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "subscription-gpu-node",
            &default_rule.name
        )
        .unwrap()
        .is_none()
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("16-list-rule-waits-for-confirmation.json"),
    );
    let mut asking = daemon.clone();
    let prompt = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/permission-under-list-rule".into(),
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
        &evidence.join("17-permission-under-list-rule.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(prompt).unwrap().duration, "once");
    runtime
        .block_on(rule_replies.send(protocol::NotificationReply {
            id: command.id,
            ..Default::default()
        }))
        .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.rules.editor_window.is_none())
    });

    assert!(cx.read_entity(&view, |v, _| v.rules.editor_window.is_none()));
    assert_eq!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "subscription-gpu-node",
            &default_rule.name
        )
        .unwrap()
        .unwrap(),
        command.rules[0]
    );
    click(&mut cx, window, "subs-collections");
    flush(&mut cx);
    fill(&mut cx, window, "subs-collection-search", "test");
    cx.read_entity(&view, |v, _| {
        let page = v.subscriptions.collection_page().unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].group.as_deref(), Some("test"));
        assert_eq!(page.rows[0].members, 1);
    });
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-collection-review", 0usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    checkpoint(&mut cx, editor, &evidence.join("24-group-rule-review.json"));
    click(&mut cx, editor, "editor-cancel");
    flush(&mut cx);
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), rule_commands.message()).await
            })
            .is_err()
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-collection-review", 0usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, editor, "editor-save");
    flush(&mut cx);
    let group_command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), rule_commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    let group_rule = group_command.rules[0].clone();
    assert_eq!(group_command.r#type, protocol::Action::ChangeRule as i32);
    assert_eq!(group_rule.name, "rustleblower-group-test-domains");
    let op = group_rule.operator.as_ref().unwrap();
    assert_eq!(op.list.len(), 2);
    assert_eq!(op.list[0].operand, "user.id");
    assert_eq!(
        op.list[0].data,
        rustix::process::getuid().as_raw().to_string()
    );
    assert_eq!(op.list[1].operand, "lists.domains");
    assert_eq!(
        op.list[1].data,
        backend
            .handle
            .subscriptions
            .root
            .join("collections/current/groups/test/domains")
            .to_string_lossy()
    );
    runtime
        .block_on(rule_replies.send(protocol::NotificationReply {
            id: group_command.id,
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx);
    assert_eq!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "subscription-gpu-node",
            &group_rule.name
        )
        .unwrap()
        .as_ref(),
        Some(&group_rule)
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-collection-inspect", 0usize), cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.subscriptions.associated_counts())) == (Some((1, 0, 1)))
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.associated_counts()),
        Some((1, 0, 1))
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("25-group-associated-rules.json"),
    );
    let mut changed_group_rule = group_rule.clone();
    changed_group_rule.description = "Concurrent confirmed description".into();
    // A second editor changes the confirmed catalogue after this UI snapshot.
    let concurrent = backend
        .handle
        .control
        .submit(
            backend.handle.node("subscription-gpu-node").unwrap().id,
            protocol::Action::ChangeRule,
            changed_group_rule.clone(),
        )
        .unwrap();
    let update = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), rule_commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(update.rules, vec![changed_group_rule.clone()]);
    runtime
        .block_on(rule_replies.send(protocol::NotificationReply {
            id: update.id,
            ..Default::default()
        }))
        .unwrap();
    runtime.block_on(concurrent).unwrap().unwrap();
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-associated-edit", 0usize), cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.rules.editor_window.is_none())
    });

    assert!(cx.read_entity(&view, |v, _| v.rules.editor_window.is_none()));
    checkpoint(
        &mut cx,
        window,
        &evidence.join("31-associated-snapshot-changed.json"),
    );
    click(&mut cx, window, "subs-associated-refresh");
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-associated-edit", 0usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, editor, "editor-cancel");
    flush(&mut cx);
    click(&mut cx, window, "subs-associated-back");
    flush(&mut cx);
    click(&mut cx, window, "subs-collection-back");
    flush(&mut cx);
    click(&mut cx, window, "subs-rule-inspect");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("18-associated-list-rules.json"),
    );
    click(&mut cx, window, "subs-associated-next");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("19-associated-rules-second-page.json"),
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-associated-edit", 1usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, editor, "editor-enabled");
    click(&mut cx, editor, "editor-save");
    flush(&mut cx);
    let rejected = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), rule_commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    let expected = fixture_rules
        .iter()
        .find(|r| r.name == "saved-list-016")
        .unwrap();
    let mut disabled = expected.clone();
    disabled.enabled = false;
    assert_eq!(rejected.rules, vec![disabled]);
    runtime
        .block_on(rule_replies.send(protocol::NotificationReply {
            id: rejected.id,
            code: 1,
            data: "Synthetic list rule rejection".into(),
        }))
        .unwrap();
    flush(&mut cx);
    assert_eq!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "subscription-gpu-node",
            &expected.name
        )
        .unwrap()
        .as_ref(),
        Some(expected)
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("20-associated-rule-rejection.json"),
    );
    click(&mut cx, editor, "editor-cancel");
    flush(&mut cx);
    click(&mut cx, window, "subs-associated-back");
    flush(&mut cx);
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    fill(&mut cx, window, "subs-groups", "Other");
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("26-group-membership-review.json"),
    );
    assert_eq!(row(1).spec.groups, ["Test", "local"]);
    click(&mut cx, window, "subs-membership-back");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("32-membership-back-preserves-draft.json"),
    );
    click(&mut cx, window, "subs-cancel");
    flush(&mut cx);
    assert_eq!(row(1).spec.groups, ["Test", "local"]);
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    fill(&mut cx, window, "subs-groups", "Other");
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    click(&mut cx, window, "subs-membership-confirm");
    flush(&mut cx);
    assert_eq!(row(1).spec.groups, ["Other"]);
    assert!(!std::path::Path::new(&op.list[1].data).exists());
    assert_eq!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "subscription-gpu-node",
            &group_rule.name
        )
        .unwrap()
        .as_ref(),
        Some(&changed_group_rule)
    );
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), rule_commands.message()).await
            })
            .is_err(),
        "Changing owned collection data must not rewrite daemon rule definitions"
    );
    click(&mut cx, window, "subs-collections");
    flush(&mut cx);
    fill(&mut cx, window, "subs-collection-search", "test");
    assert_eq!(
        cx.read_entity(&view, |v, _| v
            .subscriptions
            .collection_page()
            .unwrap()
            .total),
        0
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("27-group-removed-from-collections.json"),
    );
    click(&mut cx, window, "subs-collection-back");
    flush(&mut cx);
    // Seed bounded catalogue pages through the production backend; navigation and
    // last-page removal still use the actual rendered controls.
    for index in 0..63 {
        let receiver = backend
            .handle
            .subscriptions
            .request(Request::Save {
                previous: None,
                spec: Spec {
                    name: format!("Paging fixture {index:03}"),
                    url: format!("{url}/list"),
                    groups: if index < 10 {
                        (index * 8..index * 8 + 8)
                            .map(|g| format!("pager-{g:03}"))
                            .collect()
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                },
            })
            .unwrap();
        assert!(matches!(
            runtime.block_on(receiver).unwrap().unwrap(),
            Reply::Saved(_)
        ));
    }
    for id in 3..13 {
        let reply = backend
            .handle
            .subscriptions
            .request(Request::Refresh { id, force: true })
            .unwrap();
        runtime.block_on(reply).unwrap().unwrap();
        wait(&mut cx, id, "updated");
    }
    click(&mut cx, window, "subs-collections");
    flush(&mut cx);
    cx.read_entity(&view, |v, _| {
        assert_eq!(v.subscriptions.collection_page().unwrap().total, 86)
    });
    click(&mut cx, window, "subs-collection-next");
    flush(&mut cx);
    cx.read_entity(&view, |v, _| {
        let page = v.subscriptions.collection_page().unwrap();
        assert_eq!(page.offset, 64);
        assert_eq!(page.rows[0].group.as_deref(), Some("pager-058"));
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("28-collection-second-page.json"),
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-collection-sort", 2usize), cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| {
            v.subscriptions.collection_page().unwrap().rows[0].members
        })) == (0)
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v
            .subscriptions
            .collection_page()
            .unwrap()
            .rows[0]
            .members),
        0
    );
    click(&mut cx, window, "subs-collection-descending");
    flush(&mut cx);
    cx.read_entity(&view, |v, _| {
        let page = v.subscriptions.collection_page().unwrap();
        assert_eq!(page.offset, 0);
        assert_eq!(page.rows[0].members, 11);
        assert!(page.rows[0].group.is_none());
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("29-collection-sorted-members.json"),
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-collection-sort", 1usize), cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| {
            v.subscriptions.collection_page().unwrap().rows[0]
                .format
                .clone()
        })) == (Format::Regexp)
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v
            .subscriptions
            .collection_page()
            .unwrap()
            .rows[0]
            .format
            .clone()),
        Format::Regexp
    );
    click(&mut cx, window, "subs-collection-descending");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| {
            v.subscriptions.collection_page().unwrap().rows[0]
                .format
                .clone()
        })) == (Format::Domains)
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v
            .subscriptions
            .collection_page()
            .unwrap()
            .rows[0]
            .format
            .clone()),
        Format::Domains
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-collection-sort", 0usize), cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.subscriptions.collection_page().unwrap().rows[0]
                .group
                .is_none()
        })
    });

    assert!(cx.read_entity(&view, |v, _| {
        v.subscriptions.collection_page().unwrap().rows[0]
            .group
            .is_none()
    }));
    click(&mut cx, window, "subs-collection-descending");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| {
            v.subscriptions.collection_page().unwrap().rows[0]
                .group
                .clone()
        })
        .as_deref())
            == (Some("pager-079"))
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v
            .subscriptions
            .collection_page()
            .unwrap()
            .rows[0]
            .group
            .clone())
            .as_deref(),
        Some("pager-079")
    );
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("pager-079", cx);
    })
    .unwrap();
    flush(&mut cx);
    cx.read_entity(&view, |v, _| {
        let page = v.subscriptions.collection_page().unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].group.as_deref(), Some("pager-079"));
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("30-collection-filter-full-catalogue.json"),
    );
    click(&mut cx, window, "subs-collection-back");
    flush(&mut cx);
    fill(&mut cx, window, "subs-search", "");
    flush(&mut cx);
    let current_page = |cx: &TestAppContext| {
        cx.read_entity(&view, |v, _| {
            let page = v.subscriptions.page.as_ref().unwrap();
            (
                page.offset,
                page.total,
                page.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            )
        })
    };
    assert_eq!(current_page(&cx).2.len(), 64);
    assert_eq!(current_page(&cx).1, 65);
    click(&mut cx, window, "subs-next");
    flush(&mut cx);
    assert_eq!(current_page(&cx).0, 64);
    assert_eq!(current_page(&cx).2.len(), 1);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("13-subscription-second-page.json"),
    );
    click(&mut cx, window, "subs-previous");
    flush(&mut cx);
    assert_eq!(current_page(&cx).0, 0);
    click(&mut cx, window, "subs-next");
    flush(&mut cx);
    let last = current_page(&cx).2[0];
    let mark = |cx: &mut TestAppContext, id: u64| {
        cx.update_window(window, |_, window, cx| window.click(("subs-mark", id), cx))
            .unwrap();
        flush(cx);
    };
    mark(&mut cx, last);
    click(&mut cx, window, "subs-previous");
    flush(&mut cx);
    mark(&mut cx, 1);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().0),
        2
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("33-marks-across-pages.json"),
    );
    click(&mut cx, window, "subs-bulk");
    flush(&mut cx);
    pick(&mut cx, window, "subs-bulk-groups-mode", "Add groups");
    fill(&mut cx, window, "subs-bulk-groups", "Bulk Review");
    click(&mut cx, window, "subs-bulk-review");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1)) == (Some((2, 2)))
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1),
        Some((2, 2))
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("34-reviewed-group-batch.json"),
    );
    click(&mut cx, window, "subs-bulk-back");
    flush(&mut cx);
    click(&mut cx, window, "subs-bulk-review");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1)) == (Some((2, 2)))
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1),
        Some((2, 2)),
        "Back preserves the staged group operation"
    );
    click(&mut cx, window, "subs-bulk-cancel");
    flush(&mut cx);
    assert_eq!(row(1).spec.groups, ["Other"]);
    assert!(row(last).spec.groups.is_empty());
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-b", cx))
        .unwrap();
    flush(&mut cx);
    pick(&mut cx, window, "subs-bulk-groups-mode", "Add groups");
    fill(&mut cx, window, "subs-bulk-groups", "Bulk Review");
    click(&mut cx, window, "subs-bulk-review");
    flush(&mut cx);
    let original = row(1);
    let mut concurrent = original.spec.clone();
    concurrent.name = "Alpha concurrent".into();
    runtime
        .block_on(
            backend
                .handle
                .subscriptions
                .request(Request::Save {
                    previous: Some((1, original.revision)),
                    spec: concurrent.clone(),
                })
                .unwrap(),
        )
        .unwrap()
        .unwrap();
    click(&mut cx, window, "subs-bulk-apply");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.subscriptions
                .bulk_state()
                .2
                .unwrap()
                .contains("changed since review")
        })
    });

    assert!(cx.read_entity(&view, |v, _| {
        v.subscriptions
            .bulk_state()
            .2
            .unwrap()
            .contains("changed since review")
    }));
    assert_eq!(row(1).spec, concurrent);
    assert!(row(last).spec.groups.is_empty());
    checkpoint(
        &mut cx,
        window,
        &evidence.join("35-stale-bulk-refused.json"),
    );
    click(&mut cx, window, "subs-bulk-rebuild");
    flush(&mut cx);
    click(&mut cx, window, "subs-bulk-apply");
    flush(&mut cx);
    assert_eq!(row(1).spec.name, "Alpha concurrent");
    assert_eq!(row(1).spec.groups, ["other", "bulk-review"]);
    assert_eq!(row(last).spec.groups, ["bulk-review"]);
    assert!(row(2).spec.groups.is_empty());
    checkpoint(
        &mut cx,
        window,
        &evidence.join("36-selected-group-batch-applied.json"),
    );
    click(&mut cx, window, "subs-bulk");
    flush(&mut cx);
    click(&mut cx, window, "subs-bulk-matching");
    flush(&mut cx);
    pick(&mut cx, window, "subs-bulk-groups-mode", "Add groups");
    fill(&mut cx, window, "subs-bulk-groups", "exceeds-group-limit");
    click(&mut cx, window, "subs-bulk-review");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().2.is_some())
    });

    assert!(cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().2.is_some()));
    assert!(row(1).spec.enabled);
    assert_eq!(row(1).spec.groups, ["other", "bulk-review"]);
    pick(&mut cx, window, "subs-bulk-groups-mode", "Keep groups");
    pick(&mut cx, window, "subs-bulk-enabled", "Disable");
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-enter", cx);
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1)) == (Some((65, 65)))
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1),
        Some((65, 65))
    );
    click(&mut cx, window, "subs-bulk-next");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("37-all-matching-review-page-two.json"),
    );
    let retained = row(1).file.unwrap();
    let retained_bytes = std::fs::read(&retained).unwrap();
    click(&mut cx, window, "subs-bulk-apply");
    flush(&mut cx);
    assert!(query(Query::default()).rows.iter().all(|r| !r.spec.enabled));
    assert!(!row(last).spec.enabled);
    assert_eq!(std::fs::read(&retained).unwrap(), retained_bytes);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("38-all-pages-disabled.json"),
    );
    click(&mut cx, window, "subs-bulk");
    flush(&mut cx);
    pick(&mut cx, window, "subs-bulk-enabled", "Enable");
    for index in 0..3usize {
        cx.update_window(window, |_, window, cx| {
            window.click(("subs-bulk-override", index), cx)
        })
        .unwrap();
        flush(&mut cx);
    }
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-bulk-limit", 1usize), cx);
        window.input("7", cx);
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-bulk-limit", 2usize), cx);
        window.input("2", cx);
    })
    .unwrap();
    flush(&mut cx);
    // Preserve the prior physical limits while using explicit count/unit controls.
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-bulk-unit", 1usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| window.input("seconds", cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-bulk-unit", 2usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| window.input("KB", cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, window, "subs-bulk-review");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1)) == (Some((2, 2)))
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1),
        Some((2, 2))
    );
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-enter", cx))
        .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    for id in [1, last] {
        let r = row(id);
        assert!(
            r.spec.enabled,
            "Bulk state after shortcut: {:?}",
            cx.read_entity(&view, |v, _| {
                let (marks, review, error) = v.subscriptions.bulk_state();
                (marks, review, error.map(str::to_owned))
            })
        );
        assert_eq!(
            (r.spec.interval, r.spec.timeout, r.spec.max_bytes),
            (
                None,
                crate::subscriptions::limits::Override::parts(
                    Some(7),
                    Some(crate::subscriptions::limits::Unit::Seconds)
                ),
                crate::subscriptions::limits::Override::parts(
                    Some(2),
                    Some(crate::subscriptions::limits::Unit::Kib)
                )
            )
        );
    }
    assert!(!row(2).spec.enabled);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().0),
        2
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("39-selected-inherited-and-custom-limits.json"),
    );
    // New unit-only/count-only edits use real Kit selectors, not view setters.
    use crate::subscriptions::limits::{Override as Limit, Unit as LimitUnit};
    let bulk_before = [row(1).spec, row(last).spec];
    let Reply::Settings(default_revision, saved_defaults) = runtime
        .block_on(
            backend
                .handle
                .subscriptions
                .request(Request::Settings)
                .unwrap(),
        )
        .unwrap()
        .unwrap()
    else {
        panic!()
    };
    click(&mut cx, window, "subs-bulk");
    flush(&mut cx);
    for index in 0..3usize {
        cx.update_window(window, |_, window, cx| {
            window.click(("subs-bulk-override", index), cx)
        })
        .unwrap();
        flush(&mut cx);
    }
    for (index, label) in [(0usize, "minutes"), (2usize, "MB")] {
        cx.update_window(window, |_, window, cx| {
            window.click(("subs-bulk-unit", index), cx)
        })
        .unwrap();
        flush(&mut cx);
        cx.update_window(window, |_, window, cx| window.input(label, cx))
            .unwrap();
        flush(&mut cx);
        cx.update_window(window, |_, window, cx| window.press("enter", cx))
            .unwrap();
        flush(&mut cx);
    }
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-bulk-limit", 1usize), cx);
        window.input("3", cx);
    })
    .unwrap();
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("57-bulk-independent-count-units.json"),
    );
    click(&mut cx, window, "subs-bulk-review");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1)) == (Some((2, 2)))
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1),
        Some((2, 2))
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("58-bulk-effective-review.json"),
    );
    click(&mut cx, window, "subs-bulk-back");
    flush(&mut cx);
    click(&mut cx, window, "subs-bulk-review");
    flush(&mut cx);
    let mut changed_defaults = saved_defaults.clone();
    changed_defaults.interval = 120;
    runtime
        .block_on(
            backend
                .handle
                .subscriptions
                .request(Request::SaveSettings {
                    revision: default_revision,
                    settings: changed_defaults.clone(),
                })
                .unwrap(),
        )
        .unwrap()
        .unwrap();
    click(&mut cx, window, "subs-bulk-apply");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.subscriptions
                .bulk_state()
                .2
                .unwrap_or_default()
                .contains("defaults changed since review")
        })
    });

    assert!(cx.read_entity(&view, |v, _| {
        v.subscriptions
            .bulk_state()
            .2
            .unwrap_or_default()
            .contains("defaults changed since review")
    }));
    assert_eq!([row(1).spec, row(last).spec], bulk_before);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("59-bulk-stale-defaults-refused.json"),
    );
    click(&mut cx, window, "subs-bulk-rebuild");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("60-bulk-rebuilt-defaults.json"),
    );
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-enter", cx))
        .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    for id in [1, last] {
        let spec = row(id).spec;
        assert_eq!(
            (spec.interval, spec.timeout, spec.max_bytes),
            (
                Limit::parts(None, Some(LimitUnit::Minutes)),
                Limit::parts(Some(3), None),
                Limit::parts(None, Some(LimitUnit::Mib))
            )
        );
        assert_eq!(
            effective(&spec, &changed_defaults).unwrap(),
            (7200, 3, 1048576)
        );
    }
    checkpoint(
        &mut cx,
        window,
        &evidence.join("61-bulk-independent-limits-applied.json"),
    );
    let Reply::Settings(revision, _) = runtime
        .block_on(
            backend
                .handle
                .subscriptions
                .request(Request::Settings)
                .unwrap(),
        )
        .unwrap()
        .unwrap()
    else {
        panic!()
    };
    runtime
        .block_on(
            backend
                .handle
                .subscriptions
                .request(Request::SaveSettings {
                    revision,
                    settings: saved_defaults,
                })
                .unwrap(),
        )
        .unwrap()
        .unwrap();
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), rule_commands.message()).await
            })
            .is_err(),
        "Bulk subscription edits send no implicit policy commands"
    );
    click(&mut cx, window, "subs-next");
    flush(&mut cx);
    assert_eq!(current_page(&cx).2, [last]);
    select(&mut cx, last);
    click(&mut cx, window, "subs-remove");
    flush(&mut cx);
    click(&mut cx, window, "subs-remove-confirm");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.subscriptions
                .page
                .as_ref()
                .is_some_and(|page| page.total == 64 && page.offset == 0)
        })
    });
    assert_eq!(current_page(&cx).1, 64);
    assert_eq!(
        current_page(&cx).0,
        0,
        "Removing the final row of a later page returns to the last existing page"
    );
    assert_eq!(current_page(&cx).2.len(), 64);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("14-last-page-removal-clamped.json"),
    );
    let imports = (0..20).map(|i| serde_json::json!({
        "name":if i==0 {"Alpha concurrent".to_owned()}else{format!("Imported {i:03}")},
        "url":format!("{url}/imported-{i}"),"filename":format!("imported-{i}-domains.txt"),"format":"domains","groups":["Imported"],"group":"Legacy","enabled":false,"interval":"2h","timeout":3,"timeout_units":"seconds","max_size":2,"max_size_units":"KB","future_source":i
    })).collect::<Vec<_>>();
    let import_file = evidence.join("owned-import-action.json");
    let source_action = serde_json::json!({"name":"ownedGPUAction","x-envelope":"preserved","actions":{"other_plugin":{"enabled":true,"marker":"never execute"},"list_subscriptions":{"enabled":false,"config":{"lists_dir":"/not/adopted/by/rustleblower","interval":3,"interval_units":"hours","timeout":3,"timeout_units":"seconds","max_size":2,"max_size_units":"KB","user_agent":"Rustleblower GPU config import","notify":{"success":{"desktop":"GPU source success"}},"subscriptions":imports}}}});
    std::fs::write(
        &import_file,
        serde_json::to_vec_pretty(&source_action).unwrap(),
    )
    .unwrap();
    let invalid_file = evidence.join("invalid-owned-import.json");
    std::fs::write(&invalid_file, b"{invalid").unwrap();
    let before = row(1);
    let old_settings = match runtime
        .block_on(
            backend
                .handle
                .subscriptions
                .request(Request::Settings)
                .unwrap(),
        )
        .unwrap()
        .unwrap()
    {
        Reply::Settings(_, settings) => settings,
        _ => panic!(),
    };
    let old_requests = requests.lock().unwrap().len();
    click(&mut cx, window, "subs-clear-marks");
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-t", cx))
        .unwrap();
    flush(&mut cx);
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        invalid_file.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-review");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().1.is_some())
    });

    assert!(cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().1.is_some()));
    assert_eq!(query(Query::default()).total, 64);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("40-invalid-config-import.json"),
    );
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        import_file.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-review");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0)) == (Some((20, 19, 1)))
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0),
        Some((20, 19, 1))
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("41-import-name-conflict.json"),
    );
    click(&mut cx, window, "subs-transfer-back");
    flush(&mut cx);
    click(&mut cx, window, "subs-transfer-replace");
    click(&mut cx, window, "subs-transfer-defaults");
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-enter", cx);
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0)) == (Some((20, 20, 0)))
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0),
        Some((20, 20, 0))
    );
    click(&mut cx, window, "subs-transfer-next");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("42-import-review-second-page.json"),
    );
    click(&mut cx, window, "subs-transfer-back");
    flush(&mut cx);
    click(&mut cx, window, "subs-transfer-review");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0)) == (Some((20, 20, 0)))
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0),
        Some((20, 20, 0)),
        "Back preserves file, replacement and default options"
    );
    click(&mut cx, window, "subs-transfer-cancel");
    flush(&mut cx);
    assert_eq!(row(1).spec, before.spec);
    assert_eq!(query(Query::default()).total, 64);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("48-cancel-import-restores-catalogue.json"),
    );
    assert!(
        !cx.read_entity(&view, |v, _| v.subscriptions.has_transfer()),
        "Cancel must return to the actual catalogue"
    );
    click(&mut cx, window, "subs-transfer");
    flush(&mut cx);
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        import_file.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-replace");
    click(&mut cx, window, "subs-transfer-defaults");
    click(&mut cx, window, "subs-transfer-review");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    let beta = row(2);
    let mut concurrent = beta.spec;
    concurrent.name = "Beta concurrent".into();
    runtime
        .block_on(
            backend
                .handle
                .subscriptions
                .request(Request::Save {
                    previous: Some((2, beta.revision)),
                    spec: concurrent.clone(),
                })
                .unwrap(),
        )
        .unwrap()
        .unwrap();
    click(&mut cx, window, "subs-transfer-apply");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.subscriptions
                .transfer_state()
                .1
                .is_some_and(|message| message.contains("changed since review"))
        })
    });

    assert!(cx.read_entity(&view, |v, _| {
        v.subscriptions
            .transfer_state()
            .1
            .unwrap()
            .contains("changed since review")
    }));
    assert_eq!(row(1).spec, before.spec);
    assert_eq!(query(Query::default()).total, 64);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("43-stale-import-refused.json"),
    );
    click(&mut cx, window, "subs-transfer-rebuild");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-enter", cx))
        .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    wait_for(&mut cx, |_| query(Query::default()).total == 83);
    assert_eq!(query(Query::default()).total, 83);
    assert_eq!(row(2).spec, concurrent);
    assert_eq!(row(1).spec.groups, ["imported", "legacy"]);
    assert_eq!(
        crate::subscriptions::model::effective(&row(1).spec, &query(Query::default()).settings.1)
            .unwrap(),
        (7200, 3, 2048)
    );
    assert_eq!(
        std::fs::read(before.file.as_ref().unwrap()).unwrap(),
        retained_bytes
    );
    let imported_settings = match runtime
        .block_on(
            backend
                .handle
                .subscriptions
                .request(Request::Settings)
                .unwrap(),
        )
        .unwrap()
        .unwrap()
    {
        Reply::Settings(_, settings) => settings,
        _ => panic!(),
    };
    assert_eq!(
        (
            imported_settings.interval,
            imported_settings.timeout,
            imported_settings.max_bytes
        ),
        (10800, 3, 2048)
    );
    assert!(!imported_settings.automatic);
    assert_ne!(old_settings, imported_settings);
    assert_eq!(
        requests.lock().unwrap().len(),
        old_requests,
        "Import must not download while updates are paused"
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("44-imported-configuration.json"),
    );
    mark(&mut cx, 1);
    click(&mut cx, window, "subs-transfer");
    flush(&mut cx);
    let native_export = evidence.join("selected-native-config.json");
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        native_export.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-export");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    let native = crate::subscriptions::transfer::read(&native_export, &imported_settings).unwrap();
    assert_eq!(native.subscriptions, [row(1).spec]);
    assert_eq!(
        native.opensnitch["actions"]["list_subscriptions"]["config"]["subscriptions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("45-selected-native-export.json"),
    );
    let exported_bytes = std::fs::read(&native_export).unwrap();
    click(&mut cx, window, "subs-transfer-export");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().1.is_some())
    });

    assert!(cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().1.is_some()));
    assert_eq!(std::fs::read(&native_export).unwrap(), exported_bytes);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("46-export-preserves-existing-file.json"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    flush(&mut cx);
    fill(&mut cx, window, "subs-search", "imported");
    flush(&mut cx);
    assert_eq!(current_page(&cx).1, 20);
    click(&mut cx, window, "subs-transfer");
    flush(&mut cx);
    let action_export = evidence.join("matching-opensnitch-action.json");
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        action_export.to_str().unwrap(),
    );
    pick(
        &mut cx,
        window,
        "subs-transfer-encoding",
        "OpenSnitch action file",
    );
    click(&mut cx, window, "subs-transfer-export");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    let action: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&action_export).unwrap()).unwrap();
    assert_eq!(
        action["actions"]["list_subscriptions"]["config"]["subscriptions"]
            .as_array()
            .unwrap()
            .len(),
        20
    );
    assert_eq!(action["x-envelope"], "preserved");
    assert_eq!(action["actions"]["other_plugin"]["marker"], "never execute");
    checkpoint(
        &mut cx,
        window,
        &evidence.join("47-filtered-opensnitch-export.json"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    flush(&mut cx);
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), rule_commands.message()).await
            })
            .is_err(),
        "Import/export sends no policy commands"
    );
    // Metadata is bound to source IDs across ordinary edits and duplicate names.
    fill(&mut cx, window, "subs-search", "imported");
    flush(&mut cx);
    select(&mut cx, 1);
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    fill(&mut cx, window, "subs-name", "Renamed imported");
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    mark(&mut cx, 1);
    click(&mut cx, window, "subs-transfer");
    flush(&mut cx);
    let renamed_export = evidence.join("renamed-source-native.json");
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        renamed_export.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-export");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    let renamed =
        crate::subscriptions::transfer::read(&renamed_export, &imported_settings).unwrap();
    assert_eq!(renamed.subscriptions[0].name, "Renamed imported");
    assert_eq!(
        crate::subscriptions::transfer::metadata_records(&renamed).unwrap()[0]
            .as_ref()
            .unwrap()["future_source"],
        0
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("49-renamed-identity-keeps-extra-fields.json"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    flush(&mut cx);
    click(&mut cx, window, "subs-add");
    flush(&mut cx);
    fill(&mut cx, window, "subs-name", "Renamed imported");
    fill(
        &mut cx,
        window,
        "subs-url",
        &format!("{url}/another-source"),
    );
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    fill(&mut cx, window, "subs-search", "Renamed imported");
    flush(&mut cx);
    assert_eq!(current_page(&cx).1, 2);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("50-duplicate-display-names.json"),
    );
    click(&mut cx, window, "subs-transfer");
    flush(&mut cx);
    let duplicates_export = evidence.join("duplicate-name-native.json");
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        duplicates_export.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-export");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    let duplicates =
        crate::subscriptions::transfer::read(&duplicates_export, &imported_settings).unwrap();
    assert_eq!(duplicates.subscriptions.len(), 2);
    assert!(
        duplicates
            .subscriptions
            .iter()
            .all(|s| s.name == "Renamed imported")
    );
    let metadata = crate::subscriptions::transfer::metadata_records(&duplicates).unwrap();
    assert_eq!(metadata[0].as_ref().unwrap()["future_source"], 0);
    assert!(metadata[1].as_ref().unwrap().get("future_source").is_none());
    checkpoint(
        &mut cx,
        window,
        &evidence.join("51-duplicate-native-export-bindings.json"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    flush(&mut cx);
    let replacement = evidence.join("complete-catalogue.json");
    let replacement_json = serde_json::json!({"name":"ownedReplacement","actions":{"list_subscriptions":{"enabled":false,"config":{"subscriptions":[{"name":"Shared catalogue","url":format!("{url}/replacement-one"),"filename":"one-ips.txt","format":"ips","enabled":false,"future_source":"first"},{"name":"Shared catalogue","url":format!("{url}/replacement-two"),"filename":"two-ips.txt","format":"ips","enabled":false,"future_source":"second"}]}}}});
    std::fs::write(
        &replacement,
        serde_json::to_vec_pretty(&replacement_json).unwrap(),
    )
    .unwrap();
    let before_replacement = query(Query::default()).total;
    assert_eq!(before_replacement, 84);
    click(&mut cx, window, "subs-transfer");
    flush(&mut cx);
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        replacement.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-catalogue");
    click(&mut cx, window, "subs-transfer-review");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0)) == (Some((2, 86, 0)))
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0),
        Some((2, 86, 0))
    );
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_removals()),
        Some(84)
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("52-complete-catalogue-removal-review.json"),
    );
    click(&mut cx, window, "subs-transfer-next");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("53-retained-removals-second-page.json"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    flush(&mut cx);
    assert_eq!(query(Query::default()).total, before_replacement);
    assert!(!row(1).removed);
    assert_eq!(std::fs::read(&retained).unwrap(), retained_bytes);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("54-cancel-complete-replacement.json"),
    );
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-t", cx))
        .unwrap();
    flush(&mut cx);
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        replacement.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-catalogue");
    click(&mut cx, window, "subs-transfer-review");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-enter", cx))
        .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    fill(&mut cx, window, "subs-search", "");
    flush(&mut cx);
    assert_eq!(current_page(&cx).1, 2);
    assert!(cx.read_entity(&view, |v, _| {
        v.subscriptions
            .page
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .all(|r| r.spec.name == "Shared catalogue")
    }));
    assert!(row(1).removed);
    assert_eq!(std::fs::read(&retained).unwrap(), retained_bytes);
    assert_eq!(requests.lock().unwrap().len(), old_requests);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("55-complete-catalogue-applied.json"),
    );
    click(&mut cx, window, "subs-removed");
    flush(&mut cx);
    assert_eq!(current_page(&cx).1, 85);
    select(&mut cx, 1);
    click(&mut cx, window, "subs-edit");
    flush(&mut cx);
    click(&mut cx, window, "subs-save");
    flush(&mut cx);
    click(&mut cx, window, "subs-membership-confirm");
    flush(&mut cx);
    click(&mut cx, window, "subs-removed");
    flush(&mut cx);
    fill(&mut cx, window, "subs-search", "Renamed imported");
    flush(&mut cx);
    assert_eq!(current_page(&cx).1, 1);
    mark(&mut cx, 1);
    click(&mut cx, window, "subs-transfer");
    flush(&mut cx);
    let restored_export = evidence.join("restored-source-native.json");
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        restored_export.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-export");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.subscriptions.test_operation_idle())
    });
    let restored =
        crate::subscriptions::transfer::read(&restored_export, &imported_settings).unwrap();
    assert_eq!(
        crate::subscriptions::transfer::metadata_records(&restored).unwrap()[0]
            .as_ref()
            .unwrap()["future_source"],
        0
    );
    assert_eq!(row(1).file.as_ref(), Some(&retained));
    assert_eq!(std::fs::read(&retained).unwrap(), retained_bytes);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("56-restored-id-retains-data-and-metadata.json"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    flush(&mut cx);
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), rule_commands.message()).await
            })
            .is_err(),
        "Identity edits and complete replacement must not rewrite daemon policy"
    );
    assert!(requests.lock().unwrap().iter().all(|r| {
        r.contains("User-Agent: Rustleblower GPU fixture")
            || r.to_lowercase()
                .contains("user-agent: rustleblower gpu fixture")
    }));
    stop.store(true, Ordering::Relaxed);
    release.store(true, Ordering::Relaxed);
    http.join().unwrap();
    drop(daemon);
    drop(view);
    shutdown(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-empty-subscriptions.json","02-invalid-add-preserved.json","03-downloaded-list.json","04-downloaded-text-preview.json","05-conditional-and-forced-refresh.json","06-download-error.json","07-permission-during-slow-http.json","08-cancelled-download.json","09-edited-list-format.json","10-filtered-empty-state.json","11-removed-data-retained.json","12-restored-status-history.json","13-subscription-second-page.json","14-last-page-removal-clamped.json","15-compatible-list-rule-review.json","16-list-rule-waits-for-confirmation.json","17-permission-under-list-rule.json","18-associated-list-rules.json","19-associated-rules-second-page.json","20-associated-rule-rejection.json","21-empty-rule-collections.json","22-format-membership-review.json","23-restored-membership-review.json","24-group-rule-review.json","25-group-associated-rules.json","26-group-membership-review.json","27-group-removed-from-collections.json","28-collection-second-page.json","29-collection-sorted-members.json","30-collection-filter-full-catalogue.json","31-associated-snapshot-changed.json","32-membership-back-preserves-draft.json","33-marks-across-pages.json","34-reviewed-group-batch.json","35-stale-bulk-refused.json","36-selected-group-batch-applied.json","37-all-matching-review-page-two.json","38-all-pages-disabled.json","39-selected-inherited-and-custom-limits.json","40-invalid-config-import.json","41-import-name-conflict.json","42-import-review-second-page.json","43-stale-import-refused.json","44-imported-configuration.json","45-selected-native-export.json","46-export-preserves-existing-file.json","47-filtered-opensnitch-export.json","48-cancel-import-restores-catalogue.json","49-renamed-identity-keeps-extra-fields.json","50-duplicate-display-names.json","51-duplicate-native-export-bindings.json","52-complete-catalogue-removal-review.json","53-retained-removals-second-page.json","54-cancel-complete-replacement.json","55-complete-catalogue-applied.json","56-restored-id-retains-data-and-metadata.json","57-bulk-independent-count-units.json","58-bulk-effective-review.json","59-bulk-stale-defaults-refused.json","60-bulk-rebuilt-defaults.json","61-bulk-independent-limits-applied.json"],"proof_boundary":"Owned network namespace and HTTP fixture; actual native Kit empty/add/edit/default/format controls, conditional/forced refresh, error/cancel, independent AskRule during held HTTP, full-catalogue sort/search/paging/last-page-removal/preview/log/copy/remove/restore with durable files and metadata. Local list-rule review/cancel, exact held-ACK creation, confirmed-only associated-rule paging beyond a full catalogue page, nested-condition edit/rejection and independent permission input are included. Collection empty/search/paging/global sorts, exact UID-scoped group rule creation/inspection, stale snapshot refusal/refresh and reviewed membership changes are included. Bulk marking across pages, selected/group and whole-matching-set edits, immutable review/back/cancel, concurrent-edit refusal/rebuild, group-cap rejection, explicit and independently inherited count/unit limits, effective reviews, defaults-revision refusal/rebuild and no implicit policy commands are included. Actual import/export input adds invalid-file refusal, exact-name conflict/replacement, defaults, paged review/back/cancel, stale catalogue refusal/rebuild, canonical migration and retained data, selected native and full-filter OpenSnitch exports, existing-file preservation, unknown metadata and no paused downloads or policy commands. Per-ID opaque fields survive ordinary rename, duplicate display names, selected/native export, complete catalogue review/paging/cancel/apply and restoration of retained data. A real 84-source removal preserves old files and sends no policy commands. Notifications, retired-data cleanup, native compositor and resource proof remain separate."})).unwrap()).unwrap();
}
