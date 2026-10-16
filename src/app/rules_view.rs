//! Application rules: bounded pages and explicit acknowledged policy changes.
use super::Dashboard;
use crate::{
    backend::{Handle, Session},
    history::History,
    protocol::{Action, Operator, Rule},
    rules::{Page, Query},
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    list::ListItem,
};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

pub(super) struct RulesView {
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
    pub(super) error: Option<String>,
    message: Option<String>,
    creating: bool,
    deleting: bool,
    scroll: UniformListScrollHandle,
    pub(super) editor_window: Option<AnyWindowHandle>,
}

impl RulesView {
    pub(super) fn editor_closed(&mut self) {
        self.creating = false;
        self.page = Page::default();
        self.marked.clear();
        self.invalidate();
    }
    pub fn new(window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Filter rule name, application or description…")
        });
        let executable =
            cx.new(|cx| InputState::new(window, cx).placeholder("/absolute/path/to/application"));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Rule name (optional)"));
        cx.subscribe(&input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.rules.query.search = input.read(cx).value().to_string();
                this.rules.reset();
                cx.notify();
            }
        })
        .detach();
        Self {
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
            error: None,
            message: None,
            creating: false,
            deleting: false,
            scroll: UniformListScrollHandle::new(),
            editor_window: None,
        }
    }

    fn reset(&mut self) {
        self.query.after.clear();
        self.marked.clear();
        self.invalidate();
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
        history: &History,
        session: Option<&Session>,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> bool {
        let mut changed = false;
        let online =
            session.is_some_and(|s| s.notifications && history.health.catalogues.ready(s.id));
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
                            self.message = Some("Daemon confirmed the rule change.".into());
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
                    self.error=Some("Rule command stopped before completion; reconnect to check the daemon's rules".into());
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
                    self.error = Some("Rules catalogue worker stopped".into());
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
            && !self.query.node.is_empty()
            && self.pending_query.is_none()
            && Instant::now() >= self.due
            && let Some(rx) = history.query_rules(self.query.clone())
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
                query: self.rules.query.clone(),
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
                self.rules.error = Some(format!("Cannot open bulk rule review: {error:#}"))
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
            query: self.rules.query.clone(),
            names,
        };
        match super::rule_distribution::open(self.backend.clone(), scope, cx) {
            Ok(window) => self.rules.editor_window = Some(window),
            Err(error) => {
                self.rules.error = Some(format!("Cannot open cross-node copy: {error:#}"))
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
            query: self.rules.query.clone(),
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
            Err(error) => self.rules.error = Some(format!("Cannot open rule transfer: {error:#}")),
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
            Err(e) => self.rules.error = Some(format!("Could not open rule editor: {e:#}")),
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
            self.rules.error =
                Some("Choose an absolute executable path, at most 4096 bytes".into());
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
                if task.await.map_err(|error|format!("Cannot check rule name: {error:#}"))?.is_some(){return Err("A rule with this name already exists; choose another name or edit the existing rule".into());}
                control.submit(session,Action::ChangeRule,rule)?.await.map_err(|_|"Rule command stopped before confirmation".to_string())?
            }.await;
            let _=reply.send(result);
        }).detach();
        cx.notify();
    }
    pub(super) fn render_rules(&mut self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let disabled = !self.rules.online
            || self.rules.pending_change.is_some()
            || self.rules.editor_window.is_some();
        let rows = uniform_list(
            "application-rules",
            self.rules.page.rules.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
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
                        let blocked = awaiting
                            || this.rules.editor_window.is_some()
                            || this.rules.pending_change.is_some();
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
                                                div()
                                                    .flex()
                                                    .justify_between()
                                                    .child(rule.name.clone())
                                                    .child(format!(
                                                        "{} · {} · {}",
                                                        rule.action,
                                                        rule.duration,
                                                        if rule.enabled {
                                                            "enabled"
                                                        } else {
                                                            "disabled"
                                                        }
                                                    )),
                                            )
                                            .child(
                                                div().text_sm().text_color(muted).child(
                                                    crate::rules::executable(&rule)
                                                        .unwrap_or("Any application")
                                                        .to_owned(),
                                                ),
                                            )
                                            .when(!rule.description.is_empty() || awaiting, |v| {
                                                v.child(div().text_xs().text_color(muted).child(
                                                    if awaiting {
                                                        "Reply sent · awaiting daemon observation"
                                                            .into()
                                                    } else {
                                                        rule.description.clone()
                                                    },
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
        .track_scroll(&self.rules.scroll);

        let filters = [
            (None, "All", "rules-all"),
            (Some(false), "Permanent", "rules-permanent"),
            (Some(true), "Temporary", "rules-temporary"),
        ]
        .into_iter()
        .map(|(filter, label, id)| {
            Button::new(id)
                .label(label)
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
            .py_3()
            .flex()
            .gap_2()
            .items_center()
            .child(div().flex_1().child(Input::new(&self.rules.input)))
            .children(filters)
            .when(self.rules.editor_window.is_some(), |v| {
                v.child(
                    Button::new("rule-editor-open")
                        .label("Open rule window")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(window) = this.rules.editor_window {
                                let _ = window.update(cx, |_, window, _| window.activate_window());
                            }
                        })),
                )
            })
            .child(
                Button::new("new-rule")
                    .label("New application rule")
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
                .child("Create a permanent application rule")
                .child(Input::new(&self.rules.executable).id("new-rule-path"))
                .child(Input::new(&self.rules.name).id("new-rule-name"))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("create-rule-allow")
                                .label("Allow always")
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.create_application_rule("allow", cx)
                                })),
                        )
                        .child(
                            Button::new("create-rule-deny")
                                .label("Deny always")
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.create_application_rule("deny", cx)
                                })),
                        )
                        .child(Button::new("create-rule-cancel").label("Cancel").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.rules.creating = false;
                                this.rules.input.focus_handle(cx).focus(window, cx);
                                cx.notify();
                            }),
                        ))
                        .child(
                            Button::new("create-rule-advanced")
                                .label("All rule options…")
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
                            .label(label)
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
                                .label("Edit…")
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_rule_editor(false, false, cx)
                                })),
                        )
                        .child(
                            Button::new("rule-duplicate")
                                .label("Duplicate…")
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_rule_editor(true, false, cx)
                                })),
                        )
                        .child(
                            Button::new("rule-enabled")
                                .label(if rule.enabled { "Disable" } else { "Enable" })
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
                                .label("Delete…")
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
                        .child("Delete this rule from the daemon?")
                        .child(
                            Button::new("rule-delete-confirm")
                                .label("Delete rule")
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.change_rule(Action::DeleteRule, None, cx)
                                })),
                        )
                        .child(
                            Button::new("rule-delete-cancel")
                                .label("Keep rule")
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
                        .child(format!("Selected: {}", rule.name))
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
            .child(format!(
                "{} rules on this page",
                self.rules.page.rules.len()
            ))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("rules-first")
                            .label("First")
                            .disabled(self.rules.query.after.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.rules.query.after.clear();
                                this.rules.invalidate();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("rules-next")
                            .label("Next")
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
            .gap_2()
            .child(Button::new("rules-refresh").label("Refresh connection").disabled(self.rules.session==0||self.rules.editor_window.is_some()||self.rules.pending_change.is_some()||!self.backend.prompts.snapshot().is_empty())
                .on_click(cx.listener(|this,_,_,cx|{match this.backend.refresh_rules(this.rules.session){Ok(())=>{this.rules.message=Some("Requested a fresh inventory; waiting for the daemon to reconnect.".into());this.rules.error=None;},Err(error)=>this.rules.error=Some(error)}cx.notify();})))
            .child(
                Button::new("rules-import")
                    .label("Import…")
                    .disabled(disabled)
                    .on_click(
                        cx.listener(|this, _, _, cx| this.open_rule_transfer(true, false, cx)),
                    ),
            )
            .child(Button::new("rules-import-nodes").label("Import to nodes…").disabled(self.rules.editor_window.is_some()||self.rules.pending_change.is_some()||self.backend.status().sessions.is_empty()).on_click(cx.listener(|this,_,_,cx|{
                match super::rule_distribution::open_import(this.backend.clone(),cx){Ok(window)=>this.rules.editor_window=Some(window),Err(error)=>this.rules.error=Some(format!("Cannot open multi-node import: {error:#}"))}cx.notify();
            })))
            .child(
                Button::new("rules-export")
                    .label("Export filter…")
                    .disabled(
                        self.rules.query.node.is_empty()
                            || self.rules.editor_window.is_some()
                            || self.rules.pending_change.is_some(),
                    )
                    .on_click(
                        cx.listener(|this, _, _, cx| this.open_rule_transfer(false, false, cx)),
                    ),
            )
            .child(Button::new("rules-export-nodes").label("Export nodes…").disabled(self.rules.editor_window.is_some()||self.rules.pending_change.is_some()).on_click(cx.listener(|this,_,_,cx|{
                match super::rule_exports::open(this.backend.clone(),this.rules.query.clone(),cx){Ok(window)=>this.rules.editor_window=Some(window),Err(error)=>this.rules.error=Some(format!("Cannot open multi-node export: {error:#}"))}cx.notify();
            })))
            .child(
                Button::new("rules-export-selected")
                    .label("Export selected…")
                    .disabled(
                        (self.rules.selected.is_none() && self.rules.marked.is_empty())
                            || self.rules.editor_window.is_some()
                            || self.rules.pending_change.is_some(),
                    )
                    .on_click(
                        cx.listener(|this, _, _, cx| this.open_rule_transfer(false, true, cx)),
                    ),
            )
            .child(Button::new("rules-copy-nodes").label("Copy to nodes…").disabled(self.rules.query.node.is_empty()||self.rules.editor_window.is_some()||self.rules.pending_change.is_some()||self.rules.page.rules.is_empty()).on_click(cx.listener(|this,_,_,cx|this.copy_rules_to_nodes(cx))))
            .child(
                Button::new("rules-select-page")
                    .label("Select page")
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
                Button::new("rules-clear-selection")
                    .label("Clear selection")
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
            .child(format!("{} selected", self.rules.marked.len()))
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
                        .label(label)
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
            .child(toolbar)
            .child(transfers)
            .when(!self.rules.page.awaiting.is_empty(),|v|v.child(div().px_6().py_2().text_sm().child("Popup replies await daemon observation. Use Refresh connection if they remain unobserved, including rules with logging disabled.")))
            .when(self.backend.history.health.catalogues.uncertain(self.rules.session),|v|v.child(super::banner("A daemon command failed. Entries show last confirmed values; reconnect to reload live rules.".into(),cx)))
            .when(!self.rules.marked.is_empty(),|v|v.child(bulk))
            .when_some(self.rules.error.clone(), |v, error| {
                v.child(super::banner(error, cx))
            })
            .when_some(self.rules.message.clone(), |v, message| {
                v.child(div().px_6().py_2().text_sm().child(message))
            })
            .when(self.rules.pending_change.is_some(), |v| {
                v.child(
                    div()
                        .px_6()
                        .py_2()
                        .text_sm()
                        .child("Waiting for the daemon to confirm…"),
                )
            })
            .when(!self.rules.online, |v| {
                v.child(div().px_6().py_2().text_sm().text_color(muted).child(
                    if self.rules.session==0 {"Connect a daemon to change rules. Saved rules remain available for reference."} else {"Rules are not ready for editing. Reconnect if loading failed or a change was not confirmed."},
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
                    .when(self.rules.page.rules.is_empty(), |v| {
                        v.child(
                            div()
                                .p_4()
                                .text_color(muted)
                                .child("No matching application rules"),
                        )
                    })
                    .child(rows),
            )
            .child(pagination)
    }
}
