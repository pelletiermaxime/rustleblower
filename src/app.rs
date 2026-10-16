//! GPUI composition. Feature views own their controls and state.
mod alerts_view;
mod app_icon;
mod appearance;
mod brand_icons;
mod connection_details;
mod daemon_preferences;
mod desktop_notifications;
mod firewall_view;
#[cfg(all(test, feature = "headless-tests"))]
mod headless_tests;
mod history_memory;
mod history_preferences;
mod history_table;
mod history_tools;
mod history_view;
mod live_view;
mod nodes_view;
mod placement;
mod preferences_view;
mod prompt_checksums;
mod prompt_window;
mod rule_distribution;
mod rule_editor;
mod rule_exports;
mod rule_transfer;
mod rules_view;
mod startup_view;
mod statistics_view;
mod subscriptions_view;
mod tray;
mod window_memory;
use crate::{backend::Handle, config::Config};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Sizable,
    button::Button,
    tab::{Tab, TabBar},
};
use history_view::HistoryView;
use std::time::{Duration, Instant};

actions!(
    rustleblower,
    [
        FocusSearch,
        ShowHistory,
        ShowRules,
        ShowStatistics,
        ShowNodes,
        ShowAlerts,
        ShowLive,
        ShowSubscriptions,
        ShowCollections,
        BulkSubscriptions,
        TransferSubscriptions,
        NewSubscription,
        SaveSubscription,
        LiveStart,
        LivePause
    ]
);

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-f", FocusSearch, Some("Rustleblower")),
        KeyBinding::new("ctrl-1", ShowHistory, Some("Rustleblower")),
        KeyBinding::new("ctrl-2", ShowRules, Some("Rustleblower")),
        KeyBinding::new("ctrl-3", ShowStatistics, Some("Rustleblower")),
        KeyBinding::new("ctrl-4", ShowNodes, Some("Rustleblower")),
        KeyBinding::new("ctrl-5", ShowAlerts, Some("Rustleblower")),
        KeyBinding::new("ctrl-6", ShowLive, Some("Rustleblower")),
        KeyBinding::new("ctrl-7", ShowSubscriptions, Some("Rustleblower")),
        KeyBinding::new("ctrl-shift-l", ShowCollections, Some("Rustleblower")),
        KeyBinding::new("ctrl-shift-b", BulkSubscriptions, Some("Rustleblower")),
        KeyBinding::new("ctrl-shift-t", TransferSubscriptions, Some("Rustleblower")),
        KeyBinding::new("ctrl-n", NewSubscription, Some("Rustleblower")),
        KeyBinding::new("ctrl-s", SaveSubscription, Some("Rustleblower")),
        KeyBinding::new("ctrl-enter", LiveStart, Some("Rustleblower")),
        KeyBinding::new("ctrl-shift-enter", LivePause, Some("Rustleblower")),
    ]);
    daemon_preferences::bind_keys(cx);
    history_preferences::bind_keys(cx);
    firewall_view::bind_keys(cx);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    History,
    Rules,
    Statistics,
    Nodes,
    Alerts,
    Live,
    Subscriptions,
}

struct DashboardOwner {
    _dashboard: Entity<Dashboard>,
}
impl Global for DashboardOwner {}

fn dashboard_options() -> WindowOptions {
    WindowOptions {
        window_min_size: Some(size(px(780.), px(560.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Rustleblower".into()),
            ..Default::default()
        }),
        app_id: Some("dev.rustleblower.Rustleblower".into()),
        icon: Some(brand_icons::window_icon()),
        ..Default::default()
    }
}

pub fn run(backend: Handle, config: Config) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx: &mut App| {
            cx.set_quit_mode(QuitMode::Explicit);
            gpui_kit::init(cx);
            bind_keys(cx);
            appearance::init(&config.database, cx);
            let backend = backend.clone();
            let config = config.clone();
            let result = window_memory::open(backend, config, cx);
            match result {
                Ok(dashboard) => {
                    dashboard.update(cx, |view, _| view.tray = Some(tray::Client::start()));
                    dashboard.update(cx, |view, _| {
                        view.desktop_notifications =
                            desktop_notifications::Client::start(&view.backend)
                    });
                    cx.set_global(DashboardOwner {
                        _dashboard: dashboard,
                    });
                }
                Err(e) => {
                    eprintln!("Cannot open Rustleblower window: {e:#}");
                    cx.quit();
                }
            }
        });
}

struct Dashboard {
    backend: Handle,
    config: Config,
    main_window: Option<AnyWindowHandle>,
    main_bounds: WindowBounds,
    memory: Option<window_memory::Client>,
    last_window_state: Option<crate::window_state::State>,
    window_ready: bool,
    opening_initial: bool,
    quitting: bool,
    prompt_scale: f32,
    placement_supported: bool,
    tray: Option<tray::Client>,
    desktop_notifications: Option<desktop_notifications::Client>,
    history: HistoryView,
    rules: rules_view::RulesView,
    statistics: statistics_view::StatisticsView,
    nodes: nodes_view::NodesView,
    alerts: alerts_view::AlertsView,
    live: live_view::LiveView,
    subscriptions: subscriptions_view::SubscriptionsView,
    screen: Screen,
    prompt_window: Option<AnyWindowHandle>,
    prompt_id: Option<u64>,
    prompt_position: crate::prompts::placement::Position,
    preferences_window: Option<AnyWindowHandle>,
    startup_window: Option<AnyWindowHandle>,
    appearance_window: Option<AnyWindowHandle>,
    focus: FocusHandle,
    next_status: Instant,
}

impl Dashboard {
    fn new(backend: Handle, config: Config, window: &mut Window, cx: &mut Context<Self>) -> Self {
        placement::start(window, cx);
        Self::install_close_handler(window, cx.weak_entity(), cx);
        let history = HistoryView::new(&backend.history, window, cx);
        let rules = rules_view::RulesView::new(window, cx);
        let statistics = statistics_view::StatisticsView::new(window, cx);
        let nodes = nodes_view::NodesView::new(window, cx);
        let alerts = alerts_view::AlertsView::new(window, cx);
        let live = live_view::LiveView::new(window, cx);
        let subscriptions = subscriptions_view::SubscriptionsView::new(window, cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                if this
                    .update(cx, |view, cx| view.tick_background(cx))
                    .is_err()
                {
                    break;
                }
                let handle = this
                    .read_with(cx, |view, _| view.main_window)
                    .ok()
                    .flatten();
                if let Some(handle) = handle {
                    let _ = handle.update(cx, |_, window, cx| {
                        let _ = this.update(cx, |view, cx| view.tick(window, cx));
                    });
                }
            }
        })
        .detach();
        Self {
            backend,
            config,
            main_window: Some(window.window_handle()),
            main_bounds: window.window_bounds(),
            memory: None,
            last_window_state: None,
            window_ready: true,
            opening_initial: false,
            quitting: false,
            prompt_scale: window.scale_factor(),
            placement_supported: placement::supported(window),
            tray: None,
            desktop_notifications: None,
            history,
            rules,
            statistics,
            nodes,
            alerts,
            live,
            subscriptions,
            screen: Screen::History,
            prompt_window: None,
            prompt_id: None,
            prompt_position: Default::default(),
            preferences_window: None,
            startup_window: None,
            appearance_window: None,
            focus,
            next_status: Instant::now(),
        }
    }

    fn should_close_dashboard(&mut self, window: &mut Window) -> bool {
        self.live.pause();
        self.subscriptions.cancel_url_test();
        self.history.remember(true);
        self.remember_window(window, true);
        if self.tray.as_ref().is_some_and(tray::Client::available) {
            self.main_bounds = window.window_bounds();
            self.main_window = None;
            true
        } else {
            window.minimize_window();
            false
        }
    }

    fn install_close_handler(window: &Window, view: WeakEntity<Self>, cx: &App) {
        window.on_window_should_close(cx, move |window, cx| {
            view.update(cx, |view, _| view.should_close_dashboard(window))
                .unwrap_or(true)
        });
    }

    fn show_dashboard(&self, cx: &mut Context<Self>) {
        let view = cx.entity();
        // Root::new reads the dashboard's Focusable implementation. Build it
        // after releasing this entity's mutable update borrow.
        cx.defer(move |cx| Self::open_dashboard(view, cx));
    }

    fn open_dashboard(view: Entity<Self>, cx: &mut App) {
        let dashboard = view.read(cx);
        if let Some(handle) = dashboard.main_window {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
            return;
        }
        let focus = dashboard.focus.clone();
        let restored = if dashboard.memory.is_some() {
            window_memory::fit_bounds(dashboard.main_bounds, cx)
        } else {
            dashboard.main_bounds
        };
        let should_focus =
            !dashboard.opening_initial || dashboard.backend.prompts.snapshot().is_empty();
        let options = WindowOptions {
            window_bounds: Some(restored),
            focus: should_focus,
            ..dashboard_options()
        };
        let content = view.clone();
        match gpui_kit::open_window(options, cx, move |window, cx| {
            Self::install_close_handler(window, content.downgrade(), cx);
            focus.focus(window, cx);
            content
        }) {
            Ok((handle, _)) => {
                view.update(cx, |view, cx| {
                    view.main_window = Some(handle);
                    view.window_ready = true;
                    view.opening_initial = false;
                    view.main_bounds = restored;
                    view.last_window_state =
                        Some(window_memory::sample_bounds(restored, view.screen));
                    cx.notify();
                });
                if should_focus {
                    cx.activate(true);
                }
            }
            Err(error) => eprintln!("Cannot reopen Rustleblower: {error:#}"),
        }
    }

    fn remember_window(&mut self, window: &Window, flush: bool) {
        if !self.window_ready {
            return;
        }
        let state = window_memory::sample(window, self.screen);
        self.main_bounds = window.window_bounds();
        let changed = self.last_window_state.is_some_and(|old| old != state);
        self.last_window_state = Some(state);
        if (changed || flush)
            && let Some(memory) = &mut self.memory
        {
            memory.record(state, flush);
        }
    }
    fn quit(&mut self, window: Option<&Window>, cx: &mut Context<Self>) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        self.history.remember(true);
        if let Some(window) = window {
            self.remember_window(window, true);
        } else if let Some(state) = self.last_window_state
            && let Some(memory) = &mut self.memory
        {
            memory.record(state, true);
        }
        self.backend.shutdown();
        {
            let done = self.memory.as_ref().map(window_memory::Client::finish);
            let history_done = self.history.memory.finish();
            cx.spawn(async move |_, cx| {
                let timer = cx.background_executor().timer(Duration::from_millis(500));
                let both = async move {
                    if let Some(done) = done {
                        done.await;
                    }
                    history_done.await;
                };
                let _ = futures_util::future::select(Box::pin(both), Box::pin(timer)).await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    }
    fn tick_background(&mut self, cx: &mut Context<Self>) {
        if self.screen != Screen::Subscriptions || self.main_window.is_none() {
            self.subscriptions.cancel_url_test();
        }
        if self.screen != Screen::Live || self.main_window.is_none() {
            self.live.pause();
        }
        if self.backend.cancel.is_cancelled() && !self.quitting {
            self.quit(None, cx);
            return;
        }
        if self.quitting {
            return;
        }
        if self
            .desktop_notifications
            .as_mut()
            .is_some_and(|notices| notices.open_requested())
        {
            self.screen = Screen::Alerts;
            self.show_dashboard(cx);
            cx.notify();
        }
        if let Some(tray) = &self.tray {
            match tray.action() {
                Some(tray::Action::Quit) => {
                    self.quit(None, cx);
                    return;
                }
                Some(tray::Action::Open) => self.show_dashboard(cx),
                Some(tray::Action::Startup) => self.open_startup(cx),
                None => {}
            }
            if self.main_window.is_none()
                && self.tray.as_ref().is_some_and(|tray| !tray.available())
            {
                // Losing the panel must never strand a hidden application.
                self.show_dashboard(cx);
            }
        }
        let prompts = self.backend.prompts.snapshot();
        let first = prompts.first().map(|p| p.id);
        if first.is_some()
            && first != self.prompt_id
            && prompts.first().is_some_and(|prompt| {
                prompt.initial.position != crate::prompts::placement::Position::Center
                    || self.prompt_position != crate::prompts::placement::Position::Center
            })
            && self.placement_supported
            && let Some(handle) = self.prompt_window.take()
        {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        if first.is_some() && self.prompt_window.is_none() {
            match prompt_window::open(
                self.backend.clone(),
                self.prompt_scale,
                self.placement_supported,
                cx,
            ) {
                Ok(handle) => {
                    self.prompt_window = Some(handle);
                }
                Err(e) => {
                    self.backend.status.lock().unwrap().error =
                        Some(format!("Cannot show permission window: {e:#}"));
                }
            }
        }
        if first != self.prompt_id {
            if let Some(handle) = self.prompt_window {
                if first.is_none() {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                    self.prompt_window = None;
                } else {
                    let _ = handle.update(cx, |_, window, _| window.activate_window());
                }
            }
            self.prompt_id = first;
            self.prompt_position = prompts
                .first()
                .map(|prompt| prompt.initial.position)
                .unwrap_or_default();
        }
    }

    fn open_startup(&mut self, cx: &mut Context<Self>) {
        if let Some(window) = self.startup_window
            && window.update(cx, |_, w, _| w.activate_window()).is_ok()
        {
            return;
        }
        match startup_view::open(self.config.clone(), cx) {
            Ok(window) => self.startup_window = Some(window),
            Err(e) => {
                self.backend.status.lock().unwrap().error =
                    Some(format!("Cannot open startup preferences: {e:#}"))
            }
        }
        cx.notify();
    }
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_window(window, false);
        self.prompt_scale = window.scale_factor();
        self.placement_supported = placement::supported(window);
        if self.backend.cancel.is_cancelled() {
            return;
        }
        if let Some(window) = self.appearance_window
            && window.update(cx, |_, _, _| {}).is_err()
        {
            self.appearance_window = None;
        }
        if let Some(window) = self.startup_window
            && window.update(cx, |_, _, _| {}).is_err()
        {
            self.startup_window = None;
        }
        if let Some(preferences) = self.preferences_window
            && preferences.update(cx, |_, _, _| {}).is_err()
        {
            self.preferences_window = None;
        }
        if let Some(editor) = self.rules.editor_window
            && editor.update(cx, |_, _, _| {}).is_err()
        {
            self.rules.editor_window = None;
            self.rules.editor_closed();
            let input = match self.screen {
                Screen::History => &self.history.input,
                Screen::Rules => &self.rules.input,
                Screen::Statistics => &self.statistics.input,
                Screen::Nodes => &self.nodes.input,
                Screen::Alerts => &self.alerts.input,
                Screen::Live => &self.live.input,
                Screen::Subscriptions => self.subscriptions.search_input(),
            };
            if self.screen == Screen::Subscriptions {
                self.focus_subscription_search(window, cx);
            } else {
                input.focus_handle(cx).focus(window, cx);
            }
            cx.notify();
        }
        if let Some(details) = self.history.detail_window
            && details.update(cx, |_, _, _| {}).is_err()
        {
            self.history.detail_window = None;
            cx.notify();
        }
        if let Ok(action) = self.history.actions_rx.try_recv() {
            let result = if self.backend.session(action.session).is_some_and(|session| {
                session.id == action.session
                    && session.name == action.node
                    && session.notifications
                    && self.backend.history.health.catalogues.ready(session.id)
            }) {
                if let Some(editor) = self.rules.editor_window {
                    let _ = editor.update(cx, |_, window, _| window.activate_window());
                    Err("Close the current rule window before opening another rule".to_string())
                } else {
                    let original = action.existing.then(|| action.rule.clone());
                    match rule_editor::open(
                        self.backend.clone(),
                        action.session,
                        action.rule,
                        original,
                        cx,
                    ) {
                        Ok(editor) => {
                            self.rules.editor_window = Some(editor);
                            Ok(())
                        }
                        Err(error) => Err(format!("Cannot open rule editor: {error:#}")),
                    }
                }
            } else {
                Err("This event's daemon changed or disconnected; reopen the rule editor after reconnecting".to_string())
            };
            if let Err(error) = &result {
                self.history.error = Some(error.clone());
            }
            let _ = action.reply.send(result);
            cx.notify();
        }
        if let Ok(action) = self.history.processes_rx.try_recv() {
            let result = self
                .live
                .inspect_history(&self.backend, action.target, window, cx);
            match &result {
                Ok(()) => {
                    self.screen = Screen::Live;
                    window.activate_window();
                }
                Err(error) => self.history.error = Some(error.clone()),
            }
            let _ = action.reply.send(result);
            cx.notify();
        }
        if self.screen == Screen::Subscriptions
            && self
                .subscriptions
                .poll(&self.backend.subscriptions, window, cx)
        {
            cx.notify();
        }
        if self.screen == Screen::Live && self.live.poll(&self.backend, window, cx) {
            cx.notify();
        }
        if self.screen == Screen::Alerts && self.alerts.poll(&self.backend.history, cx) {
            cx.notify();
        }
        if self.screen == Screen::History && self.history.poll(&self.backend.history, window, cx) {
            cx.notify();
        }
        if self.screen == Screen::Statistics
            && self.statistics.poll(&self.backend.history, window, cx)
        {
            cx.notify();
        }
        if self.screen == Screen::Nodes && self.nodes.poll(&self.backend, window, cx) {
            cx.notify();
        }
        if self.screen == Screen::Rules
            && self.rules.poll(
                &self.backend.history,
                self.backend.status().session.as_ref(),
                window,
                cx,
            )
        {
            cx.notify();
        }
        if Instant::now() >= self.next_status {
            self.next_status = Instant::now() + Duration::from_secs(1);
            cx.notify();
        }
    }
}

impl Render for Dashboard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.backend.status();
        let connected = state
            .session
            .as_ref()
            .is_some_and(|s| s.last_seen.elapsed() < Duration::from_secs(5));
        let summary = state
            .session
            .as_ref()
            .map(|s| {
                format!(
                    "{} · OpenSnitch {} · {} rules{}",
                    s.name,
                    if s.version.is_empty() {
                        "connecting"
                    } else {
                        &s.version
                    },
                    s.rules,
                    if connected { "" } else { " · heartbeat stale" }
                )
            })
            .unwrap_or_else(|| "Waiting for the OpenSnitch daemon".into());
        let history_warning = self.backend.history.health.warning();
        let db_error = self.backend.history.health.error.lock().unwrap().clone();
        let pending = self.backend.prompts.snapshot().len();
        let (popup_preferences, popup_timeout, popup_fallback) = self.backend.popup_policy();
        let popups_disabled = !popup_preferences.popups;
        let error = state.error.or(db_error);
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus)
            .key_context("Rustleblower")
            .on_action(cx.listener(|this,_:&LiveStart,window,cx|{if this.screen==Screen::Subscriptions{if this.subscriptions.has_transfer(){this.review_subscription_import(window,cx);}else{this.review_subscription_bulk(window,cx);}}else if this.screen==Screen::Live{this.live.start(&this.backend,cx);cx.notify();}}))
            .on_action(cx.listener(|this,_:&LivePause,_,cx|{if this.screen==Screen::Subscriptions{if this.subscriptions.has_transfer(){this.apply_subscription_import(cx);}else{this.apply_subscription_bulk(cx);}}else if this.screen==Screen::Live{this.live.pause();cx.notify();}}))
            .on_action(cx.listener(|this,_:&TransferSubscriptions,window,cx|{if this.screen==Screen::Subscriptions{this.open_subscription_transfer(window,cx);}}))
            .on_action(cx.listener(|this,_:&BulkSubscriptions,window,cx|{if this.screen==Screen::Subscriptions{this.open_subscription_bulk(window,cx);}}))
            .on_action(cx.listener(|this,_:&NewSubscription,window,cx|{if this.screen==Screen::Subscriptions{this.new_subscription(window,cx);}}))
            .on_action(cx.listener(|this,_:&SaveSubscription,window,cx|{if this.screen==Screen::Subscriptions{this.subscription_save(window,cx);}}))
            .on_action(cx.listener(|this,_:&ShowCollections,window,cx|{if this.screen==Screen::Subscriptions{this.open_collections(window,cx);}}))
            .on_action(cx.listener(|this,_:&ShowSubscriptions,window,cx|{this.screen=Screen::Subscriptions;this.focus_subscription_search(window,cx);cx.notify();}))
            .on_action(cx.listener(|this,_:&ShowLive,window,cx|{this.screen=Screen::Live;this.live.input.focus_handle(cx).focus(window,cx);cx.notify();}))
            .on_action(cx.listener(|this,_:&ShowAlerts,window,cx|{this.screen=Screen::Alerts;this.alerts.input.focus_handle(cx).focus(window,cx);cx.notify();}))
            .on_action(cx.listener(|this, _: &ShowNodes, window,cx|{this.screen=Screen::Nodes;this.nodes.input.focus_handle(cx).focus(window,cx);cx.notify();}))
            .on_action(cx.listener(|this, _: &ShowHistory, window, cx| {
                this.screen = Screen::History;
                this.history.input.focus_handle(cx).focus(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowRules, window, cx| {
                this.screen = Screen::Rules;
                this.rules.input.focus_handle(cx).focus(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowStatistics, window, cx| {
                this.screen = Screen::Statistics;
                this.statistics.input.focus_handle(cx).focus(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                if this.screen==Screen::Subscriptions{this.focus_subscription_search(window,cx);return;}
                let input = match this.screen {
                    Screen::History => &this.history.input,
                    Screen::Rules => &this.rules.input,
                    Screen::Statistics => &this.statistics.input,
                    Screen::Nodes => &this.nodes.input,
                    Screen::Alerts => &this.alerts.input,
                    Screen::Live => &this.live.input,
                    Screen::Subscriptions=>this.subscriptions.search_input(),
                };
                input.focus_handle(cx).focus(window, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_6()
                    .py_4()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Rustleblower"),
                            )
                            .child(div().text_sm().text_color(muted).child(summary)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                Button::new("preferences")
                                    .label("Popup preferences…")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(handle) = this.preferences_window {
                                        let _ = handle
                                            .update(cx, |_, window, _| window.activate_window());
                                    } else {
                                        match preferences_view::open(this.backend.clone(), cx) {
                                            Ok(handle) => this.preferences_window = Some(handle),
                                            Err(error) => {
                                                this.backend.status.lock().unwrap().error =
                                                    Some(format!(
                                                        "Cannot open popup preferences: {error:#}"
                                                    ))
                                            }
                                        }
                                    }
                                })),
                            )
                            .child(Button::new("startup-preferences").label("Startup…").on_click(cx.listener(|this,_,_,cx|this.open_startup(cx))))
                            .child(Button::new("appearance-preferences").label("Appearance…").on_click(cx.listener(|this,_,_,cx|{
                                if let Some(window)=this.appearance_window && window.update(cx,|_,w,_|w.activate_window()).is_ok(){return;}
                                match appearance::open(&this.config.database,cx){Ok(w)=>this.appearance_window=Some(w),Err(e)=>this.backend.status.lock().unwrap().error=Some(format!("Cannot open appearance: {e:#}"))}cx.notify();
                            })))
                            .child(Button::new("quit").label("Quit Rustleblower").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.quit(Some(window),cx);
                                }),
                            )),
                    ),
            )
            .child(
                div()
                    .px_6()
                    .py_3()
                    .flex()
                    .justify_between()
                    .text_sm()
                    .child(format!(
                        "{} pending · {} answered · {} timed out · {} overflow",
                        pending, state.answered, state.timed_out, state.overloaded
                    ))
                    .child(format!(
                        "Unanswered: {} once after {}s",
                        popup_fallback.action(),
                        popup_timeout
                    )),
            )
            .when_some(self.memory.as_ref().and_then(window_memory::Client::error),|v,error|v.child(banner(error,cx)))
            .when_some(history_warning, |v, warning| v.child(banner(warning, cx)))
            .when(popups_disabled, |view|view.child(banner(format!("Permission popups are disabled. New requests receive {} once; change this in Popup preferences.",popup_fallback.action()),cx)))
            .when_some(error, |v, e| v.child(banner(e, cx)))
            .when_some(state.last_alert, |v, e| {
                v.child(div().px_6().py_2().text_sm().text_color(muted).child(e))
            })
            .child(
                TabBar::new("main-tabs")
                    .small()
                    .underline()
                    .px_6()
                    .selected_index(match self.screen { Screen::History => 0, Screen::Rules => 1, Screen::Statistics => 2, Screen::Nodes => 3, Screen::Alerts => 4, Screen::Live => 5, Screen::Subscriptions=>6 })
                    .children([
                        Tab::new().label("History"),
                        Tab::new().label("Application rules"),
                        Tab::new().label("Statistics"),
                        Tab::new().label("Nodes"),
                        Tab::new().label("Alerts"),
                        Tab::new().label("Live"),
                        Tab::new().label("Blocklists"),
                    ])
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.screen = match *index { 0 => Screen::History, 1 => Screen::Rules, 2 => Screen::Statistics, 3 => Screen::Nodes, 4 => Screen::Alerts, 5=>Screen::Live, _=>Screen::Subscriptions };
                        cx.notify();
                    })),
            )
            .child(match self.screen {
                Screen::History => self.render_history(cx),
                Screen::Rules => self.render_rules(cx),
                Screen::Statistics => self.render_statistics(cx),
                Screen::Nodes => self.render_nodes(cx),
                Screen::Alerts => self.render_alerts(cx),
                Screen::Live => self.render_live(cx),
                Screen::Subscriptions=>self.render_subscriptions(cx),
            })
            .child(
                div()
                    .px_6()
                    .py_2()
                    .text_xs()
                    .text_color(muted)
                    .border_t_1()
                    .border_color(border)
                    .child(format!(
                        "{} · {} day history · {} MiB target · {}",
                        self.config.socket.display(),
                        self.backend.history.storage.snapshot().config.days,
                        self.backend.history.storage.snapshot().config.mib,
                        if self.tray.as_ref().is_some_and(tray::Client::available) {
                            "closing this window hides it to the system tray"
                        } else {
                            "closing this window minimizes it (system tray unavailable)"
                        }
                    )),
            )
    }
}

fn banner(message: String, cx: &App) -> Div {
    div()
        .px_6()
        .py_2()
        .bg(cx.theme().warning.opacity(0.15))
        .text_color(cx.theme().warning)
        .text_sm()
        .child(message)
}
