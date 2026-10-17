use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::statistics::{self, Kind};

    use gpui::InputEvent as _;
    fn output_control(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<gpui::ElementId>) {
        let id = id.into();
        let footer = matches!(&id,gpui::ElementId::Name(name) if matches!(name.as_ref(),"statistics-export-close"|"statistics-export-stop"));
        if !footer {
            for _ in 0..32 {
                let delta = cx
                    .update_window(w, |_, w, cx| {
                        w.render_frame(cx);
                        let body = w.find("statistics-export-panel").bounds();
                        let target = w.find(id.clone()).bounds();
                        if target.center().y < body.top() + px(6.) {
                            Some(100.)
                        } else if target.center().y > body.bottom() - px(6.) {
                            Some(-100.)
                        } else {
                            None
                        }
                    })
                    .unwrap();
                let Some(delta) = delta else { break };
                cx.update_window(w, |_, w, cx| {
                    let position = w.find("statistics-export-panel").bounds().center();
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
    fn mark(cx: &mut TestAppContext, w: AnyWindowHandle, position_index: usize, shift: bool) {
        cx.update_window(w, |_, w, cx| {
            w.render_frame(cx);
            let target = w.find(("statistics-mark", position_index));
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
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (mut daemon, other) = runtime.block_on(async {
        use hyper_util::rt::TokioIo;
        async fn connect(
            path: PathBuf,
        ) -> protocol::ui_client::UiClient<tonic::transport::Channel> {
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
            protocol::ui_client::UiClient::new(channel)
        }
        let mut other = connect(config.socket.clone()).await;
        other
            .subscribe(protocol::ClientConfig {
                name: "other-node".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        other
            .ping(protocol::PingRequest {
                stats: Some(protocol::Statistics {
                    uptime: 100,
                    by_executable: [("/bin/shared".into(), 5)].into(),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
        let mut daemon = connect(config.socket.clone()).await;
        daemon
            .subscribe(protocol::ClientConfig {
                name: "stats-node".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        (daemon, other)
    });
    fn report() -> protocol::Statistics {
        protocol::Statistics {
            uptime: 100,
            connections: 9000,
            accepted: 9876,
            dropped: 123,
            by_executable: (0..350u64)
                .map(|i| (format!("/bin/stats-{i:03}"), i + 1))
                .collect(),
            by_host: [("api.example.test".into(), 234)].into(),
            by_address: [("192.0.2.42".into(), 345)].into(),
            by_port: [("443".into(), 456)].into(),
            by_uid: [("1000".into(), 567)].into(),
            by_proto: [("tcp".into(), 678)].into(),
            events: vec![protocol::Event {
                unixnano: crate::history::now_nanos(),
                connection: Some(protocol::Connection {
                    process_path: "/bin/stats-000".into(),
                    dst_host: "api.example.test".into(),
                    dst_ip: "192.0.2.42".into(),
                    dst_port: 443,
                    user_id: 1000,
                    protocol: "tcp".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    runtime.block_on(async {
        daemon
            .ping(protocol::PingRequest {
                stats: Some(report()),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    let mut cx = context(test_cx);
    let mut view = None;
    let window = cx
        .open_test_window(size(px(1180.), px(900.)), |window, cx| {
            let dashboard =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(dashboard.clone());
            cx.new(|cx| gpui_kit::base::Root::new(dashboard, window, cx))
        })
        .unwrap();
    let window: AnyWindowHandle = window.into();
    let view = view.unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("ctrl-3", cx))
        .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.statistics.page.len())) == (200)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page.len()),
        200
    );
    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page[0].hits),
        350
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("01-statistics-applications.json"),
    );
    click(&mut cx, window, "statistics-chart");
    flush(&mut cx);
    checkpoint(&mut cx, window, &evidence.join("02-statistics-chart.json"));
    click(&mut cx, window, "statistics-chart");
    for (index, kind, total) in [
        (1usize, Kind::Hosts, 234),
        (2, Kind::Addresses, 345),
        (3, Kind::Ports, 456),
        (4, Kind::Users, 567),
        (5, Kind::Protocols, 678),
    ] {
        cx.update_window(window, |_, window, cx| {
            window.within("statistics-kinds").click(index, cx)
        })
        .unwrap();
        wait_for(&mut cx, |cx| {
            (cx.read_entity(&view, |view, _| {
                (
                    view.statistics.query.kind,
                    view.statistics.snapshot.as_ref().unwrap().total,
                )
            })) == (kind, total)
        });

        assert_eq!(
            cx.read_entity(&view, |view, _| (
                view.statistics.query.kind,
                view.statistics.snapshot.as_ref().unwrap().total
            )),
            (kind, total)
        );
    }
    checkpoint(
        &mut cx,
        window,
        &evidence.join("03-statistics-protocols.json"),
    );
    cx.update_window(window, |_, window, cx| {
        window.within("statistics-kinds").click(0usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    pick(&mut cx, window, "statistics-node", "Node: stats-node");
    assert_eq!(
        cx.read_entity(&view, |view, _| view
            .statistics
            .snapshot
            .as_ref()
            .unwrap()
            .len()),
        350
    );
    pick(&mut cx, window, "statistics-sort", "Name A–Z");
    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page[0].key.clone()),
        "/bin/stats-000"
    );
    mark(&mut cx, window, 0, false);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("11-statistics-first-mark.json"),
    );
    click(&mut cx, window, "statistics-older");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.statistics.page.len())) == (150)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page.len()),
        150
    );
    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page[0].key.clone()),
        "/bin/stats-200"
    );
    let mut changed = report();
    changed.uptime = 101;
    changed.events.clear();
    changed.by_executable.insert("/bin/stats-200".into(), 999);
    runtime.block_on(async {
        daemon
            .ping(protocol::PingRequest {
                stats: Some(changed),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.statistics.page[0].hits)) == (201)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page[0].hits),
        201
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("04-frozen-statistics-page.json"),
    );
    mark(&mut cx, window, 200, true);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("12-statistics-cross-page-range.json"),
    );
    click(&mut cx, window, "statistics-previous");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.statistics.page.len())) == (200)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page.len()),
        200
    );

    let before_range = cx.update(|cx| cx.windows());
    click(&mut cx, window, "statistics-export");
    flush(&mut cx);
    let range_output = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before_range.contains(w))
            .unwrap()
    });
    cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            "owned-statistics-range-sentinel".into(),
        ))
    });
    output_control(&mut cx, range_output, "statistics-copy-selected");
    let mut range_csv = String::new();
    wait_for(&mut cx, |cx| {
        range_csv = cx.read_from_clipboard().unwrap().text().unwrap();
        range_csv != "owned-statistics-range-sentinel"
    });
    assert_eq!(
        range_csv.lines().count(),
        202,
        "Shift range spans positions0through200 acrosspages"
    );
    assert!(
        range_csv.contains("/bin/stats-000")
            && range_csv.contains("/bin/stats-200")
            && !range_csv.contains("/bin/stats-201")
    );
    std::fs::write(evidence.join("selected-range-counters.csv"), &range_csv).unwrap();
    output_control(&mut cx, range_output, "statistics-export-close");
    click(&mut cx, window, "statistics-select-all");
    flush(&mut cx);
    mark(&mut cx, window, 0, false);
    let windows_before = cx.update(|cx| cx.windows());
    click(&mut cx, window, "statistics-export");
    flush(&mut cx);
    let selected = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !windows_before.contains(w))
            .unwrap()
    });
    output_control(&mut cx, selected, ("statistics-export-column", 2usize));
    output_control(&mut cx, selected, ("statistics-export-column", 3usize));
    output_control(&mut cx, selected, "statistics-export-json");
    let selected_path = evidence.join("selected-frozen-counters.jsonl");
    output_control(&mut cx, selected, "statistics-export-path");
    fill(
        &mut cx,
        selected,
        "statistics-export-path",
        selected_path.to_str().unwrap(),
    );
    output_control(&mut cx, selected, "statistics-export-selected");
    wait_for(&mut cx, |_cx| selected_path.exists());
    let selected_text = std::fs::read_to_string(&selected_path).unwrap();
    let selected_rows = selected_text
        .lines()
        .map(|s| serde_json::from_str::<serde_json::Value>(s).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(selected_rows.len(), 349);
    assert!(
        selected_rows
            .iter()
            .all(|r| r.as_object().unwrap().len() == 2
                && r.get("value").is_some()
                && r.get("hits").is_some())
    );
    assert!(!selected_rows.iter().any(|r| r["value"] == "/bin/stats-000"));
    assert_eq!(
        selected_rows
            .iter()
            .find(|r| r["value"] == "/bin/stats-200")
            .unwrap()["hits"],
        "201"
    );
    output_control(&mut cx, selected, "statistics-copy-selected");
    wait_for(&mut cx, |cx| {
        (cx.read_from_clipboard().and_then(|item| item.text())).as_deref()
            == Some(selected_text.as_str())
    });
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        selected_text
    );
    checkpoint(
        &mut cx,
        selected,
        &evidence.join("13-statistics-selected-frozen-output.json"),
    );
    let mut permission = daemon.clone();
    let ask = runtime.spawn(async move {
        permission
            .ask_rule(protocol::Connection {
                process_path: "/owned/statistics-selection-permission".into(),
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
        &evidence.join("14-statistics-selected-output-permission.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(ask).unwrap().action, "deny");
    output_control(&mut cx, selected, "statistics-export-close");
    std::fs::write(evidence.join("statistics-selection-proof.json"),serde_json::to_vec_pretty(&serde_json::json!({"frozen_counter_rows":350,"range_selected_rows":201,"selected_after_all_minus_one":349,"excluded":"/bin/stats-000","frozen_stats_200_hits":"201","new_report_stats_200_hits":"999","chosen_fields":["value","hits"],"clipboard_equals_file":true,"permission_deny":true})).unwrap()).unwrap();
    click(&mut cx, window, "statistics-latest");
    flush(&mut cx);
    click(&mut cx, window, "statistics-older");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.statistics.page[0].hits)) == (999)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page[0].hits),
        999
    );
    click(&mut cx, window, "statistics-latest");
    flush(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window, "statistics-export");
    flush(&mut cx);
    let export = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    });
    let csv = evidence.join("daemon-counters.csv");
    output_control(&mut cx, export, "statistics-export-path");
    fill(
        &mut cx,
        export,
        "statistics-export-path",
        csv.to_str().unwrap(),
    );
    output_control(&mut cx, export, "statistics-export-save");
    flush(&mut cx);
    let original = std::fs::read_to_string(&csv).unwrap();
    assert_eq!(original.lines().count(), 351);
    output_control(&mut cx, export, "statistics-export-save");
    flush(&mut cx);
    assert_eq!(std::fs::read_to_string(&csv).unwrap(), original);
    checkpoint(
        &mut cx,
        export,
        &evidence.join("05-statistics-existing-export.json"),
    );
    output_control(&mut cx, export, "statistics-export-json");
    let json = evidence.join("daemon-counters.jsonl");
    output_control(&mut cx, export, "statistics-export-path");
    fill(
        &mut cx,
        export,
        "statistics-export-path",
        json.to_str().unwrap(),
    );
    output_control(&mut cx, export, "statistics-export-save");
    flush(&mut cx);
    assert_eq!(std::fs::read_to_string(&json).unwrap().lines().count(), 350);
    output_control(&mut cx, export, "statistics-export-copy");
    flush(&mut cx);
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(copied.lines().count(), 201);
    checkpoint(
        &mut cx,
        export,
        &evidence.join("06-statistics-export-copy.json"),
    );
    output_control(&mut cx, export, "statistics-export-close");
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("stats-000", cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.statistics.page.len())) == (1)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page.len()),
        1
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("statistics-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window, "statistics-history");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.history.page.rows.len())) == (1)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows.len()),
        1
    );
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.query.filters.len()),
        2
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("07-statistics-history-drilldown.json"),
    );
    cx.update_window(window, |_, window, cx| window.press("ctrl-3", cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("statistics-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window, "statistics-reset-selected");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("08-statistics-reset-confirmation.json"),
    );
    click(&mut cx, window, "statistics-reset-cancel");
    assert_eq!(
        statistics::capture(
            &config.database,
            statistics::Query {
                search: "stats-000".into(),
                ..Default::default()
            }
        )
        .unwrap()
        .total,
        1
    );
    click(&mut cx, window, "statistics-reset-selected");
    click(&mut cx, window, "statistics-clear-history");
    click(&mut cx, window, "statistics-reset-confirm");
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.statistics.page[0].hits)) == (0)
    });

    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page[0].hits),
        0
    );
    let history = runtime
        .block_on(
            backend
                .handle
                .history
                .query(crate::history::Query::default())
                .unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        history.rows.len(),
        1,
        "Focused statistics reset removes its own history and preserves the unrelated permission event"
    );
    assert_eq!(
        history.rows[0].process,
        "/owned/statistics-selection-permission"
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("09-statistics-reset-result.json"),
    );
    let mut repeated = report();
    repeated.uptime = 102;
    repeated.events.clear();
    runtime.block_on(async {
        daemon
            .ping(protocol::PingRequest {
                stats: Some(repeated),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    for _ in 0..4 {
        flush(&mut cx);
    }
    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page[0].hits),
        0
    );
    click(&mut cx, window, "statistics-reset-all");
    click(&mut cx, window, "statistics-reset-confirm");
    flush(&mut cx);
    assert_eq!(
        statistics::capture(
            &config.database,
            statistics::Query {
                node: "stats-node".into(),
                ..Default::default()
            }
        )
        .unwrap()
        .total,
        0
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("10-statistics-category-reset.json"),
    );
    drop(view);
    shutdown(cx);
    drop(daemon);
    drop(other);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let mut checkpoints = (1..=10)
        .zip([
            "statistics-applications",
            "statistics-chart",
            "statistics-protocols",
            "frozen-statistics-page",
            "statistics-existing-export",
            "statistics-export-copy",
            "statistics-history-drilldown",
            "statistics-reset-confirmation",
            "statistics-reset-result",
            "statistics-category-reset",
        ])
        .map(|(index, name)| format!("{index:02}-{name}.json"))
        .collect::<Vec<_>>();
    checkpoints.extend(
        [
            "11-statistics-first-mark.json",
            "12-statistics-cross-page-range.json",
            "13-statistics-selected-frozen-output.json",
            "14-statistics-selected-output-permission.json",
        ]
        .into_iter()
        .map(str::to_string),
    );
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&serde_json::json!({"flow":"Actual Kit tabs/selects/chart/list/reset/export controls, RPC absolute counters, frozen paging, CSV/JSON/clipboard, exact history drilldown and local baseline resets"})).unwrap()).unwrap();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":checkpoints,"boundary":"Production GPUI input and gRPC counters against an isolated fixture; no system daemon or firewall changes"})).unwrap()).unwrap();
}
