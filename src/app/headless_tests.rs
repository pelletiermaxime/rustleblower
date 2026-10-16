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
fn keyboard_button(cx: &mut HeadlessAppContext, window: AnyWindowHandle, id: &'static str) {
    for _ in 0..64 {
        let focused = cx
            .update_window(window, |_, window, _| {
                window.find(id).focused() == Some(true)
            })
            .unwrap();
        if focused {
            cx.update_window(window, |_, window, cx| window.press("space", cx))
                .unwrap();
            cx.run_until_parked();
            return;
        }
        cx.update_window(window, |_, window, cx| window.press("tab", cx))
            .unwrap();
        cx.run_until_parked();
    }
    panic!("{id} must be reachable with Tab");
}
fn click_prompt_option(cx: &mut HeadlessAppContext, window: AnyWindowHandle, id: gpui::ElementId) {
    for _ in 0..64 {
        let clicked = cx
            .update_window(window, |_, window, cx| {
                let body = window.find("prompt-options-body").bounds();
                let target = window.find(id.clone());
                if target.visible()
                    && target.bounds().top() >= body.top()
                    && target.bounds().bottom() <= body.bottom()
                {
                    window.click(id.clone(), cx);
                    true
                } else {
                    let delta = if target.bounds().center().y > body.center().y {
                        -80.
                    } else {
                        80.
                    };
                    window.scroll(
                        "prompt-options-body",
                        gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(delta))),
                        cx,
                    );
                    false
                }
            })
            .unwrap();
        cx.run_until_parked();
        if clicked {
            return;
        }
    }
    panic!("{id:?} must be reachable by wheel input");
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
        if text.is_empty() {
            window.press("backspace", cx);
        } else {
            window.input(text, cx);
        }
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
fn headless_tray_lifecycle() {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let mut cx = context();
    cx.update(|cx| super::appearance::init(&config.database, cx));
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(super::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    let (tray, actions, available) = super::tray::Client::fixture();
    cx.update_entity(&view, |view, _| view.tray = Some(tray));
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("ctrl-4", cx))
        .unwrap();
    settle(&mut cx);
    capture(&mut cx, window, &evidence.join("01-before-close.png"));
    cx.update_window(window, |_, window, cx| {
        let close = view.update(cx, |view, _| view.should_close_dashboard(window));
        assert!(close);
        window.remove_window();
    })
    .unwrap();
    settle(&mut cx);
    assert!(cx.update_window(window, |_, _, _| ()).is_err());
    assert!(!backend.handle.cancel.is_cancelled());

    let (_, mut reply) = backend
        .handle
        .prompts
        .insert_with_defaults(
            1,
            protocol::Connection {
                process_path: "/usr/bin/tray-fixture".into(),
                dst_host: "tray.example".into(),
                dst_ip: "192.0.2.20".into(),
                ..Default::default()
            },
            Instant::now() + Duration::from_secs(30),
            crate::prompts::Initial::default(),
        )
        .unwrap();
    settle(&mut cx);
    let popup = cx.read_entity(&view, |view, _| {
        assert!(view.main_window.is_none());
        view.prompt_window.unwrap()
    });
    capture(
        &mut cx,
        popup,
        &evidence.join("02-permission-while-hidden.png"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(reply.try_recv().unwrap().decision, Decision::Deny);
    settle(&mut cx);

    actions.try_send(super::tray::Action::Open).unwrap();
    settle(&mut cx);
    let reopened = cx.read_entity(&view, |view, _| {
        assert!(matches!(view.screen, super::Screen::Nodes));
        view.main_window.unwrap()
    });
    capture(&mut cx, reopened, &evidence.join("03-restored.png"));
    // Repeated activation uses the same window.
    actions.try_send(super::tray::Action::Open).unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.main_window),
        Some(reopened)
    );
    cx.update_window(reopened, |_, window, cx| {
        assert!(view.update(cx, |view, _| view.should_close_dashboard(window)));
        window.remove_window();
    })
    .unwrap();
    available.store(2, Ordering::Release);
    settle(&mut cx);
    let restored = cx.read_entity(&view, |view, _| view.main_window.unwrap());
    capture(&mut cx, restored, &evidence.join("04-tray-unavailable.png"));
    // The existing visible Quit control still shuts down the backend.
    click(&mut cx, restored, "quit");
    assert!(backend.handle.cancel.is_cancelled());
    drop(actions);
    drop(view);
    drop(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "status": "passed", "real_gpu": true, "scoped_socket_removed": true,
        "screenshots": ["01-before-close.png", "02-permission-while-hidden.png", "03-restored.png", "04-tray-unavailable.png"],
        "proof_boundary": "Production close-handler boundary and actual GPUI permission/Quit input; retained dashboard state, tray-action restoration and host-loss recovery. Tray callbacks are synthetic; native compositor and D-Bus protocol require separate proof."
    })).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"), serde_json::to_vec_pretty(&[
        "Select Nodes through Ctrl+4, apply production close boundary, require destroyed dashboard and running backend",
        "Deliver permission while dashboard is absent; Deny once through actual button",
        "Reopen twice through bounded tray channel, require retained Nodes screen and one window",
        "Lose tray host while hidden, require automatic restoration; Quit through actual button"
    ]).unwrap()).unwrap();
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
            keyboard_button(&mut cx, popup, "deny-once");
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
fn headless_permission_icons() {
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
                name: "icon-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        client
    });
    let data = evidence.join("desktop-data");
    assert_eq!(
        PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap()),
        data
    );
    let applications = data.join("applications");
    std::fs::create_dir_all(&applications).unwrap();
    let png_app = evidence.join("PNG sample app");
    let svg_app = evidence.join("SVG sample app");
    for path in [&png_app, &svg_app] {
        std::fs::write(path, "fixture: never executed").unwrap();
    }
    let png = evidence.join("fixture.png");
    image::RgbaImage::from_fn(64, 64, |x, _| {
        image::Rgba(if x < 32 {
            [250, 20, 20, 255]
        } else {
            [20, 230, 240, 255]
        })
    })
    .save(&png)
    .unwrap();
    let svg = evidence.join("fixture.svg");
    std::fs::write(&svg, r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><path fill="#19e632" d="M0 0h32v64H0z"/><path fill="#e6ba19" d="M32 0h32v64H32z"/></svg>"##).unwrap();
    for (name, app, icon) in [("png", &png_app, &png), ("svg", &svg_app, &svg)] {
        std::fs::write(
            applications.join(format!("{name}.desktop")),
            format!(
                "[Desktop Entry]\nType=Application\nName=Fixture\nExec=\"{}\" %U\nIcon={}\n",
                app.display(),
                icon.display()
            ),
        )
        .unwrap();
    }
    let mut cx = context();
    let mut window = None;
    let screenshots = [
        "01-desktop-png.png",
        "02-busy-fallback.png",
        "03-desktop-svg.png",
        "04-missing-icon.png",
    ];
    let png_colors = [[250u8, 20, 20], [20, 230, 240]];
    let svg_colors = [[25u8, 230, 50], [230, 186, 25]];
    for (case, screenshot) in screenshots.iter().enumerate() {
        let held = (case == 1).then(super::app_icon::hold_workers);
        let app = match case {
            0 => png_app.clone(),
            1 | 2 => svg_app.clone(),
            _ => evidence.join("unregistered-app"),
        };
        let mut asking = daemon.clone();
        let reply = runtime.spawn(async move {
            asking
                .ask_rule(protocol::Connection {
                    process_path: app.to_string_lossy().into_owned(),
                    dst_ip: "192.0.2.42".into(),
                    dst_port: 443,
                    protocol: "tcp".into(),
                    ..Default::default()
                })
                .await
                .unwrap()
                .into_inner()
        });
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
                super::prompt_window::open(backend.handle.clone(), 1., false, cx).unwrap()
            })
        });
        settle(&mut cx);
        // A fresh full frame avoids reusing composited layers from an earlier
        // offscreen readback when sampling several times without user input.
        let refresh = |cx: &mut HeadlessAppContext| {
            cx.update_window(prompt, |_, window, _| window.refresh())
                .unwrap();
            cx.advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
        };
        let frame = |cx: &mut HeadlessAppContext| {
            refresh(cx);
            cx.capture_screenshot(prompt).unwrap()
        };
        let has_colors = |image: &image::RgbaImage, colors: [[u8; 3]; 2]| {
            colors.iter().all(|color| {
                image
                    .pixels()
                    .filter(|p| (0..3).all(|c| p[c].abs_diff(color[c]) <= 4))
                    .count()
                    > 64
            })
        };
        let mut image = frame(&mut cx);
        if case == 0 || case == 2 {
            let colors = if case == 0 { png_colors } else { svg_colors };
            for _ in 0..40 {
                if has_colors(&image, colors) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
                cx.advance_clock(Duration::from_millis(100));
                cx.run_until_parked();
                image = frame(&mut cx);
            }
            assert!(
                has_colors(&image, colors),
                "actual icon pixels must be painted for case {case}"
            );
        } else {
            assert!(
                !has_colors(&image, png_colors),
                "previous PNG cannot leak into a new request"
            );
            assert!(
                !has_colors(&image, svg_colors),
                "busy/missing lookup uses fallback"
            );
        }
        image.save(evidence.join(screenshot)).unwrap();
        let started = Instant::now();
        click(&mut cx, prompt, "allow-forever");
        let rule = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), reply)
                .await
                .unwrap()
                .unwrap()
        });
        assert_eq!(
            (rule.action.as_str(), rule.duration.as_str()),
            ("allow", "always")
        );
        if case == 1 {
            assert!(
                started.elapsed() < Duration::from_millis(500),
                "icon worker saturation must not delay a permission reply"
            );
        }
        drop(held);
        settle(&mut cx);
    }
    runtime.block_on(backend.handle.history.flush()).unwrap();
    cx.update_window(window.unwrap(), |_, window, _| window.remove_window())
        .unwrap();
    drop(cx);
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"), serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","screenshots":screenshots,"real_gpu":true,"scoped_socket_removed":true,"proof_boundary":"Production desktop-entry lookup, real PNG/SVG pixels through Kit Avatar, request reset and independent actual gRPC replies while both icon workers are held. Synthetic daemon only."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"), serde_json::to_vec_pretty(&["Show an icon resolved from a quoted desktop Exec and PNG Icon", "Hold both lookup workers: show initials instead of the previous icon and answer immediately", "Release workers and show actual SVG icon pixels on a fresh request", "An unregistered app uses the fallback, not the previous SVG; every request returns allow/always"]).unwrap()).unwrap();
}

#[test]
fn headless_permission_actions() {
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
    let mut cx = context();
    let mut window = None;
    assert_eq!(backend.handle.preferences.snapshot().target, "process.path");
    let screenshots = [
        "01-app-forever.png",
        "02-app-12-hours.png",
        "03-exact-hostname.png",
        "04-ip-keyboard.png",
        "05-saved-custom-rule.png",
        "06-unknown-app.png",
    ];
    for (case, screenshot) in screenshots.iter().enumerate() {
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
                super::prompt_window::open(backend.handle.clone(), 1., false, cx).unwrap()
            })
        });
        settle(&mut cx);
        if case == 1 || case == 2 {
            cx.update_window(prompt, |_, window, cx| {
                window
                    .within("prompt-scope")
                    .click(if case == 1 { 0 } else { 1 }, cx);
            })
            .unwrap();
            settle(&mut cx);
        } else if case == 3 {
            // Both scope choices are actual Kit keyboard controls.
            cx.update_window(prompt, |_, window, cx| window.press("tab", cx))
                .unwrap();
            settle(&mut cx);
            cx.update_window(prompt, |_, window, cx| window.press("tab", cx))
                .unwrap();
            settle(&mut cx);
            cx.update_window(prompt, |_, window, cx| {
                window.press("space", cx);
            })
            .unwrap();
            settle(&mut cx);
        } else if case == 5 {
            cx.update_window(prompt, |_, window, cx| {
                window.within("prompt-scope").click(0, cx);
            })
            .unwrap();
        }
        capture(&mut cx, prompt, &evidence.join(screenshot));
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
        settle(&mut cx);
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
    drop(cx);
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
        "status": "passed", "screenshots": screenshots, "real_gpu": true, "scoped_socket_removed": true,
        "proof_boundary": "Production Kit scope radio pointer/keyboard input, actual isolated gRPC replies and persisted decisions. No system daemon changes."
    })).unwrap()).unwrap();
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
    click(&mut cx, window, "allow-selected");
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
    keyboard_button(&mut cx, window, "deny-once");
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
    // Ubuntu release runners use DejaVu; this catches first-click layout shifts.
    cx.update(|cx| Theme::global_mut(cx).font_family = "DejaVu Sans".into());
    let prompt =
        cx.update(|cx| super::prompt_window::open(backend.handle.clone(), 1., false, cx).unwrap());
    settle(&mut cx);
    capture(&mut cx, prompt, &evidence.join("01-checksum-warning.png"));
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, prompt, "prompt-checksums");
    settle(&mut cx);
    capture(
        &mut cx,
        prompt,
        &evidence.join("checksum-after-first-click.png"),
    );
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
    click(&mut cx, prompt, "allow-forever");
    let decision = runtime.block_on(reply).unwrap();
    assert_eq!(
        (decision.action.as_str(), decision.duration.as_str()),
        ("allow", "always")
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
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Open the reported-hash warning, inspect MD5/SHA1 and next page, cancel then confirm selected update", "Hold daemon ACK; require unchanged cache and immediate independent Allow forever; release ACK, stop all after one command, and reject the next update with uncertainty feedback", "Reconnect a fresh daemon inventory, reopen the review, update all remaining 33 rules across pages, require preserved metadata/conditions and cleared warning, then Deny once"]).unwrap()).unwrap();
}

#[test]
fn headless_history_tools() {
    fn search(cx: &mut HeadlessAppContext, w: AnyWindowHandle, value: &str) {
        cx.update_window(w, |_, w, cx| w.press("ctrl-f", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(w, |_, w, cx| {
            w.press("ctrl-a", cx);
            w.input(value, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
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
    // Persist actual user choices, including the Kit resize gesture.
    pick(&mut cx, tools, "history-filter-field", "UID");
    pick(&mut cx, tools, "history-filter-comparison", "Equals");
    fill(&mut cx, tools, "history-filter-value", "1001");
    click(&mut cx, tools, "history-filter-add");
    click(&mut cx, tools, "history-filter-apply");
    click(&mut cx, window.into(), "reject");
    search(&mut cx, window.into(), "history-app");
    settle(&mut cx);
    let before_width = cx
        .update_window(window.into(), |_, w, _| {
            w.within("table")
                .find(("col-header", 0usize))
                .bounds()
                .size
                .width
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        let header = w.within("table").find(("col-header", 0usize)).bounds();
        let from = gpui::point(header.right() - px(2.), header.center().y);
        w.drag(from, gpui::point(from.x + px(80.), from.y), cx);
    })
    .unwrap();
    settle(&mut cx);
    let after_width = cx
        .update_window(window.into(), |_, w, _| {
            w.within("table")
                .find(("col-header", 0usize))
                .bounds()
                .size
                .width
        })
        .unwrap();
    assert!(
        after_width > before_width + px(30.),
        "Actual Kit resize must change the visible header width"
    );
    for _ in 0..20 {
        settle(&mut cx);
        if cx.read_entity(&view, |v, _| v.history.memory.settled()) {
            break;
        }
    }
    let saved = crate::history::settings::Store::open(&config.database)
        .unwrap()
        .snapshot();
    assert_eq!(saved.presentation.search, "history-app");
    assert_eq!(saved.presentation.action, "reject");
    assert_eq!(saved.presentation.filters.len(), 1);
    assert_eq!(saved.presentation.sort, Some((Field::Time, true)));
    assert!(
        saved
            .presentation
            .widths
            .iter()
            .any(|(f, w)| *f == Field::Time && *w > 270.)
    );
    assert_eq!(cx.read_entity(&view, |v, _| v.history.page.rows.len()), 72);
    capture(
        &mut cx,
        window.into(),
        &evidence.join("11-history-remembered-controls.png"),
    );
    click(&mut cx, tools, "history-tools-close");
    // Quit must flush the final edit even before the ordinary debounce expires.
    search(&mut cx, window.into(), "history-app-");
    capture(
        &mut cx,
        window.into(),
        &evidence.join("12-history-before-quit.png"),
    );
    click(&mut cx, window.into(), "quit");
    drop(view);
    drop(cx);
    backend.join();
    let settings_path = config.database.with_extension("sqlite3.history.json");
    let durable = std::fs::read(&settings_path).unwrap();
    let saved = crate::history::settings::Store::open(&config.database)
        .unwrap()
        .snapshot();
    assert_eq!(saved.presentation.search, "history-app-");
    std::fs::write(evidence.join("saved-history-view.json"), &durable).unwrap();
    let backend = Backend::start(config.clone()).unwrap();
    let mut cx = context();
    let mut view = None;
    let window = cx
        .open_window(size(px(1100.), px(780.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let view = view.unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.search.clone()),
        saved.presentation.search
    );
    assert_eq!(
        cx.read_entity(&view, |v, cx| v.history.input.read(cx).value().to_string()),
        saved.presentation.search
    );
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.action.clone()),
        "reject"
    );
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.filters.clone()),
        saved.presentation.filters
    );
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.sort),
        Some((Field::Time, true))
    );
    assert!(
        cx.read_entity(&view, |v, _| v.history.query.before.is_none()
            && v.history.query.cursor.is_none()
            && v.history.page.rows.len() == 72)
    );
    let restored_width = cx
        .update_window(window.into(), |_, w, _| {
            w.within("table")
                .find(("col-header", 0usize))
                .bounds()
                .size
                .width
        })
        .unwrap();
    assert!((f32::from(restored_width) - f32::from(after_width)).abs() < 2.);
    capture(
        &mut cx,
        window.into(),
        &evidence.join("13-history-cold-restored.png"),
    );
    // An external edit stops automatic writes, without blocking actual RPC prompts.
    let external = b"{invalid external edit preserved}";
    std::fs::write(&settings_path, external).unwrap();
    search(&mut cx, window.into(), "no-such-history-app");
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.history.memory.error().is_some()));
    assert_eq!(std::fs::read(&settings_path).unwrap(), external);
    capture(
        &mut cx,
        window.into(),
        &evidence.join("14-history-session-only.png"),
    );
    let mut daemon = runtime.block_on(async {
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
                name: "history-view-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        client
    });
    let reply = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/history-view-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let popup = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("15-history-save-error-permission.png"),
    );
    click(&mut cx, popup, "deny-once");
    let decision = runtime.block_on(reply).unwrap();
    assert_eq!(decision.action, "deny");
    assert_eq!(decision.duration, "once");
    click(&mut cx, window.into(), "allow");
    settle(&mut cx);
    assert_eq!(std::fs::read(&settings_path).unwrap(), external);
    click(&mut cx, window.into(), "quit");
    drop(view);
    drop(cx);
    backend.join();
    // Repair only this fixture, then prove a fresh process can restore again.
    std::fs::write(&settings_path, &durable).unwrap();
    let backend = Backend::start(config.clone()).unwrap();
    let mut cx = context();
    let mut view = None;
    let window = cx
        .open_window(size(px(1100.), px(780.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let view = view.unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.history.query.search.clone()),
        saved.presentation.search
    );
    assert!(cx.read_entity(&view, |v, _| v.history.memory.error().is_none()));
    capture(
        &mut cx,
        window.into(),
        &evidence.join("16-history-repaired-restart.png"),
    );
    click(&mut cx, window.into(), "quit");
    drop(view);
    drop(cx);
    backend.join();
    drop(runtime);
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","screenshots":["01-rejected-history.png","02-history-conditions.png","03-invalid-history-port.png","04-saved-history-columns.png","05-sorted-history-table.png","06-history-csv-export.png","07-history-existing-file.png","08-history-json-export.png","09-history-clipboard.png","10-history-sorted-next-page.png","11-history-remembered-controls.png","12-history-before-quit.png","13-history-cold-restored.png","14-history-session-only.png","15-history-save-error-permission.png","16-history-repaired-restart.png"],"real_gpu":true,"scoped_socket_removed":true,"proof_boundary":"Real GPUI history conditions/column/header sort/resize/export input; actual cold backend and GUI restore of search/action/conditions/sort/widths, fresh pagination, immediate Quit flush, external-edit preservation and independent AskRule under save error, explicit fixture repair/restart; native compositor tools/pickers/width dragging and real daemon enforcement remain separate"})).unwrap()).unwrap();
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
    cx.update(|cx| super::appearance::init(&config.database, cx));
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
    click(&mut cx, prompt, "allow-forever");
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
    click(&mut cx, window.into(), "allow-forever");
    let allowed = allow.try_recv().unwrap();
    assert_eq!(allowed.decision, Decision::Allow);
    assert_eq!(allowed.rule.unwrap().duration, "always");
    // Traverse the actual scope controls and activate Block with Tab/Space.
    keyboard_button(&mut cx, window.into(), "deny-once");
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
    click(&mut cx, window.into(), "allow-selected");
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
    click(&mut cx, window.into(), "prompt-more");
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
    let(mut alpha,beta,alpha_tx,mut alpha_commands,beta_tx,mut beta_commands)=runtime.block_on(async{
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
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        None
    );
    click(&mut cx, window, "nodes-remember-pause");
    settle(&mut cx);
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        Some(true)
    );
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
    click(&mut cx, prompt, "allow-forever");
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
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        Some(false)
    );
    capture(&mut cx, window, &evidence.join("12-remembered-pause.png"));
    drop(alpha_commands);
    drop(alpha_tx);
    let (alpha_tx, mut alpha_commands) = runtime.block_on(async {
        let saved = nodes::detail(&config.database, "alpha-node").unwrap();
        alpha
            .subscribe(protocol::ClientConfig {
                name: "alpha-node".into(),
                version: "reconnected-fixture-kernel".into(),
                is_firewall_running: true,
                config: saved.config,
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let stream = alpha
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        backend.handle.history.flush().await.unwrap();
        (tx, stream)
    });
    let command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), alpha_commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(command.r#type, protocol::Action::DisableInterception as i32);
    assert_eq!(command.client_name, "alpha-node");
    settle(&mut cx);
    assert!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    capture(
        &mut cx,
        window,
        &evidence.join("13-restoration-awaits-ack.png"),
    );
    let mut asking = beta.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/pause-restore-independent".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let prompt = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        prompt,
        &evidence.join("14-permission-during-restoration.png"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    runtime
        .block_on(alpha_tx.send(protocol::NotificationReply {
            id: command.id,
            code: protocol::NotificationReplyCode::Ok as i32,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
    settle(&mut cx);
    assert!(
        !nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        Some(false)
    );
    capture(&mut cx, window, &evidence.join("15-restored-pause.png"));
    settle(&mut cx);
    assert!(
        !backend
            .handle
            .control
            .node_busy(backend.handle.node("alpha-node").unwrap().id)
    );
    click(&mut cx, window, "nodes-remember-pause");
    settle(&mut cx);
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        None
    );
    assert!(
        !nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(50), alpha_commands.message()).await
            })
            .is_err()
    );
    capture(
        &mut cx,
        window,
        &evidence.join("16-opt-out-keeps-current-pause.png"),
    );
    click(&mut cx, window, "nodes-interception");
    click(&mut cx, window, "nodes-cancel");
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(50), alpha_commands.message()).await
            })
            .is_err()
    );
    click(&mut cx, window, "nodes-interception");
    click(&mut cx, window, "nodes-confirm");
    settle(&mut cx);
    let command = runtime.block_on(alpha_commands.message()).unwrap().unwrap();
    assert_eq!(command.r#type, protocol::Action::EnableInterception as i32);
    runtime
        .block_on(alpha_tx.send(protocol::NotificationReply {
            id: command.id,
            code: protocol::NotificationReplyCode::Ok as i32,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
    assert!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        None
    );
    capture(
        &mut cx,
        window,
        &evidence.join("17-manual-resume-after-opt-out.png"),
    );
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
    let screenshots = (1..=17)
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
            "remembered-pause",
            "restoration-awaits-ack",
            "permission-during-restoration",
            "restored-pause",
            "opt-out-keeps-current-pause",
            "manual-resume-after-opt-out",
        ])
        .map(|(index, name)| format!("{index:02}-{name}.png"))
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":screenshots,"boundary":"Actual GPUI input, two isolated gRPC daemon fixtures, acknowledged node controls and SQLite; opt-in per-node pause, current-generation reconnect restoration under held ACK, independent permission, opt-out without auto-resume and manual resume; system daemon/firewall unchanged"})).unwrap()).unwrap();
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
    click(&mut cx, prompt, "allow-forever");
    settle(&mut cx);
    let reply = runtime.block_on(ask).unwrap();
    assert_eq!(reply.action, "allow");
    assert_eq!(reply.duration, "always");
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
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Enter selects first node; open native daemon forms", "Edit policy/rules/logging/statistics/firewall/TLS fields, reject negative limit before sending", "Review/back/JSON/settings preserves only changed fields and exact unknown numbers", "Hold ACK without cache update, answer Allow forever despite daemon duration change, confirm save", "Second configuration ERROR preserves confirmed data and blocks uncertain node changes"]).unwrap()).unwrap();
}

#[test]
fn headless_alerts() {
    use crate::alerts;
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mut daemon = runtime.block_on(async {
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
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "alerts-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        for id in 0..205 {
            daemon
                .post_alert(protocol::Alert {
                    id,
                    action: 1,
                    r#type: (id % 3) as i32,
                    priority: 2,
                    data: Some(protocol::alert::Data::Text(format!(
                        "Recorded alert {id:03}"
                    ))),
                    ..Default::default()
                })
                .await
                .unwrap();
        }
        daemon
            .post_alert(protocol::Alert {
                id: 205,
                action: 2,
                r#type: 1,
                data: Some(protocol::alert::Data::Proc(protocol::Process {
                    pid: 987,
                    path: "/usr/bin/alert-fixture".into(),
                    args: vec!["two words".into()],
                    ..Default::default()
                })),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
        daemon
    });
    assert_eq!(
        backend
            .handle
            .history
            .health
            .alerts_skipped
            .load(Ordering::Relaxed),
        0
    );
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
    cx.update_window(window, |_, window, cx| window.press("ctrl-5", cx))
        .unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        200
    );
    capture(&mut cx, window, &evidence.join("01-alerts-latest.png"));
    click(&mut cx, window, "alerts-older");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        6
    );
    capture(&mut cx, window, &evidence.join("02-alerts-older.png"));
    runtime.block_on(async {
        daemon
            .post_alert(protocol::Alert {
                id: 999,
                action: 1,
                r#type: 0,
                data: Some(protocol::alert::Data::Text("Later arrival".into())),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        6
    );
    click(&mut cx, window, "alerts-previous");
    settle(&mut cx);
    click(&mut cx, window, "alerts-latest");
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("alerts-severity", 1usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        70
    );
    capture(&mut cx, window, &evidence.join("03-alerts-errors.png"));
    cx.update_window(window, |_, window, cx| {
        window.click(("alerts-severity", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    fill(&mut cx, window, "alerts-search", "/usr/bin/alert-fixture");
    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        1
    );
    let typed = alerts::page(
        &config.database,
        &alerts::Query {
            search: "/usr/bin/alert-fixture".into(),
            ..Default::default()
        },
    )
    .unwrap()
    .rows[0]
        .id;
    cx.update_window(window, |_, window, cx| {
        window.click(("alert-row", typed as u64), cx)
    })
    .unwrap();
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("04-alerts-typed-details.png"),
    );
    click(&mut cx, window, "alerts-copy-detail");
    let copied = cx
        .update(|cx| cx.read_from_clipboard())
        .unwrap()
        .text()
        .unwrap();
    let copied: serde_json::Value = serde_json::from_str(&copied).unwrap();
    assert_eq!(copied["payload"]["path"], "/usr/bin/alert-fixture");
    assert_eq!(copied["payload"]["args"][0], "two words");
    click(&mut cx, window, "alerts-read");
    settle(&mut cx);
    click(&mut cx, window, "alerts-unread");
    settle(&mut cx);
    assert!(cx.read_entity(&view, |view, _| view.alerts.page.rows.is_empty()));
    capture(&mut cx, window, &evidence.join("05-alerts-read-filter.png"));
    click(&mut cx, window, "alerts-unread");
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("alert-row", typed as u64), cx)
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, window, "alerts-delete");
    click(&mut cx, window, "alerts-delete-cancel");
    assert!(alerts::detail(&config.database, typed).unwrap().is_some());
    click(&mut cx, window, "alerts-delete");
    capture(
        &mut cx,
        window,
        &evidence.join("06-alerts-delete-confirm.png"),
    );
    click(&mut cx, window, "alerts-delete-confirm");
    settle(&mut cx);
    assert!(alerts::detail(&config.database, typed).unwrap().is_none());
    fill(&mut cx, window, "alerts-search", "");
    let export = evidence.join("alerts.jsonl");
    fill(
        &mut cx,
        window,
        "alerts-export-path",
        export.to_str().unwrap(),
    );
    click(&mut cx, window, "alerts-export-json");
    settle(&mut cx);
    let original = std::fs::read_to_string(&export).unwrap();
    assert_eq!(original.lines().count(), 206);
    click(&mut cx, window, "alerts-export-csv");
    settle(&mut cx);
    assert_eq!(std::fs::read_to_string(&export).unwrap(), original);
    capture(
        &mut cx,
        window,
        &evidence.join("07-alerts-export-existing.png"),
    );
    click(&mut cx, window, "alerts-export-replace");
    click(&mut cx, window, "alerts-export-csv");
    settle(&mut cx);
    let csv = std::fs::read_to_string(&export).unwrap();
    assert!(csv.starts_with("\"Received UTC\""));
    assert!(csv.contains("Later arrival"));
    click(&mut cx, window, "alerts-copy-page");
    assert!(
        cx.update(|cx| cx.read_from_clipboard())
            .unwrap()
            .text()
            .unwrap()
            .contains("Later arrival")
    );
    let mut asking = daemon.clone();
    let request = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/bin/alert-permission".into(),
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
        &evidence.join("08-permission-from-alerts.png"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(request).unwrap().action, "deny");
    settle(&mut cx);
    fill(&mut cx, window, "alerts-search", "Recorded alert 00");
    assert_eq!(
        cx.read_entity(&view, |view, _| view.alerts.page.rows.len()),
        10
    );
    click(&mut cx, window, "alerts-clear");
    click(&mut cx, window, "alerts-delete-cancel");
    assert_eq!(
        alerts::page(
            &config.database,
            &alerts::Query {
                search: "Recorded alert 00".into(),
                ..Default::default()
            }
        )
        .unwrap()
        .rows
        .len(),
        10
    );
    click(&mut cx, window, "alerts-clear");
    capture(
        &mut cx,
        window,
        &evidence.join("09-alerts-clear-review.png"),
    );
    runtime.block_on(async {
        daemon
            .post_alert(protocol::Alert {
                id: 1001,
                action: 2,
                data: Some(protocol::alert::Data::Text(
                    "Recorded alert 005 after review".into(),
                )),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    click(&mut cx, window, "alerts-delete-confirm");
    settle(&mut cx);
    let remaining = alerts::page(
        &config.database,
        &alerts::Query {
            search: "Recorded alert 00".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(remaining.rows.len(), 1);
    assert_eq!(remaining.rows[0].summary, "Recorded alert 005 after review");
    fill(&mut cx, window, "alerts-search", "absent alert text");
    assert!(cx.read_entity(&view, |view, _| view.alerts.page.rows.is_empty()));
    capture(
        &mut cx,
        window,
        &evidence.join("10-alerts-empty-search.png"),
    );
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    drop(cx);
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let screenshots = [
        "01-alerts-latest.png",
        "02-alerts-older.png",
        "03-alerts-errors.png",
        "04-alerts-typed-details.png",
        "05-alerts-read-filter.png",
        "06-alerts-delete-confirm.png",
        "07-alerts-export-existing.png",
        "08-permission-from-alerts.png",
        "09-alerts-clear-review.png",
        "10-alerts-empty-search.png",
    ];
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":screenshots,"boundary":"Actual Kit alert browse/filter/page/detail/copy/read/delete/export controls, independent gRPC permission and SQLite assertions; real daemon untouched"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Open Alerts with Ctrl+5; browse older frozen page while a newer alert arrives; Previous/Latest", "Filter errors and typed payload, inspect/copy complete JSON; mark read and unread-only empty state", "Cancel then confirm local deletion; export all retained matches as JSON, preserve existing file, explicitly replace as CSV and copy page", "Answer a real permission from the Alerts tab; search absent text to empty state"]).unwrap()).unwrap();
}

#[test]
fn headless_desktop_notifications() {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mut daemon = runtime.block_on(async {
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
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "notice-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    let mut cx = context();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(super::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    let (tray, _tray_actions, _availability) = super::tray::Client::fixture();
    let (notices, actions) = super::desktop_notifications::Client::fixture();
    cx.update_entity(&view, |view, _| {
        view.tray = Some(tray);
        view.desktop_notifications = Some(notices);
    });
    let preferences = cx
        .update(|cx| super::preferences_view::open(backend.handle.clone(), cx))
        .unwrap();
    settle(&mut cx);
    click(&mut cx, preferences, "preferences-notifications");
    click(&mut cx, preferences, "preferences-save");
    settle(&mut cx);
    assert!(
        !crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .desktop_notifications
    );
    assert!(backend.handle.preferences.snapshot().popups);
    capture(
        &mut cx,
        preferences,
        &evidence.join("01-notifications-muted.png"),
    );
    runtime.block_on(async {
        daemon
            .post_alert(protocol::Alert {
                id: 1,
                action: 1,
                data: Some(protocol::alert::Data::Text(
                    "Archived while desktop muted".into(),
                )),
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    assert_eq!(
        crate::alerts::page(&config.database, &Default::default())
            .unwrap()
            .rows
            .len(),
        1
    );
    click(&mut cx, preferences, "preferences-notifications");
    click(&mut cx, preferences, "preferences-save");
    settle(&mut cx);
    assert!(
        crate::preferences::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .desktop_notifications
    );
    capture(
        &mut cx,
        preferences,
        &evidence.join("02-notifications-enabled.png"),
    );
    click(&mut cx, preferences, "preferences-close");
    cx.update_window(window, |_, window, cx| window.press("ctrl-2", cx))
        .unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        assert!(view.update(cx, |view, _| view.should_close_dashboard(window)));
        window.remove_window();
    })
    .unwrap();
    settle(&mut cx);
    let mut asking = daemon.clone();
    let request = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/notice-permission".into(),
                dst_host: "notice.example".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let popup = cx.read_entity(&view, |view, _| {
        assert!(view.main_window.is_none());
        view.prompt_window.unwrap()
    });
    capture(
        &mut cx,
        popup,
        &evidence.join("03-notice-permission-hidden.png"),
    );
    actions.try_send(()).unwrap();
    settle(&mut cx);
    let restored = cx.read_entity(&view, |view, _| {
        assert!(view.screen == super::Screen::Alerts);
        view.main_window.unwrap()
    });
    assert_eq!(
        backend.handle.prompts.snapshot().len(),
        1,
        "Opening an alert never answers a permission request"
    );
    capture(
        &mut cx,
        restored,
        &evidence.join("04-alert-action-restored.png"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(request).unwrap().action, "deny");
    drop(daemon);
    drop(actions);
    drop(view);
    drop(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":["01-notifications-muted.png","02-notifications-enabled.png","03-notice-permission-hidden.png","04-alert-action-restored.png"],"proof_boundary":"Actual Kit toggle/save, independent file reopen, archive while muted, production close boundary and notification callback restoration, real gRPC permission answered by GPUI input. Notification callback is synthetic; private-bus worker tests prove transport separately. No host desktop banner proof."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Disable desktop notifications through actual checkbox and Save; independently reopen preferences and retain SHOW_ALERT","Enable and save; close preferences and retain dashboard through production close boundary","Deliver real AskRule while dashboard absent; notification callback restores Alerts without answering","Deny once through actual GPUI input; require real reply and owned cleanup"]).unwrap()).unwrap();
}

#[test]
fn headless_live_tasks() {
    use super::live_view::View;
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
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "live-fixture".into(),
                version: "test-kernel".into(),
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
    let recorded = Arc::new(Mutex::new(Vec::<protocol::Notification>::new()));
    let log = recorded.clone();
    let reply = replies.clone();
    let task=runtime.spawn(async move{
 let mut periodic=tokio::task::JoinSet::new();
 let mut running=std::collections::BTreeMap::<String,tokio::task::AbortHandle>::new();
 while let Ok(Some(command))=commands.message().await{
  eprintln!("Live fixture command {}: {}",command.r#type,command.data);log.lock().unwrap().push(command.clone());
 if command.r#type==14 {if let Some(task)=running.remove(&command.data){task.abort();}continue;}
 if command.r#type!=13{continue;}
  let doc:serde_json::Value=serde_json::from_str(&command.data).unwrap();let name=doc["name"].as_str().unwrap();if name=="node-monitor"{continue;}
  let data=if name=="sockets-monitor"{
   let table=(0..225).filter(|i|doc["data"]["proto"]==0||doc["data"]["proto"]==if i%2==0{6}else{17}).filter(|i|doc["data"]["family"]==0||doc["data"]["family"]==if i%2==0{10}else{2}).map(|i|serde_json::json!({"PID":42,"Proto":if i%2==0{6}else{17},"Iface":"eth0","Socket":{"ID":{"Source":if i%2==0{"2001:db8::1"}else{"192.0.2.1"},"Destination":if i%2==0{"2001:db8::2"}else{"198.51.100.2"},"SourcePort":30000+i,"DestinationPort":443,"Cookie":[1,2]},"Family":if i%2==0{10}else{2},"State":1,"UID":1000,"INode":5000+i,"RQueue":7,"WQueue":9}})).collect::<Vec<_>>();
   serde_json::json!({"Table":table,"Processes":{"42":{"Path":"/usr/bin/live-fixture","Comm":"live-fixture"}}})
  }else{
   assert_eq!(doc["data"]["pid"],"42");serde_json::json!({"PID":42,"Path":"/usr/bin/live-fixture","Comm":"live-fixture","CWD":"/tmp/owned","Args":["live-fixture","two words"],"Status":"Name:\tlive-fixture\nPid:\t42\nVmRSS:\t4096 kB", "Descriptors":[{"Name":"3","SymLink":"socket:[5000]","Size":0},{"Name":"4","SymLink":"/tmp/owned-file","Size":10}],"IOStats":{"RChar":1234,"WChar":5678,"ReadBytes":8192},"Maps":"1000-2000 r-xp /usr/bin/live-fixture\n2000-3000 rw-p [heap]","Stack":"fixture_frame\nnext_frame","Env":{"SYNTHETIC_LIVE_ENV":"requested explicitly","UNICODE":"Ω"},"Checksums":{"md5":"fixture-md5"},"Tree":[{"key":"/usr/bin/parent","value":1}],"Statm":{"Size":1024,"Resident":128},"Future":18446744073709551615_u64})
  }.to_string();
  for _ in 0..2{if reply.send(protocol::NotificationReply{id:command.id,data:data.clone(),..Default::default()}).await.is_err(){return;}}
  let seconds=doc["data"]["interval"].as_str().unwrap().trim_end_matches('s').parse::<u64>().unwrap();
  let sender=reply.clone();let id=command.id;let key=command.data;
  assert!(!running.contains_key(&key),"fixture task identity must be stopped before restart");
  let abort=periodic.spawn(async move {loop {
   tokio::time::sleep(Duration::from_secs(seconds)).await;
   if sender.send(protocol::NotificationReply{id,data:data.clone(),..Default::default()}).await.is_err(){break;}
  }});
  running.insert(key,abort);
  while periodic.try_join_next().is_some(){}
 }});
    let mut cx = context();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(super::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-6", cx))
        .unwrap();
    settle(&mut cx);
    click(&mut cx, window, "live-start");
    settle(&mut cx);
    cx.read_entity(&view, |view, _| {
        assert!(view.live.snapshot.revision >= 2);
        let Some(View::Sockets { data, matches }) = &view.live.parsed else {
            panic!("live sockets must parse")
        };
        assert_eq!(data.rows.len(), 225);
        assert_eq!(matches.len(), 225);
    });
    capture(&mut cx, window, &evidence.join("01-live-sockets.png"));
    click(&mut cx, window, "live-next");
    capture(
        &mut cx,
        window,
        &evidence.join("02-live-sockets-page-two.png"),
    );
    click(&mut cx, window, "live-pause");
    settle(&mut cx);
    pick(&mut cx, window, "live-protocol", "TCP");
    pick(&mut cx, window, "live-family", "IPv6");
    pick(&mut cx, window, "live-state", "Established");
    pick(&mut cx, window, "live-interval", "2 seconds");
    click(&mut cx, window, "live-start");
    settle(&mut cx);
    cx.read_entity(&view, |view, _| {
        let Some(View::Sockets { data, .. }) = &view.live.parsed else {
            panic!()
        };
        assert_eq!(data.rows.len(), 113);
        assert!(
            data.rows
                .iter()
                .all(|r| r.title.contains("TCP") && r.title.contains("IPv6"))
        );
    });
    capture(
        &mut cx,
        window,
        &evidence.join("03-live-daemon-filters.png"),
    );
    click(&mut cx, window, "live-pause");
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("live-socket", 0u64), cx)
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, window, "live-copy-row");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("RQueue")
    );
    click(&mut cx, window, "live-process");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("04-live-process-status.png"),
    );
    pick(&mut cx, window, "live-field", "Env");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("05-live-process-environment.png"),
    );
    click(&mut cx, window, "live-copy-field");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("SYNTHETIC_LIVE_ENV")
    );
    pick(&mut cx, window, "live-field", "Descriptors");
    settle(&mut cx);
    fill(&mut cx, window, "live-search", "socket:[");
    settle(&mut cx);
    cx.read_entity(&view, |view, _| {
        let Some(View::Fields { lines, .. }) = &view.live.parsed else {
            panic!()
        };
        assert_eq!(lines.len(), 1);
    });
    capture(
        &mut cx,
        window,
        &evidence.join("06-live-process-sockets.png"),
    );
    fill(&mut cx, window, "live-search", "");
    pick(&mut cx, window, "live-field", "Maps");
    settle(&mut cx);
    capture(&mut cx, window, &evidence.join("07-live-process-maps.png"));
    click(&mut cx, window, "live-copy-json");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("18446744073709551615")
    );
    let pid_command = recorded
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|n| n.r#type == 13)
        .unwrap()
        .id;
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: pid_command,
            code: 1,
            data: "Synthetic process exited".into(),
        }))
        .unwrap();
    settle(&mut cx);
    cx.read_entity(&view, |view, _| {
        assert!(!view.live.snapshot.running);
        assert!(
            view.live
                .snapshot
                .error
                .as_ref()
                .unwrap()
                .contains("exited")
        );
        assert!(view.live.parsed.is_some());
    });
    capture(
        &mut cx,
        window,
        &evidence.join("08-live-process-error-retained.png"),
    );
    cx.update_window(window, |_, window, cx| {
        window.within("live-tabs").click(2usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, window, "live-start");
    settle(&mut cx);
    let mut asking = daemon.clone();
    let request = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/live-permission".into(),
                dst_host: "live.example".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let popup = cx.read_entity(&view, |view, _| view.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("09-live-permission-while-awaiting.png"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(request).unwrap().action, "deny");
    let resource = recorded
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|n| n.r#type == 13)
        .unwrap()
        .clone();
    let resource_config: serde_json::Value = serde_json::from_str(&resource.data).unwrap();
    assert_eq!(resource_config["data"]["node"], "live-fixture");
    let sysinfo=serde_json::json!({"Uptime":3600,"Loads":[65536,32768,16384],"Totalram":8388608,"Freeram":4194304,"Sharedram":0,"Bufferram":1024,"Totalswap":2097152,"Freeswap":1048576,"Procs":123,"Unit":1}).to_string();
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: resource.id,
            data: sysinfo,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("10-live-node-resources.png"),
    );
    click(&mut cx, window, "live-copy-json");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("Uptime")
    );
    cx.update_window(window, |_, window, cx| window.press("ctrl-1", cx))
        .unwrap();
    settle(&mut cx);
    cx.read_entity(&view, |view, _| {
        assert!(
            view.screen == super::Screen::History,
            "Ctrl+1 must leave Live"
        )
    });
    for _ in 0..10 {
        if recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|n| n.r#type == 14)
            .count()
            == 4
        {
            break;
        }
        settle(&mut cx);
    }
    let messages = recorded.lock().unwrap().clone();
    assert_eq!(messages.iter().filter(|n| n.r#type == 13).count(), 4);
    assert_eq!(messages.iter().filter(|n| n.r#type == 14).count(), 4);
    assert!(messages.iter().all(|n| [13, 14].contains(&n.r#type)));
    let filtered: serde_json::Value = serde_json::from_str(
        &messages
            .iter()
            .filter(|n| n.r#type == 13)
            .nth(1)
            .unwrap()
            .data,
    )
    .unwrap();
    assert_eq!(
        filtered["data"],
        serde_json::json!({"interval":"2s","state":1,"proto":6,"family":10})
    );
    // Closing the owned dashboard also requests stop, even with a held first update.
    cx.update_window(window, |_, window, cx| window.press("ctrl-6", cx))
        .unwrap();
    settle(&mut cx);
    click(&mut cx, window, "live-start");
    settle(&mut cx);
    let (tray, tray_actions, _available) = super::tray::Client::fixture();
    cx.update_entity(&view, |view, _| view.tray = Some(tray));
    cx.update_window(window, |_, window, cx| {
        assert!(view.update(cx, |view, _| view.should_close_dashboard(window)));
        window.remove_window();
    })
    .unwrap();
    settle(&mut cx);
    tray_actions.send(super::tray::Action::Open).unwrap();
    settle(&mut cx);
    let restored = cx.read_entity(&view, |view, _| {
        assert!(!view.live.snapshot.running);
        view.main_window.unwrap()
    });
    capture(
        &mut cx,
        restored,
        &evidence.join("11-live-close-stopped.png"),
    );
    for _ in 0..10 {
        if recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|n| n.r#type == 14)
            .count()
            == 5
        {
            break;
        }
        settle(&mut cx);
    }
    let messages = recorded.lock().unwrap().clone();
    assert_eq!(messages.iter().filter(|n| n.r#type == 13).count(), 5);
    assert_eq!(messages.iter().filter(|n| n.r#type == 14).count(), 5);
    drop(tray_actions);
    drop(daemon);
    drop(replies);
    task.abort();
    drop(view);
    drop(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(
        evidence.join("commands.json"),
        serde_json::to_vec_pretty(
            &messages
                .iter()
                .map(|n| serde_json::json!({"id":n.id,"action":n.r#type,"data":n.data}))
                .collect::<Vec<_>>(),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":["01-live-sockets.png","02-live-sockets-page-two.png","03-live-daemon-filters.png","04-live-process-status.png","05-live-process-environment.png","06-live-process-sockets.png","07-live-process-maps.png","08-live-process-error-retained.png","09-live-permission-while-awaiting.png","10-live-node-resources.png","11-live-close-stopped.png"],"proof_boundary":"Actual Kit start/pause/pickers/paging/selection/search/copy, real streaming gRPC fixture, two updates per task ID, process error preserving data, permission during held task update, exact start/stop JSON and cleanup. TASK_STOP has no success ACK in pinned daemon. No real kernel process/socket or firewall proof."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Open Live and start; require two updates and 225 sockets, page by actual input","Pause, select TCP/IPv6/Established/2s, restart and require 113 filtered sockets plus exact daemon JSON","Select/copy socket, inspect PID42; read/copy Env, filter descriptors and inspect Maps/full JSON","Inject task ERROR; retain displayed data, request stop without policy uncertainty","Start node monitor with first update held, answer real permission by input, then display/copy Sysinfo resources","Leave Live; require stop. Restart with first update held, close through production boundary, restore and require fifth stop with no firewall commands"]).unwrap()).unwrap();
}

#[test]
fn headless_firewall_forms() {
    use crate::firewall::{
        self,
        document::{Document, Rule as FwRule},
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
    let initial = protocol::SysFirewall {
        enabled: true,
        version: 1,
        system_rules: vec![protocol::FwChains {
            rule: Some(protocol::FwRule {
                parameters: "preserved-wrapper".into(),
                position: u64::MAX,
                ..Default::default()
            }),
            chains: vec![protocol::FwChain {
                name: "input".into(),
                table: "filter".into(),
                family: "inet".into(),
                r#type: "filter".into(),
                hook: "input".into(),
                policy: "accept".into(),
                rules: vec![
                    protocol::FwRule {
                        uuid: "existing-rule".into(),
                        enabled: true,
                        target: "accept".into(),
                        description: "Keep this rule unchanged".into(),
                        ..Default::default()
                    },
                    protocol::FwRule {
                        uuid: "budget-rule".into(),
                        enabled: true,
                        target: "accept".into(),
                        description: "Large synthetic value".into(),
                        expressions: vec![protocol::Expressions {
                            statement: Some(protocol::Statement {
                                name: "fixture".into(),
                                values: vec![protocol::StatementValues {
                                    key: "blob".into(),
                                    value: "x".repeat(crate::nodes::FIREWALL_BYTES - 6000),
                                }],
                                ..Default::default()
                            }),
                        }],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
        }],
    };
    assert!(prost::Message::encoded_len(&initial) < crate::nodes::FIREWALL_BYTES);
    let (mut daemon, replies, mut commands) = runtime.block_on(async {
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
                name: "forms-fixture".into(),
                config: r#"{"Firewall":"nftables"}"#.into(),
                system_firewall: Some(initial.clone()),
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
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(super::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-4", cx))
        .unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window, "nodes-firewall");
    settle(&mut cx);
    let editor = cx
        .update(|cx| cx.windows())
        .into_iter()
        .find(|w| !before.contains(w))
        .unwrap();
    let copy = |cx: &mut HeadlessAppContext| {
        click(cx, editor, "firewall-copy");
        Document::parse(&cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())).unwrap()
    };
    let row = |cx: &mut HeadlessAppContext, id: &'static str, index: usize| {
        cx.update_window(editor, |_, window, cx| window.click((id, index), cx))
            .unwrap();
        cx.run_until_parked();
    };
    let chain = |cx: &mut HeadlessAppContext, index: u64| {
        cx.update_window(editor, |_, window, cx| {
            window.click(("firewall-chain", index), cx)
        })
        .unwrap();
        cx.run_until_parked();
    };
    let rule_tab = |cx: &mut HeadlessAppContext, index: usize| {
        cx.update_window(editor, |_, window, cx| {
            window.within("fw-rule-tabs").click(index, cx)
        })
        .unwrap();
        cx.run_until_parked();
    };

    click(&mut cx, editor, "fw-chain-edit");
    settle(&mut cx);
    fill(&mut cx, editor, "fw-chain-name", "input-reviewed");
    fill(&mut cx, editor, "fw-chain-policy", "drop");
    capture(&mut cx, editor, &evidence.join("01-chain-form.png"));
    click(&mut cx, editor, "fw-form-save");
    settle(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "forms-fixture")
            .unwrap()
            .config,
        initial
    );
    assert_eq!(copy(&mut cx).chain(0, 0).unwrap().rules.len(), 2);
    click(&mut cx, editor, "fw-chain-new");
    settle(&mut cx);
    click(&mut cx, editor, "fw-form-save");
    settle(&mut cx); // Missing name is invalid and remains in the form.
    fill(&mut cx, editor, "fw-chain-name", "owned-output");
    fill(&mut cx, editor, "fw-chain-hook", "output");
    click(&mut cx, editor, "fw-form-save");
    settle(&mut cx);
    assert_eq!(copy(&mut cx).system_rules[0].chains.len(), 2);
    capture(&mut cx, editor, &evidence.join("02-created-chain.png"));

    chain(&mut cx, 0);
    row(&mut cx, "firewall-rule", 1);
    click(&mut cx, editor, "fw-rule-edit");
    settle(&mut cx);
    rule_tab(&mut cx, 1);
    row(&mut cx, "fw-expression-row", 0);
    click(&mut cx, editor, "fw-expression-edit");
    click(&mut cx, editor, "fw-value-new");
    fill(&mut cx, editor, "fw-value-key", "extra");
    click(&mut cx, editor, "fw-value-value");
    cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("y".repeat(8192))));
    cx.update_window(editor, |_, window, cx| window.press("ctrl-v", cx))
        .unwrap();
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    capture(
        &mut cx,
        editor,
        &evidence.join("03-value-budget-rejected.png"),
    );
    click(&mut cx, editor, "fw-value-cancel");
    click(&mut cx, editor, "fw-statement-cancel");
    click(&mut cx, editor, "fw-form-cancel");
    assert_eq!(
        copy(&mut cx).chain(0, 0).unwrap().rules[1],
        FwRule::from(initial.system_rules[0].chains[0].rules[1].clone())
    );

    chain(&mut cx, 1);
    click(&mut cx, editor, "fw-rule-new");
    settle(&mut cx);
    fill(&mut cx, editor, "fw-rule-id", "native-owned-rule");
    fill(
        &mut cx,
        editor,
        "fw-rule-description",
        "Reviewed native rule",
    );
    fill(&mut cx, editor, "fw-rule-target", "reject");
    fill(
        &mut cx,
        editor,
        "fw-rule-target-parameters",
        "with tcp reset",
    );
    fill(&mut cx, editor, "fw-rule-position", "18446744073709551615");
    click(&mut cx, editor, "fw-rule-enabled");
    rule_tab(&mut cx, 2);
    fill(&mut cx, editor, "fw-rule-table", "legacy-table");
    fill(&mut cx, editor, "fw-rule-chain", "legacy-chain");
    fill(&mut cx, editor, "fw-rule-parameters", "-p tcp");
    rule_tab(&mut cx, 1);
    click(&mut cx, editor, "fw-expression-new");
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    capture(&mut cx, editor, &evidence.join("04-invalid-statement.png"));
    fill(&mut cx, editor, "fw-statement-name", "tcp");
    fill(&mut cx, editor, "fw-statement-op", "==");
    click(&mut cx, editor, "fw-value-new");
    fill(&mut cx, editor, "fw-value-key", "dport");
    fill(&mut cx, editor, "fw-value-value", "443");
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    click(&mut cx, editor, "fw-value-new");
    fill(&mut cx, editor, "fw-value-key", "sport");
    fill(&mut cx, editor, "fw-value-value", "1000-2000");
    click(&mut cx, editor, "fw-value-save");
    row(&mut cx, "fw-value-row", 0);
    click(&mut cx, editor, "fw-value-edit");
    fill(&mut cx, editor, "fw-value-value", "8443");
    click(&mut cx, editor, "fw-value-save");
    click(&mut cx, editor, "fw-value-down");
    click(&mut cx, editor, "fw-value-up");
    row(&mut cx, "fw-value-row", 1);
    click(&mut cx, editor, "fw-value-delete");
    click(&mut cx, editor, "fw-value-new");
    fill(&mut cx, editor, "fw-value-key", "cancelled");
    click(&mut cx, editor, "fw-value-cancel");
    capture(&mut cx, editor, &evidence.join("05-statement-values.png"));
    click(&mut cx, editor, "fw-statement-save");
    click(&mut cx, editor, "fw-expression-new");
    fill(&mut cx, editor, "fw-statement-name", "counter");
    click(&mut cx, editor, "fw-statement-save");
    row(&mut cx, "fw-expression-row", 1);
    click(&mut cx, editor, "fw-expression-up");
    click(&mut cx, editor, "fw-expression-down");
    click(&mut cx, editor, "fw-expression-delete");
    click(&mut cx, editor, "fw-expression-new");
    fill(&mut cx, editor, "fw-statement-name", "cancelled");
    click(&mut cx, editor, "fw-statement-cancel");
    row(&mut cx, "fw-expression-row", 0);
    click(&mut cx, editor, "fw-expression-edit");
    fill(&mut cx, editor, "fw-statement-op", "!=");
    fill(&mut cx, editor, "fw-statement-op", "==");
    click(&mut cx, editor, "fw-statement-save");
    click(&mut cx, editor, "fw-form-save");
    settle(&mut cx);
    let doc = copy(&mut cx);
    let native = &doc.chain(0, 1).unwrap().rules[0];
    assert_eq!(native.uuid, "native-owned-rule");
    assert_eq!(native.position, u64::MAX);
    assert!(!native.enabled);
    assert_eq!(native.expressions.len(), 1);
    assert_eq!(
        native.expressions[0].statement.as_ref().unwrap().values[0].value,
        "8443"
    );
    capture(&mut cx, editor, &evidence.join("06-created-rule.png"));
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-toggle");
    settle(&mut cx);
    assert!(copy(&mut cx).chain(0, 1).unwrap().rules[0].enabled);
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-duplicate");
    settle(&mut cx);
    click(&mut cx, editor, "fw-form-cancel");
    assert_eq!(copy(&mut cx).chain(0, 1).unwrap().rules.len(), 1);
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-duplicate");
    settle(&mut cx);
    fill(&mut cx, editor, "fw-rule-description", "duplicate draft");
    click(&mut cx, editor, "fw-form-save");
    settle(&mut cx);
    let doc = copy(&mut cx);
    assert_ne!(
        doc.chain(0, 1).unwrap().rules[0].uuid,
        doc.chain(0, 1).unwrap().rules[1].uuid
    );
    row(&mut cx, "firewall-rule", 1);
    click(&mut cx, editor, "fw-rule-up");
    settle(&mut cx);
    capture(
        &mut cx,
        editor,
        &evidence.join("07-duplicate-and-order.png"),
    );
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-delete");
    click(&mut cx, editor, "fw-delete-cancel");
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-delete");
    capture(&mut cx, editor, &evidence.join("08-removal-review.png"));
    click(&mut cx, editor, "fw-delete-confirm");
    settle(&mut cx);
    click(&mut cx, editor, "fw-chain-new");
    settle(&mut cx);
    fill(&mut cx, editor, "fw-chain-name", "temporary-chain");
    click(&mut cx, editor, "fw-form-save");
    settle(&mut cx);
    click(&mut cx, editor, "fw-rule-new");
    settle(&mut cx);
    click(&mut cx, editor, "fw-form-save");
    settle(&mut cx); // Enabled empty rules are not silently accepted.
    rule_tab(&mut cx, 1);
    click(&mut cx, editor, "fw-expression-new");
    fill(&mut cx, editor, "fw-statement-name", "tcp");
    fill(&mut cx, editor, "fw-statement-op", "==");
    click(&mut cx, editor, "fw-value-new");
    fill(&mut cx, editor, "fw-value-key", "dport");
    fill(&mut cx, editor, "fw-value-value", "80");
    click(&mut cx, editor, "fw-value-save");
    click(&mut cx, editor, "fw-statement-save");
    click(&mut cx, editor, "fw-form-save");
    settle(&mut cx);
    assert_eq!(
        copy(&mut cx).chain(0, 2).unwrap().rules[0].position,
        0,
        "new insertion metadata defaults to zero, not an array index"
    );
    click(&mut cx, editor, "fw-chain-delete");
    click(&mut cx, editor, "fw-delete-cancel");
    click(&mut cx, editor, "fw-chain-delete");
    click(&mut cx, editor, "fw-delete-confirm");
    settle(&mut cx);
    let draft = copy(&mut cx);
    assert_eq!(draft.system_rules[0].chains.len(), 2);
    assert_eq!(
        draft.system_rules[0].rule,
        Some(FwRule::from(initial.system_rules[0].rule.clone().unwrap()))
    );
    let mut expected = initial.clone();
    expected.system_rules[0].chains[0].name = "input-reviewed".into();
    expected.system_rules[0].chains[0].policy = "drop".into();
    expected.system_rules[0].chains.push(protocol::FwChain {
        name: "owned-output".into(),
        table: "filter".into(),
        family: "inet".into(),
        r#type: "filter".into(),
        hook: "output".into(),
        policy: "accept".into(),
        rules: vec![protocol::FwRule {
            uuid: "native-owned-rule".into(),
            description: "Reviewed native rule".into(),
            target: "reject".into(),
            target_parameters: "with tcp reset".into(),
            position: u64::MAX,
            enabled: true,
            table: "legacy-table".into(),
            chain: "legacy-chain".into(),
            parameters: "-p tcp".into(),
            expressions: vec![protocol::Expressions {
                statement: Some(protocol::Statement {
                    name: "tcp".into(),
                    op: "==".into(),
                    values: vec![protocol::StatementValues {
                        key: "dport".into(),
                        value: "8443".into(),
                    }],
                }),
            }],
        }],
        ..Default::default()
    });
    assert_eq!(protocol::SysFirewall::from(draft), expected);
    assert_eq!(
        firewall::read(&config.database, "forms-fixture")
            .unwrap()
            .config,
        initial
    );
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), commands.message()).await
            })
            .is_err()
    );
    capture(&mut cx, editor, &evidence.join("09-staged-forms.png"));
    click(&mut cx, editor, "fw-draft-review");
    settle(&mut cx);
    capture(&mut cx, editor, &evidence.join("10-complete-review.png"));
    click(&mut cx, editor, "firewall-confirm");
    settle(&mut cx);
    let command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(command.r#type, 5);
    assert_eq!(command.sys_firewall, Some(expected.clone()));
    let mut asking = daemon.clone();
    let asking = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/forms-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let popup = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("11-permission-under-form-ack.png"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(asking).unwrap().duration, "once");
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: command.id,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "forms-fixture")
            .unwrap()
            .config,
        expected
    );
    capture(&mut cx, editor, &evidence.join("12-accepted-forms.png"));
    chain(&mut cx, 1);
    row(&mut cx, "firewall-rule", 0);
    click(&mut cx, editor, "fw-rule-edit");
    settle(&mut cx);
    fill(
        &mut cx,
        editor,
        "fw-rule-description",
        "Kept after reconnect",
    );
    runtime
        .block_on(daemon.subscribe(protocol::ClientConfig {
            name: "forms-fixture".into(),
            config: r#"{"Firewall":"nftables"}"#.into(),
            system_firewall: Some(expected.clone()),
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
    click(&mut cx, editor, "fw-form-save");
    settle(&mut cx);
    assert_eq!(
        copy(&mut cx).chain(0, 1).unwrap().rules[0].description,
        "Kept after reconnect"
    );
    assert_eq!(
        firewall::read(&config.database, "forms-fixture")
            .unwrap()
            .config,
        expected
    );
    capture(
        &mut cx,
        editor,
        &evidence.join("13-stale-form-kept-locally.png"),
    );
    click(&mut cx, editor, "firewall-close");
    drop(commands);
    drop(replies);
    drop(daemon);
    drop(view);
    drop(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":["01-chain-form.png","02-created-chain.png","03-value-budget-rejected.png","04-invalid-statement.png","05-statement-values.png","06-created-rule.png","07-duplicate-and-order.png","08-removal-review.png","09-staged-forms.png","10-complete-review.png","11-permission-under-form-ack.png","12-accepted-forms.png","13-stale-form-kept-locally.png"],"proof_boundary":"Actual native Kit chain/rule/expression/value forms, invalid/over-budget preservation, local staging, duplicate/order/toggle/removal and cancellation, exact full gRPC snapshot, permission under held ACK and stale-form local recovery after reconnect. Near-cap synthetic wire value; no real firewall enforcement or native compositor proof. This journey does not cover file/profile controls, built-in policy presets or enable/disable."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Edit chain metadata and policy; reject empty new chain then add output chain","Edit near-cap synthetic statement; reject additional 8 KiB value and cancel without changing draft","Create complete rule with uint64 max, target parameters, enabled checkbox and legacy fields","Add/edit/delete/reorder/cancel statements and values through bounded native controls","Toggle rule, cancel/keep duplicate, move it and cancel/confirm removal; create/remove temporary chain","Require no daemon command before global review; require exact full protobuf and independent permission under ACK","ACK persists accepted snapshot; reconnect during a form still allows local keep/copy without applying to replacement session"]).unwrap()).unwrap();
}

#[test]
fn headless_firewall() {
    use crate::firewall::{self, document::Document};
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let initial = protocol::SysFirewall {
        enabled: true,
        version: 1,
        system_rules: vec![protocol::FwChains {
            rule: Some(protocol::FwRule {
                parameters: "legacy-kept".into(),
                position: u64::MAX,
                ..Default::default()
            }),
            chains: vec![protocol::FwChain {
                name: "input".into(),
                table: "filter".into(),
                family: "inet".into(),
                r#type: "filter".into(),
                hook: "input".into(),
                policy: "accept".into(),
                rules: (0..225)
                    .map(|i| protocol::FwRule {
                        uuid: format!("owned-rule-{i:04}"),
                        position: i,
                        enabled: true,
                        description: format!("Synthetic service {i}"),
                        target: "accept".into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
        }],
    };
    let (daemon, replies, mut commands) = runtime.block_on(async {
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
                name: "firewall-fixture".into(),
                system_firewall: Some(initial.clone()),
                config: r#"{"Firewall":"nftables"}"#.into(),
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
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(super::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-4", cx))
        .unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window, "nodes-firewall");
    settle(&mut cx);
    let editor = cx
        .update(|cx| cx.windows())
        .into_iter()
        .find(|w| !before.contains(w))
        .unwrap();
    capture(&mut cx, editor, &evidence.join("01-firewall-chains.png"));
    click(&mut cx, editor, "firewall-next");
    capture(
        &mut cx,
        editor,
        &evidence.join("02-firewall-rules-page-two.png"),
    );
    fill(&mut cx, editor, "firewall-search", "owned-rule-0224");
    settle(&mut cx);
    capture(
        &mut cx,
        editor,
        &evidence.join("03-firewall-rule-filter.png"),
    );
    cx.update_window(editor, |_, window, cx| {
        window.within("firewall-tabs").click(1usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("[]", cx);
    })
    .unwrap();
    click(&mut cx, editor, "firewall-review");
    settle(&mut cx);
    capture(
        &mut cx,
        editor,
        &evidence.join("04-firewall-invalid-json.png"),
    );
    let mut next = initial.clone();
    next.system_rules[0].chains[0].policy = "drop".into();
    next.system_rules[0].chains[0].rules[0].description = "Reviewed firewall edit".into();
    cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            " ".repeat(firewall::MAX_JSON + 1),
        ))
    });
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.press("ctrl-v", cx);
    })
    .unwrap();
    click(&mut cx, editor, "firewall-copy");
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        "[]",
        "oversized paste must preserve the previous draft"
    );
    cx.update_window(editor, |_, window, cx| {
        window.within("firewall-tabs").click(1usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    let text = Document::from(next.clone()).json().unwrap();
    cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)));
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.press("ctrl-v", cx);
    })
    .unwrap();
    click(&mut cx, editor, "firewall-review");
    settle(&mut cx);
    capture(
        &mut cx,
        editor,
        &evidence.join("05-firewall-before-after.png"),
    );
    click(&mut cx, editor, "firewall-back");
    settle(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "firewall-fixture")
            .unwrap()
            .config,
        initial
    );
    click(&mut cx, editor, "firewall-review");
    settle(&mut cx);
    click(&mut cx, editor, "firewall-confirm");
    settle(&mut cx);
    let command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(command.r#type, 5);
    assert_eq!(command.sys_firewall.as_ref(), Some(&next));
    assert!(command.data.is_empty() && command.rules.is_empty());
    assert_eq!(
        firewall::read(&config.database, "firewall-fixture")
            .unwrap()
            .config,
        initial
    );
    let mut asking = daemon.clone();
    let asking = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/firewall-prompt".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let popup = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("06-permission-during-firewall-ack.png"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(asking).unwrap().action, "deny");
    let held_read = (0..100)
        .find_map(|_| {
            let permit = crate::nodes::read_permit();
            if permit.is_none() {
                std::thread::sleep(Duration::from_millis(2));
            }
            permit
        })
        .expect("fixture can own the single node read permit");
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: command.id,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx); // Post-ACK load must wait without retaining a busy-reader error.
    drop(held_read);
    settle(&mut cx);
    let accepted = firewall::read(&config.database, "firewall-fixture").unwrap();
    assert_eq!(accepted.config, next);
    assert!(!accepted.reported);
    capture(&mut cx, editor, &evidence.join("07-firewall-accepted.png"));
    click(&mut cx, editor, "firewall-copy");
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(
        protocol::SysFirewall::from(Document::parse(&copied).unwrap()),
        next
    );
    cx.update_window(editor, |_, window, cx| {
        window.within("firewall-tabs").click(1usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    let mut rejected = next.clone();
    rejected.system_rules[0].chains[0].policy = "accept".into();
    let text = Document::from(rejected).json().unwrap();
    cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)));
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.press("ctrl-v", cx);
    })
    .unwrap();
    click(&mut cx, editor, "firewall-review");
    settle(&mut cx);
    click(&mut cx, editor, "firewall-confirm");
    settle(&mut cx);
    let command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: command.id,
            code: 1,
            data: "Synthetic reload rejected".into(),
        }))
        .unwrap();
    settle(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "firewall-fixture")
            .unwrap()
            .config,
        next
    );
    capture(
        &mut cx,
        editor,
        &evidence.join("08-firewall-rejected-draft-retained.png"),
    );
    click(&mut cx, editor, "firewall-close");
    drop(daemon);
    drop(commands);
    drop(replies);
    drop(view);
    drop(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":["01-firewall-chains.png","02-firewall-rules-page-two.png","03-firewall-rule-filter.png","04-firewall-invalid-json.png","05-firewall-before-after.png","06-permission-during-firewall-ack.png","07-firewall-accepted.png","08-firewall-rejected-draft-retained.png"],"proof_boundary":"Actual Nodes/open/chain paging/search/complete JSON validation/review/back/apply/copy, exact complete RELOAD_FW_RULES protobuf, no optimistic cache update, independent permission under held ACK, accepted-vs-reported provenance and rejected draft/cache preservation. Synthetic daemon only. This journey does not cover dedicated forms, file/profile workflows, enable-disable sequencing or native firewall input."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Open selected node system firewall; browse 225 rules with paging and filter","Reject non-object JSON; edit policy/description in complete draft and review exact before/after","Back leaves configuration intact; confirm sends complete protobuf while cache remains reported old value","Answer independent real permission under held firewall ACK","ACK saves accepted snapshot and exact full clipboard roundtrip; ERROR preserves draft and last accepted state"]).unwrap()).unwrap();
}

#[test]
fn headless_history_process() {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let connect = |name: &str| {
        runtime.block_on(async {
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
            let mut client = protocol::ui_client::UiClient::new(channel);
            client
                .subscribe(protocol::ClientConfig {
                    name: name.into(),
                    ..Default::default()
                })
                .await
                .unwrap();
            let (tx, rx) = tokio::sync::mpsc::channel(8);
            let stream = client
                .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
                .await
                .unwrap()
                .into_inner();
            (client, tx, stream)
        })
    };
    let (mut alpha, replies, mut commands) = connect("history-alpha");
    let (mut beta, _beta_replies, mut beta_commands) = connect("selected-beta");
    let alpha_id = backend.handle.node("history-alpha").unwrap().id;
    backend
        .handle
        .select_node(backend.handle.node("selected-beta").unwrap().id)
        .unwrap();
    for (node, path, pid) in [
        ("history-alpha", "/usr/bin/recorded-valid", 42),
        ("history-alpha", "/usr/bin/recorded-unknown", 0),
        ("history-alpha", "/usr/bin/recorded-invalid", u32::MAX),
        ("offline-node", "/usr/bin/recorded-offline", 77),
    ] {
        backend.handle.history.ingest(
            node.into(),
            vec![protocol::Event {
                unixnano: crate::history::now_nanos(),
                connection: Some(protocol::Connection {
                    process_path: path.into(),
                    process_id: pid,
                    ..Default::default()
                }),
                ..Default::default()
            }],
            "observed",
        );
    }
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(super::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    let open_detail = |cx: &mut HeadlessAppContext, search: &str| {
        cx.update_window(window, |_, window, cx| {
            window.press("ctrl-1", cx);
            window.press("ctrl-f", cx);
            window.press("ctrl-a", cx);
            window.input(search, cx);
        })
        .unwrap();
        settle(cx);
        cx.read_entity(&view, |v, _| assert_eq!(v.history.page.rows.len(), 1));
        cx.update_window(window, |_, window, cx| {
            window.click(("history-row", 0usize), cx)
        })
        .unwrap();
        click(cx, window, "history-details");
        settle(cx);
        cx.read_entity(&view, |v, _| v.history.detail_window.unwrap())
    };
    let details = open_detail(&mut cx, "recorded-valid");
    capture(&mut cx, details, &evidence.join("01-recorded-process.png"));
    keyboard_button(&mut cx, details, "connection-inspect-process");
    settle(&mut cx);
    cx.read_entity(&view, |v, _| assert!(v.screen == super::Screen::Live));
    assert_eq!(backend.handle.status().session.unwrap().id, alpha_id);
    let command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(command.r#type, 13);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&command.data).unwrap(),
        serde_json::json!({"name":"pid-monitor","data":{"pid":"42","interval":"5s"}})
    );
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), beta_commands.message()).await
            })
            .is_err(),
        "the selected unrelated node must receive nothing"
    );
    let request = runtime.spawn(async move {
        beta.ask_rule(protocol::Connection {
            process_path: "/usr/bin/independent-permission".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner()
    });
    settle(&mut cx);
    let popup = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("02-permission-during-process-query.png"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(request).unwrap().duration, "once");
    runtime.block_on(replies.send(protocol::NotificationReply {
        id:command.id, data:serde_json::json!({"PID":42,"Path":"/usr/bin/reused-current-process","Status":"Name:\treused-current\nPid:\t42","Env":{"CURRENT_ONLY":"not historical"}}).to_string(), ..Default::default()
    })).unwrap();
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("03-current-process-on-event-node.png"),
    );
    click(&mut cx, window, "live-copy-json");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("reused-current-process")
    );
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: command.id,
            code: 1,
            data: "Synthetic process exited".into(),
        }))
        .unwrap();
    settle(&mut cx);
    cx.read_entity(&view, |v, _| {
        assert!(!v.live.snapshot.running);
        assert!(v.live.snapshot.error.as_ref().unwrap().contains("exited"));
        assert!(v.live.parsed.is_some());
    });
    capture(&mut cx, window, &evidence.join("04-process-exited.png"));
    click(&mut cx, details, "connection-close");
    for query in ["recorded-unknown", "recorded-invalid", "recorded-offline"] {
        let details = open_detail(&mut cx, query);
        click(&mut cx, details, "connection-inspect-process");
        settle(&mut cx);
        cx.read_entity(&view, |v, _| assert!(v.screen == super::Screen::History));
        capture(&mut cx, details, &evidence.join(format!("05-{query}.png")));
        click(&mut cx, details, "connection-close");
    }
    // Capture an old session at input time, then replace it before the main timer
    // drains the action. The new connection must never inherit a stale request.
    let details = open_detail(&mut cx, "recorded-valid");
    click(&mut cx, details, "connection-inspect-process");
    runtime
        .block_on(alpha.subscribe(protocol::ClientConfig {
            name: "history-alpha".into(),
            ..Default::default()
        }))
        .unwrap();
    assert_ne!(backend.handle.node("history-alpha").unwrap().id, alpha_id);
    settle(&mut cx);
    cx.read_entity(&view, |v, _| assert!(v.screen == super::Screen::History));
    capture(
        &mut cx,
        details,
        &evidence.join("06-reconnected-request-rejected.png"),
    );
    click(&mut cx, details, "connection-close");
    // Only one start was issued. Stopping on ERROR is intentionally unacknowledged.
    let stop = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
    });
    if let Ok(Some(stop)) = stop {
        assert_eq!(stop.r#type, 14);
    }
    drop(replies);
    drop(commands);
    drop(beta_commands);
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    drop(cx);
    backend.handle.shutdown();
    backend.join();
    assert!(!config.socket.exists());
    let screenshots = [
        "01-recorded-process.png",
        "02-permission-during-process-query.png",
        "03-current-process-on-event-node.png",
        "04-process-exited.png",
        "05-recorded-unknown.png",
        "05-recorded-invalid.png",
        "05-recorded-offline.png",
        "06-reconnected-request-rejected.png",
    ];
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":screenshots,"proof_boundary":"Actual connection-details keyboard/button input; current PID routed to the historical event's node rather than the selected node; separate real permission while task update is held; current data copy, process error retention, invalid/missing/offline PID refusal and queued stale-session rejection. Synthetic daemon only; no real process visibility proof."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Select history row and open details; Tab/Space Inspect process","Require exact PID42/5s task only on history-alpha while selected-beta receives no command","Hold update, answer real independent permission, then receive current data with a different executable and copy complete JSON","Report process exited; keep last data with error","Unknown/invalid PID and offline node cannot start inspection","Queue inspection then reconnect; reject stale session rather than target replacement"]).unwrap()).unwrap();
}

#[test]
fn headless_history_preferences() {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let make_event = |i: i64| protocol::Event {
        unixnano: crate::history::now_nanos() + i,
        connection: Some(protocol::Connection {
            process_path: format!("/usr/bin/retention-{i:04}"),
            dst_host: "history.example".into(),
            dst_ip: "192.0.2.10".into(),
            dst_port: 443,
            ..Default::default()
        }),
        ..Default::default()
    };
    backend.handle.history.ingest(
        "retention-fixture".into(),
        (0..430).map(make_event).collect(),
        "observed",
    );
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let db = rusqlite::Connection::open(&config.database).unwrap();
    db.execute(
        "UPDATE events SET received=? WHERE id<=180",
        [crate::history::now_seconds() - 2 * 86400],
    )
    .unwrap();
    let mut cx = context();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(super::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    settle(&mut cx);
    click(&mut cx, window, "history-preferences");
    settle(&mut cx);
    let preferences = cx.read_entity(&view, |view, _| view.history.preferences_window.unwrap());
    capture(
        &mut cx,
        preferences,
        &evidence.join("01-history-preferences.png"),
    );
    fill(&mut cx, preferences, "history-days", "0");
    click(&mut cx, preferences, "history-retention-review");
    settle(&mut cx);
    assert_eq!(backend.handle.history.storage.snapshot().config.days, 7);
    capture(
        &mut cx,
        preferences,
        &evidence.join("02-invalid-retention.png"),
    );
    fill(&mut cx, preferences, "history-days", "1");
    fill(&mut cx, preferences, "history-mib", "32");
    fill(&mut cx, preferences, "history-cleanup", "1");
    click(&mut cx, preferences, "history-retention-review");
    settle(&mut cx);
    capture(
        &mut cx,
        preferences,
        &evidence.join("03-retention-review.png"),
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        430
    );
    assert_eq!(backend.handle.history.storage.snapshot().config.days, 7);
    click(&mut cx, preferences, "history-retention-back");
    settle(&mut cx);
    assert_eq!(backend.handle.history.storage.snapshot().config.days, 7);
    click(&mut cx, preferences, "history-retention-review");
    settle(&mut cx);
    // Hold SQLite while Apply waits, then prove a real permission can still be answered.
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    click(&mut cx, preferences, "history-retention-apply");
    let mut daemon = runtime.block_on(async {
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
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "retention-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    let asking = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/history-save-permission".into(),
                dst_host: "history-save.example".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let popup = cx.read_entity(&view, |view, _| view.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("04-permission-during-retention.png"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(asking).unwrap().action, "deny");
    db.execute_batch("ROLLBACK").unwrap();
    settle(&mut cx);
    // A writer busy timeout is an honest failure, never a partial preference save.
    if backend.handle.history.storage.snapshot().config.days == 7 {
        click(&mut cx, preferences, "history-retention-review");
        settle(&mut cx);
        click(&mut cx, preferences, "history-retention-apply");
        settle(&mut cx);
    }
    let saved = backend.handle.history.storage.snapshot();
    assert_eq!(
        (
            saved.config.days,
            saved.config.mib,
            saved.config.cleanup_minutes
        ),
        (1, 32, 1)
    );
    assert_eq!(
        db.query_row("SELECT days FROM history_preferences", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        251
    );
    capture(
        &mut cx,
        preferences,
        &evidence.join("05-retention-applied.png"),
    );
    cx.update_window(preferences, |_, window, cx| {
        window.within("history-preferences-tabs").click(1usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    fill(&mut cx, preferences, "history-refresh-seconds", "61");
    click(&mut cx, preferences, "history-refresh-save");
    settle(&mut cx);
    assert_eq!(
        backend.handle.history.settings.snapshot().refresh_seconds,
        1
    );
    capture(
        &mut cx,
        preferences,
        &evidence.join("06-invalid-refresh.png"),
    );
    fill(&mut cx, preferences, "history-refresh-seconds", "0");
    click(&mut cx, preferences, "history-refresh-save");
    settle(&mut cx);
    assert_eq!(
        crate::history::settings::Store::open(&config.database)
            .unwrap()
            .snapshot()
            .refresh_seconds,
        0
    );
    capture(
        &mut cx,
        preferences,
        &evidence.join("07-manual-refresh-saved.png"),
    );
    click(&mut cx, preferences, "history-preferences-close");
    settle(&mut cx);
    let previous = cx.read_entity(&view, |view, _| view.history.page.rows[0].id);
    backend.handle.history.ingest(
        "retention-fixture".into(),
        (1000..1003).map(make_event).collect(),
        "observed",
    );
    runtime.block_on(backend.handle.history.flush()).unwrap();
    settle(&mut cx);
    cx.read_entity(&view, |view, _| {
        assert_eq!(view.history.page.rows[0].id, previous)
    });
    assert_eq!(
        db.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        254
    );
    capture(
        &mut cx,
        window,
        &evidence.join("08-manual-recording-continues.png"),
    );
    click(&mut cx, window, "history-refresh");
    settle(&mut cx);
    cx.read_entity(&view, |view, _| {
        assert!(view.history.page.rows[0].id > previous)
    });
    capture(
        &mut cx,
        window,
        &evidence.join("09-refreshed-on-demand.png"),
    );
    click(&mut cx, window, "history-preferences");
    settle(&mut cx);
    let preferences = cx.read_entity(&view, |view, _| view.history.preferences_window.unwrap());
    cx.update_window(preferences, |_, window, cx| {
        window.within("history-preferences-tabs").click(1usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    fill(&mut cx, preferences, "history-refresh-seconds", "2");
    click(&mut cx, preferences, "history-refresh-save");
    settle(&mut cx);
    click(&mut cx, preferences, "history-preferences-copy-path");
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        config.database.display().to_string()
    );
    capture(
        &mut cx,
        preferences,
        &evidence.join("10-auto-refresh-restored.png"),
    );
    click(&mut cx, preferences, "history-preferences-close");
    settle(&mut cx);
    let last = cx.read_entity(&view, |view, _| view.history.page.rows[0].id);
    backend.handle.history.ingest(
        "retention-fixture".into(),
        vec![make_event(2000)],
        "observed",
    );
    std::thread::sleep(Duration::from_millis(2100));
    runtime.block_on(backend.handle.history.flush()).unwrap();
    settle(&mut cx);
    cx.read_entity(&view, |view, _| {
        assert!(view.history.page.rows[0].id > last)
    });
    drop(db);
    drop(view);
    drop(cx);
    backend.join();
    assert!(!config.socket.exists());
    let reopened = Backend::start(config.clone()).unwrap();
    assert_eq!(
        reopened.handle.history.storage.snapshot().config,
        saved.config
    );
    assert_eq!(
        reopened.handle.history.settings.snapshot().refresh_seconds,
        2
    );
    reopened.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":["01-history-preferences.png","02-invalid-retention.png","03-retention-review.png","04-permission-during-retention.png","05-retention-applied.png","06-invalid-refresh.png","07-manual-refresh-saved.png","08-manual-recording-continues.png","09-refreshed-on-demand.png","10-auto-refresh-restored.png"],"proof_boundary":"Actual Kit retention validation/review/back/apply and saved display refresh; independent SQLite/file/clipboard/reopen assertions, real permission while SQLite is locked, manual display with continuing ingestion and explicit/automatic refresh. Synthetic owned fixtures only; real idle timer and rollback/CLI/physical compaction tested separately."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Open history preferences; reject zero days; review 1 day/32 MiB/1-minute cleanup; Back changes nothing","Hold writer during actual Apply, answer a real permission by input, release lock and retry if save honestly timed out","Reopen saved retention; reject refresh61, save0/manual, prove new history persists without display changes","Refresh now fetches new rows; reopen/save2 seconds, copy database path and prove automatic display resumes","Restart isolated backend and independently reopen retention and view settings; require scoped cleanup"]).unwrap()).unwrap();
}

#[test]
fn headless_firewall_files() {
    use crate::firewall::{
        self,
        document::{Chain, Document, Rule},
        transfer,
    };
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let files = evidence.join("files");
    std::fs::create_dir(&files).unwrap();
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let initial = protocol::SysFirewall {
        enabled: true,
        version: 1,
        system_rules: vec![protocol::FwChains {
            rule: Some(protocol::FwRule {
                position: u64::MAX,
                parameters: "legacy wrapper".into(),
                ..Default::default()
            }),
            chains: vec![protocol::FwChain {
                name: "input".into(),
                table: "filter".into(),
                family: "inet".into(),
                r#type: "filter".into(),
                hook: "input".into(),
                priority: "-10".into(),
                policy: "accept".into(),
                rules: vec![protocol::FwRule {
                    uuid: "original".into(),
                    description: "Original retained rule".into(),
                    enabled: true,
                    position: u64::MAX,
                    target: "accept".into(),
                    expressions: vec![protocol::Expressions {
                        statement: Some(protocol::Statement {
                            name: "tcp".into(),
                            op: "==".into(),
                            values: vec![protocol::StatementValues {
                                key: "dport".into(),
                                value: "443".into(),
                            }],
                        }),
                    }],
                    ..Default::default()
                }],
            }],
        }],
    };
    let original = Document::from(initial.clone());
    let chain = original.chain(0, 0).unwrap();
    let profile = Chain {
        name: "different profile name".into(),
        table: "FILTER".into(),
        family: "INET".into(),
        kind: "FILTER".into(),
        hook: "INPUT".into(),
        policy: "drop".into(),
        priority: "999".into(),
        rules: vec![
            Rule {
                uuid: "same-content-other-ID".into(),
                ..chain.rules[0].clone()
            },
            Rule {
                uuid: "owned-profile-rule".into(),
                description: "Imported port rule".into(),
                target: "reject".into(),
                target_parameters: "tcp reset".into(),
                ..chain.rules[0].clone()
            },
        ],
    };
    let profile_text = transfer::profile_json(&profile).unwrap();
    let profile_path = files.join("000.profile");
    for index in 0..205 {
        std::fs::write(files.join(format!("{index:03}.profile")), &profile_text).unwrap();
    }
    let initial_path = files.join("initial.json");
    std::fs::write(&initial_path, original.json().unwrap()).unwrap();
    let invalid_path = files.join("invalid.json");
    std::fs::write(&invalid_path, "{\"Unknown\":true}").unwrap();
    let enabled_path = files.join("enabled-mismatch.json");
    let mut mismatch = original.clone();
    mismatch.enabled = false;
    std::fs::write(&enabled_path, mismatch.json().unwrap()).unwrap();
    let wrong_path = files.join("wrong-chain.profile");
    let mut wrong = profile.clone();
    wrong.hook = "forward".into();
    std::fs::write(&wrong_path, transfer::profile_json(&wrong).unwrap()).unwrap();
    let (daemon, replies, mut commands) = runtime.block_on(async {
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
                name: "files-fixture".into(),
                config: r#"{"Firewall":"nftables"}"#.into(),
                system_firewall: Some(initial.clone()),
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
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(super::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-4", cx))
        .unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window, "nodes-firewall");
    settle(&mut cx);
    let editor = cx
        .update(|cx| cx.windows())
        .into_iter()
        .find(|w| !before.contains(w))
        .unwrap();
    let tab = |cx: &mut HeadlessAppContext, index: usize| {
        cx.update_window(editor, |_, window, cx| {
            window.within("firewall-tabs").click(index, cx)
        })
        .unwrap();
        settle(cx);
    };
    let path = |cx: &mut HeadlessAppContext, p: &std::path::Path| {
        fill(cx, editor, "fw-file-path", p.to_str().unwrap())
    };
    let copy = |cx: &mut HeadlessAppContext| {
        click(cx, editor, "firewall-copy");
        Document::parse(&cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())).unwrap()
    };
    tab(&mut cx, 2);
    path(&mut cx, &files);
    click(&mut cx, editor, "fw-profiles-list");
    settle(&mut cx);
    capture(&mut cx, editor, &evidence.join("01-profile-files.png"));
    click(&mut cx, editor, "fw-profiles-next");
    settle(&mut cx);
    cx.update_window(editor, |_, window, cx| {
        window.click(("fw-profile-file", 200usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    capture(&mut cx, editor, &evidence.join("02-profile-page-two.png"));
    click(&mut cx, editor, "fw-profiles-previous");
    settle(&mut cx);
    cx.update_window(editor, |_, window, cx| {
        window.click(("fw-profile-file", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, editor, "fw-profile-merge");
    settle(&mut cx);
    let mut expected = original.clone();
    expected.chain_mut(0, 0).unwrap().policy = "drop".into();
    expected
        .chain_mut(0, 0)
        .unwrap()
        .rules
        .push(profile.rules[1].clone());
    assert_eq!(copy(&mut cx), expected);
    click(&mut cx, editor, "fw-profile-merge");
    settle(&mut cx);
    assert_eq!(
        copy(&mut cx),
        expected,
        "repeat merge skips same-content and same-ID rules"
    );
    capture(&mut cx, editor, &evidence.join("03-staged-profile.png"));
    path(&mut cx, &wrong_path);
    click(&mut cx, editor, "fw-profile-merge");
    settle(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    path(&mut cx, &invalid_path);
    click(&mut cx, editor, "fw-file-load");
    settle(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    capture(
        &mut cx,
        editor,
        &evidence.join("04-invalid-import-retained.png"),
    );
    path(&mut cx, &enabled_path);
    click(&mut cx, editor, "fw-file-load");
    settle(&mut cx);
    assert_eq!(
        copy(&mut cx),
        mismatch,
        "Enabled imports stay local until transition review"
    );
    path(&mut cx, &initial_path);
    click(&mut cx, editor, "fw-file-load");
    settle(&mut cx);
    path(&mut cx, &profile_path);
    click(&mut cx, editor, "fw-profile-merge");
    settle(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    path(&mut cx, &profile_path);
    click(&mut cx, editor, "fw-profile-remove");
    settle(&mut cx);
    let mut removed = original.clone();
    removed.chain_mut(0, 0).unwrap().policy = "drop".into();
    assert_eq!(
        copy(&mut cx),
        removed,
        "remove only matching IDs; retain original semantic duplicate and policy"
    );
    click(&mut cx, editor, "fw-profile-merge");
    settle(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    let exported = files.join("exported.json");
    path(&mut cx, &exported);
    click(&mut cx, editor, "fw-file-export");
    settle(&mut cx);
    assert_eq!(
        Document::parse(&std::fs::read_to_string(&exported).unwrap()).unwrap(),
        expected
    );
    let preserved = std::fs::read(&exported).unwrap();
    click(&mut cx, editor, "fw-file-export");
    settle(&mut cx);
    assert_eq!(std::fs::read(&exported).unwrap(), preserved);
    let exported_profile = files.join("exported.profile");
    path(&mut cx, &exported_profile);
    click(&mut cx, editor, "fw-profile-export");
    settle(&mut cx);
    assert_eq!(
        &transfer::parse_profile(&std::fs::read_to_string(&exported_profile).unwrap()).unwrap(),
        expected.chain(0, 0).unwrap()
    );
    let envelope: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&exported_profile).unwrap()).unwrap();
    assert_eq!(envelope["Name"], "input");
    assert!(envelope["Profile"]["Rules"].is_array());
    click(&mut cx, editor, "fw-profile-copy");
    settle(&mut cx);
    assert_eq!(
        &transfer::parse_profile(
            &cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
        )
        .unwrap(),
        expected.chain(0, 0).unwrap()
    );
    capture(&mut cx, editor, &evidence.join("05-new-file-export.png"));
    path(&mut cx, &initial_path);
    click(&mut cx, editor, "fw-file-load");
    settle(&mut cx);
    assert_eq!(copy(&mut cx), original);
    path(&mut cx, &exported);
    click(&mut cx, editor, "fw-file-load");
    settle(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    assert_eq!(
        firewall::read(&config.database, "files-fixture")
            .unwrap()
            .config,
        initial
    );
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), commands.message()).await
            })
            .is_err()
    );
    capture(
        &mut cx,
        editor,
        &evidence.join("06-imported-complete-draft.png"),
    );
    click(&mut cx, editor, "fw-files-review");
    settle(&mut cx);
    capture(
        &mut cx,
        editor,
        &evidence.join("07-complete-file-review.png"),
    );
    click(&mut cx, editor, "firewall-back");
    settle(&mut cx);
    assert_eq!(copy(&mut cx), expected);
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    settle(&mut cx);
    click(&mut cx, editor, "firewall-confirm");
    settle(&mut cx);
    let command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(command.r#type, 5);
    assert_eq!(command.sys_firewall, Some(expected.clone().into()));
    assert!(command.data.is_empty());
    assert!(command.rules.is_empty());
    assert_eq!(
        firewall::read(&config.database, "files-fixture")
            .unwrap()
            .config,
        initial
    );
    let mut asking = daemon.clone();
    let asking = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/file-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let popup = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("08-permission-during-file-ack.png"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(asking).unwrap().duration, "once");
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: command.id,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
    let saved = firewall::read(&config.database, "files-fixture").unwrap();
    assert_eq!(saved.config, expected.clone().into());
    assert!(!saved.reported);
    capture(
        &mut cx,
        editor,
        &evidence.join("09-file-reload-accepted.png"),
    );
    // A raw JSON edit must be parsed before selecting/exporting a profile.
    tab(&mut cx, 1);
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("{invalid", cx);
    })
    .unwrap();
    settle(&mut cx);
    tab(&mut cx, 2);
    capture(
        &mut cx,
        editor,
        &evidence.join("10-invalid-json-cannot-export.png"),
    );
    // Return to a valid complete document through actual textarea input.
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input(&expected.json().unwrap(), cx);
    })
    .unwrap();
    settle(&mut cx);
    tab(&mut cx, 2);
    click(&mut cx, editor, "fw-profile-copy");
    settle(&mut cx);
    assert_eq!(
        &transfer::parse_profile(
            &cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
        )
        .unwrap(),
        expected.chain(0, 0).unwrap()
    );
    // Read-only recovery still exports after the captured node disconnects.
    drop(commands);
    drop(replies);
    drop(daemon);
    settle(&mut cx);
    assert!(
        backend
            .handle
            .node("files-fixture")
            .is_none_or(|node| !node.notifications),
        "notification stream is actually disconnected before offline export"
    );
    let offline = files.join("offline.json");
    path(&mut cx, &offline);
    click(&mut cx, editor, "fw-file-export");
    settle(&mut cx);
    assert_eq!(
        Document::parse(&std::fs::read_to_string(offline).unwrap()).unwrap(),
        expected
    );
    capture(&mut cx, editor, &evidence.join("11-offline-export.png"));
    click(&mut cx, editor, "firewall-close");
    drop(view);
    drop(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":["01-profile-files.png","02-profile-page-two.png","03-staged-profile.png","04-invalid-import-retained.png","05-new-file-export.png","06-imported-complete-draft.png","07-complete-file-review.png","08-permission-during-file-ack.png","09-file-reload-accepted.png","10-invalid-json-cannot-export.png","11-offline-export.png"],"proof_boundary":"Actual Kit path input, two profile-list pages, merge/dedup/remove, import validation and complete/chain file/clipboard export; no overwrite; local-only staging then exact complete gRPC reload, independent permission under ACK, accepted persistence and offline export. Native OS picker interaction, built-in policy presets, enable-disable and enforcement remain separate."})).unwrap()).unwrap();
}

#[test]
fn headless_firewall_policy() {
    use crate::firewall::{self, document::Document};
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mut initial = protocol::SysFirewall {
        enabled: true,
        version: 1,
        system_rules: vec![protocol::FwChains {
            rule: Some(protocol::FwRule {
                parameters: "retained wrapper".into(),
                position: u64::MAX,
                ..Default::default()
            }),
            chains: vec![protocol::FwChain {
                name: "input".into(),
                table: "opensnitch".into(),
                family: "inet".into(),
                r#type: "filter".into(),
                hook: "input".into(),
                priority: "-10".into(),
                policy: "drop".into(),
                rules: vec![protocol::FwRule {
                    uuid: "owned-user-rule".into(),
                    enabled: true,
                    position: u64::MAX,
                    description: "Retain this user rule".into(),
                    target: "accept".into(),
                    expressions: vec![protocol::Expressions {
                        statement: Some(protocol::Statement {
                            name: "tcp".into(),
                            op: "==".into(),
                            values: vec![protocol::StatementValues {
                                key: "dport".into(),
                                value: "443".into(),
                            }],
                        }),
                    }],
                    ..Default::default()
                }],
            }],
        }],
    };
    for hook in ["output", "forward"] {
        initial.system_rules[0].chains.push(protocol::FwChain {
            name: hook.into(),
            table: "opensnitch".into(),
            family: "inet".into(),
            r#type: "mangle".into(),
            hook: hook.into(),
            policy: "drop".into(),
            ..Default::default()
        });
    }
    initial.system_rules[0].chains.push(protocol::FwChain {
        name: "regular".into(),
        table: "custom".into(),
        family: "inet".into(),
        policy: "drop".into(),
        ..Default::default()
    });
    let (daemon, replies, mut commands) = runtime.block_on(async {
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
                name: "policy-fixture".into(),
                version: "1.9.0-synthetic".into(),
                config: r#"{"Firewall":"nftables"}"#.into(),
                system_firewall: Some(initial.clone()),
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
    runtime.block_on(backend.handle.history.flush()).unwrap();
    let mut cx = context();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(super::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-4", cx))
        .unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    let open_editor = |cx: &mut HeadlessAppContext| {
        let before = cx.update(|cx| cx.windows());
        click(cx, window, "nodes-firewall");
        settle(cx);
        cx.update(|cx| cx.windows())
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    };
    let editor = open_editor(&mut cx);
    cx.update_window(editor, |_, window, cx| window.press("alt-p", cx))
        .unwrap();
    settle(&mut cx);
    let select = |cx: &mut HeadlessAppContext, index: u64| {
        cx.update_window(editor, |_, window, cx| {
            window.click(("fw-policy-chain", index), cx)
        })
        .unwrap();
        settle(cx);
    };
    let copy = |cx: &mut HeadlessAppContext| {
        click(cx, editor, "firewall-copy");
        Document::parse(&cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())).unwrap()
    };
    capture(&mut cx, editor, &evidence.join("01-policy-controls.png"));
    click(&mut cx, editor, "fw-policy-accept");
    settle(&mut cx);
    let doc = copy(&mut cx);
    assert_eq!(doc.chain(0, 0).unwrap().policy, "accept");
    assert_eq!(doc.chain(0, 0).unwrap().rules.len(), 1);
    click(&mut cx, editor, "fw-policy-drop");
    settle(&mut cx);
    let input = copy(&mut cx);
    assert_eq!(input.chain(0, 0).unwrap().rules.len(), 3);
    assert_eq!(
        input.chain(0, 0).unwrap().rules[0],
        Document::from(initial.clone()).chain(0, 0).unwrap().rules[0]
    );
    click(&mut cx, editor, "fw-policy-drop");
    settle(&mut cx);
    assert_eq!(copy(&mut cx), input);
    capture(&mut cx, editor, &evidence.join("02-input-preset.png"));
    select(&mut cx, 1);
    click(&mut cx, editor, "fw-policy-drop");
    settle(&mut cx);
    assert_eq!(copy(&mut cx).chain(0, 1).unwrap().rules.len(), 1);
    click(&mut cx, editor, "fw-policy-accept");
    settle(&mut cx);
    assert!(copy(&mut cx).chain(0, 1).unwrap().rules.is_empty());
    click(&mut cx, editor, "fw-policy-drop");
    settle(&mut cx);
    select(&mut cx, 2);
    click(&mut cx, editor, "fw-policy-accept");
    settle(&mut cx);
    assert_eq!(copy(&mut cx).chain(0, 2).unwrap().policy, "accept");
    click(&mut cx, editor, "fw-policy-drop");
    settle(&mut cx);
    let before = copy(&mut cx);
    select(&mut cx, 3);
    click(&mut cx, editor, "fw-policy-drop");
    settle(&mut cx);
    assert_eq!(copy(&mut cx), before);
    capture(
        &mut cx,
        editor,
        &evidence.join("03-unsupported-preset-retained.png"),
    );
    select(&mut cx, 0);
    click(&mut cx, editor, "fw-system-enabled");
    settle(&mut cx);
    let disabled = copy(&mut cx);
    assert!(!disabled.enabled);
    assert!(
        disabled.system_rules[0].chains[..3]
            .iter()
            .all(|c| c.policy == "accept")
    );
    assert_eq!(disabled.chain(0, 3).unwrap().policy, "drop");
    assert_eq!(
        firewall::read(&config.database, "policy-fixture")
            .unwrap()
            .config,
        initial
    );
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(25), commands.message()).await
            })
            .is_err()
    );
    capture(&mut cx, editor, &evidence.join("04-staged-disable.png"));
    click(&mut cx, editor, "fw-policy-review");
    settle(&mut cx);
    capture(&mut cx, editor, &evidence.join("05-two-stage-review.png"));
    click(&mut cx, editor, "firewall-back");
    settle(&mut cx);
    assert_eq!(copy(&mut cx), disabled);
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    settle(&mut cx);
    click(&mut cx, editor, "firewall-confirm");
    settle(&mut cx);
    let first = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    let mut preparation = initial.clone();
    for chain in &mut preparation.system_rules[0].chains[..3] {
        chain.policy = "accept".into();
    }
    assert_eq!(first.r#type, 5);
    assert_eq!(first.sys_firewall, Some(preparation.clone()));
    assert!(first.data.is_empty() && first.rules.is_empty());
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(25), commands.message()).await
            })
            .is_err()
    );
    capture(
        &mut cx,
        editor,
        &evidence.join("06-preparation-awaits-ack.png"),
    );
    let mut asking = daemon.clone();
    let ask = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/policy-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let popup = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("07-permission-under-preparation.png"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(ask).unwrap().duration, "once");
    // Continue the next stage even if its originating editor is already gone.
    click(&mut cx, editor, "firewall-close");
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: first.id,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
    let second = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(second.sys_firewall, Some(disabled.clone().into()));
    assert_eq!(
        firewall::read(&config.database, "policy-fixture")
            .unwrap()
            .config,
        preparation
    );
    capture(&mut cx, window, &evidence.join("08-disable-awaits-ack.png"));
    // The editor was closed before the preparation ACK; stage 2 still arrived.
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: second.id,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "policy-fixture")
            .unwrap()
            .config,
        disabled.clone().into()
    );
    let editor = open_editor(&mut cx);
    cx.update_window(editor, |_, window, cx| {
        window.within("firewall-tabs").click(3usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    capture(
        &mut cx,
        editor,
        &evidence.join("09-closed-editor-operation-completed.png"),
    );
    cx.update_window(editor, |_, window, cx| window.press("ctrl-shift-e", cx))
        .unwrap();
    settle(&mut cx);
    let mut enabled = disabled.clone();
    enabled.enabled = true;
    click(&mut cx, editor, "fw-policy-review");
    settle(&mut cx);
    capture(&mut cx, editor, &evidence.join("10-enable-review.png"));
    click(&mut cx, editor, "firewall-confirm");
    settle(&mut cx);
    let enable = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(enable.sys_firewall, Some(enabled.clone().into()));
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: enable.id,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "policy-fixture")
            .unwrap()
            .config,
        enabled.clone().into()
    );
    // A single enabled policy reload can fail without discarding its local draft.
    click(&mut cx, editor, "fw-policy-drop");
    settle(&mut cx);
    click(&mut cx, editor, "fw-policy-review");
    settle(&mut cx);
    click(&mut cx, editor, "firewall-confirm");
    settle(&mut cx);
    let rejected = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(
        rejected.sys_firewall.as_ref().unwrap().system_rules[0].chains[0].policy,
        "drop"
    );
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: rejected.id,
            code: 1,
            data: "Synthetic policy rejection".into(),
        }))
        .unwrap();
    settle(&mut cx);
    assert_eq!(
        firewall::read(&config.database, "policy-fixture")
            .unwrap()
            .config,
        enabled.clone().into()
    );
    capture(
        &mut cx,
        editor,
        &evidence.join("11-rejected-policy-draft-retained.png"),
    );
    click(&mut cx, editor, "firewall-copy");
    let retained =
        Document::parse(&cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()))
            .unwrap();
    assert_eq!(retained.chain(0, 0).unwrap().policy, "drop");
    assert!(retained.enabled);
    click(&mut cx, editor, "firewall-close");
    drop(commands);
    drop(replies);
    drop(daemon);
    drop(view);
    drop(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":["01-policy-controls.png","02-input-preset.png","03-unsupported-preset-retained.png","04-staged-disable.png","05-two-stage-review.png","06-preparation-awaits-ack.png","07-permission-under-preparation.png","08-disable-awaits-ack.png","09-closed-editor-operation-completed.png","10-enable-review.png","11-rejected-policy-draft-retained.png"],"proof_boundary":"Actual Kit policy presets and enabled checkbox, staged changes, exact two-stage complete reloads with held ACK and independent permission, no early continuation, detached completion after editor close, reopening/enabling and rejected-draft preservation. Synthetic daemon only; native compositor and kernel enforcement remain separate."})).unwrap()).unwrap();
}

#[test]
fn headless_subscriptions() {
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
    let mut cx = context();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(super::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-7", cx))
        .unwrap();
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("01-empty-subscriptions.png"),
    );
    click(&mut cx, window, "subs-collections");
    settle(&mut cx);
    cx.read_entity(&view, |v, _| {
        let page = v.subscriptions.collection_page().unwrap();
        assert_eq!(page.total, 5);
        assert!(
            page.rows
                .iter()
                .all(|r| r.members == 0 && r.group.is_none())
        );
    });
    capture(
        &mut cx,
        window,
        &evidence.join("21-empty-rule-collections.png"),
    );
    click(&mut cx, window, "subs-collection-back");
    settle(&mut cx);
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
    let wait = |cx: &mut HeadlessAppContext, id, status: &str| {
        for _ in 0..12 {
            settle(cx);
            if row(id).status == status {
                return;
            }
        }
        panic!("Subscription {id} should be {status}, got {:?}", row(id));
    };
    let select = |cx: &mut HeadlessAppContext, id: u64| {
        cx.update_window(window, |_, window, cx| {
            window.click(("subscription-row", id), cx)
        })
        .unwrap();
        settle(cx);
    };
    assert_eq!(query(Query::default()).total, 0);
    click(&mut cx, window, "subs-defaults");
    settle(&mut cx);
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
    settle(&mut cx);
    click(&mut cx, window, "subs-add");
    settle(&mut cx);
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("02-invalid-add-preserved.png"),
    );
    fill(&mut cx, window, "subs-name", "Alpha hosts");
    fill(&mut cx, window, "subs-url", &format!("{url}/list"));
    fill(&mut cx, window, "subs-filename", "alpha.txt");
    fill(&mut cx, window, "subs-groups", "Test, local");
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    assert_eq!(query(Query::default()).total, 1);
    click(&mut cx, window, "subs-add");
    settle(&mut cx);
    fill(&mut cx, window, "subs-name", "Cancelled draft");
    click(&mut cx, window, "subs-cancel");
    settle(&mut cx);
    assert_eq!(query(Query::default()).total, 1);
    cx.update_window(window, |_, window, cx| window.press("ctrl-n", cx))
        .unwrap();
    settle(&mut cx);
    fill(&mut cx, window, "subs-name", "Beta source");
    fill(&mut cx, window, "subs-url", &format!("{url}/bad"));
    fill(&mut cx, window, "subs-filename", "beta.txt");
    cx.update_window(window, |_, window, cx| window.press("ctrl-s", cx))
        .unwrap();
    settle(&mut cx);
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
    capture(&mut cx, window, &evidence.join("03-downloaded-list.png"));
    click(&mut cx, window, "subs-preview");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("04-downloaded-text-preview.png"),
    );
    click(&mut cx, window, "subs-preview-copy");
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .as_bytes(),
        data
    );
    click(&mut cx, window, "subs-preview-back");
    settle(&mut cx);
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
    capture(
        &mut cx,
        window,
        &evidence.join("05-conditional-and-forced-refresh.png"),
    );
    select(&mut cx, 2);
    click(&mut cx, window, "subs-refresh");
    wait(&mut cx, 2, "error");
    assert!(row(2).file.is_none());
    assert!(row(2).error.contains("HTML"));
    capture(&mut cx, window, &evidence.join("06-download-error.png"));
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    fill(&mut cx, window, "subs-url", &format!("{url}/slow"));
    fill(&mut cx, window, "subs-timeout", "30");
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    click(&mut cx, window, "subs-refresh");
    settle(&mut cx);
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
    settle(&mut cx);
    let popup = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("07-permission-during-slow-http.png"),
    );
    assert_eq!(backend.handle.subscriptions.state().active, Some(2));
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(ask).unwrap().duration, "once");
    click(&mut cx, window, "subs-stop");
    wait(&mut cx, 2, "cancelled");
    release.store(true, Ordering::Relaxed);
    assert!(row(2).file.is_none());
    capture(&mut cx, window, &evidence.join("08-cancelled-download.png"));
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    fill(&mut cx, window, "subs-url", &format!("{url}/ips"));
    pick(&mut cx, window, "subs-format", "IP addresses and CIDRs");
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("22-format-membership-review.png"),
    );
    assert_eq!(row(2).spec.format, Format::Hosts);
    click(&mut cx, window, "subs-membership-confirm");
    settle(&mut cx);

    assert_eq!(row(2).spec.format, Format::Ips);
    click(&mut cx, window, "subs-refresh");
    wait(&mut cx, 2, "updated");
    assert_eq!(
        std::fs::read_to_string(row(2).file.unwrap()).unwrap(),
        "192.0.2.1\n2001:db8::/32\n"
    );
    capture(&mut cx, window, &evidence.join("09-edited-list-format.png"));
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-sort", 3usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.page.as_ref().unwrap().rows[0]
            .id),
        2
    );
    click(&mut cx, window, "subs-descending");
    settle(&mut cx);
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
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("10-filtered-empty-state.png"),
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
    settle(&mut cx);
    click(&mut cx, window, "subs-remove-cancel");
    settle(&mut cx);
    assert!(!row(2).removed);
    let retained = row(2).file.unwrap();
    click(&mut cx, window, "subs-remove");
    settle(&mut cx);
    click(&mut cx, window, "subs-remove-confirm");
    settle(&mut cx);
    assert!(row(2).removed && std::path::Path::new(&retained).is_file());
    click(&mut cx, window, "subs-removed");
    settle(&mut cx);
    select(&mut cx, 2);
    capture(
        &mut cx,
        window,
        &evidence.join("11-removed-data-retained.png"),
    );
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("23-restored-membership-review.png"),
    );
    assert!(row(2).removed);
    click(&mut cx, window, "subs-membership-confirm");
    settle(&mut cx);
    click(&mut cx, window, "subs-removed");
    settle(&mut cx);
    select(&mut cx, 2);
    assert!(!row(2).removed);
    click(&mut cx, window, "subs-logs");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("12-restored-status-history.png"),
    );
    click(&mut cx, window, "subs-preview-back");
    settle(&mut cx);
    fill(&mut cx, window, "subs-search", "Alpha");
    select(&mut cx, 1);
    click(&mut cx, window, "subs-rule-create");
    settle(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    capture(
        &mut cx,
        editor,
        &evidence.join("15-compatible-list-rule-review.png"),
    );
    click(&mut cx, editor, "editor-cancel");
    settle(&mut cx);
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), rule_commands.message()).await
            })
            .is_err(),
        "Cancelling a review must not send a command"
    );
    click(&mut cx, window, "subs-rule-create");
    settle(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
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
    capture(
        &mut cx,
        editor,
        &evidence.join("16-list-rule-waits-for-confirmation.png"),
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
    settle(&mut cx);
    let popup = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("17-permission-under-list-rule.png"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(prompt).unwrap().duration, "once");
    runtime
        .block_on(rule_replies.send(protocol::NotificationReply {
            id: command.id,
            ..Default::default()
        }))
        .unwrap();
    settle(&mut cx);
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
    settle(&mut cx);
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
    settle(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    capture(&mut cx, editor, &evidence.join("24-group-rule-review.png"));
    click(&mut cx, editor, "editor-cancel");
    settle(&mut cx);
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
    settle(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
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
    settle(&mut cx);
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
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.associated_counts()),
        Some((1, 0, 1))
    );
    capture(
        &mut cx,
        window,
        &evidence.join("25-group-associated-rules.png"),
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
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.rules.editor_window.is_none()));
    capture(
        &mut cx,
        window,
        &evidence.join("31-associated-snapshot-changed.png"),
    );
    click(&mut cx, window, "subs-associated-refresh");
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-associated-edit", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, editor, "editor-cancel");
    settle(&mut cx);
    click(&mut cx, window, "subs-associated-back");
    settle(&mut cx);
    click(&mut cx, window, "subs-collection-back");
    settle(&mut cx);
    click(&mut cx, window, "subs-rule-inspect");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("18-associated-list-rules.png"),
    );
    click(&mut cx, window, "subs-associated-next");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("19-associated-rules-second-page.png"),
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-associated-edit", 1usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, editor, "editor-enabled");
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
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
    settle(&mut cx);
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
    capture(
        &mut cx,
        editor,
        &evidence.join("20-associated-rule-rejection.png"),
    );
    click(&mut cx, editor, "editor-cancel");
    settle(&mut cx);
    click(&mut cx, window, "subs-associated-back");
    settle(&mut cx);
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    fill(&mut cx, window, "subs-groups", "Other");
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("26-group-membership-review.png"),
    );
    assert_eq!(row(1).spec.groups, ["Test", "local"]);
    click(&mut cx, window, "subs-membership-back");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("32-membership-back-preserves-draft.png"),
    );
    click(&mut cx, window, "subs-cancel");
    settle(&mut cx);
    assert_eq!(row(1).spec.groups, ["Test", "local"]);
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    fill(&mut cx, window, "subs-groups", "Other");
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    click(&mut cx, window, "subs-membership-confirm");
    settle(&mut cx);
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
    settle(&mut cx);
    fill(&mut cx, window, "subs-collection-search", "test");
    assert_eq!(
        cx.read_entity(&view, |v, _| v
            .subscriptions
            .collection_page()
            .unwrap()
            .total),
        0
    );
    capture(
        &mut cx,
        window,
        &evidence.join("27-group-removed-from-collections.png"),
    );
    click(&mut cx, window, "subs-collection-back");
    settle(&mut cx);
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
    settle(&mut cx);
    cx.read_entity(&view, |v, _| {
        assert_eq!(v.subscriptions.collection_page().unwrap().total, 86)
    });
    click(&mut cx, window, "subs-collection-next");
    settle(&mut cx);
    cx.read_entity(&view, |v, _| {
        let page = v.subscriptions.collection_page().unwrap();
        assert_eq!(page.offset, 64);
        assert_eq!(page.rows[0].group.as_deref(), Some("pager-058"));
    });
    capture(
        &mut cx,
        window,
        &evidence.join("28-collection-second-page.png"),
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-collection-sort", 2usize), cx)
    })
    .unwrap();
    settle(&mut cx);
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
    settle(&mut cx);
    cx.read_entity(&view, |v, _| {
        let page = v.subscriptions.collection_page().unwrap();
        assert_eq!(page.offset, 0);
        assert_eq!(page.rows[0].members, 11);
        assert!(page.rows[0].group.is_none());
    });
    capture(
        &mut cx,
        window,
        &evidence.join("29-collection-sorted-members.png"),
    );
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-collection-sort", 1usize), cx)
    })
    .unwrap();
    settle(&mut cx);
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
    settle(&mut cx);
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
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| {
        v.subscriptions.collection_page().unwrap().rows[0]
            .group
            .is_none()
    }));
    click(&mut cx, window, "subs-collection-descending");
    settle(&mut cx);
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
    settle(&mut cx);
    cx.read_entity(&view, |v, _| {
        let page = v.subscriptions.collection_page().unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].group.as_deref(), Some("pager-079"));
    });
    capture(
        &mut cx,
        window,
        &evidence.join("30-collection-filter-full-catalogue.png"),
    );
    click(&mut cx, window, "subs-collection-back");
    settle(&mut cx);
    fill(&mut cx, window, "subs-search", "");
    settle(&mut cx);
    let current_page = |cx: &HeadlessAppContext| {
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
    settle(&mut cx);
    assert_eq!(current_page(&cx).0, 64);
    assert_eq!(current_page(&cx).2.len(), 1);
    capture(
        &mut cx,
        window,
        &evidence.join("13-subscription-second-page.png"),
    );
    click(&mut cx, window, "subs-previous");
    settle(&mut cx);
    assert_eq!(current_page(&cx).0, 0);
    click(&mut cx, window, "subs-next");
    settle(&mut cx);
    let last = current_page(&cx).2[0];
    let mark = |cx: &mut HeadlessAppContext, id: u64| {
        cx.update_window(window, |_, window, cx| window.click(("subs-mark", id), cx))
            .unwrap();
        settle(cx);
    };
    mark(&mut cx, last);
    click(&mut cx, window, "subs-previous");
    settle(&mut cx);
    mark(&mut cx, 1);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().0),
        2
    );
    capture(&mut cx, window, &evidence.join("33-marks-across-pages.png"));
    click(&mut cx, window, "subs-bulk");
    settle(&mut cx);
    pick(&mut cx, window, "subs-bulk-groups-mode", "Add groups");
    fill(&mut cx, window, "subs-bulk-groups", "Bulk Review");
    click(&mut cx, window, "subs-bulk-review");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1),
        Some((2, 2))
    );
    capture(
        &mut cx,
        window,
        &evidence.join("34-reviewed-group-batch.png"),
    );
    click(&mut cx, window, "subs-bulk-back");
    settle(&mut cx);
    click(&mut cx, window, "subs-bulk-review");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1),
        Some((2, 2)),
        "Back preserves the staged group operation"
    );
    click(&mut cx, window, "subs-bulk-cancel");
    settle(&mut cx);
    assert_eq!(row(1).spec.groups, ["Other"]);
    assert!(row(last).spec.groups.is_empty());
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-b", cx))
        .unwrap();
    settle(&mut cx);
    pick(&mut cx, window, "subs-bulk-groups-mode", "Add groups");
    fill(&mut cx, window, "subs-bulk-groups", "Bulk Review");
    click(&mut cx, window, "subs-bulk-review");
    settle(&mut cx);
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
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| {
        v.subscriptions
            .bulk_state()
            .2
            .unwrap()
            .contains("changed since review")
    }));
    assert_eq!(row(1).spec, concurrent);
    assert!(row(last).spec.groups.is_empty());
    capture(&mut cx, window, &evidence.join("35-stale-bulk-refused.png"));
    click(&mut cx, window, "subs-bulk-rebuild");
    settle(&mut cx);
    click(&mut cx, window, "subs-bulk-apply");
    settle(&mut cx);
    assert_eq!(row(1).spec.name, "Alpha concurrent");
    assert_eq!(row(1).spec.groups, ["other", "bulk-review"]);
    assert_eq!(row(last).spec.groups, ["bulk-review"]);
    assert!(row(2).spec.groups.is_empty());
    capture(
        &mut cx,
        window,
        &evidence.join("36-selected-group-batch-applied.png"),
    );
    click(&mut cx, window, "subs-bulk");
    settle(&mut cx);
    click(&mut cx, window, "subs-bulk-matching");
    settle(&mut cx);
    pick(&mut cx, window, "subs-bulk-groups-mode", "Add groups");
    fill(&mut cx, window, "subs-bulk-groups", "exceeds-group-limit");
    click(&mut cx, window, "subs-bulk-review");
    settle(&mut cx);
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
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1),
        Some((65, 65))
    );
    click(&mut cx, window, "subs-bulk-next");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("37-all-matching-review-page-two.png"),
    );
    let retained = row(1).file.unwrap();
    let retained_bytes = std::fs::read(&retained).unwrap();
    click(&mut cx, window, "subs-bulk-apply");
    settle(&mut cx);
    assert!(query(Query::default()).rows.iter().all(|r| !r.spec.enabled));
    assert!(!row(last).spec.enabled);
    assert_eq!(std::fs::read(&retained).unwrap(), retained_bytes);
    capture(&mut cx, window, &evidence.join("38-all-pages-disabled.png"));
    click(&mut cx, window, "subs-bulk");
    settle(&mut cx);
    pick(&mut cx, window, "subs-bulk-enabled", "Enable");
    for index in 0..3usize {
        cx.update_window(window, |_, window, cx| {
            window.click(("subs-bulk-override", index), cx)
        })
        .unwrap();
        settle(&mut cx);
    }
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-bulk-limit", 1usize), cx);
        window.input("7", cx);
    })
    .unwrap();
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("subs-bulk-limit", 2usize), cx);
        window.input("2", cx);
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, window, "subs-bulk-review");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().1),
        Some((2, 2))
    );
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-enter", cx))
        .unwrap();
    settle(&mut cx);
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
                Some(crate::subscriptions::limits::Override::Absolute(7)),
                Some(crate::subscriptions::limits::Override::Absolute(2048))
            )
        );
    }
    assert!(!row(2).spec.enabled);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.bulk_state().0),
        2
    );
    capture(
        &mut cx,
        window,
        &evidence.join("39-selected-inherited-and-custom-limits.png"),
    );
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(30), rule_commands.message()).await
            })
            .is_err(),
        "Bulk subscription edits send no implicit policy commands"
    );
    click(&mut cx, window, "subs-next");
    settle(&mut cx);
    assert_eq!(current_page(&cx).2, [last]);
    select(&mut cx, last);
    click(&mut cx, window, "subs-remove");
    settle(&mut cx);
    click(&mut cx, window, "subs-remove-confirm");
    settle(&mut cx);
    assert_eq!(current_page(&cx).1, 64);
    assert_eq!(
        current_page(&cx).0,
        0,
        "Removing the final row of a later page returns to the last existing page"
    );
    assert_eq!(current_page(&cx).2.len(), 64);
    capture(
        &mut cx,
        window,
        &evidence.join("14-last-page-removal-clamped.png"),
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
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-t", cx))
        .unwrap();
    settle(&mut cx);
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        invalid_file.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-review");
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().1.is_some()));
    assert_eq!(query(Query::default()).total, 64);
    capture(
        &mut cx,
        window,
        &evidence.join("40-invalid-config-import.png"),
    );
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        import_file.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-review");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0),
        Some((20, 19, 1))
    );
    capture(
        &mut cx,
        window,
        &evidence.join("41-import-name-conflict.png"),
    );
    click(&mut cx, window, "subs-transfer-back");
    settle(&mut cx);
    click(&mut cx, window, "subs-transfer-replace");
    click(&mut cx, window, "subs-transfer-defaults");
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-enter", cx);
    })
    .unwrap();
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0),
        Some((20, 20, 0))
    );
    click(&mut cx, window, "subs-transfer-next");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("42-import-review-second-page.png"),
    );
    click(&mut cx, window, "subs-transfer-back");
    settle(&mut cx);
    click(&mut cx, window, "subs-transfer-review");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0),
        Some((20, 20, 0)),
        "Back preserves file, replacement and default options"
    );
    click(&mut cx, window, "subs-transfer-cancel");
    settle(&mut cx);
    assert_eq!(row(1).spec, before.spec);
    assert_eq!(query(Query::default()).total, 64);
    capture(
        &mut cx,
        window,
        &evidence.join("48-cancel-import-restores-catalogue.png"),
    );
    assert!(
        !cx.read_entity(&view, |v, _| v.subscriptions.has_transfer()),
        "Cancel must return to the actual catalogue"
    );
    click(&mut cx, window, "subs-transfer");
    settle(&mut cx);
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        import_file.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-replace");
    click(&mut cx, window, "subs-transfer-defaults");
    click(&mut cx, window, "subs-transfer-review");
    settle(&mut cx);
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
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| {
        v.subscriptions
            .transfer_state()
            .1
            .unwrap()
            .contains("changed since review")
    }));
    assert_eq!(row(1).spec, before.spec);
    assert_eq!(query(Query::default()).total, 64);
    capture(
        &mut cx,
        window,
        &evidence.join("43-stale-import-refused.png"),
    );
    click(&mut cx, window, "subs-transfer-rebuild");
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-enter", cx))
        .unwrap();
    settle(&mut cx);
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
    capture(
        &mut cx,
        window,
        &evidence.join("44-imported-configuration.png"),
    );
    mark(&mut cx, 1);
    click(&mut cx, window, "subs-transfer");
    settle(&mut cx);
    let native_export = evidence.join("selected-native-config.json");
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        native_export.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-export");
    settle(&mut cx);
    let native = crate::subscriptions::transfer::read(&native_export, &imported_settings).unwrap();
    assert_eq!(native.subscriptions, [row(1).spec]);
    assert_eq!(
        native.opensnitch["actions"]["list_subscriptions"]["config"]["subscriptions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    capture(
        &mut cx,
        window,
        &evidence.join("45-selected-native-export.png"),
    );
    let exported_bytes = std::fs::read(&native_export).unwrap();
    click(&mut cx, window, "subs-transfer-export");
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().1.is_some()));
    assert_eq!(std::fs::read(&native_export).unwrap(), exported_bytes);
    capture(
        &mut cx,
        window,
        &evidence.join("46-export-preserves-existing-file.png"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    settle(&mut cx);
    fill(&mut cx, window, "subs-search", "imported");
    settle(&mut cx);
    assert_eq!(current_page(&cx).1, 20);
    click(&mut cx, window, "subs-transfer");
    settle(&mut cx);
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
    settle(&mut cx);
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
    capture(
        &mut cx,
        window,
        &evidence.join("47-filtered-opensnitch-export.png"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    settle(&mut cx);
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
    settle(&mut cx);
    select(&mut cx, 1);
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    fill(&mut cx, window, "subs-name", "Renamed imported");
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    mark(&mut cx, 1);
    click(&mut cx, window, "subs-transfer");
    settle(&mut cx);
    let renamed_export = evidence.join("renamed-source-native.json");
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        renamed_export.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-export");
    settle(&mut cx);
    let renamed =
        crate::subscriptions::transfer::read(&renamed_export, &imported_settings).unwrap();
    assert_eq!(renamed.subscriptions[0].name, "Renamed imported");
    assert_eq!(
        crate::subscriptions::transfer::metadata_records(&renamed).unwrap()[0]
            .as_ref()
            .unwrap()["future_source"],
        0
    );
    capture(
        &mut cx,
        window,
        &evidence.join("49-renamed-identity-keeps-extra-fields.png"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    settle(&mut cx);
    click(&mut cx, window, "subs-add");
    settle(&mut cx);
    fill(&mut cx, window, "subs-name", "Renamed imported");
    fill(
        &mut cx,
        window,
        "subs-url",
        &format!("{url}/another-source"),
    );
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    fill(&mut cx, window, "subs-search", "Renamed imported");
    settle(&mut cx);
    assert_eq!(current_page(&cx).1, 2);
    capture(
        &mut cx,
        window,
        &evidence.join("50-duplicate-display-names.png"),
    );
    click(&mut cx, window, "subs-transfer");
    settle(&mut cx);
    let duplicates_export = evidence.join("duplicate-name-native.json");
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        duplicates_export.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-export");
    settle(&mut cx);
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
    capture(
        &mut cx,
        window,
        &evidence.join("51-duplicate-native-export-bindings.png"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    settle(&mut cx);
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
    settle(&mut cx);
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        replacement.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-catalogue");
    click(&mut cx, window, "subs-transfer-review");
    settle(&mut cx);
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_state().0),
        Some((2, 86, 0))
    );
    assert_eq!(
        cx.read_entity(&view, |v, _| v.subscriptions.transfer_removals()),
        Some(84)
    );
    capture(
        &mut cx,
        window,
        &evidence.join("52-complete-catalogue-removal-review.png"),
    );
    click(&mut cx, window, "subs-transfer-next");
    settle(&mut cx);
    capture(
        &mut cx,
        window,
        &evidence.join("53-retained-removals-second-page.png"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    settle(&mut cx);
    assert_eq!(query(Query::default()).total, before_replacement);
    assert!(!row(1).removed);
    assert_eq!(std::fs::read(&retained).unwrap(), retained_bytes);
    capture(
        &mut cx,
        window,
        &evidence.join("54-cancel-complete-replacement.png"),
    );
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-t", cx))
        .unwrap();
    settle(&mut cx);
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        replacement.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-catalogue");
    click(&mut cx, window, "subs-transfer-review");
    settle(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("ctrl-shift-enter", cx))
        .unwrap();
    settle(&mut cx);
    fill(&mut cx, window, "subs-search", "");
    settle(&mut cx);
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
    capture(
        &mut cx,
        window,
        &evidence.join("55-complete-catalogue-applied.png"),
    );
    click(&mut cx, window, "subs-removed");
    settle(&mut cx);
    assert_eq!(current_page(&cx).1, 85);
    select(&mut cx, 1);
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    click(&mut cx, window, "subs-membership-confirm");
    settle(&mut cx);
    click(&mut cx, window, "subs-removed");
    settle(&mut cx);
    fill(&mut cx, window, "subs-search", "Renamed imported");
    settle(&mut cx);
    assert_eq!(current_page(&cx).1, 1);
    mark(&mut cx, 1);
    click(&mut cx, window, "subs-transfer");
    settle(&mut cx);
    let restored_export = evidence.join("restored-source-native.json");
    fill(
        &mut cx,
        window,
        "subs-transfer-path",
        restored_export.to_str().unwrap(),
    );
    click(&mut cx, window, "subs-transfer-export");
    settle(&mut cx);
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
    capture(
        &mut cx,
        window,
        &evidence.join("56-restored-id-retains-data-and-metadata.png"),
    );
    click(&mut cx, window, "subs-transfer-cancel");
    settle(&mut cx);
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
    drop(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":["01-empty-subscriptions.png","02-invalid-add-preserved.png","03-downloaded-list.png","04-downloaded-text-preview.png","05-conditional-and-forced-refresh.png","06-download-error.png","07-permission-during-slow-http.png","08-cancelled-download.png","09-edited-list-format.png","10-filtered-empty-state.png","11-removed-data-retained.png","12-restored-status-history.png","13-subscription-second-page.png","14-last-page-removal-clamped.png","15-compatible-list-rule-review.png","16-list-rule-waits-for-confirmation.png","17-permission-under-list-rule.png","18-associated-list-rules.png","19-associated-rules-second-page.png","20-associated-rule-rejection.png","21-empty-rule-collections.png","22-format-membership-review.png","23-restored-membership-review.png","24-group-rule-review.png","25-group-associated-rules.png","26-group-membership-review.png","27-group-removed-from-collections.png","28-collection-second-page.png","29-collection-sorted-members.png","30-collection-filter-full-catalogue.png","31-associated-snapshot-changed.png","32-membership-back-preserves-draft.png","33-marks-across-pages.png","34-reviewed-group-batch.png","35-stale-bulk-refused.png","36-selected-group-batch-applied.png","37-all-matching-review-page-two.png","38-all-pages-disabled.png","39-selected-inherited-and-custom-limits.png","40-invalid-config-import.png","41-import-name-conflict.png","42-import-review-second-page.png","43-stale-import-refused.png","44-imported-configuration.png","45-selected-native-export.png","46-export-preserves-existing-file.png","47-filtered-opensnitch-export.png","48-cancel-import-restores-catalogue.png","49-renamed-identity-keeps-extra-fields.png","50-duplicate-display-names.png","51-duplicate-native-export-bindings.png","52-complete-catalogue-removal-review.png","53-retained-removals-second-page.png","54-cancel-complete-replacement.png","55-complete-catalogue-applied.png","56-restored-id-retains-data-and-metadata.png"],"proof_boundary":"Owned network namespace and HTTP fixture; actual native Kit empty/add/edit/default/format controls, conditional/forced refresh, error/cancel, independent AskRule during held HTTP, full-catalogue sort/search/paging/last-page-removal/preview/log/copy/remove/restore with durable files and metadata. Local list-rule review/cancel, exact held-ACK creation, confirmed-only associated-rule paging beyond a full catalogue page, nested-condition edit/rejection and independent permission input are included. Collection empty/search/paging/global sorts, exact UID-scoped group rule creation/inspection, stale snapshot refusal/refresh and reviewed membership changes are included. Bulk marking across pages, selected/group and whole-matching-set edits, immutable review/back/cancel, concurrent-edit refusal/rebuild, group-cap rejection, explicit inheritance/custom limits and no implicit policy commands are included. Actual import/export input adds invalid-file refusal, exact-name conflict/replacement, defaults, paged review/back/cancel, stale catalogue refusal/rebuild, canonical migration and retained data, selected native and full-filter OpenSnitch exports, existing-file preservation, unknown metadata and no paused downloads or policy commands. Per-ID opaque fields survive ordinary rename, duplicate display names, selected/native export, complete catalogue review/paging/cancel/apply and restoration of retained data. A real 84-source removal preserves old files and sends no policy commands. Notifications, retired-data cleanup, native compositor and resource proof remain separate."})).unwrap()).unwrap();
}

#[test]
fn headless_port_ranges() {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let hold = Arc::new(AtomicBool::new(false));
    let accept = Arc::new(AtomicBool::new(true));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let original = protocol::Rule {
        name: "owned-port-rule".into(),
        enabled: true,
        action: "deny".into(),
        duration: "always".into(),
        operator: Some(protocol::Operator {
            r#type: "list".into(),
            operand: "list".into(),
            list: vec![
                protocol::Operator {
                    r#type: "simple".into(),
                    operand: "source.port".into(),
                    data: "4000".into(),
                    ..Default::default()
                },
                protocol::Operator {
                    r#type: "simple".into(),
                    operand: "dest.port".into(),
                    data: "8000".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    let (mut daemon, notifications) = runtime.block_on(async {
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
                name: "port-range-fixture".into(),
                rules: vec![original.clone()],
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
        let hold = hold.clone();
        let accept = accept.clone();
        let observed = observed.clone();
        let task = tokio::spawn(async move {
            while let Ok(Some(command)) = stream.message().await {
                assert!(observed.lock().unwrap().len() < 8);
                observed.lock().unwrap().push(command.clone());
                while hold.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let ok = accept.load(Ordering::Acquire);
                if tx
                    .send(protocol::NotificationReply {
                        id: command.id,
                        code: if ok {
                            protocol::NotificationReplyCode::Ok
                        } else {
                            protocol::NotificationReplyCode::Error
                        } as i32,
                        data: if ok { "" } else { "owned range rejection" }.into(),
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
    let mut cx = context();
    let window = cx.update(|cx| {
        cx.open_window(
            gpui::WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::centered(
                    None,
                    size(px(1152.), px(850.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
            },
        )
        .unwrap()
    });
    let view = cx.update(|cx| window.entity(cx).unwrap());
    settle(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.within("main-tabs").click(1usize, cx)
    })
    .unwrap();
    settle(&mut cx);
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-edit");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    fn select(cx: &mut HeadlessAppContext, editor: AnyWindowHandle, id: u64, value: &str) {
        cx.update_window(editor, |_, window, cx| {
            window.click(("editor-match", id), cx)
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(editor, |_, window, cx| window.input(value, cx))
            .unwrap();
        settle(cx);
        cx.update_window(editor, |_, window, cx| window.press("enter", cx))
            .unwrap();
        settle(cx);
    }
    fn value(cx: &mut HeadlessAppContext, editor: AnyWindowHandle, id: u64, value: &str) {
        cx.update_window(editor, |_, window, cx| {
            window.scroll(
                "rule-editor-body",
                gpui::ScrollDelta::Pixels(gpui::point(
                    px(0.),
                    px(if id == 0 { 1000. } else { -400. }),
                )),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(editor, |_, window, cx| {
            window.click(("editor-value", id), cx);
            window.press("ctrl-a", cx);
            window.input(value, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    fn error(cx: &mut HeadlessAppContext, editor: AnyWindowHandle) -> String {
        cx.update_window(editor, |_, window, cx| {
            gpui_kit::base::Root::read(window, cx)
                .view()
                .clone()
                .downcast::<super::rule_editor::RuleEditor>()
                .unwrap()
                .read(cx)
                .test_error()
                .unwrap()
        })
        .unwrap()
    }
    select(&mut cx, editor, 0, "Port range");
    select(&mut cx, editor, 1, "Port range");
    value(&mut cx, editor, 0, "4000-4010");
    value(&mut cx, editor, 1, "8080-8000");
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
    assert!(error(&mut cx, editor).contains("min <= max"));
    assert!(observed.lock().unwrap().is_empty());
    capture(&mut cx, editor, &evidence.join("01-invalid-port-range.png"));
    value(&mut cx, editor, 1, " 8000 - 8080 ");
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
    assert!(error(&mut cx, editor).contains("not established"));
    assert!(observed.lock().unwrap().is_empty());
    capture(
        &mut cx,
        editor,
        &evidence.join("02-unknown-daemon-refused.png"),
    );
    runtime
        .block_on(daemon.ping(protocol::PingRequest {
            stats: Some(protocol::Statistics {
                daemon_version: "1.9.0 (owned range fixture)".into(),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .unwrap();
    cx.update_window(editor, |_, window, cx| {
        window.scroll(
            "rule-editor-body",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(1000.))),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    click(&mut cx, editor, "editor-json-toggle");
    capture(&mut cx, editor, &evidence.join("03-range-match-json.png"));
    click(&mut cx, editor, "editor-json-toggle");
    hold.store(true, Ordering::Release);
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
    assert_eq!(observed.lock().unwrap().len(), 1);
    assert_eq!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "port-range-fixture",
            &original.name
        )
        .unwrap(),
        Some(original.clone())
    );
    capture(&mut cx, editor, &evidence.join("04-range-awaiting-ack.png"));
    let mut asking = daemon.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/range-independent-prompt".into(),
                dst_port: 443,
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let prompt = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        prompt,
        &evidence.join("05-permission-during-range-save.png"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    hold.store(false, Ordering::Release);
    settle(&mut cx);
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.rules.editor_window.is_none()));
    let saved = crate::rules::transfer::find_confirmed_rule(
        &config.database,
        "port-range-fixture",
        &original.name,
    )
    .unwrap()
    .unwrap();
    let ops = &saved.operator.as_ref().unwrap().list;
    assert_eq!(ops[0].r#type, "range");
    assert_eq!(ops[0].data, "4000-4010");
    assert_eq!(ops[1].r#type, "range");
    assert_eq!(ops[1].data, " 8000 - 8080 ");
    assert_eq!(observed.lock().unwrap()[0].rules, vec![saved.clone()]);
    capture(
        &mut cx,
        window.into(),
        &evidence.join("06-range-confirmed.png"),
    );
    cx.update_window(window.into(), |_, window, cx| {
        window.click(("rule-row", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, window.into(), "rule-edit");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    value(&mut cx, editor, 1, "9000-9010");
    accept.store(false, Ordering::Release);
    click(&mut cx, editor, "editor-save");
    settle(&mut cx);
    assert!(error(&mut cx, editor).contains("owned range rejection"));
    assert_eq!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "port-range-fixture",
            &original.name
        )
        .unwrap(),
        Some(saved.clone())
    );
    capture(
        &mut cx,
        editor,
        &evidence.join("07-rejected-range-draft-retained.png"),
    );
    click(&mut cx, editor, "editor-cancel");
    settle(&mut cx);
    click(&mut cx, window.into(), "rules-export");
    let transfer = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    click(&mut cx, transfer, "transfer-copy");
    settle(&mut cx);
    let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(
        crate::rules::document::parse_document(&copied).unwrap(),
        vec![saved]
    );
    capture(
        &mut cx,
        transfer,
        &evidence.join("08-range-export-roundtrip.png"),
    );
    click(&mut cx, transfer, "transfer-close");
    assert_eq!(observed.lock().unwrap().len(), 2);
    drop(view);
    drop(cx);
    notifications.abort();
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":["01-invalid-port-range.png","02-unknown-daemon-refused.png","03-range-match-json.png","04-range-awaiting-ack.png","05-permission-during-range-save.png","06-range-confirmed.png","07-rejected-range-draft-retained.png","08-range-export-roundtrip.png"],"proof_boundary":"Actual rule-tab/editor Select/Input/JSON/save/cancel/clipboard input; exact source and destination range wire values; unknown capability refusal, held ACK with independent permission, durable success and rejected-draft preservation. Synthetic owned daemon; no enforcement or native compositor proof."})).unwrap()).unwrap();
}

#[test]
fn headless_cross_node_copies() {
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
    fn state(cx: &mut HeadlessAppContext, window: AnyWindowHandle) -> (bool, usize, Vec<Status>) {
        cx.update_window(window, |_, window, cx| {
            gpui_kit::base::Root::read(window, cx)
                .view()
                .clone()
                .downcast::<super::rule_distribution::Copies>()
                .unwrap()
                .read(cx)
                .test_state()
        })
        .unwrap()
    }
    fn finished(cx: &mut HeadlessAppContext, window: AnyWindowHandle) {
        for _ in 0..60 {
            settle(cx);
            if !state(cx, window).0 {
                return;
            }
        }
        panic!("Copy did not finish");
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
    let mut cx = context();
    let mut view = None;
    let main = cx
        .open_window(size(px(1250.), px(940.)), |window, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = view.unwrap();
    settle(&mut cx);
    cx.update_window(main, |_, window, cx| window.press("ctrl-2", cx))
        .unwrap();
    settle(&mut cx);
    click(&mut cx, main, "rules-select-page");
    click(&mut cx, main, "rules-next");
    settle(&mut cx);
    cx.update_window(main, |_, window, cx| {
        window.click(("rule-mark", 0usize), cx)
    })
    .unwrap();
    click(&mut cx, main, "rules-copy-nodes");
    let copy = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    capture(&mut cx, copy, &evidence.join("01-copy-destinations.png"));
    click(&mut cx, copy, "copy-select-all");
    click(&mut cx, copy, "copy-review-button");
    finished(&mut cx, copy);
    assert_eq!(state(&mut cx, copy).1, 201);
    capture(
        &mut cx,
        copy,
        &evidence.join("02-selected-cross-page-review.png"),
    );
    click(&mut cx, copy, "copy-rules-tab");
    cx.update_window(copy, |_, window, cx| {
        window.click(("copy-source-json", 0usize), cx)
    })
    .unwrap();
    settle(&mut cx);
    let json = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert_eq!(
        crate::rules::document::parse_document(&json).unwrap(),
        vec![rule(0)]
    );
    capture(&mut cx, copy, &evidence.join("03-exact-source-json.png"));
    click(&mut cx, copy, "copy-back");
    click(&mut cx, copy, "copy-all-rules");
    click(&mut cx, copy, "copy-review-button");
    finished(&mut cx, copy);
    assert_eq!(state(&mut cx, copy).1, 230);
    click(&mut cx, copy, "copy-destinations-tab");
    capture(&mut cx, copy, &evidence.join("04-full-filter-review.png"));
    assert!(observed.lock().unwrap().is_empty());
    click(&mut cx, copy, "copy-apply");
    settle(&mut cx);
    assert_eq!(observed.lock().unwrap()["beta"].len(), 1);
    assert!(
        transfer::find_confirmed_rule(&config.database, "beta", &rule(1).name)
            .unwrap()
            .is_none()
    );
    capture(&mut cx, copy, &evidence.join("05-copy-awaits-ack.png"));
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
    settle(&mut cx);
    let prompt = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        prompt,
        &evidence.join("06-permission-during-copy.png"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    click(&mut cx, copy, "copy-stop");
    hold.store(false, Ordering::Release);
    finished(&mut cx, copy);
    assert_eq!(state(&mut cx, copy).2[0], Status::Stopped);
    capture(&mut cx, copy, &evidence.join("07-copy-stopped.png"));
    click(&mut cx, copy, "copy-apply");
    finished(&mut cx, copy);
    let statuses = state(&mut cx, copy).2;
    assert_eq!(statuses[0], Status::Complete);
    assert!(matches!(statuses[1], Status::Failed(_)));
    assert_eq!(observed.lock().unwrap()["beta"].len(), 229);
    capture(
        &mut cx,
        copy,
        &evidence.join("08-partial-target-failure.png"),
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
    capture(
        &mut cx,
        copy,
        &evidence.join("09-uncertain-retry-refused.png"),
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
    capture(&mut cx, copy, &evidence.join("10-fresh-target-review.png"));
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
    capture(&mut cx, copy, &evidence.join("11-targets-completed.png"));
    click(&mut cx, copy, "copy-back");
    click(&mut cx, copy, "copy-replace");
    click(&mut cx, copy, "copy-review-button");
    finished(&mut cx, copy);
    capture(
        &mut cx,
        copy,
        &evidence.join("12-explicit-replacement-review.png"),
    );
    click(&mut cx, copy, "copy-apply");
    finished(&mut cx, copy);
    assert_eq!(
        transfer::find_confirmed_rule(&config.database, "beta", &rule(0).name).unwrap(),
        Some(rule(0))
    );
    capture(
        &mut cx,
        copy,
        &evidence.join("13-replacement-confirmed.png"),
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
    settle(&mut cx);
    assert_eq!(observed.lock().unwrap()["beta"].len(), 231);
    cx.update_window(copy, |_, window, _| window.remove_window())
        .unwrap();
    settle(&mut cx);
    hold.store(false, Ordering::Release);
    settle(&mut cx);
    settle(&mut cx);
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
    capture(
        &mut cx,
        main,
        &evidence.join("14-closed-copy-stops-unsent-targets.png"),
    );
    drop(view);
    drop(cx);
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
    .map(|(i, name)| format!("{:02}-{name}.png", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":frames,"proof_boundary":"Actual GPUI selection across pages, target checkboxes, source JSON, scope/replacement/rebuild/retry/stop/results input; exact synthetic gRPC ACK/error and SQLite persistence; independent permission and forced owned-window destruction while ACK held. Source inventories and synthetic reconnections enter at the daemon fixture boundary. No real daemon, native compositor or kernel enforcement proof."})).unwrap()).unwrap();
}

#[test]
fn headless_multi_node_drafts() {
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
    fn state(cx: &mut HeadlessAppContext, window: AnyWindowHandle) -> (bool, usize, Vec<Status>) {
        cx.update_window(window, |_, window, cx| {
            gpui_kit::base::Root::read(window, cx)
                .view()
                .clone()
                .downcast::<super::rule_distribution::Copies>()
                .unwrap()
                .read(cx)
                .test_state()
        })
        .unwrap()
    }
    fn finish(cx: &mut HeadlessAppContext, window: AnyWindowHandle) {
        for _ in 0..60 {
            settle(cx);
            if !state(cx, window).0 {
                return;
            }
        }
        panic!("Multi-node operation did not finish");
    }
    fn opened(cx: &mut HeadlessAppContext, before: &[AnyWindowHandle]) -> AnyWindowHandle {
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
    let mut cx = context();
    let mut view = None;
    let main = cx
        .open_window(size(px(1240.), px(940.)), |window, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = view.unwrap();
    settle(&mut cx);
    cx.update_window(main, |_, window, cx| window.press("ctrl-2", cx))
        .unwrap();
    settle(&mut cx);
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
    capture(
        &mut cx,
        editor,
        &evidence.join("01-unsaved-editor-draft.png"),
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
    capture(&mut cx, copy, &evidence.join("02-draft-node-selection.png"));
    click(&mut cx, copy, "copy-close");
    settle(&mut cx);
    assert!(ao.lock().unwrap().is_empty() && bo.lock().unwrap().is_empty());
    capture(
        &mut cx,
        editor,
        &evidence.join("03-cancel-preserves-editor-draft.png"),
    );
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, editor, "editor-nodes");
    let copy = opened(&mut cx, &before);
    click(&mut cx, copy, "copy-select-all");
    click(&mut cx, copy, "copy-review-button");
    finish(&mut cx, copy);
    assert_eq!(state(&mut cx, copy).1, 1);
    capture(
        &mut cx,
        copy,
        &evidence.join("04-two-node-rename-review.png"),
    );
    click(&mut cx, copy, "copy-apply");
    settle(&mut cx);
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
    capture(&mut cx, copy, &evidence.join("05-new-name-awaits-ack.png"));
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
    settle(&mut cx);
    let prompt = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        prompt,
        &evidence.join("06-permission-during-rename.png"),
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
    capture(
        &mut cx,
        copy,
        &evidence.join("07-stopped-before-old-name-removal.png"),
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
    capture(
        &mut cx,
        copy,
        &evidence.join("08-rename-completed-on-both-nodes.png"),
    );
    click(&mut cx, copy, "copy-close");
    settle(&mut cx);
    click(&mut cx, editor, "editor-cancel");
    settle(&mut cx);
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
    capture(
        &mut cx,
        import,
        &evidence.join("09-invalid-multi-node-import.png"),
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
    capture(&mut cx, import, &evidence.join("10-file-import-review.png"));
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
    capture(
        &mut cx,
        import,
        &evidence.join("11-captured-file-import-applied.png"),
    );
    click(&mut cx, import, "copy-back");
    cx.update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("not JSON".to_string())));
    click(&mut cx, import, "copy-import-clipboard");
    click(&mut cx, import, "copy-review-button");
    finish(&mut cx, import);
    assert_eq!(state(&mut cx, import).1, 0);
    assert_eq!(ao.lock().unwrap().len(), 4);
    capture(
        &mut cx,
        import,
        &evidence.join("12-invalid-clipboard-has-no-file-fallback.png"),
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
    capture(
        &mut cx,
        import,
        &evidence.join("13-captured-clipboard-review.png"),
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
    capture(
        &mut cx,
        import,
        &evidence.join("14-captured-clipboard-applied.png"),
    );
    click(&mut cx, import, "copy-close");
    drop(view);
    drop(cx);
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
    .map(|(i, name)| format!("{:02}-{name}.png", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":frames,"proof_boundary":"Actual editor draft/name/metadata and parent-preserving Cancel, destination selection, rename review, held ACK/permission/Stop/continue, durable per-node removal, typed file import and captured clipboard input. Owned fixture writes change file/clipboard after review to prove immutable source; two synthetic gRPC daemons only. No real daemon, native compositor or kernel enforcement proof."})).unwrap()).unwrap();
}

#[test]
fn headless_multi_node_export() {
    use crate::rules::{export_nodes, transfer::Import};
    type Client = protocol::ui_client::UiClient<tonic::transport::Channel>;
    fn rule(i: usize, action: &str) -> protocol::Rule {
        protocol::Rule {
            name: format!("export.{i:03}"),
            description: format!("Export rule {i}"),
            enabled: true,
            created: 123456,
            precedence: true,
            nolog: true,
            action: action.into(),
            duration: "always".into(),
            operator: Some(protocol::Operator {
                r#type: "simple".into(),
                operand: "process.path".into(),
                data: format!("/owned/export-{i}"),
                sensitive: true,
                ..Default::default()
            }),
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
                rules,
                ..Default::default()
            })
            .await
            .unwrap();
        client
    }
    fn state(cx: &mut HeadlessAppContext, w: AnyWindowHandle) -> (bool, usize) {
        cx.update_window(w, |_, window, cx| {
            gpui_kit::base::Root::read(window, cx)
                .view()
                .clone()
                .downcast::<super::rule_exports::Exports>()
                .unwrap()
                .read(cx)
                .test_state()
        })
        .unwrap()
    }
    fn finish(cx: &mut HeadlessAppContext, w: AnyWindowHandle) {
        for _ in 0..60 {
            settle(cx);
            if !state(cx, w).0 {
                return;
            }
        }
        panic!("Export did not finish");
    }
    fn folders(parent: &Path) -> Vec<PathBuf> {
        let mut p = std::fs::read_dir(parent)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_dir())
            .collect::<Vec<_>>();
        p.sort();
        p
    }
    fn verify(
        folder: &Path,
        alpha: Vec<protocol::Rule>,
        beta: Vec<protocol::Rule>,
        offline: Vec<protocol::Rule>,
    ) {
        let m: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.join("manifest.json")).unwrap()).unwrap();
        for n in m["nodes"].as_array().unwrap() {
            let mut got = Import::from_path(&folder.join(n["directory"].as_str().unwrap()))
                .map(|i| i.rules)
                .unwrap_or_default();
            got.sort_by(|a, b| a.name.cmp(&b.name));
            let expected = match n["name"].as_str().unwrap() {
                "alpha" => &alpha,
                "beta" => &beta,
                _ => &offline,
            };
            assert_eq!(&got, expected);
        }
    }
    let evidence = evidence();
    let output = evidence.join("exports");
    std::fs::create_dir(&output).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (mut alpha, beta) = runtime.block_on(async {
        let a = connect(
            config.socket.clone(),
            "alpha",
            (0..250).map(|i| rule(i, "allow")).collect(),
        )
        .await;
        let b = connect(config.socket.clone(), "beta", vec![rule(0, "deny")]).await;
        backend
            .handle
            .history
            .replace_rules(900, "offline".into(), vec![rule(205, "reject")]);
        backend.handle.history.flush().await.unwrap();
        (a, b)
    });
    assert_eq!(
        export_nodes::catalogue(&config.database).unwrap(),
        vec!["alpha", "beta", "offline"]
    );
    let mut cx = context();
    let mut view = None;
    let main = cx
        .open_window(size(px(1240.), px(940.)), |window, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = view.unwrap();
    settle(&mut cx);
    cx.update_window(main, |_, w, cx| w.press("ctrl-2", cx))
        .unwrap();
    settle(&mut cx);
    cx.update_window(main, |_, w, cx| {
        w.press("ctrl-f", cx);
        w.input("export-205", cx);
    })
    .unwrap();
    settle(&mut cx);
    click(&mut cx, main, "rules-export-nodes");
    let export = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    finish(&mut cx, export);
    fill(
        &mut cx,
        export,
        "nodes-export-path",
        output.to_str().unwrap(),
    );
    click(&mut cx, export, "nodes-export-all");
    capture(
        &mut cx,
        export,
        &evidence.join("01-cached-node-selection.png"),
    );
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    assert_eq!(state(&mut cx, export).1, 252);
    capture(
        &mut cx,
        export,
        &evidence.join("02-complete-node-review.png"),
    );
    click(&mut cx, export, "nodes-export-close");
    settle(&mut cx);
    assert!(folders(&output).is_empty());
    capture(
        &mut cx,
        main,
        &evidence.join("03-cancel-publishes-nothing.png"),
    );
    click(&mut cx, main, "rules-export-nodes");
    let export = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    finish(&mut cx, export);
    fill(
        &mut cx,
        export,
        "nodes-export-path",
        output.to_str().unwrap(),
    );
    click(&mut cx, export, "nodes-export-all");
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    assert_eq!(state(&mut cx, export).1, 252);
    // Replacing the live inventory after review must not change the snapshot.
    runtime.block_on(async {
        alpha
            .subscribe(protocol::ClientConfig {
                name: "alpha".into(),
                rules: vec![rule(205, "reject")],
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    let mut asking = beta.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/export-prompt".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let prompt = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        prompt,
        &evidence.join("04-permission-with-export-review.png"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    let first = folders(&output);
    assert_eq!(first.len(), 1);
    verify(
        &first[0],
        (0..250).map(|i| rule(i, "allow")).collect(),
        vec![rule(0, "deny")],
        vec![rule(205, "reject")],
    );
    capture(
        &mut cx,
        export,
        &evidence.join("05-frozen-export-completed.png"),
    );
    click(&mut cx, export, "nodes-export-back");
    click(&mut cx, export, "nodes-export-filter");
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    assert_eq!(state(&mut cx, export).1, 2);
    capture(
        &mut cx,
        export,
        &evidence.join("06-filtered-node-review.png"),
    );
    let marker = output.join("existing-file");
    std::fs::write(&marker, b"preserve").unwrap();
    fill(
        &mut cx,
        export,
        "nodes-export-path",
        marker.to_str().unwrap(),
    );
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    assert_eq!(folders(&output).len(), 1);
    assert_eq!(std::fs::read(&marker).unwrap(), b"preserve");
    capture(
        &mut cx,
        export,
        &evidence.join("07-invalid-parent-preserved.png"),
    );
    fill(
        &mut cx,
        export,
        "nodes-export-path",
        output.to_str().unwrap(),
    );
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    let both = folders(&output);
    assert_eq!(both.len(), 2);
    let second = both.iter().find(|p| !first.contains(p)).unwrap();
    verify(
        second,
        vec![rule(205, "reject")],
        vec![],
        vec![rule(205, "reject")],
    );
    capture(
        &mut cx,
        export,
        &evidence.join("08-filtered-export-completed.png"),
    );
    click(&mut cx, export, "nodes-export-close");
    drop(view);
    drop(cx);
    drop(alpha);
    drop(beta);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let frames = [
        "cached-node-selection",
        "complete-node-review",
        "cancel-publishes-nothing",
        "permission-with-export-review",
        "frozen-export-completed",
        "filtered-node-review",
        "invalid-parent-preserved",
        "filtered-export-completed",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| format!("{:02}-{n}.png", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":frames,"proof_boundary":"Actual cached-node selection, typed path, Cancel, immutable full/filter review, filesystem error/retry and complete per-node JSON outputs. Real synthetic Subscribe/AskRule; offline inventory supplied through production history boundary. Permission proof is while export review is open, not under a deliberately stalled disk operation. No native OS picker, system daemon or kernel enforcement proof."})).unwrap()).unwrap();
}

#[test]
fn headless_startup() {
    use crate::startup::{ENTRY, Launch, Manager, Paths};
    fn state(cx: &mut HeadlessAppContext, w: AnyWindowHandle) -> (bool, bool, bool) {
        cx.update_window(w, |_, window, cx| {
            gpui_kit::base::Root::read(window, cx)
                .view()
                .clone()
                .downcast::<super::startup_view::Startup>()
                .unwrap()
                .read(cx)
                .test_state()
        })
        .unwrap()
    }
    fn finish(cx: &mut HeadlessAppContext, w: AnyWindowHandle) {
        for _ in 0..40 {
            settle(cx);
            if !state(cx, w).0 {
                return;
            }
        }
        panic!("Startup preferences did not finish");
    }
    let evidence = evidence();
    assert_eq!(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        Some(evidence.join("xdg"))
    );
    assert_eq!(
        std::env::var_os("XDG_CONFIG_DIRS").map(PathBuf::from),
        Some(evidence.join("system-xdg"))
    );
    let entry = evidence.join("xdg/autostart").join(ENTRY);
    assert!(!entry.exists());
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mut daemon = runtime.block_on(async {
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
                name: "startup-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        client
    });
    let mut cx = context();
    let mut view = None;
    let main = cx
        .open_window(size(px(1240.), px(940.)), |window, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = view.unwrap();
    let (tray, actions, _availability) = super::tray::Client::fixture();
    cx.update_entity(&view, |v, _| v.tray = Some(tray));
    settle(&mut cx);
    click(&mut cx, main, "startup-preferences");
    let prefs = cx.read_entity(&view, |v, _| v.startup_window.unwrap());
    finish(&mut cx, prefs);
    assert_eq!(state(&mut cx, prefs), (false, false, false));
    capture(&mut cx, prefs, &evidence.join("01-startup-disabled.png"));
    click(&mut cx, prefs, "startup-enabled");
    click(&mut cx, prefs, "startup-close");
    settle(&mut cx);
    assert!(!entry.exists());
    actions.try_send(super::tray::Action::Startup).unwrap();
    settle(&mut cx);
    let prefs = cx.read_entity(&view, |v, _| v.startup_window.unwrap());
    finish(&mut cx, prefs);
    assert!(!state(&mut cx, prefs).1);
    capture(
        &mut cx,
        prefs,
        &evidence.join("02-tray-opens-unsaved-preference.png"),
    );
    click(&mut cx, prefs, "startup-enabled");
    click(&mut cx, prefs, "startup-save");
    finish(&mut cx, prefs);
    let manager = Manager {
        paths: Paths::environment().unwrap(),
        launch: Launch::environment(&config).unwrap(),
    };
    assert!(manager.load().unwrap().enabled);
    assert!(
        std::fs::read_to_string(&entry)
            .unwrap()
            .contains("--socket")
    );
    capture(&mut cx, prefs, &evidence.join("03-enabled-entry-saved.png"));
    click(&mut cx, prefs, "startup-close");
    settle(&mut cx);
    click(&mut cx, main, "startup-preferences");
    let prefs = cx.read_entity(&view, |v, _| v.startup_window.unwrap());
    finish(&mut cx, prefs);
    assert!(state(&mut cx, prefs).1);
    let reply = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/startup-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let prompt = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    click(&mut cx, prefs, "startup-enabled");
    click(&mut cx, prefs, "startup-save");
    finish(&mut cx, prefs);
    assert!(!manager.load().unwrap().enabled);
    capture(
        &mut cx,
        prompt,
        &evidence.join("04-permission-during-startup-save.png"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    // External replacement after load must retain the staged checkbox and file.
    let external = std::fs::read_to_string(&entry).unwrap() + "# external edit\n";
    std::fs::write(&entry, &external).unwrap();
    click(&mut cx, prefs, "startup-enabled");
    click(&mut cx, prefs, "startup-save");
    finish(&mut cx, prefs);
    assert_eq!(state(&mut cx, prefs), (false, true, true));
    assert_eq!(std::fs::read_to_string(&entry).unwrap(), external);
    capture(
        &mut cx,
        prefs,
        &evidence.join("05-stale-entry-preserves-selection.png"),
    );
    click(&mut cx, prefs, "startup-reload");
    finish(&mut cx, prefs);
    assert_eq!(state(&mut cx, prefs), (false, false, false));
    capture(
        &mut cx,
        prefs,
        &evidence.join("06-reloaded-disabled-entry.png"),
    );
    click(&mut cx, prefs, "startup-close");
    drop(view);
    drop(cx);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let frames = [
        "startup-disabled",
        "tray-opens-unsaved-preference",
        "enabled-entry-saved",
        "permission-during-startup-save",
        "stale-entry-preserves-selection",
        "reloaded-disabled-entry",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| format!("{:02}-{n}.png", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":frames,"proof_boundary":"Actual startup checkbox/save/close/reload, isolated XDG file enable/disable/reopen, staged Cancel, external stale refusal and retained selection, synthetic tray Startup action and real independent AskRule. No real user autostart changes, real login scheduler, actual tray menu D-Bus callback or compositor proof."})).unwrap()).unwrap();
}

#[test]
fn headless_appearance() {
    use crate::appearance::{ColorMode, Store};
    fn finish(cx: &mut HeadlessAppContext) {
        for _ in 0..50 {
            settle(cx);
            if !cx.update(|cx| super::appearance::test_state(cx).0) {
                return;
            }
        }
        panic!("Appearance I/O did not finish");
    }
    fn mode(cx: &mut HeadlessAppContext) -> (ColorMode, bool) {
        cx.update(|cx| {
            (
                super::appearance::test_state(cx).1,
                cx.global::<Theme>().is_dark(),
            )
        })
    }
    fn field(cx: &mut HeadlessAppContext, w: AnyWindowHandle, id: &'static str) -> String {
        click(cx, w, id);
        cx.update_window(w, |_, w, cx| {
            w.press("ctrl-a", cx);
            w.press("ctrl-c", cx);
        })
        .unwrap();
        settle(cx);
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
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
    let mut daemon = runtime.block_on(async {
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
        let mut c = protocol::ui_client::UiClient::new(channel);
        c.subscribe(protocol::ClientConfig {
            name: "appearance-fixture".into(),
            rules: vec![protocol::Rule {
                name: "theme-original".into(),
                enabled: true,
                action: "deny".into(),
                duration: "always".into(),
                operator: Some(protocol::Operator {
                    r#type: "simple".into(),
                    operand: "process.path".into(),
                    data: "/owned/theme-app".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let stream = c
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        backend.handle.history.flush().await.unwrap();
        (c, tx, stream)
    });
    let mut cx = context();
    cx.update(|cx| super::appearance::init(&config.database, cx));
    let mut view = None;
    let main = cx
        .open_window(size(px(1240.), px(940.)), |window, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = view.unwrap();
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::System, false));
    click(&mut cx, main, "appearance-preferences");
    let settings = cx.read_entity(&view, |v, _| v.appearance_window.unwrap());
    capture(&mut cx, settings, &evidence.join("01-system-settings.png"));
    click(&mut cx, settings, "appearance-dark");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::Dark, true));
    capture(&mut cx, main, &evidence.join("02-dark-dashboard.png"));
    cx.update_window(main, |_, w, cx| w.press("ctrl-2", cx))
        .unwrap();
    settle(&mut cx);
    cx.update_window(main, |_, w, cx| w.click(("rule-row", 0usize), cx))
        .unwrap();
    click(&mut cx, main, "rule-edit");
    let editor = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    fill(&mut cx, editor, "editor-name", "theme-unsaved");
    fill(
        &mut cx,
        editor,
        "editor-description",
        "Keep this while theme changes",
    );
    capture(
        &mut cx,
        editor,
        &evidence.join("03-dark-rule-editor-draft.png"),
    );
    click(&mut cx, settings, "appearance-light");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::Light, false));
    assert_eq!(field(&mut cx, editor, "editor-name"), "theme-unsaved");
    assert_eq!(
        field(&mut cx, editor, "editor-description"),
        "Keep this while theme changes"
    );
    capture(
        &mut cx,
        editor,
        &evidence.join("04-light-editor-retains-draft.png"),
    );
    click(&mut cx, editor, "editor-cancel");
    runtime.block_on(backend.handle.history.flush()).unwrap();
    assert!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "appearance-fixture",
            "theme-unsaved"
        )
        .unwrap()
        .is_none()
    );
    assert!(
        crate::rules::transfer::find_confirmed_rule(
            &config.database,
            "appearance-fixture",
            "theme-original"
        )
        .unwrap()
        .is_some()
    );

    let reply = runtime.spawn(async move {
        daemon
            .0
            .ask_rule(protocol::Connection {
                process_path: "/owned/theme-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let prompt = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(&mut cx, prompt, &evidence.join("05-light-permission.png"));
    click(&mut cx, settings, "appearance-dark");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::Dark, true));
    capture(&mut cx, prompt, &evidence.join("06-dark-permission.png"));
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    let store = Store::new(&config.database);
    std::fs::write(&store.path, b"{external invalid}").unwrap();
    click(&mut cx, settings, "appearance-light");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::Light, false));
    assert_eq!(std::fs::read(&store.path).unwrap(), b"{external invalid}");
    capture(
        &mut cx,
        settings,
        &evidence.join("07-session-only-save-error.png"),
    );
    click(&mut cx, settings, "appearance-reload");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::System, false));
    capture(
        &mut cx,
        settings,
        &evidence.join("08-invalid-file-system-fallback.png"),
    );
    click(&mut cx, settings, "appearance-dark");
    finish(&mut cx);
    assert_eq!(store.load().unwrap().mode, ColorMode::Dark);
    capture(
        &mut cx,
        settings,
        &evidence.join("09-repaired-dark-preference.png"),
    );
    click(&mut cx, settings, "appearance-close");
    drop(view);
    drop(cx);
    let mut cx = context();
    cx.update(|cx| super::appearance::init(&config.database, cx));
    let mut view = None;
    let main = cx
        .open_window(size(px(1240.), px(940.)), |window, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = view.unwrap();
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::Dark, true));
    capture(
        &mut cx,
        main,
        &evidence.join("10-restarted-dark-dashboard.png"),
    );
    click(&mut cx, main, "appearance-preferences");
    let settings = cx.read_entity(&view, |v, _| v.appearance_window.unwrap());
    click(&mut cx, settings, "appearance-reset");
    finish(&mut cx);
    assert_eq!(mode(&mut cx), (ColorMode::System, false));
    assert_eq!(store.load().unwrap().mode, ColorMode::System);
    capture(&mut cx, settings, &evidence.join("11-reset-system.png"));
    click(&mut cx, settings, "appearance-close");
    drop(view);
    drop(cx);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let frames = [
        "system-settings",
        "dark-dashboard",
        "dark-rule-editor-draft",
        "light-editor-retains-draft",
        "light-permission",
        "dark-permission",
        "session-only-save-error",
        "invalid-file-system-fallback",
        "repaired-dark-preference",
        "restarted-dark-dashboard",
        "reset-system",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| format!("{:02}-{n}.png", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":frames,"proof_boundary":"Actual Kit Settings color/reset/reload controls, dashboard/editor/popup repaint, copied unsaved editor fields, independent real AskRule, stale save preserved on disk with session-only theme, explicit invalid-file recovery and fresh GPUI context persistence. Platform is fixed Light here: actual native system appearance-change events, compositor, density/custom themes and whole-app visual pass remain separate."})).unwrap()).unwrap();
}

#[test]
fn headless_window_memory() {
    use crate::window_state::{Mode, Page, Rect, State, Store};
    fn ready(cx: &mut HeadlessAppContext, v: &gpui::Entity<Dashboard>) -> AnyWindowHandle {
        for _ in 0..50 {
            settle(cx);
            if let Some(w) =
                cx.read_entity(v, |v, _| v.window_ready.then_some(v.main_window).flatten())
            {
                return w;
            }
        }
        panic!("Dashboard restoration did not finish");
    }
    fn saved(cx: &mut HeadlessAppContext, v: &gpui::Entity<Dashboard>) -> (u64, u64, u64) {
        for _ in 0..50 {
            settle(cx);
            let state = cx.read_entity(v, |v, _| v.memory.as_ref().unwrap().test_saved());
            if state.0 >= state.2 {
                return state;
            }
        }
        panic!("Layout write did not settle");
    }
    fn open(
        cx: &mut HeadlessAppContext,
        backend: &Backend,
        config: &Config,
    ) -> gpui::Entity<Dashboard> {
        cx.update(|cx| {
            super::appearance::init(&config.database, cx);
            super::window_memory::open(backend.handle.clone(), config.clone(), cx).unwrap()
        })
    }
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let store = Store::new(&config.database);
    let seed = State {
        version: 1,
        bounds: Rect {
            x: 50000.,
            y: 50000.,
            width: 960.,
            height: 680.,
        },
        mode: Mode::Windowed,
        page: Page::Nodes,
    };
    store.save(&store.load().unwrap(), seed).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (mut daemon, notifications, stream) = runtime.block_on(async {
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
        let mut c = protocol::ui_client::UiClient::new(channel);
        c.subscribe(protocol::ClientConfig {
            name: "window-fixture".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let stream = c
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        backend.handle.history.flush().await.unwrap();
        (c, tx, stream)
    });
    let mut cx = context();
    let view = open(&mut cx, &backend, &config);
    let main = ready(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.screen == super::Screen::Nodes));
    let bounds = cx.update_window(main, |_, w, _| w.bounds()).unwrap();
    assert_eq!(bounds.size, size(px(960.), px(680.)));
    assert!(f32::from(bounds.origin.x) < 50000. && f32::from(bounds.origin.y) < 50000.);
    capture(&mut cx, main, &evidence.join("01-restored-nodes-page.png"));
    cx.update_window(main, |_, w, cx| w.press("ctrl-2", cx))
        .unwrap();
    saved(&mut cx, &view);
    assert_eq!(store.load().unwrap().state.unwrap().page, Page::Rules);
    capture(&mut cx, main, &evidence.join("02-rules-page-saved.png"));
    let burst_before = saved(&mut cx, &view);
    for i in 0..25 {
        cx.update_window(main, |_, w, _| {
            w.resize(size(px(976. + i as f32), px(720.)))
        })
        .unwrap();
        sync_geometry(&mut cx);
        cx.advance_clock(Duration::from_millis(60));
        cx.run_until_parked();
    }
    let after = saved(&mut cx, &view);
    assert!(after.2 >= burst_before.2 + 5);
    assert!(
        after.1 - burst_before.1 <= 2,
        "Resize burst must be coalesced"
    );
    assert_eq!(store.load().unwrap().state.unwrap().bounds.width, 1000.);
    capture(&mut cx, main, &evidence.join("03-resize-burst-saved.png"));
    let (tray, actions, _) = super::tray::Client::fixture();
    cx.update_entity(&view, |v, _| v.tray = Some(tray));
    cx.update_window(main, |_, window, cx| {
        if view.update(cx, |v, _| v.should_close_dashboard(window)) {
            window.remove_window();
        }
    })
    .unwrap();
    saved(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.main_window.is_none()));
    let mut asking = daemon.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/hidden-layout".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let prompt = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(&mut cx, prompt, &evidence.join("04-permission-hidden.png"));
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    actions.try_send(super::tray::Action::Open).unwrap();
    let main = ready(&mut cx, &view);
    assert_eq!(
        cx.update_window(main, |_, w, _| w.bounds().size).unwrap(),
        size(px(1000.), px(720.))
    );
    assert!(cx.read_entity(&view, |v, _| v.screen == super::Screen::Rules));
    capture(&mut cx, main, &evidence.join("05-warm-reopen.png"));
    cx.update_window(main, |_, window, cx| {
        if view.update(cx, |v, _| v.should_close_dashboard(window)) {
            window.remove_window();
        }
    })
    .unwrap();
    saved(&mut cx, &view);
    drop(view);
    drop(cx);
    // Hold only load-result delivery; the production bounded UI wait must still
    // expose a default dashboard and answer a request without accepting late data.
    let mut cx = context();
    let (deliver, loaded) = tokio::sync::oneshot::channel();
    let view = cx.update(|cx| {
        super::appearance::init(&config.database, cx);
        super::window_memory::open_delayed(backend.handle.clone(), config.clone(), loaded, cx)
            .unwrap()
    });
    let mut asking = daemon.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/loading-layout".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    for _ in 0..100 {
        if !backend.handle.prompts.snapshot().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    cx.run_until_parked();
    cx.advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    sync_geometry(&mut cx);
    let prompt = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    assert!(!cx.read_entity(&view, |v, _| v.window_ready));
    capture(
        &mut cx,
        prompt,
        &evidence.join("06-permission-before-load-result.png"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    let main = ready(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.screen == super::Screen::History));
    assert!(deliver.send(Ok(Some(seed))).is_err());
    assert_eq!(store.load().unwrap().state.unwrap().page, Page::Rules);
    capture(&mut cx, main, &evidence.join("07-timeout-default-main.png"));
    drop(view);
    drop(cx);
    let mut cx = context();
    let view = open(&mut cx, &backend, &config);
    let main = ready(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.screen == super::Screen::Rules));
    assert_eq!(
        cx.update_window(main, |_, w, _| w.bounds().size).unwrap(),
        size(px(1000.), px(720.))
    );
    capture(&mut cx, main, &evidence.join("08-cold-restored-rules.png"));
    let bytes = std::fs::read(&store.path).unwrap();
    std::fs::write(&store.path, b"{external}").unwrap();
    cx.update_window(main, |_, w, cx| w.press("ctrl-4", cx))
        .unwrap();
    for _ in 0..50 {
        settle(&mut cx);
        if cx.read_entity(&view, |v, _| v.memory.as_ref().unwrap().error().is_some()) {
            break;
        }
    }
    assert!(cx.read_entity(&view, |v, _| v.memory.as_ref().unwrap().error().is_some()));
    let before = cx.read_entity(&view, |v, _| v.memory.as_ref().unwrap().test_saved().1);
    for key in ["ctrl-1", "ctrl-2", "ctrl-3"] {
        cx.update_window(main, |_, w, cx| w.press(key, cx)).unwrap();
        settle(&mut cx);
    }
    assert_eq!(
        cx.read_entity(&view, |v, _| v.memory.as_ref().unwrap().test_saved().1),
        before
    );
    assert_eq!(std::fs::read(&store.path).unwrap(), b"{external}");
    capture(
        &mut cx,
        main,
        &evidence.join("09-stale-state-preserved.png"),
    );
    drop(view);
    drop(cx);
    std::fs::write(&store.path, bytes).unwrap();
    let mut cx = context();
    let view = open(&mut cx, &backend, &config);
    let main = ready(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.screen == super::Screen::Rules));
    capture(
        &mut cx,
        main,
        &evidence.join("10-restarted-after-external-repair.png"),
    );
    let reply = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/quit-layout".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    cx.update_window(main, |_, w, cx| w.press("ctrl-3", cx))
        .unwrap();
    cx.run_until_parked();
    capture(&mut cx, main, &evidence.join("11-quit-flush-state.png"));
    click(&mut cx, main, "quit");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    assert_eq!(store.load().unwrap().state.unwrap().page, Page::Statistics);
    drop(view);
    drop(cx);
    drop(notifications);
    drop(stream);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let frames = [
        "restored-nodes-page",
        "rules-page-saved",
        "resize-burst-saved",
        "permission-hidden",
        "warm-reopen",
        "permission-before-load-result",
        "timeout-default-main",
        "cold-restored-rules",
        "stale-state-preserved",
        "restarted-after-external-repair",
        "quit-flush-state",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| format!("{:02}-{n}.png", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":frames,"resize_updates":after.2-burst_before.2,"resize_writes":after.1-burst_before.1,"proof_boundary":"Actual navigation/Quit and permission input with production bootstrap/worker; synthetic tray close/Open boundary and native GPUI resize requests plus documented compositor-callback shim. Held load-result delivery is not a deliberately stalled filesystem. Exact disk state and cold/warm GUI reopens, bounded coalescing/stale refusal and pending-request shutdown are proved. Actual compositor resize/maximize/fullscreen/position behavior and native focus need separate proof."})).unwrap()).unwrap();
}

mod url_probe_fixture {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/probe_http.rs"
    ));
}
#[test]
fn headless_subscription_url_test() {
    fn done(cx: &mut HeadlessAppContext, v: &gpui::Entity<Dashboard>) {
        for _ in 0..30 {
            settle(cx);
            if !cx.read_entity(v, |v, _| v.subscriptions.url_test_state().0) {
                return;
            }
        }
        panic!("URL test did not complete");
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
    let server = runtime.block_on(url_probe_fixture::Server::start());
    let mut daemon = runtime.block_on(async {
        use hyper_util::rt::TokioIo;
        let socket = config.socket.clone();
        let channel = tonic::transport::Endpoint::try_from("http://localhost")
            .unwrap()
            .connect_with_connector(tower::service_fn(move |_| {
                let path = socket.clone();
                async move {
                    tokio::net::UnixStream::connect(path)
                        .await
                        .map(TokioIo::new)
                }
            }))
            .await
            .unwrap();
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "url-probe-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    let mut cx = context();
    let mut view = None;
    let main = cx
        .open_window(size(px(1200.), px(1100.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let view = view.unwrap();
    let main: AnyWindowHandle = main.into();
    settle(&mut cx);
    cx.update_window(main, |_, w, cx| w.press("ctrl-7", cx))
        .unwrap();
    settle(&mut cx);
    click(&mut cx, main, "subs-add");
    settle(&mut cx);
    click(&mut cx, main, "subs-test-url");
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().2));
    assert!(server.requests.lock().unwrap().is_empty());
    capture(&mut cx, main, &evidence.join("01-empty-url-refused.png"));
    fill(
        &mut cx,
        main,
        "subs-url",
        &format!("{}/redirect", server.url),
    );
    click(&mut cx, main, "subs-test-url");
    done(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| {
        v.subscriptions
            .url_test_state()
            .1
            .is_some_and(|m| m.contains("via redirect") && m.contains("resembles hosts"))
    }));
    capture(
        &mut cx,
        main,
        &evidence.join("02-redirect-hosts-sample.png"),
    );
    fill(&mut cx, main, "subs-url", &format!("{}/bad", server.url));
    click(&mut cx, main, "subs-test-url");
    done(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().2));
    capture(&mut cx, main, &evidence.join("03-invalid-hosts-sample.png"));
    pick(&mut cx, main, "subs-format", "Domain names / patterns");
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().1.is_none()));
    let request_count = server.requests.lock().unwrap().len();
    click(&mut cx, main, "subs-test-url");
    done(&mut cx, &view);
    assert!(!cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().2));
    assert_eq!(
        &server.requests.lock().unwrap()[request_count..],
        &["HEAD /bad"]
    );
    capture(&mut cx, main, &evidence.join("04-non-hosts-head-only.png"));
    pick(&mut cx, main, "subs-format", "Hosts file");
    fill(&mut cx, main, "subs-url", &format!("{}/slow", server.url));
    click(&mut cx, main, "subs-test-url");
    settle(&mut cx);
    assert!(server.held.load(Ordering::Acquire));
    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().0));
    capture(&mut cx, main, &evidence.join("05-held-url-test.png"));
    let reply = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/url-probe-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let popup = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("06-permission-during-url-test.png"),
    );
    click(&mut cx, popup, "deny-once");
    let rule = runtime.block_on(reply).unwrap();
    assert_eq!(rule.action, "deny");
    assert_eq!(rule.duration, "once");
    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().0));
    click(&mut cx, main, "subs-test-cancel");
    settle(&mut cx);
    assert!(
        cx.read_entity(&view, |v, _| !v.subscriptions.url_test_state().0
            && v.subscriptions.url_test_state().1 == Some("URL test cancelled"))
    );
    capture(&mut cx, main, &evidence.join("07-cancel-url-test.png"));
    click(&mut cx, main, "subs-test-url");
    settle(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.subscriptions.url_test_state().0));
    fill(&mut cx, main, "subs-url", &format!("{}/hosts", server.url));
    settle(&mut cx);
    assert!(
        cx.read_entity(&view, |v, _| !v.subscriptions.url_test_state().0
            && v.subscriptions.url_test_state().1.is_none())
    );
    click(&mut cx, main, "subs-test-url");
    done(&mut cx, &view);
    assert!(cx.read_entity(&view, |v, _| {
        !v.subscriptions.url_test_state().2
            && v.subscriptions
                .url_test_state()
                .1
                .is_some_and(|m| m.contains("resembles hosts"))
    }));
    capture(
        &mut cx,
        main,
        &evidence.join("08-edited-url-discards-old-result.png"),
    );
    fill(&mut cx, main, "subs-url", &format!("{}/slow", server.url));
    click(&mut cx, main, "subs-test-url");
    settle(&mut cx);
    click(&mut cx, main, "subs-cancel");
    settle(&mut cx);
    click(&mut cx, main, "subs-add");
    settle(&mut cx);
    assert!(
        cx.read_entity(&view, |v, _| !v.subscriptions.url_test_state().0
            && v.subscriptions.url_test_state().1.is_none())
    );
    capture(
        &mut cx,
        main,
        &evidence.join("09-cancel-form-drops-test.png"),
    );
    fill(&mut cx, main, "subs-url", &format!("{}/slow", server.url));
    click(&mut cx, main, "subs-test-url");
    settle(&mut cx);
    cx.update_window(main, |_, w, cx| w.press("ctrl-1", cx))
        .unwrap();
    settle(&mut cx);
    cx.update_window(main, |_, w, cx| w.press("ctrl-7", cx))
        .unwrap();
    settle(&mut cx);
    assert!(
        cx.read_entity(&view, |v, _| !v.subscriptions.url_test_state().0
            && v.subscriptions.url_test_state().1 == Some("URL test cancelled"))
    );
    capture(
        &mut cx,
        main,
        &evidence.join("10-leave-page-cancels-test.png"),
    );
    let db = rusqlite::Connection::open(
        backend
            .handle
            .subscriptions
            .root
            .join("subscriptions.sqlite3"),
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM subscriptions", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM logs", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        0
    );
    fill(&mut cx, main, "subs-url", &format!("{}/hosts", server.url));
    click(&mut cx, main, "subs-test-url");
    done(&mut cx, &view);
    fill(&mut cx, main, "subs-name", "Explicitly saved after test");
    click(&mut cx, main, "subs-save");
    settle(&mut cx);
    assert_eq!(
        db.query_row("SELECT count(*) FROM subscriptions", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT sum(bytes) FROM subscriptions", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    capture(&mut cx, main, &evidence.join("11-explicit-save-only.png"));
    server.release();
    drop(db);
    click(&mut cx, main, "quit");
    drop(view);
    drop(cx);
    drop(server);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let names = [
        "empty-url-refused",
        "redirect-hosts-sample",
        "invalid-hosts-sample",
        "non-hosts-head-only",
        "held-url-test",
        "permission-during-url-test",
        "cancel-url-test",
        "edited-url-discards-old-result",
        "cancel-form-drops-test",
        "leave-page-cancels-test",
        "explicit-save-only",
    ];
    let frames = names
        .iter()
        .enumerate()
        .map(|(i, n)| format!("{:02}-{n}.png", i + 1))
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":frames,"proof_boundary":"Owned loopback HTTP and actual Kit URL/format/test/cancel/form/save/navigation controls; redirect and bounded hosts sample, non-host HEAD-only, stale-input result refusal, independent real AskRule/Deny once while GET is held, no subscription/log/data writes before explicit Save. Native compositor input, actual external endpoints and certificate/compression variants remain separate."})).unwrap()).unwrap();
}

#[test]
fn headless_subscription_inheritance() {
    use crate::subscriptions::{
        Reply, Request,
        limits::{Override, Unit},
        model::*,
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
    let request = |r| {
        runtime.block_on(async {
            backend
                .handle
                .subscriptions
                .request(r)
                .unwrap()
                .await
                .unwrap()
                .unwrap()
        })
    };
    let rows = || match request(Request::Page(Query::default())) {
        Reply::Page(p) => p,
        _ => panic!(),
    };
    let legacy = match request(Request::Save {
        previous: None,
        spec: Spec {
            name: "Legacy absolute".into(),
            url: "http://127.0.0.1:9/owned-legacy".into(),
            interval: Some(Override::Absolute(60)),
            timeout: Some(Override::Absolute(2)),
            max_bytes: Some(Override::Absolute(1024)),
            ..Default::default()
        },
    }) {
        Reply::Saved(id) => id,
        _ => panic!(),
    };
    let mut cx = context();
    let mut view = None;
    let window = cx
        .open_window(size(px(1200.), px(1000.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let view = view.unwrap();
    let window: AnyWindowHandle = window.into();
    settle(&mut cx);
    cx.update_window(window, |_, w, cx| w.press("ctrl-7", cx))
        .unwrap();
    settle(&mut cx);
    let select = |cx: &mut HeadlessAppContext, id| {
        cx.update_window(window, |_, w, cx| w.click(("subscription-row", id), cx))
            .unwrap();
        settle(cx);
    };
    click(&mut cx, window, "subs-defaults");
    settle(&mut cx);
    fill(&mut cx, window, "subs-default-interval", "2");
    pick(&mut cx, window, "subs-default-interval-unit", "hours");
    fill(&mut cx, window, "subs-default-timeout", "2");
    pick(&mut cx, window, "subs-default-timeout-unit", "minutes");
    fill(&mut cx, window, "subs-default-size", "3");
    pick(&mut cx, window, "subs-default-size-unit", "MB");
    capture(
        &mut cx,
        window,
        &evidence.join("01-default-count-unit-controls.png"),
    );
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    click(&mut cx, window, "subs-add");
    fill(&mut cx, window, "subs-name", "Independent inherited limits");
    fill(
        &mut cx,
        window,
        "subs-url",
        "http://127.0.0.1:9/owned-inheritance",
    );
    pick(&mut cx, window, "subs-interval-unit", "minutes");
    pick(&mut cx, window, "subs-timeout-unit", "seconds");
    pick(&mut cx, window, "subs-size-unit", "KB");
    settle(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert_eq!(state.0, vec!["", "", ""]);
    assert_eq!(state.1, vec!["minutes", "seconds", "KB"]);
    assert!(
        state.2.contains("120 seconds between")
            && state.2.contains("2 seconds timeout")
            && state.2.contains("3072 bytes")
    );
    capture(&mut cx, window, &evidence.join("02-unit-only-draft.png"));
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    let id = rows()
        .rows
        .iter()
        .find(|r| r.spec.name == "Independent inherited limits")
        .unwrap()
        .id;
    select(&mut cx, id);
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert_eq!(state.0, vec!["", "", ""]);
    assert_eq!(state.1, vec!["minutes", "seconds", "KB"]);
    capture(&mut cx, window, &evidence.join("03-unit-only-reopened.png"));
    click(&mut cx, window, "subs-cancel");
    settle(&mut cx);
    click(&mut cx, window, "subs-defaults");
    settle(&mut cx);
    fill(&mut cx, window, "subs-default-interval", "3");
    fill(&mut cx, window, "subs-default-timeout", "4");
    fill(&mut cx, window, "subs-default-size", "5");
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    select(&mut cx, id);
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert!(
        state.2.contains("180 seconds between")
            && state.2.contains("4 seconds timeout")
            && state.2.contains("5120 bytes")
    );
    capture(
        &mut cx,
        window,
        &evidence.join("04-default-counts-recompute-unit-only.png"),
    );
    click(&mut cx, window, "subs-cancel");
    settle(&mut cx);
    click(&mut cx, window, "subs-defaults");
    settle(&mut cx);
    fill(&mut cx, window, "subs-default-interval", "4");
    pick(&mut cx, window, "subs-default-interval-unit", "minutes");
    fill(&mut cx, window, "subs-default-timeout", "10");
    pick(&mut cx, window, "subs-default-timeout-unit", "seconds");
    fill(&mut cx, window, "subs-default-size", "10");
    pick(&mut cx, window, "subs-default-size-unit", "KB");
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    select(&mut cx, id);
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert!(
        state.2.contains("240 seconds between")
            && state.2.contains("10 seconds timeout")
            && state.2.contains("10240 bytes")
    );
    capture(&mut cx, window, &evidence.join("05-new-default-units.png"));
    fill(&mut cx, window, "subs-interval", "3");
    fill(&mut cx, window, "subs-timeout", "1");
    fill(&mut cx, window, "subs-size", "2");
    for id in ["subs-interval-unit", "subs-timeout-unit", "subs-size-unit"] {
        pick(&mut cx, window, id, "Inherit default");
    }
    settle(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert_eq!(state.1, vec!["Inherit default"; 3]);
    assert!(
        state.2.contains("180 seconds between")
            && state.2.contains("1 seconds timeout")
            && state.2.contains("2048 bytes")
    );
    capture(&mut cx, window, &evidence.join("06-count-only-draft.png"));
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    click(&mut cx, window, "subs-defaults");
    settle(&mut cx);
    fill(&mut cx, window, "subs-default-interval", "60");
    pick(&mut cx, window, "subs-default-interval-unit", "seconds");
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    assert_eq!(rows().settings.1.interval, 240);
    capture(
        &mut cx,
        window,
        &evidence.join("07-invalid-dependent-default-refused.png"),
    );
    let mut daemon = runtime.block_on(async {
        use hyper_util::rt::TokioIo;
        let socket = config.socket.clone();
        let channel = tonic::transport::Endpoint::try_from("http://localhost")
            .unwrap()
            .connect_with_connector(tower::service_fn(move |_| {
                let path = socket.clone();
                async move {
                    tokio::net::UnixStream::connect(path)
                        .await
                        .map(TokioIo::new)
                }
            }))
            .await
            .unwrap();
        let mut c = protocol::ui_client::UiClient::new(channel);
        c.subscribe(protocol::ClientConfig {
            name: "inheritance-fixture".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        c
    });
    let reply = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/inheritance-prompt".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    settle(&mut cx);
    let popup = cx.read_entity(&view, |v, _| v.prompt_window.unwrap());
    capture(
        &mut cx,
        popup,
        &evidence.join("08-permission-with-default-editor.png"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    click(&mut cx, window, "subs-cancel");
    settle(&mut cx);
    select(&mut cx, id);
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    let page = rows();
    let mut changed = page.settings.1;
    changed.interval = 3600;
    changed.units.as_mut().unwrap().interval = Unit::Hours;
    request(Request::SaveSettings {
        revision: page.settings.0,
        settings: changed,
    });
    fill(&mut cx, window, "subs-name", "Stale draft must not apply");
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    assert!(
        rows()
            .rows
            .iter()
            .any(|r| r.id == id && r.spec.name == "Independent inherited limits")
    );
    capture(
        &mut cx,
        window,
        &evidence.join("09-stale-defaults-editor-refused.png"),
    );
    click(&mut cx, window, "subs-cancel");
    settle(&mut cx);
    select(&mut cx, id);
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert_eq!(state.0, vec!["3", "1", "2"]);
    assert!(state.2.contains("10800 seconds between"));
    capture(
        &mut cx,
        window,
        &evidence.join("10-reopened-current-defaults.png"),
    );
    click(&mut cx, window, "subs-cancel");
    settle(&mut cx);
    select(&mut cx, legacy);
    click(&mut cx, window, "subs-edit");
    settle(&mut cx);
    let state = cx.read_entity(&view, |v, cx| v.subscriptions.limit_state(cx).unwrap());
    assert_eq!(state.0, vec!["60", "2", "1"]);
    assert_eq!(state.1, vec!["seconds", "seconds", "KB"]);
    assert!(state.2.contains("60 seconds between") && state.2.contains("1024 bytes"));
    capture(
        &mut cx,
        window,
        &evidence.join("11-legacy-absolute-preserved.png"),
    );
    click(&mut cx, window, "subs-save");
    settle(&mut cx);
    let final_rows = rows();
    assert_eq!(final_rows.total, 2);
    assert!(
        final_rows
            .rows
            .iter()
            .all(|r| r.bytes == 0 && r.checked == 0)
    );
    let old = final_rows.rows.iter().find(|r| r.id == legacy).unwrap();
    assert_eq!(
        effective(&old.spec, &final_rows.settings.1).unwrap(),
        (60, 2, 1024)
    );
    let current = final_rows.rows.iter().find(|r| r.id == id).unwrap();
    assert_eq!(current.spec.interval, Override::parts(Some(3), None));
    capture(
        &mut cx,
        window,
        &evidence.join("12-saved-independent-and-legacy-limits.png"),
    );
    click(&mut cx, window, "quit");
    drop(view);
    drop(cx);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let names = [
        "default-count-unit-controls",
        "unit-only-draft",
        "unit-only-reopened",
        "default-counts-recompute-unit-only",
        "new-default-units",
        "count-only-draft",
        "invalid-dependent-default-refused",
        "permission-with-default-editor",
        "stale-defaults-editor-refused",
        "reopened-current-defaults",
        "legacy-absolute-preserved",
        "saved-independent-and-legacy-limits",
    ];
    let frames = names
        .iter()
        .enumerate()
        .map(|(i, n)| format!("{:02}-{n}.png", i + 1))
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","real_gpu":true,"scoped_socket_removed":true,"screenshots":frames,"proof_boundary":"Actual Kit count/Input/unit Select/Save/Cancel controls, reopened raw inheritance, recomputed effective limits, invalid-dependent defaults rollback, stale-defaults save refusal and current reload, legacy absolute compatibility and independent real AskRule. Metadata-only fixture with paused downloads; no external HTTP or real daemon policy changes. Native unit selectors and real bounded-download behavior are separate evidence."})).unwrap()).unwrap();
}
