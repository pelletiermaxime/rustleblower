//! FerryDock's real WGPU + GPUI input pattern, with synthetic daemon data at the backend boundary.
use super::{Dashboard, prompt_window::PromptWindow};
use crate::{
    backend::Backend,
    config::{Config, Decision},
    protocol,
};
use gpui::{AnyWindowHandle, AppContext, HeadlessAppContext, px, size};
use gpui_kit::{
    component::{Theme, ThemeMode},
    test::TestWindowExt,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn context() -> HeadlessAppContext {
    assert!(std::env::var_os("DISPLAY").is_none());
    assert!(std::env::var_os("WAYLAND_DISPLAY").is_none());
    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        platform.text_system(),
        Arc::new(gpui_kit::assets::Assets),
        || {
            Ok(Some(Box::new(
                gpui_wgpu::WgpuHeadlessRenderer::new().expect("real headless GPU"),
            )))
        },
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        super::bind_keys(cx);
    });
    cx
}
fn options(dir: &Path) -> Config {
    Config {
        socket: dir.join("ui.sock"),
        database: dir.join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    }
}
fn evidence() -> PathBuf {
    let path = std::env::var_os("RUSTLEBLOWER_HEADLESS_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".verification/headless/direct")
        });
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn settle(cx: &mut HeadlessAppContext) {
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(10));
        cx.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        sync_geometry(cx);
    }
}
// TestPlatform's resize request updates bounds without the compositor callback.
// Deliver that documented platform boundary when the requested size changed.
fn sync_geometry(cx: &mut HeadlessAppContext) {
    let windows = cx.update(|cx| cx.windows());
    for window in windows {
        cx.update_window(window, |_, window, cx| {
            if window.viewport_size() != window.bounds().size {
                window.bounds_changed(cx);
            }
        })
        .unwrap();
    }
    cx.run_until_parked();
}
fn capture(cx: &mut HeadlessAppContext, window: AnyWindowHandle, path: &Path) {
    let image = cx.capture_screenshot(window).expect("GPU screenshot");
    assert!(image.width() >= 580 && image.height() >= 440);
    assert!(
        image
            .pixels()
            .filter(|p| p.0[..3].iter().any(|v| *v > 150))
            .take(100)
            .count()
            >= 100,
        "painted glyphs required"
    );
    image.save(path).unwrap();
}
fn click(cx: &mut HeadlessAppContext, window: AnyWindowHandle, id: &'static str) {
    cx.update_window(window, |_, window, cx| window.click(id, cx))
        .unwrap();
    cx.run_until_parked();
    sync_geometry(cx);
}
fn pick(cx: &mut HeadlessAppContext, window: AnyWindowHandle, id: &'static str, label: &str) {
    click(cx, window, id);
    // The popover's search input takes focus after mounting in a frame.
    settle(cx);
    cx.update_window(window, |_, window, cx| window.input(label, cx))
        .unwrap();
    settle(cx);
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    // Select commits/close/focus restoration are deferred across frames.
    settle(cx);
}

fn fill(cx: &mut HeadlessAppContext, window: AnyWindowHandle, id: &'static str, text: &str) {
    click(cx, window, id);
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input(text, cx);
    })
    .unwrap();
    settle(cx);
}
fn advanced_defaults(cx: &mut HeadlessAppContext, window: AnyWindowHandle) {
    cx.update_window(window, |_, window, cx| {
        window.within("preferences-tabs").click(1usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    for id in [
        "preferences-expanded",
        "preferences-condition-dest.ip",
        "preferences-condition-dest.port",
        "preferences-condition-user.id",
        "preferences-condition-checksum",
    ] {
        click(cx, window, id);
    }
    cx.update_window(window, |_, window, cx| {
        window.scroll(
            "preferences-condition-checksum",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-240.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
}

#[test]
fn headless_preferences_controls() {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let mut cx = context();
    let window = cx
        .update(|cx| super::preferences_view::open(backend.handle.clone(), cx))
        .unwrap();
    settle(&mut cx);
    pick(&mut cx, window, "preferences-duration", "12 hours");
    click(&mut cx, window, "preferences-target");
    settle(&mut cx);
    capture(&mut cx, window, &evidence.join("01-target-open.png"));
    cx.update_window(window, |_, window, cx| window.input("This executable", cx))
        .unwrap();
    settle(&mut cx);
    capture(&mut cx, window, &evidence.join("02-target-query.png"));
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    settle(&mut cx);
    click(&mut cx, window, "preferences-priority");
    click(&mut cx, window, "preferences-logging");
    click(&mut cx, window, "preferences-save");
    settle(&mut cx);
    capture(&mut cx, window, &evidence.join("03-saved.png"));
    let value = backend.handle.preferences.snapshot();
    assert_eq!(value.target, "process.path");
    assert_eq!(value.duration, "12h");
    assert!(value.priority && !value.logging);
    assert_eq!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot(),
        value
    );
    advanced_defaults(&mut cx, window);
    fill(&mut cx, window, "preferences-timeout", "0");
    click(&mut cx, window, "preferences-save");
    settle(&mut cx);
    assert_eq!(backend.handle.preferences.snapshot(), value);
    capture(&mut cx, window, &evidence.join("04-invalid-timeout.png"));
    fill(&mut cx, window, "preferences-timeout", "5");
    pick(&mut cx, window, "preferences-fallback", "Reject once");
    click(&mut cx, window, "preferences-save");
    settle(&mut cx);
    let advanced = backend.handle.preferences.snapshot();
    assert!(advanced.show_advanced && advanced.constraints.len() == 4);
    assert_eq!(advanced.timeout, Some(5));
    assert_eq!(advanced.fallback, Some(Decision::Reject));
    assert_eq!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot(),
        advanced
    );
    capture(&mut cx, window, &evidence.join("05-advanced-defaults.png"));
    cx.update_window(window, |_, window, cx| {
        window.within("preferences-tabs").click(2usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    pick(&mut cx, window, "preferences-position", "Bottom right");
    click(&mut cx, window, "preferences-save");
    settle(&mut cx);
    assert_eq!(
        backend.handle.preferences.snapshot().position,
        crate::prompts::placement::Position::BottomRight
    );
    assert_eq!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .position,
        crate::prompts::placement::Position::BottomRight
    );
    capture(&mut cx, window, &evidence.join("06-popup-position.png"));
    for (index, (key, _)) in crate::prompts::placement::POSITIONS.iter().enumerate() {
        let position = crate::prompts::placement::Position::parse(key).unwrap();
        let (_, mut reply) = backend
            .handle
            .prompts
            .insert_with_defaults(
                1,
                protocol::Connection {
                    process_path: "/usr/bin/placement-test".into(),
                    dst_ip: "192.0.2.1".into(),
                    ..Default::default()
                },
                Instant::now() + Duration::from_secs(30),
                crate::prompts::Initial {
                    position,
                    ..Default::default()
                },
            )
            .unwrap();
        let popup = cx
            .update(|cx| super::prompt_window::open(backend.handle.clone(), 1., true, cx))
            .unwrap();
        cx.run_until_parked();
        let actual = cx
            .update_window(popup, |_, window, _| window.bounds())
            .unwrap();
        let screen = cx.update(|cx| cx.primary_display().unwrap().visible_bounds());
        let expected_x = match *key {
            "top-left" | "bottom-left" => screen.origin.x + px(16.),
            "top-right" | "bottom-right" => {
                screen.origin.x + screen.size.width - actual.size.width - px(16.)
            }
            _ => screen.origin.x + (screen.size.width - actual.size.width) / 2.,
        };
        let expected_y = match *key {
            "top-left" | "top-right" => screen.origin.y + px(16.),
            "bottom-left" | "bottom-right" => {
                screen.origin.y + screen.size.height - actual.size.height - px(16.)
            }
            _ => screen.origin.y + (screen.size.height - actual.size.height) / 2.,
        };
        assert_eq!(actual.origin, gpui::point(expected_x, expected_y));
        capture(
            &mut cx,
            popup,
            &evidence.join(format!("{:02}-position-{key}.png", index + 7)),
        );
        if *key == "center" {
            cx.update_window(popup, |_, window, cx| {
                window.press("tab", cx);
                window.render_frame(cx);
                window.press("space", cx);
            })
            .unwrap();
            cx.run_until_parked();
        } else {
            click(&mut cx, popup, "deny-once");
        }
        assert_eq!(reply.try_recv().unwrap().decision, Decision::Deny);
        cx.update_window(popup, |_, window, _| window.remove_window())
            .unwrap();
    }
    let mut dashboard_view = None;
    let dashboard = cx
        .open_window(size(px(1100.), px(780.)), |window, cx| {
            let view =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            dashboard_view = Some(view.clone());
            cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
        })
        .unwrap();
    let dashboard_view = dashboard_view.unwrap();
    let mut replies = Vec::new();
    for position in [
        crate::prompts::placement::Position::TopRight,
        crate::prompts::placement::Position::Center,
    ] {
        let (_, reply) = backend
            .handle
            .prompts
            .insert_with_defaults(
                1,
                protocol::Connection {
                    process_path: "/usr/bin/queued-placement".into(),
                    ..Default::default()
                },
                Instant::now() + Duration::from_secs(30),
                crate::prompts::Initial {
                    position,
                    ..Default::default()
                },
            )
            .unwrap();
        replies.push(reply);
    }
    settle(&mut cx);
    let first = cx.read_entity(&dashboard_view, |view, _| view.prompt_window.unwrap());
    capture(&mut cx, first, &evidence.join("12-queued-corner.png"));
    click(&mut cx, first, "deny-once");
    assert_eq!(replies[0].try_recv().unwrap().decision, Decision::Deny);
    settle(&mut cx);
    assert!(
        cx.update_window(first, |_, _, _| ()).is_err(),
        "queued position change must replace the previous popup"
    );
    let second = cx.read_entity(&dashboard_view, |view, _| view.prompt_window.unwrap());
    let actual = cx
        .update_window(second, |_, window, _| window.bounds())
        .unwrap();
    let screen = cx.update(|cx| cx.primary_display().unwrap().visible_bounds());
    assert_eq!(
        actual.origin,
        gpui::point(
            screen.origin.x + (screen.size.width - actual.size.width) / 2.,
            screen.origin.y + (screen.size.height - actual.size.height) / 2.
        )
    );
    capture(&mut cx, second, &evidence.join("13-queued-center.png"));
    click(&mut cx, second, "deny-once");
    assert_eq!(replies[1].try_recv().unwrap().decision, Decision::Deny);
    settle(&mut cx);
    cx.update_window(dashboard.into(), |_, window, _| window.remove_window())
        .unwrap();
    drop(dashboard_view);
    click(&mut cx, window, "preferences-close");
    drop(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","screenshots":["01-target-open.png","02-target-query.png","03-saved.png","04-invalid-timeout.png","05-advanced-defaults.png","06-popup-position.png","07-position-center.png","08-position-top-right.png","09-position-bottom-right.png","10-position-top-left.png","11-position-bottom-left.png","12-queued-corner.png","13-queued-center.png"],"real_gpu":true,"scoped_socket_removed":true,"proof_boundary":"Real GPUI preference controls, timeout validation, file persistence and initial virtual-window geometry; native compositor placement and daemon enforcement require separate proof"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Select duration and match target, toggle priority/logging, save and reopen preferences from disk", "Open Conditions & timeout; select four default conditions and expanded options; reject zero seconds without publishing staged settings; save five seconds and Reject once; reopen the file with exact values", "Save Bottom Right placement and reopen preferences; open all five production popup positions; compare virtual geometry and answer by pointer or Tab/Space", "Queue Top Right then Center requests; answer the first; require its window to close and the next centered popup to return its own reply"]).unwrap()).unwrap();
}

#[test]
fn headless_countdown_review() {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let mut config = options(dir.path());
    config.prompt_timeout = 2;
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
                name: "countdown-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        client
    });
    let connection = protocol::Connection {
        process_path: "/usr/bin/review-fixture".into(),
        dst_ip: "192.0.2.42".into(),
        dst_port: 443,
        protocol: "tcp".into(),
        ..Default::default()
    };
    let mut asking = daemon.clone();
    let con = connection.clone();
    let first = runtime.spawn(async move { asking.ask_rule(con).await.unwrap().into_inner() });
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), async {
            while backend.handle.prompts.snapshot().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    });
    let mut cx = context();
    let window =
        cx.update(|cx| super::prompt_window::open(backend.handle.clone(), 1., false, cx).unwrap());
    settle(&mut cx);
    assert!(!backend.handle.prompts.snapshot()[0].reviewing);
    capture(&mut cx, window, &evidence.join("01-normal-countdown.png"));
    click(&mut cx, window, "prompt-more");
    assert!(
        backend.handle.prompts.snapshot()[0].reviewing,
        "pointer input pauses countdown before child handling"
    );
    std::thread::sleep(Duration::from_millis(2200));
    settle(&mut cx);
    assert!(
        !first.is_finished(),
        "review survives the ordinary two-second countdown"
    );
    capture(&mut cx, window, &evidence.join("02-pointer-review.png"));
    click(&mut cx, window, "allow-once");
    let rule = runtime.block_on(first).unwrap();
    assert_eq!(
        (rule.action.as_str(), rule.duration.as_str()),
        ("allow", "once")
    );
    let mut asking = daemon.clone();
    let second =
        runtime.spawn(async move { asking.ask_rule(connection).await.unwrap().into_inner() });
    settle(&mut cx);
    assert!(
        !backend.handle.prompts.snapshot()[0].reviewing,
        "next request gets a fresh countdown"
    );
    capture(&mut cx, window, &evidence.join("03-next-countdown.png"));
    cx.update_window(window, |_, window, cx| window.press("tab", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(
        backend.handle.prompts.snapshot()[0].reviewing,
        "keyboard input pauses countdown"
    );
    std::thread::sleep(Duration::from_millis(2200));
    settle(&mut cx);
    assert!(!second.is_finished());
    capture(&mut cx, window, &evidence.join("04-keyboard-review.png"));
    cx.update_window(window, |_, window, cx| window.press("space", cx))
        .unwrap();
    cx.run_until_parked();
    let rule = runtime.block_on(second).unwrap();
    assert_eq!(
        (rule.action.as_str(), rule.duration.as_str()),
        ("deny", "once")
    );
    assert_eq!(backend.handle.status().timed_out, 0);
    assert_eq!(backend.handle.status().answered, 2);
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let db = rusqlite::Connection::open(&config.database).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM events WHERE reason='decision'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    drop(db);
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(cx);
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","screenshots":["01-normal-countdown.png","02-pointer-review.png","03-next-countdown.png","04-keyboard-review.png"],"real_gpu":true,"scoped_socket_removed":true,"proof_boundary":"Production GPUI pointer and keyboard review, survival beyond ordinary countdown, next-request reset, actual gRPC replies and persisted decisions; real daemon enforcement and native compositor input require separate proof"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"), serde_json::to_vec_pretty(&["Open a real AskRule request with two-second countdown; click Rule options and wait beyond two seconds; require pending review then Allow once", "Open the next request; require normal countdown; Tab to Deny, wait beyond two seconds, then Space; require Deny once and two persisted input decisions"]).unwrap()).unwrap();
}

#[test]
fn headless_checksum_review() {
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
    let mut cx = context();
    let prompt =
        cx.update(|cx| super::prompt_window::open(backend.handle.clone(), 1., false, cx).unwrap());
    settle(&mut cx);
    capture(&mut cx, prompt, &evidence.join("01-checksum-warning.png"));
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, prompt, "prompt-checksums");
    settle(&mut cx);
    let review = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|window| !before.contains(window))
            .unwrap()
    });
    capture(&mut cx, review, &evidence.join("02-checksum-review.png"));
    click(&mut cx, review, "checksum-next");
    settle(&mut cx);
    capture(&mut cx, review, &evidence.join("03-checksum-next-page.png"));
    click(&mut cx, review, "checksum-reload");
    settle(&mut cx);
    click(&mut cx, review, "checksum-update-selected");
    capture(&mut cx, review, &evidence.join("04-checksum-confirm.png"));
    click(&mut cx, review, "checksum-cancel");
    assert!(observed.lock().unwrap().is_empty());
    hold.store(true, Ordering::Relaxed);
    click(&mut cx, review, "checksum-update-selected");
    click(&mut cx, review, "checksum-confirm");
    settle(&mut cx);
    assert_eq!(observed.lock().unwrap().len(), 1);
    let saved =
        transfer::find_confirmed_rule(&config.database, "checksum-fixture", &originals[0].name)
            .unwrap()
            .unwrap();
    assert_eq!(
        saved, originals[0],
        "no optimistic cache update before daemon ACK"
    );
    capture(
        &mut cx,
        review,
        &evidence.join("05-checksum-awaiting-ack.png"),
    );
    let started = Instant::now();
    click(&mut cx, prompt, "allow-once");
    let decision = runtime.block_on(reply).unwrap();
    assert_eq!(
        (decision.action.as_str(), decision.duration.as_str()),
        ("allow", "once")
    );
    assert!(started.elapsed() < Duration::from_millis(500));
    hold.store(false, Ordering::Relaxed);
    for _ in 0..10 {
        settle(&mut cx);
        if checksums::page(&config.database, "checksum-fixture", &hash_context, "")
            .unwrap()
            .changes
            .first()
            .is_some_and(|change| change.rule.name == originals[1].name)
        {
            break;
        }
    }
    settle(&mut cx);
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
    hold.store(true, Ordering::Relaxed);
    click(&mut cx, review, "checksum-update-all");
    click(&mut cx, review, "checksum-confirm");
    settle(&mut cx);
    assert_eq!(observed.lock().unwrap().len(), 2);
    click(&mut cx, review, "checksum-stop");
    hold.store(false, Ordering::Relaxed);
    settle(&mut cx);
    settle(&mut cx);
    assert_eq!(
        observed.lock().unwrap().len(),
        2,
        "Stop must prevent unsent updates"
    );
    capture(&mut cx, review, &evidence.join("06-checksum-stopped.png"));
    accept.store(false, Ordering::Relaxed);
    click(&mut cx, review, "checksum-update-selected");
    click(&mut cx, review, "checksum-confirm");
    settle(&mut cx);
    settle(&mut cx);
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
    capture(&mut cx, review, &evidence.join("07-checksum-error.png"));
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
    settle(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, prompt, "prompt-checksums");
    settle(&mut cx);
    let review = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|window| !before.contains(window))
            .unwrap()
    });
    click(&mut cx, review, "checksum-update-all");
    click(&mut cx, review, "checksum-confirm");
    for _ in 0..40 {
        settle(&mut cx);
        if checksums::page(&config.database, "checksum-fixture", &hash_context, "")
            .unwrap()
            .changes
            .is_empty()
        {
            break;
        }
    }
    settle(&mut cx);
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
    capture(
        &mut cx,
        review,
        &evidence.join("08-checksum-all-confirmed.png"),
    );
    capture(&mut cx, prompt, &evidence.join("09-warning-cleared.png"));
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    click(&mut cx, review, "checksum-close");
    cx.update_window(prompt, |_, window, _| window.remove_window())
        .unwrap();
    drop(cx);
    notifications.abort();
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","screenshots":["01-checksum-warning.png","02-checksum-review.png","03-checksum-next-page.png","04-checksum-confirm.png","05-checksum-awaiting-ack.png","06-checksum-stopped.png","07-checksum-error.png","08-checksum-all-confirmed.png","09-warning-cleared.png"],"real_gpu":true,"scoped_socket_removed":true,"proof_boundary":"Actual GPUI checksum warning/review input, paged reads, confirmed rule changes over isolated gRPC, failure/stop/reconnect/all-page workflow and permission reply while command ACK is held; real daemon enforcement and native window input remain separate"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Open the reported-hash warning, inspect MD5/SHA1 and next page, cancel then confirm selected update", "Hold daemon ACK; require unchanged cache and immediate independent Allow once; release ACK, stop all after one command, and reject the next update with uncertainty feedback", "Reconnect a fresh daemon inventory, reopen the review, update all remaining 33 rules across pages, require preserved metadata/conditions and cleared warning, then Deny once"]).unwrap()).unwrap();
}

#[test]
fn headless_history_tools() {
    use crate::history::query::Field;
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let events = (0..430usize)
        .map(|index| protocol::Event {
            unixnano: crate::history::now_nanos() + index as i64,
            connection: Some(protocol::Connection {
                process_path: format!("/usr/bin/history-app-{}", index % 7),
                process_args: vec![
                    "tool".into(),
                    format!("--session blue{}", index % 4),
                    "comma, \"argument\"".into(),
                ],
                src_ip: "192.0.2.10".into(),
                src_port: 40000,
                dst_ip: "198.51.100.20".into(),
                dst_host: "api.example.test".into(),
                dst_port: 443,
                protocol: "tcp".into(),
                user_id: if index.is_multiple_of(2) { 0 } else { 1001 },
                process_id: 100 + index as u32,
                ..Default::default()
            }),
            rule: Some(protocol::Rule {
                name: "history-filter-rule".into(),
                action: if index.is_multiple_of(3) {
                    "reject"
                } else {
                    "allow"
                }
                .into(),
                duration: "5m".into(),
                precedence: true,
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    backend
        .handle
        .history
        .ingest("history-tools-fixture".into(), events, "observed");
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context();
    let mut view = None;
    let window = cx
        .open_window(size(px(1100.), px(780.)), |window, cx| {
            let dashboard =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(dashboard.clone());
            cx.new(|cx| gpui_kit::base::Root::new(dashboard, window, cx))
        })
        .unwrap();
    let view = view.unwrap();
    settle(&mut cx);
    click(&mut cx, window.into(), "reject");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows.len()),
        144
    );
    capture(
        &mut cx,
        window.into(),
        &evidence.join("01-rejected-history.png"),
    );
    click(&mut cx, window.into(), "all");
    settle(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window.into(), "history-tools");
    settle(&mut cx);
    let tools = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|handle| !before.contains(handle))
            .unwrap()
    });
    pick(&mut cx, tools, "history-filter-field", "UID");
    pick(&mut cx, tools, "history-filter-comparison", "Equals");
    fill(&mut cx, tools, "history-filter-value", "1001");
    click(&mut cx, tools, "history-filter-add");
    pick(&mut cx, tools, "history-filter-field", "Command line");
    pick(&mut cx, tools, "history-filter-comparison", "Contains");
    fill(&mut cx, tools, "history-filter-value", "blue1");
    click(&mut cx, tools, "history-filter-add");
    click(&mut cx, tools, "history-filter-apply");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows.len()),
        108
    );
    assert!(
        cx.read_entity(&view, |view, _| view.history.page.rows.iter().all(
            |row| row.metadata.user == Some(1001) && row.metadata.command.contains("blue1")
        ))
    );
    capture(&mut cx, tools, &evidence.join("02-history-conditions.png"));
    pick(&mut cx, tools, "history-filter-field", "Destination port");
    pick(&mut cx, tools, "history-filter-comparison", "Equals");
    fill(&mut cx, tools, "history-filter-value", "65536");
    click(&mut cx, tools, "history-filter-add");
    capture(
        &mut cx,
        tools,
        &evidence.join("03-invalid-history-port.png"),
    );
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.query.filters.len()),
        2
    );
    cx.update_window(tools, |_, window, cx| {
        window.within("history-tools-tabs").click(1usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, tools, "history-table-mode");
    cx.update_window(tools, |_, window, cx| {
        window.scroll(
            "history-table-mode",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-300.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, tools, "history-column-user");
    click(&mut cx, tools, "history-column-command");
    cx.update_window(tools, |_, window, cx| {
        window.scroll(
            "history-column-command",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-1000.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, tools, "history-columns-save");
    settle(&mut cx);
    let saved = crate::history::settings::Store::open(&config.database)
        .unwrap()
        .snapshot();
    assert!(
        saved.table
            && saved.columns.contains(&Field::User)
            && saved.columns.contains(&Field::Command)
    );
    capture(
        &mut cx,
        tools,
        &evidence.join("04-saved-history-columns.png"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.within("table").click(("icon-sort", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.query.sort),
        Some((Field::Time, false))
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.within("table").click(("icon-sort", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.query.sort),
        Some((Field::Time, true))
    );
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows[0].metadata.pid),
        Some(101)
    );
    capture(
        &mut cx,
        window.into(),
        &evidence.join("05-sorted-history-table.png"),
    );
    cx.update_window(tools, |_, window, cx| {
        window.within("history-tools-tabs").click(2usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    let csv = evidence.join("filtered-history.csv");
    fill(&mut cx, tools, "history-export-path", csv.to_str().unwrap());
    click(&mut cx, tools, "history-export-save");
    for _ in 0..10 {
        settle(&mut cx);
        if csv.exists() {
            break;
        }
    }
    settle(&mut cx);
    let original = std::fs::read_to_string(&csv).unwrap();
    assert_eq!(original.lines().count(), 109);
    assert!(original.contains("Command line") && original.contains("\"\"argument\"\""));
    capture(&mut cx, tools, &evidence.join("06-history-csv-export.png"));
    click(&mut cx, tools, "history-export-save");
    settle(&mut cx);
    assert_eq!(std::fs::read_to_string(&csv).unwrap(), original);
    capture(
        &mut cx,
        tools,
        &evidence.join("07-history-existing-file.png"),
    );
    let json = evidence.join("filtered-history.jsonl");
    fill(
        &mut cx,
        tools,
        "history-export-path",
        json.to_str().unwrap(),
    );
    pick(&mut cx, tools, "history-export-format", "JSON Lines");
    click(&mut cx, tools, "history-export-save");
    for _ in 0..10 {
        settle(&mut cx);
        if json.exists() {
            break;
        }
    }
    settle(&mut cx);
    let text = std::fs::read_to_string(&json).unwrap();
    assert_eq!(text.lines().count(), 108);
    assert!(text.lines().all(|line| {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        value["user"] == "1001" && value["command"].as_str().unwrap().contains("blue1")
    }));
    capture(&mut cx, tools, &evidence.join("08-history-json-export.png"));
    click(&mut cx, tools, "history-copy-page");
    settle(&mut cx);
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(copied.lines().count(), 109);
    assert!(copied.contains("blue1"));
    capture(&mut cx, tools, &evidence.join("09-history-clipboard.png"));
    cx.update_window(tools, |_, window, cx| {
        window.within("history-tools-tabs").click(0usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, tools, "history-filter-clear");
    click(&mut cx, tools, "history-filter-apply");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows.len()),
        200
    );
    click(&mut cx, window.into(), "older");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows.len()),
        200
    );
    assert!(cx.read_entity(&view, |view, _| view.history.query.cursor.is_some()));
    capture(
        &mut cx,
        window.into(),
        &evidence.join("10-history-sorted-next-page.png"),
    );
    click(&mut cx, tools, "history-tools-close");
    cx.update_window(window.into(), |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    drop(cx);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","screenshots":["01-rejected-history.png","02-history-conditions.png","03-invalid-history-port.png","04-saved-history-columns.png","05-sorted-history-table.png","06-history-csv-export.png","07-history-existing-file.png","08-history-json-export.png","09-history-clipboard.png","10-history-sorted-next-page.png"],"real_gpu":true,"scoped_socket_removed":true,"proof_boundary":"Real GPUI history conditions/column/table sort/export controls, SQLite filters, independently reopened settings and actual CSV/JSON/clipboard data; native tools input/pickers and real daemon enforcement remain separate"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Filter Rejected; combine UID equals 1001 and command contains blue1; reject invalid port 65536 without changing conditions", "Enable pinned Kit table, UID and command columns; save/reopen history settings; sort all matching history by time through the actual header", "Export all 108 matches to CSV and JSON Lines, preserve existing destination on refusal, copy a matching page; clear conditions and page sorted results with a frozen ceiling"]).unwrap()).unwrap();
}

#[test]
fn headless_gpu_user_flow() {
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
    let mut cx = context();
    let mut view = None;
    let window = cx
        .open_window(size(px(1100.), px(780.)), |window, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config, window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    let view = view.unwrap();
    cx.run_until_parked();
    settle(&mut cx);
    assert_eq!(cx.read_entity(&view, |v, _| v.history.page.rows.len()), 200);
    capture(&mut cx, window.into(), &evidence.join("01-history.png"));
    click(&mut cx, window.into(), "older");
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.history.query.cursor.is_some()));
    assert_eq!(cx.read_entity(&view, |v, _| v.history.page.rows.len()), 200);
    capture(&mut cx, window.into(), &evidence.join("02-older.png"));
    click(&mut cx, window.into(), "latest");
    settle(&mut cx);
    click(&mut cx, window.into(), "deny");
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| {
        v.history.page.rows.iter().all(|r| r.action == "deny")
    }));
    cx.update_window(window.into(), |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("does-not-exist", cx);
    })
    .unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.search.clone()),
        "does-not-exist"
    );
    assert!(cx.read_entity(&view, |v, _| v.history.page.rows.is_empty()));
    capture(
        &mut cx,
        window.into(),
        &evidence.join("03-empty-search.png"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("firefox", cx);
    })
    .unwrap();
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| {
        !v.history.page.rows.is_empty()
            && v.history
                .page
                .rows
                .iter()
                .all(|r| r.process.contains("firefox") && r.action == "deny")
    }));
    capture(&mut cx, window.into(), &evidence.join("04-filtered.png"));
    assert!(backend.handle.history.health.warning().is_none());
    backend.handle.history.ingest(
        "fixture".into(),
        vec![protocol::Event::default(); 4097],
        "observed",
    );
    settle(&mut cx);
    assert!(
        backend
            .handle
            .history
            .health
            .warning()
            .unwrap()
            .contains("4097 that exceeded event limits")
    );
    capture(
        &mut cx,
        window.into(),
        &evidence.join("07-history-warning.png"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.within("main-tabs").click(1usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    assert_eq!(cx.read_entity(&view, |v, _| v.rules.page.rules.len()), 200);
    capture(
        &mut cx,
        window.into(),
        &evidence.join("08-application-rules.png"),
    );
    click(&mut cx, window.into(), "rules-next");
    settle(&mut cx);
    assert_eq!(cx.read_entity(&view, |v, _| v.rules.page.rules.len()), 30);
    click(&mut cx, window.into(), "rules-first");
    settle(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-allow");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.rules.page.rules[0].action.clone()),
        "allow"
    );
    capture(
        &mut cx,
        window.into(),
        &evidence.join("09-rule-allowed.png"),
    );
    click(&mut cx, window.into(), "new-rule");
    cx.update_window(window.into(), |_, window, cx| {
        window.input("/usr/bin/new-application", cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "create-rule-allow");
    settle(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-a", cx);
        window.input("/usr/bin/new-application", cx);
    })
    .unwrap();
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.rules.page.rules.len() == 1
        && v.rules.page.rules[0].duration == "always"
        && v.rules.page.rules[0].action == "allow"));
    capture(
        &mut cx,
        window.into(),
        &evidence.join("10-rule-created.png"),
    );
    accept_changes.store(false, Ordering::Relaxed);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-deny");
    settle(&mut cx);
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
    capture(
        &mut cx,
        window.into(),
        &evidence.join("11-rule-rejected.png"),
    );
    accept_changes.store(true, Ordering::Relaxed);
    click(&mut cx, window.into(), "rule-delete");
    click(&mut cx, window.into(), "rule-delete-cancel");
    assert_eq!(observed_commands.lock().unwrap().len(), 3);
    click(&mut cx, window.into(), "rule-delete");
    click(&mut cx, window.into(), "rule-delete-confirm");
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.rules.page.rules.is_empty()));
    assert_eq!(observed_commands.lock().unwrap().len(), 4);
    capture(
        &mut cx,
        window.into(),
        &evidence.join("12-rule-deleted.png"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-a", cx);
        window.input("/usr/bin/synthetic-app-0", cx);
    })
    .unwrap();
    settle(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-edit");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    capture(&mut cx, editor, &evidence.join("13-rule-editor.png"));
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
    settle(&mut cx);
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
    settle(&mut cx);
    cx.update_window(editor, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(editor, |_, window, cx| {
        window.click(("editor-value", 1u64), cx);
        window.input(".*\\.example\\.test", cx);
        window.click(("editor-case", 1u64), cx);
    })
    .unwrap();
    capture(&mut cx, editor, &evidence.join("14-rule-edited-draft.png"));
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
    if cx.read_entity(&view, |v, _| v.rules.editor_window.is_some()) {
        capture(&mut cx, editor, &evidence.join("editor-save-failed.png"));
    }
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
    settle(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 6);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-duplicate");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    capture(&mut cx, editor, &evidence.join("15-rule-duplicate.png"));
    click(&mut cx, editor, "editor-json-toggle");
    capture(&mut cx, editor, &evidence.join("16-rule-match-json.png"));
    click(&mut cx, editor, "editor-json-toggle");
    click(&mut cx, editor, "editor-custom-duration-enabled");
    cx.update_window(editor, |_, window, cx| {
        window.click("editor-custom-duration", cx);
        window.press("ctrl-a", cx);
        window.input("tomorrow", cx);
    })
    .unwrap();
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 6);
    capture(
        &mut cx,
        editor,
        &evidence.join("17-rule-invalid-duration.png"),
    );
    cx.update_window(editor, |_, window, cx| {
        window.click("editor-custom-duration", cx);
        window.press("ctrl-a", cx);
        window.input("90s", cx);
    })
    .unwrap();
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
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
    settle(&mut cx);
    assert_eq!(
        crate::rules::transfer::Import::from_path(&export_path)
            .unwrap()
            .rules
            .len(),
        2
    );
    click(&mut cx, transfer, "transfer-copy");
    settle(&mut cx);
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(
        crate::rules::transfer::Import::from_text(&copied)
            .unwrap()
            .rules
            .len(),
        2
    );
    capture(&mut cx, transfer, &evidence.join("18-rules-export.png"));
    click(&mut cx, transfer, "transfer-close");
    settle(&mut cx);
    click(&mut cx, window.into(), "rules-import");
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    cx.update_window(transfer, |_, window, cx| {
        window.input(export_path.to_str().unwrap(), cx)
    })
    .unwrap();
    click(&mut cx, transfer, "transfer-read");
    settle(&mut cx);
    click(&mut cx, transfer, "transfer-apply");
    settle(&mut cx);
    assert_eq!(
        observed_commands.lock().unwrap().len(),
        7,
        "existing names are skipped by default"
    );
    capture(&mut cx, transfer, &evidence.join("19-import-skipped.png"));
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
    settle(&mut cx);
    capture(&mut cx, transfer, &evidence.join("20-import-review.png"));
    reject_at.store(9, Ordering::Relaxed);
    click(&mut cx, transfer, "transfer-apply");
    settle(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 9);
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
    capture(
        &mut cx,
        transfer,
        &evidence.join("21-import-partial-failure.png"),
    );
    reject_at.store(usize::MAX, Ordering::Relaxed);
    click(&mut cx, transfer, "transfer-apply");
    settle(&mut cx);
    assert_eq!(
        observed_commands.lock().unwrap().len(),
        10,
        "retry sends only the remaining rule"
    );
    click(&mut cx, transfer, "transfer-close");
    settle(&mut cx);
    assert_eq!(cx.read_entity(&view, |v, _| v.rules.page.rules.len()), 4);
    // Select one rule through its Kit checkbox, then the whole bounded page.
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-mark", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, window.into(), "rules-clear-selection");
    click(&mut cx, window.into(), "rules-select-page");
    capture(
        &mut cx,
        window.into(),
        &evidence.join("22-bulk-selection.png"),
    );
    click(&mut cx, window.into(), "rules-bulk-disable");
    settle(&mut cx);
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
    settle(&mut cx);
    let prompt = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(&mut cx, prompt, &evidence.join("25-prompt-during-bulk.png"));
    click(&mut cx, prompt, "allow-once");
    assert_eq!(during_bulk.try_recv().unwrap().decision, Decision::Allow);
    assert_eq!(
        observed_commands.lock().unwrap().len(),
        11,
        "prompt handling must not wait for the bulk acknowledgement"
    );
    click(&mut cx, transfer, "transfer-stop");
    hold_reply.store(false, Ordering::Release);
    settle(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 11);
    capture(&mut cx, transfer, &evidence.join("23-bulk-stopped.png"));
    click(&mut cx, transfer, "transfer-apply");
    settle(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 14);
    click(&mut cx, transfer, "transfer-close");
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.rules.page.rules.iter().all(|r| !r.enabled)));
    click(&mut cx, window.into(), "rules-select-page");
    click(&mut cx, window.into(), "rules-bulk-delete");
    settle(&mut cx);
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    capture(
        &mut cx,
        transfer,
        &evidence.join("24-bulk-delete-review.png"),
    );
    click(&mut cx, transfer, "transfer-close");
    settle(&mut cx);
    assert_eq!(
        observed_commands.lock().unwrap().len(),
        14,
        "closing bulk delete review sends nothing"
    );
    click(&mut cx, window.into(), "rules-select-page");
    click(&mut cx, window.into(), "rules-bulk-delete");
    settle(&mut cx);
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, transfer, "transfer-apply");
    settle(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 18);
    click(&mut cx, transfer, "transfer-close");
    settle(&mut cx);
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
    settle(&mut cx);
    click(&mut cx, transfer, "transfer-replace");
    click(&mut cx, transfer, "transfer-apply");
    settle(&mut cx);
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
    settle(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    settle(&mut cx);
    assert_eq!(cx.read_entity(&view, |v, _| v.rules.page.rules.len()), 200);
    click(&mut cx, window.into(), "rules-select-page");
    click(&mut cx, window.into(), "rules-next");
    settle(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-mark", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, window.into(), "rules-first");
    settle(&mut cx);
    click(&mut cx, window.into(), "rules-export-selected");
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, transfer, "transfer-copy");
    settle(&mut cx);
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(
        crate::rules::transfer::Import::from_text(&copied)
            .unwrap()
            .rules
            .len(),
        201
    );
    click(&mut cx, transfer, "transfer-close");
    settle(&mut cx);
    click(&mut cx, window.into(), "rules-select-page");
    click(&mut cx, window.into(), "rules-bulk-disable");
    settle(&mut cx);
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, transfer, "transfer-all-matching");
    settle(&mut cx);
    capture(&mut cx, transfer, &evidence.join("26-bulk-all-pages.png"));
    click(&mut cx, transfer, "transfer-apply");
    for _ in 0..10 {
        settle(&mut cx);
        if observed_commands.lock().unwrap().len() == 248 {
            break;
        }
    }
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
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.rules.page.rules.iter().all(|r| !r.enabled)));
    cx.update_window(window.into(), |_, window, cx| {
        window.within("main-tabs").click(0usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("history-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "history-details");
    settle(&mut cx);
    let details = cx.read_entity(&view, |v, _| v.history.detail_window.unwrap());
    capture(
        &mut cx,
        details,
        &evidence.join("27-connection-overview.png"),
    );
    click(&mut cx, details, "connection-copy");
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(
        copied.contains("UID: 1000")
            && copied.contains("PID: 4567")
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
    capture(
        &mut cx,
        details,
        &evidence.join("28-connection-arguments.png"),
    );
    cx.update_window(details, |_, window, cx| {
        window.within("connection-tabs").click(2usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    capture(
        &mut cx,
        details,
        &evidence.join("29-connection-matched-rule.png"),
    );
    click(&mut cx, details, "connection-edit-rule");
    settle(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    capture(
        &mut cx,
        editor,
        &evidence.join("30-current-matched-rule.png"),
    );
    click(&mut cx, editor, "editor-cancel");
    settle(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 248);
    click(&mut cx, details, "connection-create-destination");
    settle(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    cx.update_window(editor, |_, window, cx| {
        window.click("editor-name", cx);
        window.press("ctrl-a", cx);
        window.input("fixture.history.from-event", cx);
    })
    .unwrap();
    capture(&mut cx, editor, &evidence.join("31-rule-from-history.png"));
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
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
    settle(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    cx.update_window(editor, |_, window, cx| {
        window.click("editor-name", cx);
        window.press("ctrl-a", cx);
        window.input("fixture.history.from-event", cx);
    })
    .unwrap();
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
    assert_eq!(
        observed_commands.lock().unwrap().len(),
        249,
        "a new rule cannot silently overwrite an existing name"
    );
    capture(
        &mut cx,
        editor,
        &evidence.join("32-history-rule-name-collision.png"),
    );
    click(&mut cx, editor, "editor-cancel");
    settle(&mut cx);
    click(&mut cx, details, "connection-close");
    settle(&mut cx);
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
    settle(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("history-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "history-details");
    settle(&mut cx);
    let details = cx.read_entity(&view, |v, _| v.history.detail_window.unwrap());
    capture(
        &mut cx,
        details,
        &evidence.join("33-limited-connection-details.png"),
    );
    click(&mut cx, details, "connection-copy");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("Limited fields: arguments")
    );
    click(&mut cx, details, "connection-close");
    settle(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.within("main-tabs").click(1usize, cx)
    })
    .unwrap();
    settle(&mut cx);
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
    settle(&mut cx);
    assert_eq!(observed_commands.lock().unwrap().len(), 249);
    assert!(cx.read_entity(&view, |v, _| {
        v.rules
            .error
            .as_ref()
            .is_some_and(|error| error.contains("already exists"))
    }));
    capture(
        &mut cx,
        window.into(),
        &evidence.join("34-quick-rule-name-collision.png"),
    );
    click(&mut cx, window.into(), "create-rule-cancel");
    drop(view);
    drop(cx);

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
    let mut cx = context();
    let mut view = None;
    let window = cx
        .open_window(size(px(650.), px(490.)), |window, cx| {
            let v = cx.new(|cx| PromptWindow::new(backend.handle.clone(), window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    capture(&mut cx, window.into(), &evidence.join("05-permission.png"));
    click(&mut cx, window.into(), "allow-once");
    assert_eq!(allow.try_recv().unwrap().decision, Decision::Allow);
    // Tab/Space use the actual Kit focus and activation path, with Deny first.
    cx.update_window(window.into(), |_, window, cx| {
        window.press("tab", cx);
        window.render_frame(cx);
        window.press("space", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(deny.try_recv().unwrap().decision, Decision::Deny);
    assert!(backend.handle.prompts.snapshot().is_empty());
    capture(
        &mut cx,
        window.into(),
        &evidence.join("06-permissions-complete.png"),
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
    settle(&mut cx);
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
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("prompt-constraint", uid), cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.scroll(
            ("prompt-constraint", uid),
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-260.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("prompt-constraint", port), cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.scroll(
            ("prompt-constraint", port),
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-1200.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("prompt-constraint", checksum), cx)
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, window.into(), "prompt-priority");
    click(&mut cx, window.into(), "prompt-logging");
    cx.update_window(window.into(), |_, window, cx| {
        window.scroll(
            ("prompt-constraint", checksum),
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(2000.))),
            cx,
        )
    })
    .unwrap();
    cx.run_until_parked();
    capture(
        &mut cx,
        window.into(),
        &evidence.join("35-prompt-lasting-options.png"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.within("prompt-tabs").click(1usize, cx)
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, window.into(), "prompt-copy-details");
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(copied.contains("POPUP_TEST: live-only-value") && copied.contains("process.hash.md5"));
    capture(
        &mut cx,
        window.into(),
        &evidence.join("36-prompt-live-details.png"),
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
        .open_window(size(px(1100.), px(780.)), |window, cx| {
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
    settle(&mut cx);
    cx.update_window(catalogue.into(), |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input(&lasting.name, cx);
    })
    .unwrap();
    settle(&mut cx);
    assert!(cx.read_entity(&catalogue_view, |view, _| {
        view.rules.page.awaiting.contains(&lasting.name)
    }));
    cx.update_window(catalogue.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    cx.run_until_parked();
    capture(
        &mut cx,
        catalogue.into(),
        &evidence.join("37-popup-rule-awaiting.png"),
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
    settle(&mut cx);
    assert!(cx.read_entity(
        &catalogue_view,
        |view, _| view.rules.page.awaiting.is_empty() && view.rules.page.rules[0] == observed
    ));
    capture(
        &mut cx,
        catalogue.into(),
        &evidence.join("38-popup-rule-observed.png"),
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
    settle(&mut cx);
    capture(
        &mut cx,
        window.into(),
        &evidence.join("39-prompt-reset.png"),
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
    settle(&mut cx);
    click(&mut cx, window.into(), "prompt-more");
    pick(&mut cx, window.into(), "prompt-duration", "5 minutes");
    pick(
        &mut cx,
        window.into(),
        "prompt-target",
        "Destination network: 192.168.1.0/24",
    );
    capture(
        &mut cx,
        window.into(),
        &evidence.join("40-prompt-temporary-network.png"),
    );
    click(&mut cx, window.into(), "allow-once");
    let temporary = runtime.block_on(temporary).unwrap();
    assert_eq!(temporary.action, "allow");
    assert_eq!(temporary.duration, "5m");
    assert_eq!(temporary.operator.as_ref().unwrap().r#type, "network");
    assert_eq!(temporary.operator.unwrap().data, "192.168.1.0/24");
    let mut settings_owner = None;
    let dashboard = cx
        .open_window(size(px(1100.), px(780.)), |window, cx| {
            let entity = cx
                .new(|cx| Dashboard::new(backend.handle.clone(), options(dir.path()), window, cx));
            settings_owner = Some(entity.clone());
            cx.new(|cx| gpui_kit::base::Root::new(entity, window, cx))
        })
        .unwrap();
    cx.run_until_parked();
    click(&mut cx, dashboard.into(), "preferences");
    let owner = settings_owner.unwrap();
    let preferences = cx.read_entity(&owner, |view, _| view.preferences_window.unwrap());
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
    settle(&mut cx);
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
    capture(
        &mut cx,
        preferences,
        &evidence.join("41-popup-preferences.png"),
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
    settle(&mut cx);
    capture(
        &mut cx,
        window.into(),
        &evidence.join("42-configured-popup.png"),
    );
    click(&mut cx, window.into(), "reject-once");
    let configured = runtime.block_on(configured).unwrap();
    assert_eq!(configured.duration, "12h");
    assert!(configured.precedence && configured.nolog);
    assert_eq!(configured.operator.unwrap().operand, "process.path");
    click(&mut cx, preferences, "preferences-popups");
    click(&mut cx, preferences, "preferences-save");
    settle(&mut cx);
    assert!(!backend.handle.preferences.snapshot().popups);
    capture(
        &mut cx,
        preferences,
        &evidence.join("43-disabled-popups.png"),
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
    settle(&mut cx);
    assert!(
        !backend.handle.preferences.snapshot().popups,
        "invalid timeout must not save staged changes"
    );
    capture(
        &mut cx,
        preferences,
        &evidence.join("44-invalid-popup-timeout.png"),
    );
    fill(&mut cx, preferences, "preferences-timeout", "5");
    pick(&mut cx, preferences, "preferences-fallback", "Reject once");
    click(&mut cx, preferences, "preferences-save");
    settle(&mut cx);
    let advanced = backend.handle.preferences.snapshot();
    assert!(advanced.popups && advanced.show_advanced && advanced.constraints.len() == 4);
    assert_eq!(advanced.timeout, Some(5));
    assert_eq!(advanced.fallback, Some(Decision::Reject));
    capture(
        &mut cx,
        preferences,
        &evidence.join("45-saved-advanced-conditions.png"),
    );
    let mut asking = daemon.clone();
    let con = popup_connection.clone();
    let configured = runtime.spawn(async move { asking.ask_rule(con).await.unwrap().into_inner() });
    settle(&mut cx);
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
    capture(
        &mut cx,
        window.into(),
        &evidence.join("46-advanced-default-prompt.png"),
    );
    click(&mut cx, window.into(), "allow-once");
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
    settle(&mut cx);
    assert_eq!(
        backend.handle.prompts.snapshot()[0]
            .initial
            .unavailable
            .len(),
        3
    );
    capture(
        &mut cx,
        window.into(),
        &evidence.join("47-unavailable-default-conditions.png"),
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
    settle(&mut cx);
    assert_eq!(backend.handle.preferences.snapshot().timeout, Some(1));
    let timed_before = backend.handle.status().timed_out;
    let mut asking = daemon.clone();
    let con = popup_connection.clone();
    let timed = runtime.spawn(async move { asking.ask_rule(con).await.unwrap().into_inner() });
    settle(&mut cx);
    capture(
        &mut cx,
        window.into(),
        &evidence.join("48-configured-fallback-countdown.png"),
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
    drop(cx);
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
        "Click Allow once",
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
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","screenshots":["01-history.png","02-older.png","03-empty-search.png","04-filtered.png","05-permission.png","06-permissions-complete.png","07-history-warning.png","08-application-rules.png","09-rule-allowed.png","10-rule-created.png","11-rule-rejected.png","12-rule-deleted.png","13-rule-editor.png","14-rule-edited-draft.png","15-rule-duplicate.png","16-rule-match-json.png","17-rule-invalid-duration.png","18-rules-export.png","19-import-skipped.png","20-import-review.png","21-import-partial-failure.png","22-bulk-selection.png","23-bulk-stopped.png","24-bulk-delete-review.png","25-prompt-during-bulk.png","26-bulk-all-pages.png","27-connection-overview.png","28-connection-arguments.png","29-connection-matched-rule.png","30-current-matched-rule.png","31-rule-from-history.png","32-history-rule-name-collision.png","33-limited-connection-details.png","34-quick-rule-name-collision.png","35-prompt-lasting-options.png","36-prompt-live-details.png","37-popup-rule-awaiting.png","38-popup-rule-observed.png","39-prompt-reset.png","40-prompt-temporary-network.png","41-popup-preferences.png","42-configured-popup.png","43-disabled-popups.png","44-invalid-popup-timeout.png","45-saved-advanced-conditions.png","46-advanced-default-prompt.png","47-unavailable-default-conditions.png","48-configured-fallback-countdown.png"],"real_gpu":true,"scoped_socket_removed":true,"proof_boundary":"Real GPUI views/input and SQLite; rules use an isolated gRPC daemon fixture with acknowledgements; synthetic history and prompts enter at the backend boundary."})).unwrap()).unwrap();
}

#[test]
fn headless_statistics() {
    use crate::statistics::{self, Kind};
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
    let mut cx = context();
    let mut view = None;
    let window = cx
        .open_window(size(px(1180.), px(900.)), |window, cx| {
            let dashboard =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(dashboard.clone());
            cx.new(|cx| gpui_kit::base::Root::new(dashboard, window, cx))
        })
        .unwrap();
    let window: AnyWindowHandle = window.into();
    let view = view.unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("ctrl-3", cx))
        .unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page.len()),
        200
    );
    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page[0].hits),
        350
    );
    capture(
        &mut cx,
        window,
        &evidence.join("01-statistics-applications.png"),
    );
    click(&mut cx, window, "statistics-chart");
    settle(&mut cx);
    capture(&mut cx, window, &evidence.join("02-statistics-chart.png"));
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
        settle(&mut cx);
        assert_eq!(
            cx.read_entity(&view, |view, _| (
                view.statistics.query.kind,
                view.statistics.snapshot.as_ref().unwrap().total
            )),
            (kind, total)
        );
    }
    capture(
        &mut cx,
        window,
        &evidence.join("03-statistics-protocols.png"),
    );
    cx.update_window(window, |_, window, cx| {
        window.within("statistics-kinds").click(0usize, cx)
    })
    .unwrap();
    settle(&mut cx);
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
    click(&mut cx, window, "statistics-older");
    settle(&mut cx);
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
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page[0].hits),
        201
    );
    capture(
        &mut cx,
        window,
        &evidence.join("04-frozen-statistics-page.png"),
    );
    click(&mut cx, window, "statistics-previous");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page.len()),
        200
    );
    click(&mut cx, window, "statistics-latest");
    settle(&mut cx);
    click(&mut cx, window, "statistics-older");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page[0].hits),
        999
    );
    click(&mut cx, window, "statistics-latest");
    settle(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window, "statistics-export");
    settle(&mut cx);
    let export = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    });
    let csv = evidence.join("daemon-counters.csv");
    fill(
        &mut cx,
        export,
        "statistics-export-path",
        csv.to_str().unwrap(),
    );
    click(&mut cx, export, "statistics-export-save");
    settle(&mut cx);
    let original = std::fs::read_to_string(&csv).unwrap();
    assert_eq!(original.lines().count(), 351);
    click(&mut cx, export, "statistics-export-save");
    settle(&mut cx);
    assert_eq!(std::fs::read_to_string(&csv).unwrap(), original);
    capture(
        &mut cx,
        export,
        &evidence.join("05-statistics-existing-export.png"),
    );
    click(&mut cx, export, "statistics-export-json");
    let json = evidence.join("daemon-counters.jsonl");
    fill(
        &mut cx,
        export,
        "statistics-export-path",
        json.to_str().unwrap(),
    );
    click(&mut cx, export, "statistics-export-save");
    settle(&mut cx);
    assert_eq!(std::fs::read_to_string(&json).unwrap().lines().count(), 350);
    click(&mut cx, export, "statistics-export-copy");
    settle(&mut cx);
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(copied.lines().count(), 201);
    capture(
        &mut cx,
        export,
        &evidence.join("06-statistics-export-copy.png"),
    );
    click(&mut cx, export, "statistics-export-close");
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("stats-000", cx)
    })
    .unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page.len()),
        1
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("statistics-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window, "statistics-history");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.page.rows.len()),
        1
    );
    assert_eq!(
        cx.read_entity(&view, |view, _| view.history.query.filters.len()),
        2
    );
    capture(
        &mut cx,
        window,
        &evidence.join("07-statistics-history-drilldown.png"),
    );
    cx.update_window(window, |_, window, cx| window.press("ctrl-3", cx))
        .unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("statistics-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window, "statistics-reset-selected");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("08-statistics-reset-confirmation.png"),
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
    settle(&mut cx);
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
    assert!(history.rows.is_empty());
    capture(
        &mut cx,
        window,
        &evidence.join("09-statistics-reset-result.png"),
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
        settle(&mut cx);
    }
    assert_eq!(
        cx.read_entity(&view, |view, _| view.statistics.page[0].hits),
        0
    );
    click(&mut cx, window, "statistics-reset-all");
    click(&mut cx, window, "statistics-reset-confirm");
    settle(&mut cx);
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
    capture(
        &mut cx,
        window,
        &evidence.join("10-statistics-category-reset.png"),
    );
    drop(view);
    drop(cx);
    drop(daemon);
    drop(other);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let screenshots = (1..=10)
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
        .map(|(index, name)| format!("{index:02}-{name}.png"))
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&serde_json::json!({"flow":"Actual Kit tabs/selects/chart/list/reset/export controls, RPC absolute counters, frozen paging, CSV/JSON/clipboard, exact history drilldown and local baseline resets"})).unwrap()).unwrap();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":screenshots,"boundary":"Production GPUI input and gRPC counters against an isolated fixture; no system daemon or firewall changes"})).unwrap()).unwrap();
}

#[test]
fn headless_nodes() {
    use crate::nodes;
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let(alpha,beta,alpha_tx,mut alpha_commands,beta_tx,mut beta_commands)=runtime.block_on(async{
  use hyper_util::rt::TokioIo;
  async fn connect(path:PathBuf,name:&str)->(protocol::ui_client::UiClient<tonic::transport::Channel>,tokio::sync::mpsc::Sender<protocol::NotificationReply>,tonic::Streaming<protocol::Notification>){let channel=tonic::transport::Endpoint::try_from("http://localhost").unwrap().connect_with_connector(tower::service_fn(move |_|{let path=path.clone();async move{tokio::net::UnixStream::connect(path).await.map(TokioIo::new)}})).await.unwrap();let mut daemon=protocol::ui_client::UiClient::new(channel);daemon.subscribe(protocol::ClientConfig{name:name.into(),version:"synthetic-kernel".into(),is_firewall_running:true,log_level:2,config:r#"{"DefaultAction":"deny","DefaultDuration":"once","Future":{"keep":true}}"#.into(),rules:vec![protocol::Rule{name:format!("{name}-only-rule"),action:"deny".into(),duration:"always".into(),enabled:true,operator:Some(protocol::Operator{r#type:"simple".into(),operand:"process.path".into(),data:format!("/bin/{name}"),..Default::default()}),..Default::default()}],..Default::default()}).await.unwrap();let(tx,rx)=tokio::sync::mpsc::channel(8);let stream=daemon.notifications(tokio_stream::wrappers::ReceiverStream::new(rx)).await.unwrap().into_inner();(daemon,tx,stream)}
  let(a,at,as_)=connect(config.socket.clone(),"alpha-node").await;let(b,bt,bs)=connect(config.socket.clone(),"beta-node").await;backend.handle.history.flush().await.unwrap();(a,b,at,as_,bt,bs)
 });
    let mut cx = context();
    let mut view = None;
    let window = cx
        .open_window(size(px(1180.), px(900.)), |window, cx| {
            let dashboard =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(dashboard.clone());
            cx.new(|cx| gpui_kit::base::Root::new(dashboard, window, cx))
        })
        .unwrap();
    let window: AnyWindowHandle = window.into();
    let view = view.unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.within("main-tabs").click(3usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    assert_eq!(cx.read_entity(&view, |view, _| view.nodes.page.len()), 2);
    capture(&mut cx, window, &evidence.join("01-connected-nodes.png"));
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 1usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, window, "nodes-rules");
    settle(&mut cx);
    assert_eq!(backend.handle.status().session.unwrap().name, "beta-node");
    assert_eq!(
        cx.read_entity(&view, |view, _| view.rules.page.rules[0].name.clone()),
        "beta-node-only-rule"
    );
    capture(
        &mut cx,
        window,
        &evidence.join("02-selected-node-rules.png"),
    );
    cx.update_window(window, |_, window, cx| window.press("ctrl-4", cx))
        .unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, window, "nodes-interception");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("03-stop-interception-review.png"),
    );
    click(&mut cx, window, "nodes-cancel");
    assert!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    click(&mut cx, window, "nodes-interception");
    click(&mut cx, window, "nodes-confirm");
    settle(&mut cx);
    let command = runtime.block_on(alpha_commands.message()).unwrap().unwrap();
    assert_eq!(command.client_name, "alpha-node");
    assert_eq!(command.r#type, protocol::Action::DisableInterception as i32);
    assert!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    capture(
        &mut cx,
        window,
        &evidence.join("04-held-interception-ack.png"),
    );
    // Another node's actual AskRule remains usable while interception ACK is held.
    let mut asking = beta.clone();
    let ask = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/bin/beta-prompt".into(),
                dst_ip: "192.0.2.10".into(),
                dst_port: 443,
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let prompt = cx.read_entity(&view, |view, _| view.prompt_window.unwrap());
    capture(
        &mut cx,
        prompt,
        &evidence.join("05-other-node-permission.png"),
    );
    click(&mut cx, prompt, "allow-once");
    settle(&mut cx);
    assert_eq!(runtime.block_on(ask).unwrap().action, "allow");
    runtime
        .block_on(alpha_tx.send(protocol::NotificationReply {
            id: command.id,
            code: protocol::NotificationReplyCode::Ok as i32,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
    assert!(
        !nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    assert!(
        nodes::detail(&config.database, "beta-node")
            .unwrap()
            .row
            .running
    );
    capture(
        &mut cx,
        window,
        &evidence.join("06-confirmed-interception.png"),
    );
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window, "nodes-config");
    settle(&mut cx);
    let editor = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|handle| !before.contains(handle))
            .unwrap()
    });
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-config-modes").click(1usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("[]", cx)
    })
    .unwrap();
    click(&mut cx, editor, "node-config-review");
    settle(&mut cx);
    capture(
        &mut cx,
        editor,
        &evidence.join("07-invalid-node-config.png"),
    );
    let document = r#"{"DefaultAction":"allow","DefaultDuration":"5m","Future":{"keep":true}}"#;
    cx.update_window(editor, |_, window, cx| {
        window.press("tab", cx);
    })
    .unwrap();
    // The initial textarea is the first focusable control; click Review restored
    // button focus, so Shift+Tab returns to the actual multiline editor.
    cx.update_window(editor, |_, window, cx| {
        window.press("shift-tab", cx);
        window.press("shift-tab", cx);
        window.press("ctrl-a", cx);
        window.input(document, cx)
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, editor, "node-config-review");
    settle(&mut cx);
    capture(&mut cx, editor, &evidence.join("08-review-node-config.png"));
    click(&mut cx, editor, "node-config-cancel");
    settle(&mut cx);
    // Review again uses the staged user document, not a state setter.
    click(&mut cx, editor, "node-config-review");
    click(&mut cx, editor, "node-config-confirm");
    settle(&mut cx);
    let command = runtime.block_on(alpha_commands.message()).unwrap().unwrap();
    assert_eq!(command.r#type, protocol::Action::ChangeConfig as i32);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&command.data).unwrap()["Future"]["keep"],
        true
    );
    assert_ne!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .config,
        document
    );
    runtime
        .block_on(alpha_tx.send(protocol::NotificationReply {
            id: command.id,
            code: protocol::NotificationReplyCode::Ok as i32,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
    assert_eq!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .config,
        document
    );
    capture(
        &mut cx,
        editor,
        &evidence.join("09-confirmed-node-config.png"),
    );
    click(&mut cx, editor, "node-config-close");
    settle(&mut cx);
    // An ERROR keeps beta's confirmed running state and marks only beta uncertain.
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 1usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, window, "nodes-interception");
    click(&mut cx, window, "nodes-confirm");
    settle(&mut cx);
    let command = runtime.block_on(beta_commands.message()).unwrap().unwrap();
    runtime
        .block_on(beta_tx.send(protocol::NotificationReply {
            id: command.id,
            code: protocol::NotificationReplyCode::Error as i32,
            data: "synthetic interception failure".into(),
        }))
        .unwrap();
    settle(&mut cx);
    assert!(
        nodes::detail(&config.database, "beta-node")
            .unwrap()
            .row
            .running
    );
    assert!(
        !backend
            .handle
            .history
            .health
            .catalogues
            .uncertain(backend.handle.node("alpha-node").unwrap().id)
    );
    capture(&mut cx, window, &evidence.join("10-node-command-error.png"));
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("absent-node", cx)
    })
    .unwrap();
    settle(&mut cx);
    assert!(cx.read_entity(&view, |view, _| view.nodes.page.is_empty()));
    capture(&mut cx, window, &evidence.join("11-node-search-empty.png"));
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    drop(cx);
    drop(alpha_tx);
    drop(beta_tx);
    drop(alpha_commands);
    drop(beta_commands);
    drop(alpha);
    drop(beta);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let screenshots = (1..=11)
        .zip([
            "connected-nodes",
            "selected-node-rules",
            "stop-interception-review",
            "held-interception-ack",
            "other-node-permission",
            "confirmed-interception",
            "invalid-node-config",
            "review-node-config",
            "confirmed-node-config",
            "node-command-error",
            "node-search-empty",
        ])
        .map(|(index, name)| format!("{index:02}-{name}.png"))
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":screenshots,"boundary":"Actual GPUI input, two isolated gRPC daemon fixtures, acknowledged node controls and SQLite; system daemon/firewall unchanged"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Select Nodes through main tab; choose beta for its rules", "Cancel then confirm alpha interception; hold ACK, answer beta permission through popup; ACK updates only alpha", "Reject nonobject JSON; edit complete configuration, review/back/confirm; ACK persists unknown fields", "Beta ERROR preserves confirmed values and marks only beta uncertain; typed search reaches empty state"]).unwrap()).unwrap();
}

#[test]
fn headless_daemon_preferences() {
    use crate::nodes;
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
    let accept = Arc::new(AtomicBool::new(true));
    let observed = Arc::new(Mutex::new(Vec::<protocol::Notification>::new()));
    let (daemon,task)=runtime.block_on(async {
  use hyper_util::rt::TokioIo;let path=config.socket.clone();let channel=tonic::transport::Endpoint::try_from("http://localhost").unwrap().connect_with_connector(tower::service_fn(move |_|{let path=path.clone();async move{tokio::net::UnixStream::connect(path).await.map(TokioIo::new)}})).await.unwrap();let mut daemon=protocol::ui_client::UiClient::new(channel);
  daemon.subscribe(protocol::ClientConfig{name:"preferences-fixture".into(),is_firewall_running:true,config:r#"{"DefaultAction":"deny","DefaultDuration":"once","ProcMonitorMethod":"proc","Rules":{"Path":"/old/rules","Future":true},"Server":{"Address":"unix:///fixture.sock","Authentication":{"Type":"simple","Future":"keep"}},"Future":{"precise":1.2345678901234567890123456789}}"#.into(),..Default::default()}).await.unwrap();let(tx,rx)=tokio::sync::mpsc::channel(8);let mut stream=daemon.notifications(tokio_stream::wrappers::ReceiverStream::new(rx)).await.unwrap().into_inner();let hold=hold.clone();let accept=accept.clone();let observed=observed.clone();let task=tokio::spawn(async move{while let Some(command)=stream.message().await.unwrap(){assert!(observed.lock().unwrap().len()<16);observed.lock().unwrap().push(command.clone());while hold.load(Ordering::Acquire){tokio::time::sleep(Duration::from_millis(5)).await;}tx.send(protocol::NotificationReply{id:command.id,code:if accept.load(Ordering::Acquire){protocol::NotificationReplyCode::Ok}else{protocol::NotificationReplyCode::Error}as i32,data:if accept.load(Ordering::Acquire){""}else{"synthetic config save failure"}.into()}).await.unwrap();}});backend.handle.history.flush().await.unwrap();(daemon,task)
 });
    let original = nodes::detail(&config.database, "preferences-fixture")
        .unwrap()
        .config;
    let mut cx = context();
    let mut view = None;
    let window = cx
        .open_window(size(px(1180.), px(900.)), |window, cx| {
            let dashboard =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(dashboard.clone());
            cx.new(|cx| gpui_kit::base::Root::new(dashboard, window, cx))
        })
        .unwrap();
    let window: AnyWindowHandle = window.into();
    let view = view.unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-4", cx);
    })
    .unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    settle(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window, "nodes-config");
    settle(&mut cx);
    let editor = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|handle| !before.contains(handle))
            .unwrap()
    });
    capture(&mut cx, editor, &evidence.join("01-daemon-general.png"));
    pick(&mut cx, editor, "daemon-default-action", "Allow");
    pick(&mut cx, editor, "daemon-default-duration", "Forever");
    fill(&mut cx, editor, "daemon-rules-path", "/new/rules");
    cx.update_window(editor, |_, window, cx| {
        window.scroll(
            "daemon-rules-path",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-250.))),
            cx,
        )
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, editor, "daemon-checksums");
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-setting-groups").click(1usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    pick(&mut cx, editor, "daemon-log-level", "Warning");
    click(&mut cx, editor, "daemon-log-utc");
    capture(&mut cx, editor, &evidence.join("02-daemon-logging.png"));
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-setting-groups").click(3usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    fill(&mut cx, editor, "daemon-max-events", "-1");
    click(&mut cx, editor, "node-config-review");
    settle(&mut cx);
    assert!(observed.lock().unwrap().is_empty());
    capture(
        &mut cx,
        editor,
        &evidence.join("03-invalid-daemon-limit.png"),
    );
    fill(&mut cx, editor, "daemon-max-events", "300");
    fill(&mut cx, editor, "daemon-max-stats", "75");
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-setting-groups").click(2usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    fill(&mut cx, editor, "daemon-firewall-interval", "1m30s");
    capture(&mut cx, editor, &evidence.join("04-daemon-firewall.png"));
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-setting-groups").click(4usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    pick(&mut cx, editor, "daemon-auth-type", "Mutual TLS");
    fill(&mut cx, editor, "daemon-ca-cert", "/daemon/ca.pem");
    capture(&mut cx, editor, &evidence.join("05-daemon-connection.png"));
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    settle(&mut cx);
    assert!(observed.lock().unwrap().is_empty());
    capture(
        &mut cx,
        editor,
        &evidence.join("06-review-daemon-changes.png"),
    );
    click(&mut cx, editor, "node-config-cancel");
    settle(&mut cx);
    // Switching through JSON and back must preserve staged typed values and unknown numbers.
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-config-modes").click(1usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    capture(
        &mut cx,
        editor,
        &evidence.join("07-complete-daemon-json.png"),
    );
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-config-modes").click(0usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, editor, "node-config-review");
    click(&mut cx, editor, "node-config-confirm");
    settle(&mut cx);
    assert_eq!(observed.lock().unwrap().len(), 1);
    assert_eq!(
        nodes::detail(&config.database, "preferences-fixture")
            .unwrap()
            .config,
        original
    );
    let command = observed.lock().unwrap()[0].clone();
    assert_eq!(command.r#type, protocol::Action::ChangeConfig as i32);
    let value: serde_json::Value = serde_json::from_str(&command.data).unwrap();
    assert_eq!(value["DefaultAction"], "allow");
    assert_eq!(value["DefaultDuration"], "always");
    assert_eq!(value["Rules"]["Path"], "/new/rules");
    assert_eq!(value["Rules"]["EnableChecksums"], true);
    assert_eq!(value["Rules"]["Future"], true);
    assert_eq!(value["LogLevel"], 3);
    assert_eq!(value["LogUTC"], true);
    assert_eq!(value["Stats"]["MaxEvents"], 300);
    assert_eq!(value["Stats"]["MaxStats"], 75);
    assert!(value["Stats"].get("Workers").is_none());
    assert_eq!(value["FwOptions"]["MonitorInterval"], "1m30s");
    assert!(value["FwOptions"].get("QueueNum").is_none());
    assert_eq!(value["Server"]["Address"], "unix:///fixture.sock");
    assert_eq!(value["Server"]["Authentication"]["Type"], "tls-mutual");
    assert_eq!(value["Server"]["Authentication"]["Future"], "keep");
    assert_eq!(
        value["Server"]["Authentication"]["TLSOptions"]["CACert"],
        "/daemon/ca.pem"
    );
    assert!(command.data.contains("1.2345678901234567890123456789"));
    capture(
        &mut cx,
        editor,
        &evidence.join("08-awaiting-daemon-save.png"),
    );
    let mut asking = daemon.clone();
    let ask = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/bin/config-prompt".into(),
                dst_ip: "192.0.2.1".into(),
                dst_port: 443,
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let prompt = cx.read_entity(&view, |view, _| view.prompt_window.unwrap());
    capture(
        &mut cx,
        prompt,
        &evidence.join("09-permission-during-config.png"),
    );
    click(&mut cx, prompt, "allow-once");
    settle(&mut cx);
    let reply = runtime.block_on(ask).unwrap();
    assert_eq!(reply.action, "allow");
    assert_eq!(reply.duration, "once");
    hold.store(false, Ordering::Release);
    settle(&mut cx);
    assert_eq!(
        nodes::detail(&config.database, "preferences-fixture")
            .unwrap()
            .config,
        command.data
    );
    capture(
        &mut cx,
        editor,
        &evidence.join("10-confirmed-daemon-settings.png"),
    );
    // A subsequent error must preserve the first confirmed document.
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-setting-groups").click(0usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    pick(&mut cx, editor, "daemon-default-action", "Reject");
    accept.store(false, Ordering::Release);
    click(&mut cx, editor, "node-config-review");
    click(&mut cx, editor, "node-config-confirm");
    settle(&mut cx);
    assert_eq!(observed.lock().unwrap().len(), 2);
    assert_eq!(
        nodes::detail(&config.database, "preferences-fixture")
            .unwrap()
            .config,
        command.data
    );
    assert!(
        backend
            .handle
            .history
            .health
            .catalogues
            .uncertain(backend.handle.node("preferences-fixture").unwrap().id)
    );
    capture(&mut cx, editor, &evidence.join("11-daemon-save-error.png"));
    click(&mut cx, editor, "node-config-close");
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    drop(cx);
    task.abort();
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let screenshots = (1..=11)
        .zip([
            "daemon-general",
            "daemon-logging",
            "invalid-daemon-limit",
            "daemon-firewall",
            "daemon-connection",
            "review-daemon-changes",
            "complete-daemon-json",
            "awaiting-daemon-save",
            "permission-during-config",
            "confirmed-daemon-settings",
            "daemon-save-error",
        ])
        .map(|(index, name)| format!("{index:02}-{name}.png"))
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":screenshots,"boundary":"Actual Kit typed settings, review/JSON modes and input, isolated gRPC ACK/error, SQLite persistence and independent permission; real daemon untouched"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Enter selects first node; open native daemon forms", "Edit policy/rules/logging/statistics/firewall/TLS fields, reject negative limit before sending", "Review/back/JSON/settings preserves only changed fields and exact unknown numbers", "Hold ACK without cache update, answer Allow once despite daemon duration change, confirm save", "Second configuration ERROR preserves confirmed data and blocks uncertain node changes"]).unwrap()).unwrap();
}
