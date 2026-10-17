//! Application rules: bounded pages and explicit acknowledged policy changes.
mod cached_nodes;
mod filters;
use super::Dashboard;
use crate::{
    backend::{Handle, Session},
    protocol::{Action, Operator, Rule},
    rules::{Page, Query},
};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    scroll::{Scrollbar, ScrollbarMode},
    select::Select,
};
use std::{
    sync::{
        atomic::Ordering,
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

const ADDED_COLUMN_WIDTH: f32 = 156.;
const RULE_STATUS_COLUMN_WIDTH: f32 = 312.;

fn added_date(created: i64) -> String {
    (created > 0)
        .then(|| chrono::DateTime::from_timestamp(created, 0))
        .flatten()
        .map(|date| date.format("%Y-%m-%d\n%H:%M:%S").to_string())
        .unwrap_or_else(|| crate::i18n::tr("Unknown"))
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_secs()).ok())
        .unwrap_or(0)
}
fn cached_warning(status: crate::rules::catalogue_policy::Status) -> String {
    use crate::rules::catalogue_policy::Status;
    match status {
        Status::ExpiredByAge => crate::i18n::tr("Appears expired"),
        Status::ExpiredAtUiRestart => crate::i18n::tr("May have expired (previous UI session)"),
        Status::DaemonDisabled | Status::Active | Status::Unknown => String::new(),
    }
}
pub(super) struct RulesView {
    cache_nodes: cached_nodes::State,
    pub input: Entity<InputState>,
    pub executable: Entity<InputState>,
    pub name: Entity<InputState>,
    pub page: Page,
    query: Query,
    session: u64,
    online: bool,
    selected: Option<Rule>,
    marked: std::collections::BTreeSet<String>,
    generation: u64,
    pending_query: Option<(u64, oneshot::Receiver<Result<Page, String>>)>,
    pending_change: Option<oneshot::Receiver<Result<(), String>>>,
    dirty: bool,
    due: Instant,
    revision: u64,
    policy_ready: bool,
    policy_warning: Option<String>,
    pub(super) error: Option<String>,
    message: Option<String>,
    creating: bool,
    deleting: bool,
    scroll: UniformListScrollHandle,
    pub(super) editor_window: Option<AnyWindowHandle>,
    pub(super) filter_window: Option<AnyWindowHandle>,
    filter_sender: SyncSender<filters::Update>,
    filter_receiver: Receiver<filters::Update>,
}

impl RulesView {
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn test_query_ready(&self) -> bool {
        self.pending_query.is_none() && (!self.dirty || self.query.node.is_empty())
    }

    pub(super) fn editor_closed(&mut self) {
        self.creating = false;
        self.page = Page::default();
        self.marked.clear();
        self.invalidate();
    }
    pub fn new(window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "Filter rule name, application or description…",
                    window,
                    cx,
                ))
                .validate(|value, _| value.len() <= 256)
        });
        let executable = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::i18n::placeholder(
                "/absolute/path/to/application",
                window,
                cx,
            ))
        });
        let name = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::i18n::placeholder(
                "Rule name (optional)",
                window,
                cx,
            ))
        });
        cx.subscribe(&input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.rules.query.search = input.read(cx).value().to_string();
                this.rules.reset();
                cx.notify();
            }
        })
        .detach();
        let (filter_sender, filter_receiver) = mpsc::sync_channel(1);
        let cache_nodes = cached_nodes::State::new(window, cx);
        Self {
            cache_nodes,
            input,
            executable,
            name,
            page: Page::default(),
            query: Query::default(),
            session: 0,
            online: false,
            selected: None,
            marked: Default::default(),
            generation: 0,
            pending_query: None,
            pending_change: None,
            dirty: false,
            due: Instant::now(),
            revision: 0,
            policy_ready: false,
            policy_warning: None,
            error: None,
            message: None,
            creating: false,
            deleting: false,
            scroll: UniformListScrollHandle::new(),
            editor_window: None,
            filter_window: None,
            filter_sender,
            filter_receiver,
        }
    }

    fn reset(&mut self) {
        self.query.after.clear();
        self.marked.clear();
        self.invalidate();
    }
    fn frozen_query(&self) -> Query {
        let mut query = self.query.clone();
        if let Some(policy) = query.policy.as_mut() {
            policy.now_unix = unix_now();
        }
        query
    }
    fn invalidate(&mut self) {
        self.generation += 1;
        self.dirty = true;
        self.selected = None;
        self.deleting = false;
        self.due = Instant::now() + Duration::from_millis(150);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
    }
    pub fn poll(
        &mut self,
        backend: &Handle,
        session: Option<&Session>,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> bool {
        let mut changed = false;
        let history = &backend.history;
        let (nodes_changed, cached_choice) =
            self.cache_nodes.poll(history, &self.query.node, window, cx);
        changed |= nodes_changed;
        if let Some(name) = cached_choice {
            self.query.node = name;
            self.reset();
        }
        let policy = backend.catalogue_policy.snapshot();
        if self.policy_warning != policy.warning {
            self.policy_warning = policy.warning.clone();
            changed = true;
        }
        self.policy_ready = policy.ready;
        if let Some(context) = crate::rules::catalogue_policy::Context::capture(
            &policy,
            backend.catalogue_run,
            unix_now(),
        ) && self
            .query
            .policy
            .map(|old| (old.settings, old.revision, old.run))
            != Some((context.settings, context.revision, context.run))
        {
            self.query.policy = Some(context);
            self.page = Page::default();
            self.reset();
            changed = true;
        }
        if let Ok(update) = self.filter_receiver.try_recv() {
            if update.session != session.map_or(0, |s| s.id) || update.node != self.query.node {
                self.error = Some(crate::i18n::tr(
                    "The selected node changed; reopen rule filters before applying.",
                ));
            } else {
                self.query.filters = update.filters;
                self.query.logic = update.logic;
                self.message = Some(crate::i18n::tr(
                    "Rule conditions applied. Reviews and exports use the same expression.",
                ));
                self.error = None;
                self.reset();
            }
            changed = true;
        }
        if let Some(handle) = self.filter_window
            && handle.update(cx, |_, _, _| {}).is_err()
        {
            self.filter_window = None;
            changed = true;
        }
        let online = session.is_some_and(|s| {
            s.name == self.query.node && s.notifications && history.health.catalogues.ready(s.id)
        });
        changed |= online != self.online;
        self.online = online;
        let id = session.map_or(0, |s| s.id);
        if id != self.session {
            self.session = id;
            self.selected = None;
            self.marked.clear();
            self.pending_change = None;
            self.message = None;
            if let Some(session) = session {
                self.page = Page::default();
                self.error = None;
                self.query.node = session.name.clone();
                self.cache_nodes.select(&self.query.node, window, cx);
                self.reset();
            }
            changed = true;
        }
        if let Some(rx) = self.pending_change.as_mut() {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending_change = None;
                    match result {
                        Ok(()) => {
                            self.message =
                                Some(crate::i18n::tr("Daemon confirmed the rule change."));
                            self.error = None;
                            if self.creating {
                                self.creating = false;
                                self.input.focus_handle(cx).focus(window, cx);
                            }
                            self.reset();
                        }
                        Err(error) => self.error = Some(error),
                    }
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending_change = None;
                    self.error = Some(crate::i18n::tr(
                        "Rule command stopped before completion; reconnect to check the daemon's rules",
                    ));
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        if let Some((generation, rx)) = self.pending_query.as_mut() {
            match rx.try_recv() {
                Ok(result) => {
                    if *generation == self.generation {
                        match result {
                            Ok(page) => {
                                if let Some(selected) = self.selected.as_ref() {
                                    self.selected = page
                                        .rules
                                        .iter()
                                        .find(|rule| rule.name == selected.name)
                                        .cloned();
                                }
                                self.page = page;
                            }
                            Err(error) => self.error = Some(error),
                        }
                        changed = true;
                    }
                    self.pending_query = None;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending_query = None;
                    self.error = Some(crate::i18n::tr("Rules catalogue worker stopped"));
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        let revision = history.health.revision.load(Ordering::Acquire);
        if revision != self.revision && Instant::now() >= self.due {
            self.dirty = true;
        }
        if self.dirty
            && self.policy_ready
            && !self.query.node.is_empty()
            && self.pending_query.is_none()
            && Instant::now() >= self.due
            && let Some(rx) = history.query_rules(self.frozen_query())
        {
            self.pending_query = Some((self.generation, rx));
            self.dirty = false;
            self.revision = revision;
            self.due = Instant::now() + Duration::from_secs(1);
        }
        changed
    }
    fn send(&mut self, backend: &Handle, action: Action, rule: Rule) {
        if self.pending_change.is_some() {
            return;
        }
        self.error = None;
        self.message = None;
        self.deleting = false;
        match backend.control.submit(self.session, action, rule) {
            Ok(rx) => self.pending_change = Some(rx),
            Err(error) => self.error = Some(error),
        }
    }
}

impl Dashboard {
    fn open_rule_batch(
        &mut self,
        action: Action,
        decision: Option<&'static str>,
        label: &'static str,
        cx: &mut Context<Self>,
    ) {
        if self.rules.editor_window.is_some() || self.rules.pending_change.is_some() {
            return;
        }
        let mut rules = self
            .rules
            .page
            .rules
            .iter()
            .filter(|rule| self.rules.marked.contains(&rule.name))
            .cloned()
            .collect::<Vec<_>>();
        if self.rules.marked.is_empty() {
            return;
        }
        for rule in &mut rules {
            if let Some(decision) = decision {
                rule.action = decision.into();
            }
            if action == Action::EnableRule {
                rule.enabled = true;
            }
            if action == Action::DisableRule {
                rule.enabled = false;
            }
        }
        match super::rule_transfer::open_batch(
            self.backend.clone(),
            self.rules.session,
            crate::rules::transfer::ExportScope {
                query: self.rules.frozen_query(),
                names: Some(self.rules.marked.clone()),
            },
            rules,
            super::rule_transfer::Mode::Bulk {
                action,
                label,
                decision,
            },
            cx,
        ) {
            Ok(window) => self.rules.editor_window = Some(window),
            Err(error) => {
                self.rules.error = Some(crate::i18n::tr_args(
                    "Cannot open bulk rule review: %{error}",
                    &[("error", &format!("{:#}", error))],
                ))
            }
        }
        cx.notify();
    }
    fn copy_rules_to_nodes(&mut self, cx: &mut Context<Self>) {
        let names = if !self.rules.marked.is_empty() {
            Some(self.rules.marked.clone())
        } else {
            self.rules
                .selected
                .as_ref()
                .map(|rule| std::collections::BTreeSet::from([rule.name.clone()]))
        };
        let scope = crate::rules::transfer::ExportScope {
            query: self.rules.frozen_query(),
            names,
        };
        match super::rule_distribution::open(self.backend.clone(), scope, cx) {
            Ok(window) => self.rules.editor_window = Some(window),
            Err(error) => {
                self.rules.error = Some(crate::i18n::tr_args(
                    "Cannot open cross-node copy: %{error}",
                    &[("error", &format!("{:#}", error))],
                ))
            }
        }
        cx.notify();
    }
    fn open_rule_transfer(&mut self, import: bool, selected: bool, cx: &mut Context<Self>) {
        if let Some(window) = self.rules.editor_window
            && window
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let scope = crate::rules::transfer::ExportScope {
            query: self.rules.frozen_query(),
            names: if selected {
                if self.rules.marked.is_empty() {
                    self.rules
                        .selected
                        .as_ref()
                        .map(|rule| std::collections::BTreeSet::from([rule.name.clone()]))
                } else {
                    Some(self.rules.marked.clone())
                }
            } else {
                None
            },
        };
        match super::rule_transfer::open(
            self.backend.clone(),
            self.rules.session,
            scope,
            if import {
                super::rule_transfer::Mode::Import
            } else {
                super::rule_transfer::Mode::Export
            },
            cx,
        ) {
            Ok(window) => self.rules.editor_window = Some(window),
            Err(error) => {
                self.rules.error = Some(crate::i18n::tr_args(
                    "Cannot open rule transfer: %{error}",
                    &[("error", &format!("{:#}", error))],
                ))
            }
        }
        cx.notify();
    }
    fn open_rule_editor(&mut self, duplicate: bool, new: bool, cx: &mut Context<Self>) {
        if let Some(window) = self.rules.editor_window
            && window
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let (mut rule, original) = if new {
            let name = self.rules.name.read(cx).value().to_string();
            (
                Rule {
                    name: if name.is_empty() {
                        format!("rustleblower.rule.{}", crate::history::now_nanos())
                    } else {
                        name
                    },
                    created: crate::history::now_seconds(),
                    enabled: true,
                    action: "allow".into(),
                    duration: "always".into(),
                    operator: Some(Operator {
                        r#type: "simple".into(),
                        operand: "process.path".into(),
                        data: self.rules.executable.read(cx).value().to_string(),
                        sensitive: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
            )
        } else if let Some(rule) = self.rules.selected.clone() {
            let original = (!duplicate).then(|| rule.clone());
            (rule, original)
        } else {
            return;
        };
        if duplicate {
            rule.name = format!(
                "{}.copy.{}",
                crate::history::bounded(&rule.name, 970),
                crate::history::now_nanos()
            );
            rule.created = crate::history::now_seconds();
        }
        match super::rule_editor::open(self.backend.clone(), self.rules.session, rule, original, cx)
        {
            Ok(window) => self.rules.editor_window = Some(window),
            Err(e) => {
                self.rules.error = Some(crate::i18n::tr_args(
                    "Could not open rule editor: %{e}",
                    &[("e", &format!("{:#}", e))],
                ))
            }
        }
        cx.notify();
    }
    fn change_rule(&mut self, action: Action, decision: Option<&str>, cx: &mut Context<Self>) {
        if let Some(mut rule) = self.rules.selected.clone() {
            if let Some(decision) = decision {
                rule.action = decision.into();
            }
            self.rules.send(&self.backend, action, rule);
            cx.notify();
        }
    }
    fn create_application_rule(&mut self, action: &str, cx: &mut Context<Self>) {
        if self.rules.pending_change.is_some() {
            return;
        }
        let path = self.rules.executable.read(cx).value().to_string();
        let name = self.rules.name.read(cx).value().to_string();
        if !path.starts_with('/') || path.len() > 4096 {
            self.rules.error = Some(crate::i18n::tr(
                "Choose an absolute executable path, at most 4096 bytes",
            ));
            cx.notify();
            return;
        }
        let created = crate::history::now_seconds();
        let rule = Rule {
            created,
            name: if name.is_empty() {
                format!("rustleblower.application.{}", crate::history::now_nanos())
            } else {
                name
            },
            description: format!("Application rule for {path}"),
            enabled: true,
            action: action.into(),
            duration: "always".into(),
            operator: Some(Operator {
                r#type: "simple".into(),
                operand: "process.path".into(),
                data: path,
                sensitive: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let database = self.backend.history.database_path().to_owned();
        let node = self.rules.query.node.clone();
        let check_name = rule.name.clone();
        let task = cx
            .background_executor()
            .spawn(async move { crate::rules::transfer::find_rule(&database, &node, &check_name) });
        let control = self.backend.control.clone();
        let session = self.rules.session;
        let (reply, pending) = oneshot::channel();
        self.rules.pending_change = Some(pending);
        self.rules.error = None;
        self.rules.message = None;
        cx.spawn(async move |_,_|{
            let result=async {
                if task.await.map_err(|error|crate::i18n::tr_args("Cannot check rule name: %{error}", &[("error", &format!("{:#}", error))]))?.is_some(){return Err("A rule with this name already exists; choose another name or edit the existing rule".into());}
                control.submit(session,Action::ChangeRule,rule)?.await.map_err(|_|"Rule command stopped before confirmation".to_string())?
            }.await;
            let _=reply.send(result);
        }).detach();
        cx.notify();
    }
    pub(super) fn render_rules(&mut self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let cached_offline = self
            .backend
            .session(self.rules.session)
            .is_none_or(|session| session.name != self.rules.query.node);
        let disabled = !self.rules.online
            || !self.rules.policy_ready
            || self.rules.pending_change.is_some()
            || self.rules.editor_window.is_some();
        let cached_node = div().w(px(220.)).min_w_0().child(
            Select::new(&self.rules.cache_nodes.picker)
                .id("rules-cache-node")
                .disabled(
                    !self.rules.policy_ready
                        || self.rules.cache_nodes.empty()
                        || self.rules.creating
                        || self.rules.editor_window.is_some()
                        || self.rules.pending_change.is_some(),
                ),
        );
        let rows = uniform_list(
            "application-rules",
            self.rules.page.rules.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let highlights = this.backend.highlighting.snapshot();
                range
                    .map(|index| {
                        let rule = this.rules.page.rules[index].clone();
                        let selected = this
                            .rules
                            .selected
                            .as_ref()
                            .is_some_and(|r| r.name == rule.name);
                        let muted = cx.theme().muted_foreground;
                        let border = cx.theme().border;
                        let name = rule.name.clone();
                        let awaiting = this.rules.page.awaiting.contains(&name);
                        let marked = this.rules.marked.contains(&name);
                        let cache_status = if awaiting {
                            crate::i18n::tr("Awaiting daemon observation")
                        } else {
                            let origin = this
                                .rules
                                .page
                                .origins
                                .get(&name)
                                .copied()
                                .unwrap_or(crate::rules::catalogue_policy::Origin::Unknown);
                            let settings = this
                                .rules
                                .query
                                .policy
                                .map(|policy| policy.settings)
                                .unwrap_or_default();
                            cached_warning(
                                crate::rules::catalogue_policy::derive(
                                    settings,
                                    &rule,
                                    origin,
                                    unix_now(),
                                )
                                .status,
                            )
                        };
                        let blocked = awaiting
                            || this.rules.editor_window.is_some()
                            || this.rules.pending_change.is_some();
                        let enabled =
                            crate::i18n::tr(if rule.enabled { "enabled" } else { "disabled" });
                        let action_color = match rule.action.as_str() {
                            "allow" => cx.theme().success,
                            "deny" | "drop" => cx.theme().danger,
                            "reject" => cx.theme().warning,
                            _ => muted,
                        };
                        let state_color = if awaiting {
                            cx.theme().warning
                        } else if rule.enabled {
                            cx.theme().success
                        } else {
                            muted
                        };
                        let process = crate::rules::executable(&rule)
                            .map(str::to_owned)
                            .unwrap_or_else(|| crate::i18n::tr("Any application"));
                        let description = if awaiting {
                            crate::i18n::tr("Reply sent · awaiting daemon observation")
                        } else {
                            rule.description.clone()
                        };
                        let created = added_date(rule.created);
                        let cells = [
                            ("name", rule.name.as_str()),
                            ("action", rule.action.as_str()),
                            ("duration", rule.duration.as_str()),
                            ("enabled", enabled.as_str()),
                            ("process", process.as_str()),
                            ("description", description.as_str()),
                            ("cache_status", cache_status.as_str()),
                        ];
                        let prepared = highlights.prepare_row(
                            crate::highlighting::View::Rules,
                            &cells,
                            selected,
                        );
                        let style = |key: &str, value: &str, cell: Div| {
                            super::highlighting::cell_prepared(
                                &highlights,
                                crate::highlighting::View::Rules,
                                key,
                                value,
                                prepared,
                                cell,
                            )
                        };
                        ListItem::new(("rule-row", index))
                            .accessibility_label(rule.name.clone())
                            .h(px(84.))
                            .w_full()
                            .px_4()
                            .py_2()
                            .border_b_1()
                            .border_color(border)
                            .selected(selected)
                            .cursor_pointer()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        Checkbox::new(("rule-mark", index))
                                            .checked(marked)
                                            .disabled(blocked)
                                            .on_change(cx.listener(
                                                move |this, value: &bool, _, cx| {
                                                    if *value {
                                                        this.rules.marked.insert(name.clone());
                                                    } else {
                                                        this.rules.marked.remove(&name);
                                                    }
                                                    cx.notify();
                                                },
                                            )),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(
                                                div().flex().justify_between().child(style(
                                                    "name",
                                                    &rule.name,
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .child(rule.name.clone()),
                                                )),
                                            )
                                            .child(style(
                                                "process",
                                                &process,
                                                div()
                                                    .text_sm()
                                                    .text_color(muted)
                                                    .truncate()
                                                    .child(process.clone()),
                                            ))
                                            .when(!rule.description.is_empty() || awaiting, |v| {
                                                v.child(style(
                                                    "description",
                                                    &description,
                                                    div()
                                                        .text_xs()
                                                        .text_color(muted)
                                                        .truncate()
                                                        .child(description.clone()),
                                                ))
                                            }),
                                    )
                                    .child({
                                        let cell = style(
                                            "created",
                                            &created,
                                            div()
                                                .w(px(ADDED_COLUMN_WIDTH))
                                                .flex_shrink_0()
                                                .text_sm()
                                                .when(!selected, |cell| cell.text_color(muted))
                                                .child(created.clone()),
                                        )
                                        .id(("rule-added", index));
                                        #[cfg(feature = "headless-tests")]
                                        let cell = cell.test_support();
                                        cell
                                    })
                                    .child(
                                        div()
                                            .w(px(RULE_STATUS_COLUMN_WIDTH))
                                            .flex_shrink_0()
                                            .min_w_0()
                                            .flex()
                                            .flex_wrap()
                                            .items_center()
                                            .gap_2()
                                            .child(style(
                                                "action",
                                                &rule.action,
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .px_2()
                                                    .py_1()
                                                    .rounded_md()
                                                    .when(!selected, |cell| {
                                                        cell.text_color(action_color)
                                                            .bg(action_color.opacity(0.12))
                                                    })
                                                    .child(rule.action.clone()),
                                            ))
                                            .child(style(
                                                "duration",
                                                &rule.duration,
                                                div()
                                                    .text_xs()
                                                    .when(!selected, |cell| cell.text_color(muted))
                                                    .child(rule.duration.clone()),
                                            ))
                                            .child(style(
                                                "enabled",
                                                &enabled,
                                                div()
                                                    .text_xs()
                                                    .when(!selected, |cell| {
                                                        cell.text_color(state_color)
                                                    })
                                                    .child(enabled.clone()),
                                            ))
                                            .when(!cache_status.is_empty(), |row| {
                                                row.child(style(
                                                    "cache_status",
                                                    &cache_status,
                                                    div()
                                                        .text_xs()
                                                        .when(!selected, |cell| {
                                                            cell.text_color(cx.theme().warning)
                                                        })
                                                        .child(cache_status.clone()),
                                                ))
                                            }),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.rules.selected = Some(rule.clone());
                                this.rules.deleting = false;
                                cx.notify();
                            }))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .min_h_0()
        .pr_3()
        .track_scroll(&self.rules.scroll);

        let rows = div()
            .id("rules-list-viewport")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(rows)
            .child(
                Scrollbar::vertical(&self.rules.scroll)
                    .id("rules-scrollbar")
                    .mode(ScrollbarMode::Always),
            );
        #[cfg(feature = "headless-tests")]
        let rows = rows.test_support();

        let filters = [
            (None, "All", "rules-all"),
            (Some(false), "Permanent", "rules-permanent"),
            (Some(true), "Temporary", "rules-temporary"),
        ]
        .into_iter()
        .map(|(filter, label, id)| {
            Button::new(id)
                .ghost()
                .label(crate::i18n::tr(label))
                .selected(self.rules.query.temporary == filter)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.rules.query.temporary = filter;
                    this.rules.reset();
                    cx.notify();
                }))
        })
        .collect::<Vec<_>>();
        let toolbar = div()
            .px_6()
            .pb_3()
            .flex()
            .gap_2()
            .items_center()
            .flex_wrap()
            .child(cached_node)
            .child(
                div()
                    .flex_1()
                    .min_w(px(220.))
                    .child(Input::new(&self.rules.input)),
            )
            .children(filters)
            .child(
                Button::new("rule-filter-open")
                    .ghost()
                    .label(crate::i18n::tr_args(
                        "Conditions (%{arg0})…",
                        &[("arg0", &(self.rules.query.filters.len()).to_string())],
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(handle) = this.rules.filter_window
                            && handle
                                .update(cx, |_, window, _| window.activate_window())
                                .is_ok()
                        {
                            return;
                        }
                        match filters::open(
                            this.rules.frozen_query(),
                            this.rules.session,
                            this.rules.filter_sender.clone(),
                            cx,
                        ) {
                            Ok(handle) => this.rules.filter_window = Some(handle),
                            Err(error) => {
                                this.rules.error = Some(crate::i18n::tr_args(
                                    "Cannot open rule filters: %{error}",
                                    &[("error", &format!("{:#}", error))],
                                ))
                            }
                        }
                        cx.notify();
                    })),
            )
            .when(self.rules.editor_window.is_some(), |v| {
                v.child(
                    Button::new("rule-editor-open")
                        .ghost()
                        .label(crate::i18n::tr("Open rule window"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(window) = this.rules.editor_window {
                                let _ = window.update(cx, |_, window, _| window.activate_window());
                            }
                        })),
                )
            })
            .child(
                Button::new("new-rule")
                    .primary()
                    .label(crate::i18n::tr("New application rule"))
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.rules.creating = true;
                        this.rules.executable.focus_handle(cx).focus(window, cx);
                        cx.notify();
                    })),
            );
        let create_form =
            div()
                .px_6()
                .py_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(crate::i18n::tr("Create a permanent application rule"))
                .child(Input::new(&self.rules.executable).id("new-rule-path"))
                .child(Input::new(&self.rules.name).id("new-rule-name"))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("create-rule-allow")
                                .label(crate::i18n::tr("Allow always"))
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.create_application_rule("allow", cx)
                                })),
                        )
                        .child(
                            Button::new("create-rule-deny")
                                .label(crate::i18n::tr("Deny always"))
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.create_application_rule("deny", cx)
                                })),
                        )
                        .child(
                            Button::new("create-rule-cancel")
                                .label(crate::i18n::tr("Cancel"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.rules.creating = false;
                                    this.rules.input.focus_handle(cx).focus(window, cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("create-rule-advanced")
                                .label(crate::i18n::tr("All rule options…"))
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_rule_editor(false, true, cx)
                                })),
                        ),
                );
        let selection =
            self.rules
                .selected
                .clone()
                .filter(|_| self.rules.marked.is_empty())
                .map(|rule| {
                    let disabled = disabled || self.rules.page.awaiting.contains(&rule.name);
                    let decisions = [
                        ("allow", "Allow", "rule-allow"),
                        ("deny", "Deny", "rule-deny"),
                        ("reject", "Reject", "rule-reject"),
                    ]
                    .into_iter()
                    .map(|(action, label, id)| {
                        Button::new(id)
                            .label(crate::i18n::tr(label))
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.change_rule(Action::ChangeRule, Some(action), cx)
                            }))
                    })
                    .collect::<Vec<_>>();
                    let buttons = div()
                        .flex()
                        .gap_2()
                        .children(decisions)
                        .child(
                            Button::new("rule-edit")
                                .primary()
                                .label(crate::i18n::tr("Edit…"))
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_rule_editor(false, false, cx)
                                })),
                        )
                        .child(
                            Button::new("rule-duplicate")
                                .ghost()
                                .label(crate::i18n::tr("Duplicate…"))
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_rule_editor(true, false, cx)
                                })),
                        )
                        .child(
                            Button::new("rule-enabled")
                                .ghost()
                                .label(crate::i18n::tr(if rule.enabled {
                                    "Disable"
                                } else {
                                    "Enable"
                                }))
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.change_rule(
                                        if rule.enabled {
                                            Action::DisableRule
                                        } else {
                                            Action::EnableRule
                                        },
                                        None,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new("rule-delete")
                                .ghost()
                                .label(crate::i18n::tr("Delete…"))
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.rules.deleting = true;
                                    cx.notify();
                                })),
                        );
                    let confirmation = div()
                        .flex()
                        .gap_2()
                        .items_center()
                        .child(crate::i18n::tr("Delete this rule from the daemon?"))
                        .child(
                            Button::new("rule-delete-confirm")
                                .danger()
                                .label(crate::i18n::tr("Delete rule"))
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.change_rule(Action::DeleteRule, None, cx)
                                })),
                        )
                        .child(
                            Button::new("rule-delete-cancel")
                                .ghost()
                                .label(crate::i18n::tr("Keep rule"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.rules.deleting = false;
                                    cx.notify();
                                })),
                        );
                    div()
                        .px_6()
                        .py_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .border_b_1()
                        .border_color(border)
                        .child(crate::i18n::tr_args(
                            "Selected: %{arg0}",
                            &[("arg0", &(rule.name).to_string())],
                        ))
                        .child(buttons)
                        .when(self.rules.deleting, |v| v.child(confirmation))
                });
        let pagination = div()
            .px_6()
            .py_3()
            .flex()
            .justify_between()
            .items_center()
            .text_sm()
            .text_color(muted)
            .child(crate::i18n::tr_args(
                "%{arg0} rules on this page",
                &[("arg0", &(self.rules.page.rules.len()).to_string())],
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("rules-first")
                            .ghost()
                            .label(crate::i18n::tr("First"))
                            .disabled(self.rules.query.after.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.rules.query.after.clear();
                                this.rules.invalidate();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("rules-next")
                            .ghost()
                            .label(crate::i18n::tr("Next"))
                            .disabled(!self.rules.page.has_more)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(rule) = this.rules.page.rules.last() {
                                    this.rules.query.after = rule.name.clone();
                                    this.rules.invalidate();
                                    cx.notify();
                                }
                            })),
                    ),
            );
        let transfers = div()
            .px_6()
            .pb_2()
            .flex()
            .flex_wrap()
            .gap_1()
            .child(Button::new("rules-catalogue-preferences").ghost().label(crate::i18n::tr("Catalogue preferences…")).on_click(cx.listener(|this,_,_,cx| { this.open_settings(super::settings::Page::Catalogue,cx); })))
            .child(Button::new("rules-refresh").ghost().label(crate::i18n::tr("Refresh connection")).disabled(cached_offline||self.rules.editor_window.is_some()||self.rules.pending_change.is_some()||!self.backend.prompts.snapshot().is_empty())
                .on_click(cx.listener(|this,_,_,cx|{match this.backend.refresh_rules(this.rules.session){Ok(())=>{this.rules.message=Some(crate::i18n::tr("Requested a fresh inventory; waiting for the daemon to reconnect."));this.rules.error=None;},Err(error)=>this.rules.error=Some(error)}cx.notify();})))
            .child(
                Button::new("rules-import").ghost()
                    .label(crate::i18n::tr("Import…"))
                    .disabled(disabled)
                    .on_click(
                        cx.listener(|this, _, _, cx| this.open_rule_transfer(true, false, cx)),
                    ),
            )
            .child(Button::new("rules-import-nodes").ghost().label(crate::i18n::tr("Import to nodes…")).disabled(self.rules.editor_window.is_some()||self.rules.pending_change.is_some()||self.backend.status().sessions.is_empty()).on_click(cx.listener(|this,_,_,cx|{
                match super::rule_distribution::open_import(this.backend.clone(),cx){Ok(window)=>this.rules.editor_window=Some(window),Err(error)=>this.rules.error=Some(crate::i18n::tr_args("Cannot open multi-node import: %{error}", &[("error", &format!("{:#}", error))]))}cx.notify();
            })))
            .child(
                Button::new("rules-export").ghost()
                    .label(crate::i18n::tr("Export filter…"))
                    .disabled(
                        self.rules.query.node.is_empty()
                            || !self.rules.policy_ready
                            || self.rules.editor_window.is_some()
                            || self.rules.pending_change.is_some(),
                    )
                    .on_click(
                        cx.listener(|this, _, _, cx| this.open_rule_transfer(false, false, cx)),
                    ),
            )
            .child(Button::new("rules-export-nodes").ghost().label(crate::i18n::tr("Export nodes…")).disabled(!self.rules.policy_ready||self.rules.editor_window.is_some()||self.rules.pending_change.is_some()).on_click(cx.listener(|this,_,_,cx|{
                match super::rule_exports::open(this.backend.clone(),this.rules.frozen_query(),cx){Ok(window)=>this.rules.editor_window=Some(window),Err(error)=>this.rules.error=Some(crate::i18n::tr_args("Cannot open multi-node export: %{error}", &[("error", &format!("{:#}", error))]))}cx.notify();
            })))
            .child(
                Button::new("rules-export-selected").ghost()
                    .label(crate::i18n::tr("Export selected…"))
                    .disabled(
                        (self.rules.selected.is_none() && self.rules.marked.is_empty())
                            || !self.rules.policy_ready
                            || self.rules.editor_window.is_some()
                            || self.rules.pending_change.is_some(),
                    )
                    .on_click(
                        cx.listener(|this, _, _, cx| this.open_rule_transfer(false, true, cx)),
                    ),
            )
            .child(Button::new("rules-copy-nodes").ghost().label(crate::i18n::tr("Copy to nodes…")).disabled(self.rules.query.node.is_empty()||self.rules.editor_window.is_some()||self.rules.pending_change.is_some()||self.rules.page.rules.is_empty()).on_click(cx.listener(|this,_,_,cx|this.copy_rules_to_nodes(cx))))
            .child(
                Button::new("rules-select-page").ghost()
                    .label(crate::i18n::tr("Select page"))
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.rules.marked.extend(
                            this.rules
                                .page
                                .rules
                                .iter()
                                .filter(|rule| !this.rules.page.awaiting.contains(&rule.name))
                                .map(|rule| rule.name.clone()),
                        );
                        cx.notify();
                    })),
            )
            .child(
                Button::new("rules-clear-selection").ghost()
                    .label(crate::i18n::tr("Clear selection"))
                    .disabled(self.rules.marked.is_empty() || disabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.rules.marked.clear();
                        cx.notify();
                    })),
            );
        let bulk = div()
            .px_6()
            .pb_2()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(crate::i18n::tr_args(
                "%{arg0} selected",
                &[("arg0", &(self.rules.marked.len()).to_string())],
            ))
            .children(
                [
                    (
                        Action::ChangeRule,
                        Some("allow"),
                        "Allow selected",
                        "rules-bulk-allow",
                    ),
                    (
                        Action::ChangeRule,
                        Some("deny"),
                        "Deny selected",
                        "rules-bulk-deny",
                    ),
                    (
                        Action::ChangeRule,
                        Some("reject"),
                        "Reject selected",
                        "rules-bulk-reject",
                    ),
                    (
                        Action::EnableRule,
                        None,
                        "Enable selected",
                        "rules-bulk-enable",
                    ),
                    (
                        Action::DisableRule,
                        None,
                        "Disable selected",
                        "rules-bulk-disable",
                    ),
                    (
                        Action::DeleteRule,
                        None,
                        "Delete selected…",
                        "rules-bulk-delete",
                    ),
                ]
                .into_iter()
                .map(|(action, decision, label, id)| {
                    Button::new(id)
                        .label(crate::i18n::tr(label))
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_rule_batch(action, decision, label, cx)
                        }))
                }),
            );
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .pt_4()
            .child(toolbar)
            .child(div().px_6().pb_3().flex().gap_2().items_center().flex_wrap()
                .children([("rules-action-all", "All", ""), ("rules-action-allow", "Allowed", "allow"), ("rules-action-deny", "Denied", "deny"), ("rules-action-reject", "Rejected", "reject")].into_iter().map(|(id, label, action)| {
                    Button::new(id).ghost().label(crate::i18n::tr(label)).selected(self.rules.query.action == action)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.rules.query.action = action.into();
                            this.rules.reset();
                            cx.notify();
                        }))
                })))
            .child(transfers)
            .when(!self.rules.page.awaiting.is_empty(),|v|v.child(div().px_6().py_2().text_sm().child(crate::i18n::tr("Popup replies await daemon observation. Use Refresh connection if they remain unobserved, including rules with logging disabled."))))
            .when_some(self.rules.policy_warning.clone(), |container,warning|container.child(super::banner(warning,cx)))
            .when_some(self.rules.cache_nodes.error.clone(), |container,error|container.child(super::banner(error,cx)))
            .when(self.backend.history.health.catalogues.uncertain(self.rules.session),|v|v.child(super::banner("A daemon command failed. Entries show last confirmed values; reconnect to reload live rules.".into(),cx)))
            .when(!self.rules.marked.is_empty(),|v|v.child(bulk))
            .when_some(self.rules.error.clone(), |v, error| {
                v.child(super::banner(error, cx))
            })
            .when_some(self.rules.message.clone(), |v, message| {
                v.child(div().px_6().py_2().text_sm().child(crate::i18n::presentation(&message)))
            })
            .when(self.rules.pending_change.is_some(), |v| {
                v.child(
                    div()
                        .px_6()
                        .py_2()
                        .text_sm()
                        .child(crate::i18n::tr("Waiting for the daemon to confirm…")),
                )
            })
            .when(!self.rules.online, |v| {
                v.child(div().px_6().py_2().text_sm().text_color(muted).child(
                    if cached_offline {crate::i18n::tr("This cached node is offline. Connect its daemon to change rules.")} else {crate::i18n::tr("Rules are not ready for editing. Reconnect if loading failed or a change was not confirmed.")},
                ))
            })
            .when(self.rules.creating, |v| v.child(create_form))
            .when_some(selection, |v, selection| v.child(selection))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .mx_6()
                    .border_1()
                    .border_color(border)
                    .rounded_lg()
                    .overflow_hidden()
                    .child(
                        // Match the rows' pr_3 scrollbar gutter as well as their px_4 padding.
                        div().flex().items_center().gap_3().px_4().pr_7().py_1()
                            .flex_shrink_0().border_b_1().border_color(border)
                            .bg(cx.theme().muted.opacity(0.45))
                            .child(div().flex_1().min_w_0().child(
                                Button::new("rules-sort-name").small().ghost()
                                    .label(crate::i18n::tr("Rule name"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.rules.query.sort = None;
                                        this.rules.reset();
                                        cx.notify();
                                    })),
                            ))
                            .child(
                                Button::new("rules-sort-added").small().ghost()
                                    .w(px(ADDED_COLUMN_WIDTH)).flex_shrink_0()
                                    .label(format!("{}{}", crate::i18n::tr("Added (UTC)"),
                                        match self.rules.query.sort {
                                            Some(sort) if sort.field == crate::view_preferences::RuleField::Created => {
                                                if sort.ascending { " ↑" } else { " ↓" }
                                            }
                                            _ => "",
                                        }))
                                    .tooltip(crate::i18n::tr("Rule creation date reported by the daemon"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let ascending = this.rules.query.sort.is_some_and(|sort| {
                                            sort.field == crate::view_preferences::RuleField::Created && !sort.ascending
                                        });
                                        this.rules.query.sort = Some(crate::view_preferences::Sort {
                                            field: crate::view_preferences::RuleField::Created,
                                            ascending,
                                        });
                                        this.rules.reset();
                                        cx.notify();
                                    })),
                            )
                            .child(div().w(px(RULE_STATUS_COLUMN_WIDTH)).flex_shrink_0()
                                .text_xs().text_color(muted)
                                .child(format!("{} · {} · {}", crate::i18n::tr("Action"), crate::i18n::tr("Duration"), crate::i18n::tr("Status")))),
                    )
                    .when(self.rules.page.rules.is_empty(), |v| {
                        v.child(
                            div()
                                .p_4()
                                .text_color(muted)
                                .child(crate::i18n::tr("No matching application rules")),
                        )
                    })
                    .child(rows),
            )
            .child(pagination)
    }
}
