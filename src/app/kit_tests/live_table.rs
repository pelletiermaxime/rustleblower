use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::live::presentation::{self, Field};
    use gpui::InputEvent as _;
    fn control(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<gpui::ElementId>) {
        let id = id.into();
        let close = matches!(&id,gpui::ElementId::Name(n) if n.as_ref()=="live-tools-close");
        if !close {
            for _ in 0..48 {
                let delta = cx
                    .update_window(w, |_, w, cx| {
                        w.render_frame(cx);
                        let body = w.find("live-tools-scroll").bounds();
                        let target = w.find(id.clone()).bounds();
                        if target.center().y < body.top() + px(6.) {
                            Some(120.)
                        } else if target.center().y > body.bottom() - px(6.) {
                            Some(-120.)
                        } else {
                            None
                        }
                    })
                    .unwrap();
                let Some(delta) = delta else { break };
                cx.update_window(w, |_, w, cx| {
                    let position = w.find("live-tools-scroll").bounds().center();
                    w.dispatch_event(
                        gpui::MouseMoveEvent {
                            position,
                            pressed_button: None,
                            modifiers: Default::default(),
                        }
                        .to_platform_input(),
                        cx,
                    );
                    w.render_frame(cx);
                    w.dispatch_event(
                        gpui::ScrollWheelEvent {
                            position,
                            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(delta))),
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
        }
        cx.update_window(w, |_, w, cx| w.click(id, cx)).unwrap();
        flush(cx);
    }
    fn input(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        id: impl Into<gpui::ElementId>,
        text: &str,
    ) {
        let id = id.into();
        control(cx, w, id.clone());
        cx.update_window(w, |_, w, cx| {
            w.press("ctrl-a", cx);
            w.input(text, cx)
        })
        .unwrap();
        flush(cx);
    }
    fn open(cx: &mut TestAppContext, main: AnyWindowHandle, id: &'static str) -> AnyWindowHandle {
        let before = cx.update(|cx| cx.windows());
        click(cx, main, id);
        flush(cx);
        cx.update(|cx| {
            cx.windows()
                .into_iter()
                .find(|w| !before.contains(w))
                .unwrap()
        })
    }
    fn mark(cx: &mut TestAppContext, w: AnyWindowHandle, index: usize, shift: bool) {
        cx.update_window(w, |_, w, cx| {
            w.render_frame(cx);
            let target = w.find(("live-mark", index as u64));
            assert!(target.visible());
            let position = target.bounds().center();
            let modifiers = gpui::Modifiers {
                shift,
                ..Default::default()
            };
            w.dispatch_event(
                gpui::MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }
                .to_platform_input(),
                cx,
            );
            w.render_frame(cx);
            w.dispatch_event(
                gpui::MouseDownEvent {
                    button: gpui::MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            w.render_frame(cx);
            w.dispatch_event(
                gpui::MouseUpEvent {
                    button: gpui::MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
            w.render_frame(cx);
        })
        .unwrap();
        flush(cx);
    }
    fn data(extra: bool) -> String {
        let ports = (1..=650).chain(extra.then_some(999));
        serde_json::json!({"Table":ports.map(|port|serde_json::json!({"PID":42,"Proto":6,"Iface":"owned0","Future":18446744073709551615_u64,"Socket":{"ID":{"Source":"192.0.2.1","Destination":"198.51.100.2","SourcePort":10000+port,"DestinationPort":port,"Cookie":[1,2]},"Family":2,"State":1,"UID":1000,"INode":5000+port,"RQueue":7,"WQueue":9}})).collect::<Vec<_>>(),"Processes":{"42":{"Path":"/owned/live-table","Comm":"owned-live"}}}).to_string()
    }
    fn file(cx: &mut TestAppContext, p: &Path) {
        wait_for(cx, |_cx| p.exists());
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
    let (daemon, replies, mut commands) = runtime.block_on(async {
        use hyper_util::rt::TokioIo;
        let socket = config.socket.clone();
        let channel = tonic::transport::Endpoint::try_from("http://localhost")
            .unwrap()
            .connect_with_connector(tower::service_fn(move |_| {
                let socket = socket.clone();
                async move {
                    tokio::net::UnixStream::connect(socket)
                        .await
                        .map(TokioIo::new)
                }
            }))
            .await
            .unwrap();
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "live-table-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let commands = daemon
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        (daemon, tx, commands)
    });
    let mut cx = context(test_cx);
    let mut view = None;
    let main = cx
        .open_test_window(size(px(1320.), px(980.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = view.unwrap();
    cx.update_window(main, |_, w, cx| w.press("ctrl-6", cx))
        .unwrap();
    wait_for(&mut cx, |_cx| {
        backend.handle.live_presentation.snapshot().ready
    });
    let tools = open(&mut cx, main, "live-view-tools");
    for index in 0..13u64 {
        if ![6, 9].contains(&index) {
            control(&mut cx, tools, ("live-column", index));
        }
    }
    input(&mut cx, tools, ("live-width", 6u64), "333");
    input(&mut cx, tools, ("live-width", 9u64), "111");
    control(&mut cx, tools, "live-view-save");
    wait_for(&mut cx, |_cx| {
        backend
            .handle
            .live_presentation
            .snapshot()
            .settings
            .columns
            .len()
            == 2
    });
    let saved = backend.handle.live_presentation.snapshot();
    assert_eq!(
        saved.settings.columns,
        [
            presentation::Column {
                field: Field::DestinationPort,
                width: 333
            },
            presentation::Column {
                field: Field::Pid,
                width: 111
            }
        ]
    );
    assert!(!cx.read_entity(&view, |v, _| v.live.snapshot.running));
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("01-live-saved-two-columns.json"),
    );
    drop(saved);
    control(&mut cx, tools, "live-tools-close");
    click(&mut cx, main, "live-start");
    let start = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert!(
        start.id > 10_000,
        "OpenSnitch task stream requires an ID above10000"
    );
    assert_eq!(start.r#type, 13);
    let spec: serde_json::Value = serde_json::from_str(&start.data).unwrap();
    assert_eq!(spec["name"], "sockets-monitor");
    let mut permission = daemon.clone();
    let ask = runtime.spawn(async move {
        permission
            .ask_rule(protocol::Connection {
                process_path: "/owned/live-table-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    let mut popup = None;
    wait_for(&mut cx, |cx| {
        popup = cx.read_entity(&view, |v, _| v.prompt_window);
        popup.is_some()
    });
    let popup = popup.unwrap();
    checkpoint(
        &mut cx,
        popup,
        &evidence.join("02-live-held-update-permission.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(ask).unwrap().action, "deny");
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: start.id,
            data: data(false),
            ..Default::default()
        }))
        .unwrap();
    for _ in 0..60 {
        flush(&mut cx);
        if cx.read_entity(&view,|v,_|matches!(&v.live.parsed,Some(crate::app::live_view::View::Sockets{data,..})if data.rows.len()==650)){break}
    }
    let head_width = cx
        .update_window(main, |_, w, cx| {
            w.render_frame(cx);
            f32::from(
                w.within("table")
                    .find(("col-header", 0usize))
                    .bounds()
                    .size
                    .width,
            )
        })
        .unwrap();
    assert_eq!(head_width, 333.);
    fit_table_column(&mut cx, main, 0);
    // Pinned Kit cycles Default -> Descending -> Ascending -> Default.
    // Two actual clicks reach Ascending from the initially unsorted header.
    for _ in 0..2 {
        cx.update_window(main, |_, w, cx| {
            w.within("table").click(("icon-sort", 0usize), cx)
        })
        .unwrap();
        flush(&mut cx);
    }
    assert_eq!(
        cx.read_entity(&view, |v, _| {
            let Some(crate::app::live_view::View::Sockets { data, matches }) = &v.live.parsed
            else {
                panic!()
            };
            Field::DestinationPort.text(&data.rows[matches[0]])
        }),
        "1"
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("03-live-global-numeric-ascending.json"),
    );
    // Kit emits Default after Ascending. The always-sorted Live adapter must
    // turn this into Descending, rather than refresh back to stuck Ascending.
    cx.update_window(main, |_, w, cx| {
        w.within("table").click(("icon-sort", 0usize), cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |v, _| {
            let Some(crate::app::live_view::View::Sockets { data, matches }) = &v.live.parsed
            else {
                panic!()
            };
            Field::DestinationPort.text(&data.rows[matches[0]])
        })) == ("650")
    });

    assert_eq!(
        cx.read_entity(&view, |v, _| {
            let Some(crate::app::live_view::View::Sockets { data, matches }) = &v.live.parsed
            else {
                panic!()
            };
            Field::DestinationPort.text(&data.rows[matches[0]])
        }),
        "650"
    );
    checkpoint(
        &mut cx,
        main,
        &evidence.join("04-live-global-numeric-descending.json"),
    );
    mark(&mut cx, main, 649, false);
    click(&mut cx, main, "live-next");
    flush(&mut cx);
    mark(&mut cx, main, 449, true);
    checkpoint(
        &mut cx,
        main,
        &evidence.join("05-live-cross-page-range.json"),
    );
    let output = open(&mut cx, main, "live-selected-output");
    let selected_csv = evidence.join("selected-live.csv");
    input(
        &mut cx,
        output,
        "live-output-path",
        selected_csv.to_str().unwrap(),
    );
    control(&mut cx, output, "live-output-save");
    file(&mut cx, &selected_csv);
    let csv = std::fs::read_to_string(&selected_csv).unwrap();
    assert_eq!(csv.lines().count(), 202);
    assert_eq!(csv.lines().next().unwrap().split(',').count(), 2);
    assert!(csv.lines().nth(1).unwrap().starts_with("\"650\""));
    assert!(csv.lines().last().unwrap().starts_with("\"450\""));
    control(&mut cx, output, "live-output-copy");
    wait_for(&mut cx, |cx| {
        (cx.read_from_clipboard().and_then(|i| i.text())).as_deref() == Some(csv.as_str())
    });
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        csv
    );
    checkpoint(
        &mut cx,
        output,
        &evidence.join("06-live-selected-csv-copy.json"),
    );
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: start.id,
            data: data(true),
            ..Default::default()
        }))
        .unwrap();
    for _ in 0..60 {
        flush(&mut cx);
        if cx.read_entity(&view,|v,_|matches!(&v.live.parsed,Some(crate::app::live_view::View::Sockets{data,..})if data.rows.len()==651)){break}
    }
    let windows = cx.update(|cx| cx.windows());
    click(&mut cx, main, "live-selected-output");
    wait_for(&mut cx, |cx| (cx.windows()) == (windows));

    assert_eq!(
        cx.update(|cx| cx.windows()),
        windows,
        "New parsed snapshot clears marks; disabled selected-output leaves captured window intact"
    );
    control(&mut cx, output, "live-output-copy");
    wait_for(&mut cx, |cx| {
        (cx.read_from_clipboard().unwrap().text().unwrap()) == (csv)
    });

    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        csv
    );
    control(&mut cx, output, "live-output-format");
    cx.update_window(output, |_, w, cx| {
        w.press("down", cx);
        w.press("enter", cx)
    })
    .unwrap();
    flush(&mut cx);
    let selected_json = evidence.join("selected-live.json");
    input(
        &mut cx,
        output,
        "live-output-path",
        selected_json.to_str().unwrap(),
    );
    control(&mut cx, output, "live-output-save");
    file(&mut cx, &selected_json);
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&selected_json).unwrap()).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 201);
    assert!(json.as_array().unwrap().iter().all(|r| {
        r["Socket"]["ID"]["DestinationPort"].as_u64().unwrap() >= 450
            && r["Socket"]["ID"]["DestinationPort"] != 999
    }));
    assert_eq!(json[0]["Future"].as_u64(), Some(u64::MAX));
    checkpoint(
        &mut cx,
        output,
        &evidence.join("07-live-captured-json-after-new-update.json"),
    );
    let all = open(&mut cx, main, "live-all-output");
    assert!(!cx.update(|cx| cx.windows()).contains(&output));
    let all_csv = evidence.join("all-live.csv");
    input(&mut cx, all, "live-output-path", all_csv.to_str().unwrap());
    control(&mut cx, all, "live-output-save");
    file(&mut cx, &all_csv);
    let all_text = std::fs::read_to_string(&all_csv).unwrap();
    assert_eq!(all_text.lines().count(), 652);
    assert!(all_text.contains("\"999\""));
    checkpoint(
        &mut cx,
        all,
        &evidence.join("08-live-new-capture-all-output.json"),
    );
    control(&mut cx, all, "live-tools-close");
    click(&mut cx, main, "live-pause");
    let stop = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(stop.r#type, 14);
    assert_eq!(stop.data, start.data);
    let tools = open(&mut cx, main, "live-view-tools");
    input(&mut cx, tools, ("live-width", 6u64), "444");
    let path = presentation::path_for(&config.database);
    let revision = backend.handle.live_presentation.snapshot().revision.clone();
    let mut external = b"\n".to_vec();
    external.extend(std::fs::read(&path).unwrap());
    std::fs::write(&path, &external).unwrap();
    control(&mut cx, tools, "live-view-save");
    assert_eq!(std::fs::read(&path).unwrap(), external);
    assert_eq!(
        backend.handle.live_presentation.snapshot().revision,
        revision
    );
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("09-live-stale-settings-refused.json"),
    );
    control(&mut cx, tools, "live-tools-close");
    click(&mut cx, main, "live-view-reload");
    wait_for(&mut cx, |_cx| {
        backend.handle.live_presentation.snapshot().revision != revision
    });
    assert_ne!(
        backend.handle.live_presentation.snapshot().revision,
        revision
    );
    std::fs::write(evidence.join("live-table-proof.json"),serde_json::to_vec_pretty(&serde_json::json!({"initial_rows":650,"new_rows":651,"selected_range_rows":201,"selected_columns":["destination-port","pid"],"actual_first_column_logical_width":head_width,"csv_clipboard_exact":true,"json_unknown_u64":u64::MAX,"new_capture_contains_999_old_capture_excludes_it":true,"commands":[{"type":start.r#type,"data":start.data},{"type":stop.r#type,"data":stop.data}]})).unwrap()).unwrap();
    drop(view);
    shutdown(cx);
    drop(daemon);
    drop(replies);
    drop(commands);
    backend.join();
    assert!(!config.socket.exists());
    let backend = Backend::start(config.clone()).unwrap();
    let mut cx = context(test_cx);
    let mut reopened = None;
    let main = cx
        .open_test_window(size(px(1320.), px(980.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            reopened = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = reopened.unwrap();
    cx.update_window(main, |_, w, cx| w.press("ctrl-6", cx))
        .unwrap();
    wait_for(&mut cx, |_cx| {
        backend.handle.live_presentation.snapshot().ready
    });
    assert!(!cx.read_entity(&view, |v, _| v.live.snapshot.running));
    let saved = backend.handle.live_presentation.snapshot();
    assert_eq!(
        saved.settings.columns,
        [
            presentation::Column {
                field: Field::DestinationPort,
                width: 333
            },
            presentation::Column {
                field: Field::Pid,
                width: 111
            }
        ]
    );
    drop(saved);
    let tools = open(&mut cx, main, "live-view-tools");
    control(&mut cx, tools, ("live-width", 6u64));
    assert_eq!(
        cx.update_window(tools, |_, w, _| w
            .find(("live-width", 6u64))
            .value()
            .unwrap()
            .to_string())
            .unwrap(),
        "333"
    );
    checkpoint(
        &mut cx,
        tools,
        &evidence.join("10-live-fresh-backend-saved-stopped.json"),
    );
    drop(view);
    shutdown(cx);
    backend.join();
    drop(runtime);
    assert!(!config.socket.exists());
    let checkpoints = std::fs::read_dir(&evidence)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".json"))
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":checkpoints,"proof_boundary":"Actual Kit numeric global sort, pointer/Shift acrosspages, selected/full capturedCSV/rawJSON and clipboard, acceptedupdate isolation, save/stale/reload and freshbackend/GPUI stoppedrestore on controlled650/651ownedRPC socket fixture. No host kernel/daemon/policy, OS-process restart, compositor/package/RSS proof."})).unwrap()).unwrap();
}
