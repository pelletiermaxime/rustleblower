//! One cached primary-monitor work area; no display-server I/O on prompt delivery.
use crate::prompts::placement::{Position, Rect};
use gpui::*;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

#[derive(Clone, Default)]
struct WorkArea {
    rect: Arc<RwLock<Option<Rect>>>,
    native: bool,
}
impl Global for WorkArea {}
#[derive(Clone, Copy)]
struct MoveRequest {
    window: u32,
    position: Position,
    scale: f32,
}
struct Moves(tokio::sync::watch::Sender<Option<MoveRequest>>);
impl Global for Moves {}
pub(super) fn supported(window: &Window) -> bool {
    let handle = window.display_handle().map(|handle| handle.as_raw());
    matches!(
        &handle,
        Ok(RawDisplayHandle::Xcb(_) | RawDisplayHandle::Xlib(_))
    ) || (cfg!(all(test, feature = "headless-tests")) && handle.is_err())
}
pub(super) fn start(window: &Window, cx: &mut App) {
    if cx.has_global::<WorkArea>() || !supported(window) {
        return;
    }
    let cache = WorkArea {
        native: window.display_handle().is_ok(),
        ..Default::default()
    };
    cx.set_global(cache.clone());
    if cfg!(all(test, feature = "headless-tests")) && window.display_handle().is_err() {
        return;
    }
    let (sender, mut receiver) = tokio::sync::watch::channel::<Option<MoveRequest>>(None);
    cx.set_global(Moves(sender));
    let movement_executor = cx.background_executor().clone();
    movement_executor
        .clone()
        .spawn(async move {
            while receiver.changed().await.is_ok() {
                let request = *receiver.borrow_and_update();
                if let Some(request) = request {
                    let diagnostics =
                        std::env::var_os("RUSTLEBLOWER_GRAPHICS_DIAGNOSTICS").is_some();
                    if diagnostics {
                        eprintln!(
                            "Placement request: window={} position={}",
                            request.window,
                            request.position.key()
                        );
                    }
                    for attempt in 0..200 {
                        if receiver.has_changed().unwrap_or(true) {
                            break;
                        }
                        match move_when_mapped(request) {
                            Ok(false) => {
                                if attempt == 199 && diagnostics {
                                    eprintln!(
                                        "Placement request expired before the window was mapped"
                                    );
                                }
                            }
                            Ok(true) => {
                                if diagnostics {
                                    eprintln!("Placement request sent after {attempt} map checks");
                                }
                                break;
                            }
                            Err(error) => {
                                if diagnostics {
                                    eprintln!("Placement request failed: {error:#}");
                                }
                                break;
                            }
                        }
                        movement_executor.timer(Duration::from_millis(25)).await;
                    }
                }
            }
        })
        .detach();
    let executor = cx.background_executor().clone();
    executor
        .clone()
        .spawn(async move {
            loop {
                let area = x11_work_area().ok();
                *cache.rect.write().unwrap() = area;
                executor.timer(Duration::from_secs(5)).await;
                if Arc::strong_count(&cache.rect) == 1 {
                    break;
                }
            }
        })
        .detach();
}
pub(super) fn request(window: &Window, position: Position, cx: &App) {
    let native = HasWindowHandle::window_handle(window).map(|handle| handle.as_raw());
    let id = match native {
        Ok(RawWindowHandle::Xcb(handle)) => handle.window.get(),
        Ok(RawWindowHandle::Xlib(handle)) => handle.window as u32,
        _ => return,
    };
    if let Some(moves) = cx.try_global::<Moves>() {
        moves.0.send_replace(Some(MoveRequest {
            window: id,
            position,
            scale: window.scale_factor(),
        }));
    }
}
fn move_when_mapped(request: MoveRequest) -> anyhow::Result<bool> {
    use x11rb::{
        connection::Connection,
        protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, MapState},
    };
    let (connection, screen) = x11rb::connect(None)?;
    let root = connection.setup().roots[screen].root;
    if connection
        .get_window_attributes(request.window)?
        .reply()?
        .map_state
        != MapState::VIEWABLE
    {
        return Ok(false);
    }
    // XWayland can report Viewable before the WM finishes initial placement.
    // ICCCM NormalState acknowledges a managed, shown top-level window.
    let state_atom = connection.intern_atom(false, b"WM_STATE")?.reply()?.atom;
    let state = connection
        .get_property(false, request.window, state_atom, AtomEnum::ANY, 0, 1)?
        .reply()?
        .value32()
        .and_then(|mut values| values.next());
    if state != Some(1) {
        return Ok(false);
    }
    let pid_atom = connection.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
    let pid = connection
        .get_property(false, request.window, pid_atom, AtomEnum::CARDINAL, 0, 1)?
        .reply()?
        .value32()
        .and_then(|mut values| values.next());
    anyhow::ensure!(
        pid == Some(std::process::id()),
        "Window no longer belongs to this process"
    );
    let work = x11_work_area()?;
    let geometry = connection.get_geometry(request.window)?.reply()?;
    let extents_atom = connection
        .intern_atom(false, b"_NET_FRAME_EXTENTS")?
        .reply()?
        .atom;
    let reply = connection
        .get_property(
            false,
            request.window,
            extents_atom,
            AtomEnum::CARDINAL,
            0,
            4,
        )?
        .reply()?;
    let extents = reply
        .value32()
        .map(|values| values.take(4).collect::<Vec<_>>())
        .unwrap_or_default();
    let horizontal = if extents.len() == 4 {
        extents[0].saturating_add(extents[1])
    } else {
        0
    };
    let vertical = if extents.len() == 4 {
        extents[2].saturating_add(extents[3])
    } else {
        0
    };
    let scale = request.scale;
    anyhow::ensure!(scale.is_finite() && scale > 0., "Invalid window scale");
    let work = Rect {
        x: work.x / scale,
        y: work.y / scale,
        width: work.width / scale,
        height: work.height / scale,
    };
    let target = request
        .position
        .place(
            work,
            (f32::from(geometry.width) + horizontal as f32) / scale,
            (f32::from(geometry.height) + vertical as f32) / scale,
        )
        .ok_or_else(|| anyhow::anyhow!("Invalid work area"))?;
    let atom = connection
        .intern_atom(false, b"_NET_MOVERESIZE_WINDOW")?
        .reply()?
        .atom;
    // NorthWest gravity, x/y present, application source. Leave size unchanged.
    let flags = 1 | (1 << 8) | (1 << 9) | (1 << 12);
    let message = ClientMessageEvent::new(
        32,
        request.window,
        atom,
        [
            flags,
            (target.x * scale).round() as i32 as u32,
            (target.y * scale).round() as i32 as u32,
            0,
            0,
        ],
    );
    // This connection is short-lived: acknowledge delivery before disconnecting.
    connection
        .send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            message,
        )?
        .check()?;
    connection.flush()?;
    Ok(true)
}
pub(super) fn bounds(position: Position, expanded: bool, scale: f32, cx: &App) -> Bounds<Pixels> {
    let desired = if expanded {
        size(px(860.), px(860.))
    } else {
        size(px(650.), px(540.))
    };
    let display = cx.primary_display();
    let scale = if scale.is_finite() && scale > 0. {
        scale
    } else {
        1.
    };
    let cached = cx
        .try_global::<WorkArea>()
        .and_then(|cache| *cache.rect.read().unwrap());
    if cached.is_none()
        && cx
            .try_global::<WorkArea>()
            .is_some_and(|cache| cache.native)
    {
        return Bounds::centered(None, desired, cx);
    }
    let area = cached
        .map(|area| Rect {
            x: area.x / scale,
            y: area.y / scale,
            width: area.width / scale,
            height: area.height / scale,
        })
        .or_else(|| {
            display.as_ref().map(|display| {
                let bounds = display.visible_bounds();
                Rect {
                    x: f32::from(bounds.origin.x),
                    y: f32::from(bounds.origin.y),
                    width: f32::from(bounds.size.width),
                    height: f32::from(bounds.size.height),
                }
            })
        });
    if let Some(area) = area
        && let Some(rect) =
            position.place(area, f32::from(desired.width), f32::from(desired.height))
    {
        Bounds::new(
            point(px(rect.x), px(rect.y)),
            size(px(rect.width), px(rect.height)),
        )
    } else {
        Bounds::centered(None, desired, cx)
    }
}

fn x11_work_area() -> anyhow::Result<Rect> {
    use x11rb::{
        connection::Connection,
        protocol::{
            randr::ConnectionExt as _,
            xproto::{AtomEnum, ConnectionExt as _},
        },
    };
    let (connection, screen) = x11rb::connect(None)?;
    let screen = &connection.setup().roots[screen];
    let root = screen.root;
    let mut monitor = Rect {
        x: 0.,
        y: 0.,
        width: f32::from(screen.width_in_pixels),
        height: f32::from(screen.height_in_pixels),
    };
    if let Ok(cookie) = connection.randr_get_monitors(root, true)
        && let Ok(reply) = cookie.reply()
        && let Some(primary) = reply
            .monitors
            .iter()
            .take(32)
            .find(|monitor| monitor.primary)
            .or_else(|| reply.monitors.first())
    {
        monitor = Rect {
            x: f32::from(primary.x),
            y: f32::from(primary.y),
            width: f32::from(primary.width),
            height: f32::from(primary.height),
        };
    }
    let desktop_atom = connection
        .intern_atom(false, b"_NET_CURRENT_DESKTOP")?
        .reply()?
        .atom;
    let work_atom = connection
        .intern_atom(false, b"_NET_WORKAREA")?
        .reply()?
        .atom;
    let desktop = connection
        .get_property(false, root, desktop_atom, AtomEnum::CARDINAL, 0, 1)?
        .reply()?
        .value32()
        .and_then(|mut values| values.next())
        .unwrap_or(0)
        .min(63);
    let reply = connection
        .get_property(false, root, work_atom, AtomEnum::CARDINAL, desktop * 4, 4)?
        .reply()?;
    if let Some(values) = reply.value32() {
        let values = values.take(4).collect::<Vec<_>>();
        if values.len() == 4 {
            let x = (values[0] as i32 as f32).max(monitor.x);
            let y = (values[1] as i32 as f32).max(monitor.y);
            let right = (values[0] as i32 as f32 + values[2] as f32).min(monitor.x + monitor.width);
            let bottom =
                (values[1] as i32 as f32 + values[3] as f32).min(monitor.y + monitor.height);
            if right > x && bottom > y {
                monitor = Rect {
                    x,
                    y,
                    width: right - x,
                    height: bottom - y,
                };
            }
        }
    }
    Ok(monitor)
}
