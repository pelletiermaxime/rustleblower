//! Captured History selection through Kit input and condition waits.
use crate::app::Dashboard;
use crate::{backend::Backend, config::Config, protocol};
use gpui::{AnyWindowHandle, App, AppContext, InputEvent as _, TestAppContext, Window, px, size};
use gpui_kit::{
    component::{Theme, ThemeMode},
    test::{TestAppContextExt, TestWindowExt},
};
use std::{collections::BTreeSet, path::PathBuf, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(10);

fn ui<R>(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    action: impl FnOnce(&mut Window, &mut App) -> R,
) -> R {
    cx.update_window(handle, |_, window, cx| action(window, cx))
        .unwrap()
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    ui(cx, window, |window, cx| window.click(id, cx));
}

// Kit dispatches real wheel input; scrolling needs completed frames, not sleeps.
fn control(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    ui(cx, handle, |window, cx| {
        for _ in 0..32 {
            window.render_frame(cx);
            let panel = window.find("history-tools-panel").bounds();
            let target = window.find(id).bounds();
            let delta = if target.center().y < panel.top() + px(6.) {
                120.
            } else if target.center().y > panel.bottom() - px(6.) {
                -120.
            } else {
                window.click(id, cx);
                return;
            };
            window.scroll(
                "history-tools-panel",
                gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(delta))),
                cx,
            );
        }
        panic!("History tools control {id} did not scroll into view");
    });
}

async fn tools(cx: &mut TestAppContext, main: AnyWindowHandle, tab: usize) -> AnyWindowHandle {
    // The dashboard releases its cached tools handle on its next background tick.
    cx.wait_for(main, TIMEOUT, |window, cx| {
        let view = gpui_kit::base::Root::read(window, cx)
            .view()
            .clone()
            .downcast::<Dashboard>()
            .unwrap();
        view.read(cx).history.test_tools_closed()
    })
    .await;
    let before = cx.update(|cx| cx.windows());
    click(cx, main, "history-tools");
    let mut handle = None;
    cx.wait_for(main, TIMEOUT, |_, cx| {
        handle = cx.windows().into_iter().find(|w| !before.contains(w));
        handle.is_some()
    })
    .await;
    let handle = handle.unwrap();
    ui(cx, handle, |window, cx| {
        window.within("history-tools-tabs").click(tab, cx);
    });
    let target = match tab {
        1 => "history-columns-save",
        2 => "history-export-selected",
        _ => unreachable!(),
    };
    cx.wait_for(handle, TIMEOUT, |window, _| {
        window.try_find(target).is_some()
    })
    .await;
    handle
}

fn mark(cx: &mut TestAppContext, handle: AnyWindowHandle, id: i64, shift: bool) {
    ui(cx, handle, |window, cx| {
        window.render_frame(cx);
        let target = window.find(("history-mark", id as u64));
        assert!(target.visible());
        let position = target.bounds().center();
        let modifiers = gpui::Modifiers {
            shift,
            ..Default::default()
        };
        window.dispatch_event(
            gpui::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
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
        window.render_frame(cx);
        window.dispatch_event(
            gpui::MouseUpEvent {
                button: gpui::MouseButton::Left,
                position,
                modifiers,
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    });
}

async fn copy_disabled(cx: &mut TestAppContext, main: AnyWindowHandle) {
    let tool = tools(cx, main, 2).await;
    cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            "owned-empty-selection-sentinel".into(),
        ))
    });
    control(cx, tool, "history-copy-selected");
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        "owned-empty-selection-sentinel"
    );
    click(cx, tool, "history-tools-close");
}

#[gpui_kit::test]
async fn journey(cx: &mut TestAppContext) {
    // This integration test intentionally permits wakes from real SQLite/gRPC
    // workers. Pure Kit component tests can keep the default deterministic mode.
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let evidence = std::env::var_os("RUSTLEBLOWER_HEADLESS_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.path().join("evidence"));
    std::fs::create_dir_all(&evidence).unwrap();
    let config = Config {
        socket: dir.path().join("ui.sock"),
        database: dir.path().join("history.sqlite3"),
        transport: Default::default(),
        prompt_timeout: 30,
        timeout_action: crate::config::Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    };
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mut daemon = runtime.block_on(async {
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
                name: "selection-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    let base = crate::history::now_nanos() - 1_000_000_000;
    let event = |n: i64| protocol::Event {
        unixnano: base + n,
        connection: Some(protocol::Connection {
            process_path: format!("/owned/selection-{n:03}"),
            dst_host: "owned-selection.test".into(),
            dst_port: 443,
            protocol: "tcp".into(),
            ..Default::default()
        }),
        rule: Some(protocol::Rule {
            action: "deny".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    backend.handle.history.ingest(
        "selection-fixture".into(),
        (1..=205).map(event).collect(),
        "owned-selection",
    );
    runtime.block_on(backend.handle.history.flush()).unwrap();
    // Drive Kit timers with simulated time rather than parking on the real I/O.
    cx.executor().forbid_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
        crate::app::bind_keys(cx);
    });
    let (main, view) = cx
        .update(|cx| {
            gpui_kit::open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds {
                        origin: gpui::Point::default(),
                        size: size(px(1320.), px(980.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
                },
            )
        })
        .unwrap();

    cx.wait_for(main, TIMEOUT, |_, cx| {
        view.read(cx).history.page.rows.len() == 200
    })
    .await;
    let top = cx.read_entity(&view, |v, _| v.history.page.rows[0].id);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.page.rows[0].process.clone()),
        "/owned/selection-205"
    );
    mark(cx, main, top, false);
    ui(cx, main, |window, _| {
        assert_eq!(
            window.find(("history-mark", top as u64)).checked(),
            Some(true)
        )
    });

    click(cx, main, "older");
    cx.wait_for(main, TIMEOUT, |_, cx| {
        view.read(cx).history.page.rows.len() == 5
    })
    .await;
    let range_end = cx.read_entity(&view, |v, _| v.history.page.rows[0].id);
    let separate = cx.read_entity(&view, |v, _| v.history.page.rows[2].id);
    mark(cx, main, range_end, true);
    mark(cx, main, separate, false);
    click(cx, main, "history-first-page");
    cx.wait_for(main, TIMEOUT, |_, cx| {
        view.read(cx).history.page.rows.first().map(|r| r.id) == Some(top)
    })
    .await;

    // Details must not change the marked membership used by the subsequent export.
    ui(cx, main, |window, cx| {
        window.click(("history-row", 1usize), cx)
    });
    click(cx, main, "history-details");
    cx.wait_for(main, TIMEOUT, |_, cx| {
        view.read(cx).history.detail_window.is_some()
    })
    .await;
    let details = cx.read_entity(&view, |v, _| v.history.detail_window.unwrap());
    click(cx, details, "connection-close");

    let tool = tools(cx, main, 2).await;
    control(cx, tool, "history-export-format");
    ui(cx, tool, |window, cx| {
        window.input("JSON Lines", cx);
        window.press("enter", cx);
    });
    // Select commits are deferred until after the borrowed window update.
    cx.wait_for(tool, TIMEOUT, |window, _| {
        window
            .find("history-export-format")
            .value()
            .is_some_and(|value| value.contains("JSON Lines"))
    })
    .await;
    let range_path = evidence.join("selected-range.jsonl");
    control(cx, tool, "history-export-path");
    ui(cx, tool, |window, cx| {
        window.press("ctrl-a", cx);
        window.input(range_path.to_str().unwrap(), cx);
    });
    control(cx, tool, "history-export-selected");
    cx.wait_for(tool, TIMEOUT, |_, _| range_path.exists()).await;
    cx.wait_for(tool, TIMEOUT, |window, cx| {
        crate::app::history_tools::test_idle(window, cx)
    })
    .await;
    let range = std::fs::read_to_string(&range_path)
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["process"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<BTreeSet<_>>();
    let expected = (5..=205)
        .chain([3])
        .map(|n| format!("/owned/selection-{n:03}"))
        .collect::<BTreeSet<_>>();
    assert_eq!(range, expected);
    click(cx, tool, "history-tools-close");

    click(cx, main, "history-select-all");
    mark(cx, main, top, false);
    backend.handle.history.ingest(
        "selection-fixture".into(),
        vec![event(206)],
        "owned-selection",
    );
    runtime.block_on(backend.handle.history.flush()).unwrap();
    click(cx, main, "history-refresh");
    cx.wait_for(main, TIMEOUT, |_, cx| view.read(cx).history.test_idle())
        .await;
    assert!(!cx.read_entity(&view, |v, _| {
        v.history
            .page
            .rows
            .iter()
            .any(|r| r.process == "/owned/selection-206")
    }));

    let tool = tools(cx, main, 1).await;
    for id in [
        "history-column-time",
        "history-column-host",
        "history-column-port",
        "history-column-protocol",
    ] {
        control(cx, tool, id);
    }
    control(cx, tool, "history-columns-save");
    cx.wait_for(tool, TIMEOUT, |window, cx| {
        crate::app::history_tools::test_idle(window, cx)
    })
    .await;
    ui(cx, tool, |window, cx| {
        window.within("history-tools-tabs").click(2usize, cx);
    });
    cx.wait_for(tool, TIMEOUT, |window, _| {
        window.try_find("history-export-path").is_some()
    })
    .await;
    let csv_path = evidence.join("selected-all-except-first.csv");
    control(cx, tool, "history-export-path");
    ui(cx, tool, |window, cx| {
        window.press("ctrl-a", cx);
        window.input(csv_path.to_str().unwrap(), cx);
    });
    control(cx, tool, "history-export-selected");
    cx.wait_for(tool, TIMEOUT, |_, _| csv_path.exists()).await;
    cx.wait_for(tool, TIMEOUT, |window, cx| {
        crate::app::history_tools::test_idle(window, cx)
    })
    .await;
    let csv = std::fs::read_to_string(&csv_path).unwrap();
    assert_eq!(csv.lines().count(), 205);
    assert_eq!(
        csv.lines().next().unwrap().split(',').count(),
        2,
        "Only the two chosen data columns are exported"
    );
    assert!(!csv.contains("selection-205") && !csv.contains("selection-206"));
    assert!(csv.contains("selection-001") && csv.contains("selection-204"));
    control(cx, tool, "history-copy-selected");
    cx.wait_for(tool, TIMEOUT, |_, cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref()
            == Some(csv.as_str())
    })
    .await;
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        csv
    );

    let ask = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/selection-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    cx.wait_for(main, TIMEOUT, |_, cx| view.read(cx).prompt_window.is_some())
        .await;
    let popup = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    click(cx, popup, "deny-once");
    cx.wait_for(main, TIMEOUT, |_, _| ask.is_finished()).await;
    assert_eq!(runtime.block_on(ask).unwrap().action, "deny");
    click(cx, tool, "history-tools-close");
    click(cx, main, "latest");
    cx.wait_for(main, TIMEOUT, |_, cx| {
        view.read(cx)
            .history
            .page
            .rows
            .iter()
            .any(|r| r.process == "/owned/selection-206")
    })
    .await;
    assert!(cx.read_entity(&view, |v, _| v.history.query.ceiling.is_none()));
    copy_disabled(cx, main).await;

    let tool = tools(cx, main, 1).await;
    control(cx, tool, "history-table-mode");
    control(cx, tool, "history-columns-save");
    cx.wait_for(tool, TIMEOUT, |window, cx| {
        crate::app::history_tools::test_idle(window, cx)
    })
    .await;
    click(cx, tool, "history-tools-close");
    cx.wait_for(main, TIMEOUT, |window, _| {
        window.try_find("table").is_some()
    })
    .await;
    let table_id = cx.read_entity(&view, |v, _| v.history.page.rows[0].id);
    mark(cx, main, table_id, false);
    ui(cx, main, |window, cx| {
        window.within("table").click(("icon-sort", 0usize), cx);
    });
    cx.wait_for(main, TIMEOUT, |_, cx| view.read(cx).history.test_idle())
        .await;
    copy_disabled(cx, main).await;
    let filtered_id = cx.read_entity(&view, |v, _| v.history.page.rows[0].id);
    mark(cx, main, filtered_id, false);
    ui(cx, main, |window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-a", cx);
        window.input("/owned/selection-1", cx);
    });
    cx.wait_for(main, TIMEOUT, |_, cx| {
        let history = &view.read(cx).history;
        history.test_idle()
            && history
                .page
                .rows
                .iter()
                .all(|row| row.process.starts_with("/owned/selection-1"))
    })
    .await;
    copy_disabled(cx, main).await;

    std::fs::write(
        evidence.join("selection-proof.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "initial_events":205, "range_plus_manual_count":range.len(), "range_processes":range,
            "all_minus_one_csv_rows":csv.lines().count()-1, "csv_clipboard_exact":true,
            "new_arrival_excluded":true, "details_independent":true,
            "latest_sort_filter_copy_inert_after_clear":true
        }))
        .unwrap(),
    )
    .unwrap();
    for window in cx.update(|cx| cx.windows()) {
        ui(cx, window, |window, _| window.remove_window());
    }
    drop(view);
    // Release view-owned history senders before joining the storage worker.
    cx.run_until_parked();
    cx.quit();
    cx.run_until_parked();
    backend.join();
    drop(runtime);
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "status":"passed", "framework":"gpui-kit 0.7.3", "real_gpu":false,
        "scoped_socket_removed":true, "production_view":true, "checkpoints":["selection-proof.json"],
        "proof_boundary":"Kit native event dispatch, control snapshots, exact selection/export/clipboard and independent gRPC permission on owned backend. Standard test text system; no GPU/font/pixel/compositor/package claim."
    })).unwrap()).unwrap();
}
