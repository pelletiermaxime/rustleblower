//! Headless integration journeys using GPUI Kit's test runner and real input.
use super::{Dashboard, prompt_window::PromptWindow};
use crate::{
    backend::Backend,
    config::{Config, Decision},
    protocol,
};
use gpui::{AnyWindowHandle, App, AppContext, Render, TestAppContext, WindowHandle, px, size};
use gpui_kit::{
    component::{Theme, ThemeMode},
    test::{TestAppContextExt, TestWindowExt},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn context(parent: &TestAppContext) -> TestAppContext {
    assert!(std::env::var_os("DISPLAY").is_none());
    assert!(std::env::var_os("WAYLAND_DISPLAY").is_none());
    let cx = parent.new_app();
    // Real fixture workers may wake GPUI; UI timers still use the test clock.
    cx.executor().allow_parking();
    cx.executor().forbid_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
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
        transport: Default::default(),
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
trait TestContextExt {
    fn open_test_window<V: Render + 'static>(
        &mut self,
        size: gpui::Size<gpui::Pixels>,
        build: impl FnOnce(&mut gpui::Window, &mut App) -> gpui::Entity<V>,
    ) -> gpui::Result<WindowHandle<V>>;
    fn advance_clock(&self, duration: Duration);
}
impl TestContextExt for TestAppContext {
    fn open_test_window<V: Render + 'static>(
        &mut self,
        size: gpui::Size<gpui::Pixels>,
        build: impl FnOnce(&mut gpui::Window, &mut App) -> gpui::Entity<V>,
    ) -> gpui::Result<WindowHandle<V>> {
        self.update(|cx| {
            cx.open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds {
                        origin: gpui::Point::default(),
                        size,
                    })),
                    focus: false,
                    show: false,
                    ..Default::default()
                },
                build,
            )
        })
    }
    fn advance_clock(&self, duration: Duration) {
        self.executor().advance_clock(duration);
    }
}
// Complete GPUI effects and layout without imposing a wall-clock delay.
#[track_caller]
fn flush(cx: &mut TestAppContext) {
    cx.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    sync_geometry(cx);
    for handle in cx.update(|cx| cx.windows()) {
        let dashboard = cx
            .update_window(handle, |root, _, cx| {
                root.clone().downcast::<Dashboard>().ok().or_else(|| {
                    root.downcast::<gpui_kit::base::Root>()
                        .ok()
                        .and_then(|root| root.read(cx).view().clone().downcast::<Dashboard>().ok())
                })
            })
            .unwrap();
        if let Some(view) = dashboard {
            wait_window(cx, handle, |_, cx| {
                let view = view.read(cx);
                // Permission input must not wait for an unrelated page read.
                !view.backend.prompts.snapshot().is_empty() || view.test_query_ready()
            });
        }
    }
}
fn sync_geometry(cx: &mut TestAppContext) {
    for handle in cx.update(|cx| cx.windows()) {
        cx.update_window(handle, |_, window, cx| {
            if window.viewport_size() != window.bounds().size {
                window.bounds_changed(cx);
            }
            window.render_frame(cx);
        })
        .unwrap();
    }
    cx.run_until_parked();
}
#[track_caller]
fn wait_for(cx: &mut TestAppContext, mut predicate: impl FnMut(&mut App) -> bool) {
    flush(cx);
    if cx.update(&mut predicate) {
        return;
    }
    let window = cx.update(|cx| cx.windows().first().copied());
    let Some(window) = window else {
        // Bootstrap/hidden-dashboard workers can complete before any window exists.
        let executor = cx.foreground_executor().clone();
        let started = Instant::now();
        executor.block_test(async {
            while !cx.update(&mut predicate) {
                assert!(
                    started.elapsed() < Duration::from_secs(15),
                    "Windowless UI condition timed out"
                );
                cx.executor().timer(Duration::from_millis(10)).await;
            }
        });
        flush(cx);
        return;
    };
    wait_window(cx, window, |_, cx| predicate(cx));
    flush(cx);
}
#[track_caller]
fn wait_window(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    mut predicate: impl FnMut(&mut gpui::Window, &mut App) -> bool,
) {
    let executor = cx.foreground_executor().clone();
    let location = std::panic::Location::caller();
    let started = Instant::now();
    executor.block_test(cx.wait_for(window, Duration::from_secs(120), |window, cx| {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "UI condition exceeded wall-clock deadline at {location}"
        );
        predicate(window, cx)
    }));
}

fn shutdown(cx: TestAppContext) {
    cx.quit();
    cx.run_until_parked();
    drop(cx);
}
fn permission_window(cx: &mut TestAppContext, view: &gpui::Entity<Dashboard>) -> AnyWindowHandle {
    wait_for(cx, |cx| {
        cx.read_entity(view, |view, _| {
            view.prompt_window.is_some()
                && view
                    .backend
                    .prompts
                    .snapshot()
                    .first()
                    .is_some_and(|prompt| view.prompt_id == Some(prompt.id))
        })
    });
    cx.read_entity(view, |view, _| view.prompt_window.unwrap())
}
#[track_caller]
fn checkpoint(cx: &mut TestAppContext, handle: AnyWindowHandle, path: &Path) {
    flush(cx);
    let snapshot=cx.update_window(handle,|_,window,cx| {
        window.render_frame(cx);
        serde_json::json!({"kind":"kit-layout-checkpoint","real_gpu":false,
            "viewport":format!("{:?}",window.viewport_size()),
            "controls":gpui_kit::base::test_support::registered_paths(window).chars().take(65536).collect::<String>()})
    }).unwrap();
    std::fs::write(path, serde_json::to_vec_pretty(&snapshot).unwrap()).unwrap();
}

const SETTINGS_ACTIONS: [&str; 8] = [
    "preferences",
    "appearance-preferences",
    "startup-preferences",
    "updates-preferences",
    "reputation-preferences",
    "raw-download-preferences",
    "highlighting-reload",
    "quit",
];
#[track_caller]
fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    flush(cx);
    if !SETTINGS_ACTIONS.contains(&id) {
        wait_window(cx, window, |window, _| {
            window.try_find(id).is_some_and(|target| target.visible())
        });
    }

    if SETTINGS_ACTIONS.contains(&id)
        && id != "quit"
        && !cx
            .update_window(window, |_, w, _| {
                w.try_find(id).is_some_and(|target| target.visible())
            })
            .unwrap()
    {
        cx.update_window(window, |_, w, cx| w.click("settings-menu-trigger", cx))
            .unwrap();
        flush(cx);
        let settings = cx.update(|cx| crate::app::settings::test_window(cx));
        let target = match id {
            "preferences" => "settings-popup",
            "appearance-preferences" => "settings-appearance",
            "startup-preferences" => "settings-startup",
            "updates-preferences" => "settings-updates",
            "reputation-preferences" => "settings-reputation",
            "raw-download-preferences" => "settings-downloads",
            "highlighting-reload" => "settings-highlighting",
            _ => unreachable!(),
        };
        cx.update_window(settings, |_, w, cx| w.click(target, cx))
            .unwrap();
        flush(cx);
        if id == "highlighting-reload" {
            cx.update_window(settings, |_, w, cx| w.click(id, cx))
                .unwrap();
            flush(cx);
        }
        return;
    }
    cx.update_window(window, |_, window, cx| window.click(id, cx))
        .unwrap();
    cx.run_until_parked();
    sync_geometry(cx);
}
fn keyboard_button(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
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
fn click_prompt_option(cx: &mut TestAppContext, window: AnyWindowHandle, id: gpui::ElementId) {
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
fn pick(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str, label: &str) {
    click(cx, window, id);
    // The popover's search input takes focus after mounting in a frame.
    flush(cx);
    cx.update_window(window, |_, window, cx| window.input(label, cx))
        .unwrap();
    flush(cx);
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    // Select commits/close/focus restoration are deferred across frames.
    flush(cx);
}

fn fill(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str, text: &str) {
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
    flush(cx);
}
fn advanced_defaults(cx: &mut TestAppContext, window: AnyWindowHandle) {
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

mod alerts;
mod bandwidth;

mod appearance;

mod browsing;

mod catalogue_policy;

mod checksum_review;

mod countdown_review;

mod cross_node_copies;

mod daemon_preferences;

mod desktop_notifications;

mod firewall;

mod firewall_files;

mod firewall_forms;

mod firewall_policy;

mod highlighting;

mod history_advanced;

mod history_preferences;

mod history_process;

mod history_selection;

mod history_tools;

mod language;

mod live_table;

mod live_tasks;

mod multi_node_drafts;

mod multi_node_export;

mod nodes;

mod permission_actions;

mod permission_icons;

mod port_ranges;

mod preferences_controls;

mod raw_downloads;

mod reputation;

mod rules_advanced;

mod settings;

mod startup;

mod statistics;

mod subscription_inheritance;

mod subscription_url_test;

mod subscriptions;

mod tray_lifecycle;

mod updates;

mod user_flow;

mod window_memory;

mod url_probe_fixture {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/probe_http.rs"
    ));
}
