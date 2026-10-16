//! Checksum reads and acknowledged updates never gate AskRule replies.
use crate::{
    backend::Handle,
    protocol::{Action, Rule},
    rules::{
        self,
        checksums::{self, Context, Page},
    },
};
use gpui::{prelude::*, *};
use gpui_kit::component::{ActiveTheme, Disableable, button::Button, list::ListItem};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

static REVISION: AtomicU64 = AtomicU64::new(0);
type ReadResult<T> = oneshot::Receiver<Result<T, String>>;

fn read<T: Send + 'static>(
    cx: &App,
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Option<ReadResult<T>> {
    let permit = checksums::permit()?;
    let (tx, rx) = oneshot::channel();
    cx.background_executor()
        .spawn(async move {
            let _permit = permit;
            let _ = tx.send(work().map_err(|error| format!("{error:#}")));
        })
        .detach();
    Some(rx)
}

pub(super) struct Probe {
    pub context: Context,
    pub found: bool,
    pub error: Option<String>,
    pending: Option<ReadResult<Page>>,
    done: bool,
    due: Instant,
    revision: u64,
}
impl Probe {
    pub fn new(connection: &crate::protocol::Connection) -> Option<Self> {
        Some(Self {
            context: Context::from_connection(connection)?,
            found: false,
            error: None,
            pending: None,
            done: false,
            due: Instant::now(),
            revision: REVISION.load(Ordering::Relaxed),
        })
    }
    pub fn poll(&mut self, backend: &Handle, session: u64, cx: &App) {
        let revision = REVISION.load(Ordering::Relaxed);
        if revision != self.revision && self.pending.is_none() {
            self.done = false;
            self.revision = revision;
        }
        if let Some(rx) = self.pending.as_mut() {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    self.done = true;
                    match result {
                        Ok(page) => {
                            self.found = !page.changes.is_empty();
                            self.error = None;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending = None;
                    self.done = true;
                    self.error = Some(
                        "Checksum catalogue check stopped; permission actions remain available"
                            .into(),
                    );
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        if self.done || self.pending.is_some() || Instant::now() < self.due {
            return;
        }
        self.due = Instant::now() + Duration::from_secs(1);
        if !backend.history.health.catalogues.ready(session) {
            self.error = Some("Waiting for this daemon's saved rule inventory".into());
            return;
        }
        let Some(node) = backend.session(session) else {
            return;
        };
        let path = backend.history.database.clone();
        let context = self.context.clone();
        self.pending = read(cx, move || checksums::page(&path, &node.name, &context, ""));
    }
}

struct OpenReview {
    handle: AnyWindowHandle,
    prompt: u64,
}
impl Global for OpenReview {}

pub(super) fn open(
    backend: Handle,
    prompt: u64,
    session: u64,
    context: Context,
    cx: &mut App,
) -> gpui::Result<()> {
    if let Some(open) = cx.try_global::<OpenReview>() {
        let handle = open.handle;
        let same = open.prompt == prompt;
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            if !same {
                anyhow::bail!(
                    "Another checksum review is open. Finish or close it before reviewing this request."
                );
            }
            return Ok(());
        }
    }
    let node = backend
        .session(session)
        .ok_or_else(|| {
            anyhow::anyhow!("The daemon changed; this request's rules can no longer be updated")
        })?
        .name;
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(940.), px(760.)),
                cx,
            ))),
            window_min_size: Some(size(px(700.), px(600.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Rustleblower — checksum review".into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| Review::new(backend, session, node, context, window, cx)),
    )?;
    cx.set_global(OpenReview { handle, prompt });
    Ok(())
}

enum Plan {
    Selected(Box<Rule>),
    All,
}
enum ReadKind {
    Browse,
    All,
}
struct Job {
    all: bool,
    more: bool,
    after: String,
    updated: usize,
    attempted: usize,
    stop: bool,
}
struct Review {
    backend: Handle,
    session: u64,
    node: String,
    context: Context,
    page: Page,
    selected: Option<usize>,
    plan: Option<Plan>,
    job: Option<Job>,
    query: Option<(ReadKind, ReadResult<Page>)>,
    prepare: Option<ReadResult<Rule>>,
    change: Option<ReadResult<()>>,
    queue: VecDeque<Rule>,
    browse_after: String,
    dirty: bool,
    error: Option<String>,
    message: Option<String>,
    focus: FocusHandle,
}
impl Review {
    fn new(
        backend: Handle,
        session: u64,
        node: String,
        context: Context,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let busy = weak
                .upgrade()
                .is_some_and(|view| view.read(cx).job.is_some());
            if busy {
                window.minimize_window();
            }
            !busy
        });
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this
                    .update_in(cx, |this, _, cx| {
                        this.poll(cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            backend,
            session,
            node,
            context,
            page: Page::default(),
            selected: None,
            plan: None,
            job: None,
            query: None,
            prepare: None,
            change: None,
            queue: VecDeque::new(),
            browse_after: String::new(),
            dirty: true,
            error: None,
            message: None,
            focus,
        }
    }
    fn online(&self) -> bool {
        self.backend
            .session(self.session)
            .is_some_and(|node| node.notifications)
            && self.backend.history.health.catalogues.ready(self.session)
            && !self
                .backend
                .history
                .health
                .catalogues
                .uncertain(self.session)
    }

    fn fail(&mut self, error: String) {
        if let Some(job) = self.job.take() {
            self.message = Some(format!(
                "{} update(s) confirmed before stopping. Reload to review the remaining rules.",
                job.updated
            ));
        }
        self.queue.clear();
        self.plan = None;
        self.error = Some(error);
    }
    fn begin(&mut self) {
        let Some(plan) = self.plan.take() else {
            return;
        };
        self.queue.clear();
        self.error = None;
        self.message = None;
        let all = matches!(plan, Plan::All);
        if let Plan::Selected(rule) = plan {
            self.queue.push_back(*rule);
        }
        self.job = Some(Job {
            all,
            more: all,
            after: String::new(),
            updated: 0,
            attempted: 0,
            stop: false,
        });
    }
    fn poll(&mut self, cx: &App) {
        if let Some(rx) = self.change.as_mut() {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.change = None;
                    if let Some(job) = self.job.as_mut() {
                        job.updated += 1;
                    }
                    REVISION.fetch_add(1, Ordering::Relaxed);
                }
                Ok(Err(error)) => {
                    self.change = None;
                    self.fail(error);
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.change = None;
                    self.fail("The command ended without confirmation; refresh the daemon inventory before retrying".into());
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = self.prepare.as_mut() {
            match rx.try_recv() {
                Ok(result) => {
                    self.prepare = None;
                    if self.job.as_ref().is_some_and(|job| !job.stop) {
                        match result {
                            Ok(rule) if self.online() => match self.backend.control.submit(
                                self.session,
                                Action::ChangeRule,
                                rule,
                            ) {
                                Ok(rx) => self.change = Some(rx),
                                Err(error) => self.fail(error),
                            },
                            Ok(_) => self.fail(
                                "Daemon inventory changed; reconnect and review before updating"
                                    .into(),
                            ),
                            Err(error) => self.fail(error),
                        }
                    }
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.prepare = None;
                    self.fail("Checksum preparation stopped; reload this review".into());
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        if let Some((kind, rx)) = self.query.as_mut() {
            match rx.try_recv() {
                Ok(result) => {
                    let all = matches!(kind, ReadKind::All);
                    self.query = None;
                    match result {
                        Ok(page) if all => {
                            if let Some(job) = self.job.as_mut()
                                && !job.stop
                            {
                                job.after = page.after;
                                job.more = page.has_more;
                                self.queue =
                                    page.changes.into_iter().map(|change| change.rule).collect();
                            }
                        }
                        Ok(page) => {
                            self.page = page;
                            self.selected = (!self.page.changes.is_empty()).then_some(0);
                            self.error = None;
                        }
                        Err(error) => self.fail(error),
                    }
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.query = None;
                    self.fail("Checksum catalogue check stopped; reload this review".into());
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        if self.prepare.is_some() || self.change.is_some() || self.query.is_some() {
            return;
        }
        if self.job.is_some() && !self.online() {
            self.fail(
                "Daemon connection or rule inventory changed; reload it before retrying".into(),
            );
            return;
        }
        if let Some(job) = self.job.as_ref() {
            if job.stop || (self.queue.is_empty() && !(job.all && job.more)) {
                self.message = Some(format!(
                    "{} update(s) confirmed.{} Permission decisions remain separate.",
                    job.updated,
                    if job.stop {
                        " Stopped before sending further updates."
                    } else {
                        ""
                    }
                ));
                self.job = None;
                self.queue.clear();
                self.browse_after.clear();
                self.dirty = true;
            } else if let Some(expected) = self.queue.front() {
                if job.attempted >= rules::MAX_RULES {
                    self.fail("Stopped at the 10,000-rule workflow limit; reload to review remaining changes".into());
                    return;
                }
                let path = self.backend.history.database.clone();
                let node = self.node.clone();
                let context = self.context.clone();
                let expected = expected.clone();
                if let Some(rx) = read(cx, move || {
                    checksums::prepare(&path, &node, &expected, &context)
                }) {
                    self.queue.pop_front();
                    self.prepare = Some(rx);
                    self.job.as_mut().unwrap().attempted += 1;
                }
                return;
            }
        }
        let (kind, after) = if let Some(job) = self.job.as_ref() {
            (ReadKind::All, job.after.clone())
        } else if self.dirty {
            (ReadKind::Browse, self.browse_after.clone())
        } else {
            return;
        };
        let path = self.backend.history.database.clone();
        let node = self.node.clone();
        let context = self.context.clone();
        if let Some(rx) = read(cx, move || checksums::page(&path, &node, &context, &after)) {
            self.query = Some((kind, rx));
            self.dirty = false;
        }
    }
}

impl Render for Review {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.poll(cx);
        let busy = self.job.is_some();
        let online = self.online();
        let rows = uniform_list(
            "checksum-rules",
            self.page.changes.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| {
                        let rule = &this.page.changes[index].rule;
                        ListItem::new(("checksum-rule", index))
                            .h(px(54.))
                            .selected(this.selected == Some(index))
                            .child(
                                div().flex().flex_col().child(rule.name.clone()).child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "{} · {} · {} changed checksum condition(s)",
                                            rule.action,
                                            rule.duration,
                                            this.page.changes[index].differences.len()
                                        )),
                                ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.job.is_none() && this.plan.is_none() {
                                    this.selected = Some(index);
                                    cx.notify();
                                }
                            }))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .min_h_0()
        .w_full();
        let selected = self.selected.and_then(|index| self.page.changes.get(index));
        let mut body = div().size_full().p_5().flex().flex_col().gap_3().track_focus(&self.focus).bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(div().text_xl().child("Executable checksum changes"))
            .child(div().text_sm().child(format!("{} · {}", self.node, self.context.process)))
            .child(div().text_sm().child("Saved checksums differ from this request's reported binary. Updating trusts the reported checksum and preserves the rule's other settings. It does not answer the pending connection."))
            .when(!online, |body| body.child(super::banner("Updates require this daemon's connected, confirmed rule inventory.".into(), cx)))
            .when_some(self.error.clone(), |body, error| body.child(super::banner(error, cx)))
            .when_some(self.message.clone(), |body, message| body.child(div().text_sm().child(message)))
            .child(div().flex().gap_3()
                .child(Button::new("checksum-reload").label("First page / reload").disabled(busy || self.query.is_some()).on_click(cx.listener(|this, _, _, cx| { this.browse_after.clear(); this.dirty = true; this.plan = None; cx.notify(); })))
                .child(Button::new("checksum-next").label("Next page").disabled(busy || self.query.is_some() || !self.page.has_more).on_click(cx.listener(|this, _, _, cx| { this.browse_after = this.page.after.clone(); this.dirty = true; this.plan = None; cx.notify(); }))))
            .child(rows);
        if self.page.changes.is_empty() {
            body = body.child(
                div()
                    .text_sm()
                    .child(if self.query.is_some() || self.dirty {
                        "Checking saved rules…"
                    } else {
                        "No changed reported checksums in enabled rules for this executable."
                    }),
            );
        }
        if let Some(selected) = selected {
            let differences = selected.differences.clone();
            body = body.child(
                uniform_list(
                    "checksum-differences",
                    differences.len(),
                    move |range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| {
                                let d = &differences[index];
                                div()
                                    .h(px(76.))
                                    .flex()
                                    .flex_col()
                                    .child(div().text_sm().child(d.operand.clone()))
                                    .child(div().text_xs().child(format!(
                                        "Saved: {}",
                                        crate::history::bounded(&d.expected, 1024)
                                    )))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!("Reported: {}", d.reported)),
                                    )
                            })
                            .collect::<Vec<_>>()
                    },
                )
                .h(px(160.))
                .w_full(),
            );
        }
        if self.plan.is_some() {
            body = body.child(div().text_sm().child(if matches!(self.plan, Some(Plan::All)) { "Update every changed checksum rule for this executable across all pages? Commands are sent one at a time and stop on failure." } else { "Update the selected rule with the reported checksum(s)?" }))
                .child(div().flex().gap_3().child(Button::new("checksum-confirm").label("Confirm checksum update").disabled(!online).on_click(cx.listener(|this, _, _, cx| { this.begin(); cx.notify(); })))
                    .child(Button::new("checksum-cancel").label("Cancel").on_click(cx.listener(|this, _, _, cx| { this.plan = None; cx.notify(); }))));
        }
        if let Some(job) = self.job.as_ref() {
            body = body.child(div().text_sm().child(format!(
                "{} update(s) confirmed. {}",
                job.updated,
                if job.stop {
                    "Stopping after the in-flight command…"
                } else {
                    "Waiting for daemon confirmation…"
                }
            )));
        }
        body.child(
            div()
                .flex()
                .gap_3()
                .flex_wrap()
                .child(
                    Button::new("checksum-update-selected")
                        .label("Update selected…")
                        .disabled(!online || busy || selected.is_none() || self.plan.is_some())
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(change) =
                                this.selected.and_then(|index| this.page.changes.get(index))
                            {
                                this.plan = Some(Plan::Selected(Box::new(change.rule.clone())));
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("checksum-update-all")
                        .label("Update all changed rules…")
                        .disabled(
                            !online || busy || self.page.changes.is_empty() || self.plan.is_some(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.plan = Some(Plan::All);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("checksum-stop")
                        .label("Stop")
                        .disabled(!busy || self.job.as_ref().is_some_and(|job| job.stop))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(job) = this.job.as_mut() {
                                job.stop = true;
                            }
                            this.queue.clear();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("checksum-close")
                        .label("Close")
                        .disabled(busy)
                        .on_click(|_, window, _| window.remove_window()),
                ),
        )
    }
}
