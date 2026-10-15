//! GPUI composition. Feature views own their controls and state.
mod connection_details;
mod daemon_preferences;
#[cfg(all(test, feature = "headless-tests"))]
mod headless_tests;
mod history_table;
mod history_tools;
mod history_view;
mod nodes_view;
mod placement;
mod preferences_view;
mod prompt_checksums;
mod prompt_window;
mod rule_editor;
mod rule_transfer;
mod rules_view;
mod statistics_view;
use crate::{backend::Handle, config::Config};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Sizable, Theme, ThemeMode,
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
        ShowNodes
    ]
);

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-f", FocusSearch, Some("Rustleblower")),
        KeyBinding::new("ctrl-1", ShowHistory, Some("Rustleblower")),
        KeyBinding::new("ctrl-2", ShowRules, Some("Rustleblower")),
        KeyBinding::new("ctrl-3", ShowStatistics, Some("Rustleblower")),
        KeyBinding::new("ctrl-4", ShowNodes, Some("Rustleblower")),
    ]);
    daemon_preferences::bind_keys(cx);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    History,
    Rules,
    Statistics,
    Nodes,
}

pub fn run(backend: Handle, config: Config) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            bind_keys(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            let backend = backend.clone();
            let config = config.clone();
            let result = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1100.), px(780.)),
                        cx,
                    ))),
                    window_min_size: Some(size(px(780.), px(560.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Rustleblower".into()),
                        ..Default::default()
                    }),
                    app_id: Some("dev.rustleblower.Rustleblower".into()),
                    ..Default::default()
                },
                cx,
                move |window, cx| cx.new(|cx| Dashboard::new(backend, config, window, cx)),
            );
            if let Err(e) = result {
                eprintln!("Cannot open Rustleblower window: {e:#}");
                cx.quit();
            }
            cx.activate(true);
        });
}

struct Dashboard {
    backend: Handle,
    config: Config,
    history: HistoryView,
    rules: rules_view::RulesView,
    statistics: statistics_view::StatisticsView,
    nodes: nodes_view::NodesView,
    screen: Screen,
    prompt_window: Option<AnyWindowHandle>,
    prompt_id: Option<u64>,
    prompt_position: crate::prompts::placement::Position,
    preferences_window: Option<AnyWindowHandle>,
    focus: FocusHandle,
    next_status: Instant,
}

impl Dashboard {
    fn new(backend: Handle, config: Config, window: &mut Window, cx: &mut Context<Self>) -> Self {
        placement::start(window, cx);
        // Closing the dashboard minimizes it; only the explicit Quit button stops the server.
        window.on_window_should_close(cx, |window, _| {
            window.minimize_window();
            false
        });
        let history = HistoryView::new(&backend.history, window, cx);
        let rules = rules_view::RulesView::new(window, cx);
        let statistics = statistics_view::StatisticsView::new(window, cx);
        let nodes = nodes_view::NodesView::new(window, cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            backend,
            config,
            history,
            rules,
            statistics,
            nodes,
            screen: Screen::History,
            prompt_window: None,
            prompt_id: None,
            prompt_position: Default::default(),
            preferences_window: None,
            focus,
            next_status: Instant::now(),
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.backend.cancel.is_cancelled() {
            cx.quit();
            return;
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
            };
            input.focus_handle(cx).focus(window, cx);
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
        let prompts = self.backend.prompts.snapshot();
        let first = prompts.first().map(|p| p.id);
        if first.is_some()
            && first != self.prompt_id
            && prompts.first().is_some_and(|prompt| {
                prompt.initial.position != crate::prompts::placement::Position::Center
                    || self.prompt_position != crate::prompts::placement::Position::Center
            })
            && placement::supported(window)
            && let Some(handle) = self.prompt_window.take()
        {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        if first.is_some() && self.prompt_window.is_none() {
            match prompt_window::open(
                self.backend.clone(),
                window.scale_factor(),
                placement::supported(window),
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
                let input = match this.screen {
                    Screen::History => &this.history.input,
                    Screen::Rules => &this.rules.input,
                    Screen::Statistics => &this.statistics.input,
                    Screen::Nodes => &this.nodes.input,
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
                            .child(Button::new("quit").label("Quit Rustleblower").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.backend.shutdown();
                                    cx.quit();
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
                    .selected_index(match self.screen { Screen::History => 0, Screen::Rules => 1, Screen::Statistics => 2, Screen::Nodes => 3 })
                    .children([
                        Tab::new().label("History"),
                        Tab::new().label("Application rules"),
                        Tab::new().label("Statistics"),
                        Tab::new().label("Nodes"),
                    ])
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.screen = match *index { 0 => Screen::History, 1 => Screen::Rules, 2 => Screen::Statistics, _ => Screen::Nodes };
                        cx.notify();
                    })),
            )
            .child(match self.screen {
                Screen::History => self.render_history(cx),
                Screen::Rules => self.render_rules(cx),
                Screen::Statistics => self.render_statistics(cx),
                Screen::Nodes => self.render_nodes(cx),
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
                        "{} · {} day history · {} MiB target · closing this window minimizes it",
                        self.config.socket.display(),
                        self.config.history_days,
                        self.config.history_mib
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
