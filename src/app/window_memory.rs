//! One coalesced snapshot and one background writer. The startup loader has a
//! bounded UI wait; permission delivery exists even while its read is pending.
use super::{Dashboard, Screen};
use crate::{
    backend::Handle,
    config::Config,
    window_state::{Mode, Page, Rect, State, Store},
};
use futures_util::future::{Either, select};
use gpui::*;
use std::time::Duration;
use tokio::sync::{oneshot, watch};
#[derive(Clone)]
struct Update {
    state: State,
    generation: u64,
    flush: bool,
}
#[derive(Clone, Default)]
struct Status {
    saved: u64,
    #[cfg(all(test, feature = "headless-tests"))]
    writes: u64,
    error: Option<String>,
}
pub(super) struct Client {
    updates: watch::Sender<Option<Update>>,
    status: watch::Receiver<Status>,
    generation: u64,
}
impl Client {
    fn start(
        database: &std::path::Path,
        cx: &App,
    ) -> (Self, oneshot::Receiver<Result<Option<State>, String>>) {
        let store = Store::new(database);
        let (tx, mut rx) = watch::channel::<Option<Update>>(None);
        let (status, read) = watch::channel(Status::default());
        let (ready, loaded) = oneshot::channel();
        let executor = cx.background_executor().clone();
        executor
            .clone()
            .spawn(async move {
                let mut snapshot = match store.load() {
                    Ok(s) => {
                        let _ = ready.send(Ok(s.state));
                        s
                    }
                    Err(e) => {
                        let message = crate::history::bounded(
                            &crate::i18n::tr_args(
                                "Window layout could not be loaded: %{e}",
                                &[("e", &format!("{:#}", e))],
                            ),
                            1024,
                        );
                        let _ = ready.send(Err(message.clone()));
                        status.send_replace(Status {
                            error: Some(message),
                            ..Default::default()
                        });
                        return;
                    }
                };
                #[cfg(all(test, feature = "headless-tests"))]
                let mut writes = 0;
                while rx.changed().await.is_ok() {
                    let Some(mut update) = rx.borrow_and_update().clone() else {
                        continue;
                    };
                    while !update.flush {
                        let changed = matches!(
                            select(
                                Box::pin(rx.changed()),
                                Box::pin(executor.timer(Duration::from_millis(500))),
                            )
                            .await,
                            Either::Left((Ok(()), _))
                        );
                        if !changed {
                            break;
                        }
                        if let Some(next) = rx.borrow_and_update().clone() {
                            update = next;
                        }
                    }
                    match store.save(&snapshot, update.state) {
                        Ok(saved) => {
                            snapshot = saved;
                            #[cfg(all(test, feature = "headless-tests"))]
                            {
                                writes += 1;
                            }
                            status.send_replace(Status {
                                saved: update.generation,
                                #[cfg(all(test, feature = "headless-tests"))]
                                writes,
                                error: None,
                            });
                        }
                        Err(e) => {
                            status.send_replace(Status {
                                saved: 0,
                                #[cfg(all(test, feature = "headless-tests"))]
                                writes,
                                error: Some(crate::history::bounded(
                                    &crate::i18n::tr_args(
                                        "Window layout could not be saved: %{e}",
                                        &[("e", &format!("{:#}", e))],
                                    ),
                                    1024,
                                )),
                            });
                            break;
                        }
                    }
                }
            })
            .detach();
        (
            Self {
                updates: tx,
                status: read,
                generation: 0,
            },
            loaded,
        )
    }
    pub fn record(&mut self, state: State, flush: bool) {
        self.generation += 1;
        self.updates.send_replace(Some(Update {
            state,
            generation: self.generation,
            flush,
        }));
    }
    pub fn finish(&self) -> impl std::future::Future<Output = ()> + use<> {
        let target = self.generation;
        let mut status = self.status.clone();
        async move {
            while status.borrow().saved < target && status.borrow().error.is_none() {
                if status.changed().await.is_err() {
                    break;
                }
            }
        }
    }
    pub fn error(&self) -> Option<String> {
        self.status.borrow().error.clone()
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub fn test_saved(&self) -> (u64, u64, u64) {
        let s = self.status.borrow();
        (s.saved, s.writes, self.generation)
    }
}
pub(super) fn sample(window: &Window, page: Screen) -> State {
    sample_bounds(window.window_bounds(), page)
}
pub(super) fn sample_bounds(wb: WindowBounds, page: Screen) -> State {
    let b = wb.get_bounds();
    State {
        version: 1,
        bounds: Rect {
            x: b.origin.x.into(),
            y: b.origin.y.into(),
            width: b.size.width.into(),
            height: b.size.height.into(),
        },
        mode: match wb {
            WindowBounds::Windowed(_) => Mode::Windowed,
            WindowBounds::Maximized(_) => Mode::Maximized,
            WindowBounds::Fullscreen(_) => Mode::Fullscreen,
        },
        page: page.into(),
    }
}
fn bounds(state: State) -> WindowBounds {
    let r = state.bounds;
    let b = Bounds::new(point(px(r.x), px(r.y)), size(px(r.width), px(r.height)));
    match state.mode {
        Mode::Windowed => WindowBounds::Windowed(b),
        Mode::Maximized => WindowBounds::Maximized(b),
        Mode::Fullscreen => WindowBounds::Fullscreen(b),
    }
}
fn fit(state: State, cx: &App) -> State {
    let mut displays = Vec::new();
    if let Some(d) = cx.primary_display() {
        displays.push(d.bounds());
    }
    for d in cx.displays() {
        let b = d.bounds();
        if displays.len() < 16 && !displays.contains(&b) {
            displays.push(b);
        }
    }
    let rects = displays
        .iter()
        .map(|b| Rect {
            x: b.origin.x.into(),
            y: b.origin.y.into(),
            width: b.size.width.into(),
            height: b.size.height.into(),
        })
        .collect::<Vec<_>>();
    state.fit(&rects).unwrap_or(state)
}
pub(super) fn fit_bounds(wb: WindowBounds, cx: &App) -> WindowBounds {
    bounds(fit(sample_bounds(wb, Screen::History), cx))
}
pub(super) fn open(
    backend: Handle,
    config: Config,
    cx: &mut App,
) -> gpui::Result<Entity<Dashboard>> {
    open_inner(backend, config, None, cx)
}
#[cfg(all(test, feature = "headless-tests"))]
pub(super) fn open_delayed(
    backend: Handle,
    config: Config,
    loaded: oneshot::Receiver<Result<Option<State>, String>>,
    cx: &mut App,
) -> gpui::Result<Entity<Dashboard>> {
    open_inner(backend, config, Some(loaded), cx)
}
fn open_inner(
    backend: Handle,
    config: Config,
    override_loaded: Option<oneshot::Receiver<Result<Option<State>, String>>>,
    cx: &mut App,
) -> gpui::Result<Entity<Dashboard>> {
    let (memory, loaded) = Client::start(&config.database, cx);
    let loaded = override_loaded.unwrap_or(loaded);
    let (bootstrap, view) = gpui_kit::open_window(
        WindowOptions {
            show: false,
            focus: false,
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Rustleblower — initializing").into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1100.), px(780.)),
                cx,
            ))),
            ..super::dashboard_options()
        },
        cx,
        |window, cx| {
            cx.new(|cx| {
                let mut view = Dashboard::new(backend, config, window, cx);
                view.memory = Some(memory);
                view.window_ready = false;
                view
            })
        },
    )?;
    crate::i18n::register_title(bootstrap, "Rustleblower — initializing", cx);
    let weak = view.downgrade();
    let timer = cx.background_executor().timer(Duration::from_millis(250));
    cx.spawn(async move |cx| {
        let saved = match select(Box::pin(loaded), Box::pin(timer)).await {
            Either::Left((Ok(Ok(state)), _)) => state,
            _ => None,
        };
        cx.update(|cx| {
            if let Some(view) = weak.upgrade() {
                let ready = view.update(cx, |v, cx| {
                    if v.quitting {
                        return false;
                    }
                    if let Some(state) = saved {
                        let state = fit(state, cx);
                        v.main_bounds = bounds(state);
                        v.screen = state.page.into();
                    }
                    v.main_window = None;
                    v.opening_initial = true;
                    true
                });
                if ready {
                    let _ = bootstrap.update(cx, |_, window, _| window.remove_window());
                    Dashboard::open_dashboard(view, cx);
                }
            }
        });
    })
    .detach();
    Ok(view)
}
impl From<Screen> for Page {
    fn from(s: Screen) -> Self {
        match s {
            Screen::History => Self::History,
            Screen::Rules => Self::Rules,
            Screen::Statistics => Self::Statistics,
            Screen::Bandwidth => Self::Bandwidth,
            Screen::Nodes => Self::Nodes,
            Screen::Alerts => Self::Alerts,
            Screen::Live => Self::Live,
            Screen::Subscriptions => Self::Subscriptions,
        }
    }
}
impl From<Page> for Screen {
    fn from(s: Page) -> Self {
        match s {
            Page::History => Self::History,
            Page::Rules => Self::Rules,
            Page::Statistics => Self::Statistics,
            Page::Bandwidth => Self::Bandwidth,
            Page::Nodes => Self::Nodes,
            Page::Alerts => Self::Alerts,
            Page::Live => Self::Live,
            Page::Subscriptions => Self::Subscriptions,
        }
    }
}
