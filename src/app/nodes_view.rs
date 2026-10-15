//! Node summaries and explicit daemon controls, with background configuration reads.
use super::{Dashboard, Screen};
use crate::{
    backend::{Handle, Session},
    nodes::{self, Detail, Row},
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::Button,
    input::{Input, InputEvent, InputState},
    list::ListItem,
};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
type Read = oneshot::Receiver<Result<(Vec<Row>, Option<Detail>), String>>;
pub(super) struct NodesView {
    pub input: Entity<InputState>,
    pub page: Vec<Row>,
    selected: Option<String>,
    detail: Option<Detail>,
    pending: Option<(u64, Read)>,
    generation: u64,
    dirty: bool,
    due: Instant,
    revision: u64,
    command: Option<oneshot::Receiver<Result<(), String>>>,
    confirm: Option<(Session, bool)>,
    error: Option<String>,
    message: Option<String>,
    editor: Option<AnyWindowHandle>,
}
impl NodesView {
    pub fn new(window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter node name or kernel…")
                .validate(|text, _| text.len() <= 256)
        });
        cx.subscribe(&input, |this, _, event, cx| {
            if matches!(event, InputEvent::PressEnter { .. })
                && let Some(row) = this.nodes.page.first()
            {
                let name = row.name.clone();
                this.nodes.select(name);
                cx.notify();
            }
            if matches!(event, InputEvent::Change) {
                this.nodes.generation += 1;
                this.nodes.selected = None;
                this.nodes.detail = None;
                this.nodes.dirty = true;
                this.nodes.due = Instant::now() + Duration::from_millis(200);
                this.nodes.confirm = None;
                cx.notify();
            }
        })
        .detach();
        Self {
            input,
            page: Vec::new(),
            selected: None,
            detail: None,
            pending: None,
            generation: 0,
            dirty: true,
            due: Instant::now(),
            revision: 0,
            command: None,
            confirm: None,
            error: None,
            message: None,
            editor: None,
        }
    }
    fn select(&mut self, name: String) {
        self.selected = Some(name);
        self.detail = None;
        self.confirm = None;
        self.generation += 1;
        self.dirty = true;
        self.due = Instant::now();
    }
    pub fn poll(
        &mut self,
        backend: &Handle,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> bool {
        let mut changed = false;
        if let Some(handle) = self.editor
            && handle.update(cx, |_, _, _| {}).is_err()
        {
            self.editor = None;
        }
        if let Some(reply) = self.command.as_mut() {
            match reply.try_recv() {
                Ok(result) => {
                    self.command = None;
                    self.confirm = None;
                    match result {
                        Ok(()) => {
                            self.message = Some("Daemon confirmed the interception change.".into());
                            self.error = None;
                        }
                        Err(error) => self.error = Some(error),
                    }
                    self.dirty = true;
                    self.due = Instant::now();
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.command = None;
                    self.error = Some(
                        "Node command stopped before confirmation; reconnect to verify its state"
                            .into(),
                    );
                    changed = true;
                }
                _ => {}
            }
        }
        if let Some((generation, reply)) = self.pending.as_mut() {
            match reply.try_recv() {
                Ok(result) => {
                    let current = *generation == self.generation;
                    self.pending = None;
                    if current {
                        match result {
                            Ok((page, detail)) => {
                                self.page = page;
                                self.detail = detail;
                                self.error = None;
                            }
                            Err(error) => self.error = Some(error),
                        }
                        changed = true;
                    }
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending = None;
                    self.error = Some("Node reader stopped".into());
                    changed = true;
                }
                _ => {}
            }
        }
        let revision = backend
            .history
            .health
            .nodes_revision
            .load(Ordering::Acquire);
        if revision != self.revision {
            self.dirty = true;
        }
        if self.dirty
            && self.pending.is_none()
            && Instant::now() >= self.due
            && let Some(permit) = nodes::read_permit()
        {
            let path = backend.history.database_path().to_owned();
            let search = self.input.read(cx).value().to_string();
            let selected = self.selected.clone();
            let (tx, rx) = oneshot::channel();
            cx.background_executor()
                .spawn(async move {
                    let _permit = permit;
                    let result = (|| -> anyhow::Result<_> {
                        let page = nodes::page(&path, &search)?;
                        let detail = selected
                            .filter(|name| page.iter().any(|row| &row.name == name))
                            .map(|name| nodes::detail(&path, &name))
                            .transpose()?;
                        Ok((page, detail))
                    })();
                    let _ = tx.send(result.map_err(|error| format!("{error:#}")));
                })
                .detach();
            self.pending = Some((self.generation, rx));
            self.dirty = false;
            self.revision = revision;
            self.due = Instant::now() + Duration::from_secs(1);
        }
        let _ = window;
        changed
    }
    fn current(&self, backend: &Handle) -> Option<Session> {
        let detail = self.detail.as_ref()?;
        backend
            .node(&detail.row.name)
            .filter(|session| session.id == detail.row.session && session.notifications)
    }
}
impl Dashboard {
    pub(super) fn render_nodes(&mut self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let current = self.nodes.current(&self.backend);
        let busy = self.nodes.command.is_some();
        let uncertain = current
            .as_ref()
            .is_some_and(|session| self.backend.history.health.catalogues.uncertain(session.id));
        let mut body=div().flex_1().min_h_0().px_6().pb_3().flex().flex_col().gap_3().child(div().h_8().flex_shrink_0().child(Input::new(&self.nodes.input).id("nodes-search")))
   .child(div().text_sm().text_color(muted).child(format!("{} connected nodes · {} cached nodes in this search. Choose a node to manage its application rules, interception or daemon configuration.",self.backend.status().sessions.len(),self.nodes.page.len())))
   .when_some(self.nodes.error.clone(),|body,error|body.child(super::banner(error,cx))).when_some(self.nodes.message.clone(),|body,message|body.child(div().text_sm().child(message)));
        let rows = uniform_list(
            "nodes-rows",
            self.nodes.page.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| {
                        let row = &this.nodes.page[index];
                        let node = this.backend.node(&row.name);
                        let online = node.as_ref().is_some_and(|session| {
                            session.id == row.session && session.notifications
                        });
                        let name = row.name.clone();
                        ListItem::new(("node-row", index))
                            .h(px(64.))
                            .w_full()
                            .px_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .selected(this.nodes.selected.as_ref() == Some(&row.name))
                            .child(
                                div()
                                    .flex()
                                    .gap_3()
                                    .items_center()
                                    .child(div().flex_1().text_sm().child(row.name.clone()))
                                    .child(div().w(px(150.)).text_sm().child(if online {
                                        "Connected"
                                    } else {
                                        "Offline / cached"
                                    }))
                                    .child(div().w(px(190.)).text_sm().child(if row.running {
                                        "Interception enabled"
                                    } else {
                                        "Interception disabled"
                                    })),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.nodes.select(name.clone());
                                cx.notify();
                            }))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .min_h_0()
        .w_full();
        body = body.child(
            div()
                .flex_1()
                .min_h_0()
                .border_1()
                .border_color(border)
                .rounded_lg()
                .overflow_hidden()
                .flex()
                .flex_col()
                .when(self.nodes.page.is_empty(), |body| {
                    body.child(div().p_4().child(if self.nodes.pending.is_some() {
                        "Loading nodes…"
                    } else {
                        "No matching cached nodes."
                    }))
                })
                .child(rows),
        );
        if let Some(detail) = self.nodes.detail.as_ref() {
            body = body.child(div().text_sm().child(format!(
                    "{} · kernel {} · reported log level {} · last confirmed {} UTC",
                    detail.row.name,
                    detail.row.kernel,
                    detail.row.log_level,
                    chrono::DateTime::from_timestamp_nanos(detail.row.updated)
                        .format("%Y-%m-%d %H:%M:%S")
                )));
        }
        if uncertain {
            body=body.child(super::banner("A command failed or timed out for this node. The displayed values are last confirmed; reconnect to verify its live state.".into(),cx));
        }
        if let Some((session, running)) = self.nodes.confirm.as_ref() {
            let session = session.clone();
            body = body
                .child(div().text_sm().child(format!(
                    "{} interception on {}? {}",
                    if *running { "Start" } else { "Stop" },
                    session.name,
                    if *running {
                        "The daemon will resume interception using its saved policy."
                    } else {
                        "The daemon will stop intercepting application traffic."
                    }
                )))
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            Button::new("nodes-confirm")
                                .label("Confirm interception change")
                                .disabled(
                                    busy || !self
                                        .backend
                                        .session(session.id)
                                        .is_some_and(|node| node.notifications),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some((session, running)) = this.nodes.confirm.as_ref() {
                                        match this.backend.control.submit_node(
                                            session.id,
                                            nodes::Change::Interception(*running),
                                        ) {
                                            Ok(reply) => this.nodes.command = Some(reply),
                                            Err(error) => this.nodes.error = Some(error),
                                        }
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("nodes-cancel")
                                .label("Cancel")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.nodes.confirm = None;
                                    cx.notify();
                                })),
                        ),
                );
        }
        body.child(
            div()
                .flex()
                .gap_3()
                .flex_wrap()
                .child(
                    Button::new("nodes-rules")
                        .label("Use node for application rules")
                        .disabled(current.is_none() || busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(session) = this.nodes.current(&this.backend) {
                                match this.backend.select_node(session.id) {
                                    Ok(()) => {
                                        this.screen = Screen::Rules;
                                        this.rules.input.focus_handle(cx).focus(window, cx);
                                    }
                                    Err(error) => this.nodes.error = Some(error),
                                }
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("nodes-interception")
                        .label(
                            if self
                                .nodes
                                .detail
                                .as_ref()
                                .is_some_and(|detail| detail.row.running)
                            {
                                "Stop interception…"
                            } else {
                                "Start interception…"
                            },
                        )
                        .disabled(current.is_none() || busy || uncertain)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(session) = this.nodes.current(&this.backend)
                                && let Some(detail) = this.nodes.detail.as_ref()
                            {
                                this.nodes.confirm = Some((session, !detail.row.running));
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("nodes-config")
                        .label("Daemon configuration…")
                        .disabled(current.is_none() || busy || uncertain)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(handle) = this.nodes.editor {
                                let _ = handle.update(cx, |_, window, _| window.activate_window());
                                return;
                            } else if let (Some(session), Some(detail)) = (
                                this.nodes.current(&this.backend),
                                this.nodes.detail.as_ref(),
                            ) {
                                match super::daemon_preferences::open(
                                    this.backend.clone(),
                                    session.id,
                                    detail.row.name.clone(),
                                    detail.config.clone(),
                                    cx,
                                ) {
                                    Ok(handle) => this.nodes.editor = Some(handle),
                                    Err(error) => this.nodes.error = Some(error.to_string()),
                                }
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("nodes-refresh")
                        .label("Refresh node inventory")
                        .disabled(current.is_none() || busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(session) = this.nodes.current(&this.backend)
                                && let Err(error) = this.backend.refresh_rules(session.id)
                            {
                                this.nodes.error = Some(error);
                            }
                            cx.notify();
                        })),
                ),
        )
    }
}
