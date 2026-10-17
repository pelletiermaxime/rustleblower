//! Alert browsing and export use background readers; only bounded summaries render.
use super::Dashboard;
use crate::{
    alerts::{self, Change, Detail, Page, Query},
    history::History,
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
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, oneshot};
struct Preview {
    detail: Detail,
    lines: Vec<String>,
    shortened: bool,
}
enum Read {
    Page(Page),
    Detail(Option<Preview>),
}
type Pending = (u64, oneshot::Receiver<Result<Read, String>>);
pub(super) struct AlertsView {
    pub input: Entity<InputState>,
    node: Entity<InputState>,
    path: Entity<InputState>,
    pub page: Page,
    query: Query,
    selected: Option<i64>,
    detail: Option<Preview>,
    detail_due: bool,
    pending: Option<Pending>,
    generation: u64,
    dirty: bool,
    due: Instant,
    revision: (u64, u64),
    older: Vec<Option<i64>>,
    command: Option<oneshot::Receiver<Result<usize, String>>>,
    confirm: Option<Change>,
    exporting: Option<oneshot::Receiver<Result<alerts::Report, String>>>,
    stop: Arc<AtomicBool>,
    replace: bool,
    message: Option<String>,
    error: Option<String>,
}
fn permit() -> Option<tokio::sync::OwnedSemaphorePermit> {
    static READ: OnceLock<Arc<Semaphore>> = OnceLock::new();
    READ.get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
        .try_acquire_owned()
        .ok()
}
impl Drop for AlertsView {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl AlertsView {
    pub fn new(window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "Search alert text or payload…",
                    window,
                    cx,
                ))
                .validate(|v, _| v.len() <= 256)
        });
        let node = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "Exact node name (all when empty)",
                    window,
                    cx,
                ))
                .validate(|v, _| v.len() <= 128)
        });
        let path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "/absolute/path/alerts.jsonl",
                    window,
                    cx,
                ))
                .validate(|v, _| v.len() <= 4096)
        });
        cx.subscribe(&input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.alerts.query.search = input.read(cx).value().to_string();
                this.alerts.reset();
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&node, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.alerts.query.node = input.read(cx).value().to_string();
                this.alerts.reset();
                cx.notify();
            }
        })
        .detach();
        Self {
            input,
            node,
            path,
            page: Page::default(),
            query: Query::default(),
            selected: None,
            detail: None,
            detail_due: false,
            pending: None,
            generation: 0,
            dirty: true,
            due: Instant::now(),
            revision: (0, 0),
            older: Vec::new(),
            command: None,
            confirm: None,
            exporting: None,
            stop: Arc::new(AtomicBool::new(false)),
            replace: false,
            message: None,
            error: None,
        }
    }
    fn reset(&mut self) {
        self.generation += 1;
        self.dirty = true;
        self.due = Instant::now() + Duration::from_millis(200);
        self.query.before = None;
        self.query.ceiling = None;
        self.selected = None;
        self.detail = None;
        self.detail_due = false;
        self.confirm = None;
        self.older.clear();
    }
    fn select(&mut self, id: i64) {
        self.selected = Some(id);
        self.detail = None;
        self.detail_due = true;
        self.generation += 1;
        self.confirm = None;
    }
    pub fn poll(&mut self, history: &History, cx: &mut Context<Dashboard>) -> bool {
        let mut changed = false;
        if let Some(reply) = self.command.as_mut() {
            match reply.try_recv() {
                Ok(result) => {
                    self.command = None;
                    self.confirm = None;
                    self.message = Some(match result {
                        Ok(count) => crate::i18n::tr_args(
                            "Updated %{count} archived alert(s).",
                            &[("count", &format!("{}", count))],
                        ),
                        Err(error) => {
                            self.error = Some(error);
                            "Alert archive was not changed successfully.".into()
                        }
                    });
                    self.reset();
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.command = None;
                    self.error = Some(crate::i18n::tr("Alert archive command stopped"));
                    changed = true;
                }
                _ => {}
            }
        }
        if let Some(reply) = self.exporting.as_mut() {
            match reply.try_recv() {
                Ok(result) => {
                    self.exporting = None;
                    match result {
                        Ok(report) => {
                            self.message = Some(crate::i18n::tr_args(
                                "Exported %{arg0} alert(s), %{arg1} bytes.",
                                &[
                                    ("arg0", &format!("{}", report.rows)),
                                    ("arg1", &format!("{}", report.bytes)),
                                ],
                            ))
                        }
                        Err(error) => self.error = Some(error),
                    }
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.exporting = None;
                    self.error = Some(crate::i18n::tr("Alert export stopped before completion"));
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
                            Ok(Read::Page(page)) => {
                                self.page = page;
                                self.error = None;
                            }
                            Ok(Read::Detail(detail)) => {
                                self.detail = detail;
                                if self.detail.is_none() {
                                    self.error =
                                        Some(crate::i18n::tr("This alert is no longer retained."));
                                }
                            }
                            Err(error) => self.error = Some(error),
                        }
                        changed = true;
                    }
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending = None;
                    self.error = Some(crate::i18n::tr("Alert reader stopped"));
                    changed = true;
                }
                _ => {}
            }
        }
        let revision = (
            history.health.alerts_revision.load(Ordering::Acquire),
            history.health.revision.load(Ordering::Acquire),
        );
        if self.revision != revision {
            self.revision = revision;
            if self.query.ceiling.is_none() {
                self.dirty = true;
            }
        }
        if self.pending.is_none()
            && (self.detail_due || (self.dirty && Instant::now() >= self.due))
            && let Some(permit) = permit()
        {
            let path = history.database.clone();
            let q = self.query.clone();
            let id = if self.detail_due { self.selected } else { None };
            self.detail_due = false;
            if id.is_none() {
                self.dirty = false;
                self.due = Instant::now() + Duration::from_millis(500);
            }
            let (tx, rx) = oneshot::channel();
            self.pending = Some((self.generation, rx));
            cx.background_executor()
                .spawn(async move {
                    let _permit = permit;
                    let result = (|| -> anyhow::Result<Read> {
                        if let Some(id) = id {
                            let detail = alerts::detail(&path, id)?;
                            Ok(Read::Detail(detail.map(|detail| {
                                let formatted =
                                    serde_json::from_str::<serde_json::Value>(&detail.document)
                                        .and_then(|v| serde_json::to_string_pretty(&v))
                                        .unwrap_or_else(|_| detail.document.clone());
                                let mut shortened = false;
                                let lines = formatted
                                    .lines()
                                    .take(2048)
                                    .map(|line| {
                                        shortened |= line.len() > 512;
                                        crate::history::bounded(line, 512)
                                    })
                                    .collect::<Vec<_>>();
                                shortened |= formatted.lines().count() > 2048;
                                Preview {
                                    detail,
                                    lines,
                                    shortened,
                                }
                            })))
                        } else {
                            Ok(Read::Page(alerts::page(&path, &q)?))
                        }
                    })()
                    .map_err(|error| format!("{error:#}"));
                    let _ = tx.send(result);
                })
                .detach();
        }
        changed
    }
    fn change(&mut self, history: &History, change: Change) {
        match history.change_alert(change) {
            Ok(reply) => self.command = Some(reply),
            Err(error) => self.error = Some(error),
        }
    }
    fn export(&mut self, history: &History, format: alerts::Format, cx: &mut Context<Dashboard>) {
        if self.exporting.is_some() {
            return;
        }
        let Some(permit) = permit() else {
            self.error = Some(crate::i18n::tr(
                "Alert reader is busy; retry after the current read finishes.",
            ));
            return;
        };
        let path = std::path::PathBuf::from(self.path.read(cx).value().to_string());
        let database = history.database.clone();
        let mut query = self.query.clone();
        query.before = None;
        query.ceiling = Some(self.page.ceiling);
        let replace = self.replace;
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = self.stop.clone();
        let (tx, rx) = oneshot::channel();
        self.exporting = Some(rx);
        self.error = None;
        self.message = None;
        cx.background_executor()
            .spawn(async move {
                let _permit = permit;
                let _ = tx.send(
                    alerts::export(&database, &query, &path, format, replace, &stop)
                        .map_err(|error| format!("{error:#}")),
                );
            })
            .detach();
    }
}
impl Dashboard {
    pub(super) fn render_alerts(&mut self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let busy = self.alerts.command.is_some();
        let exporting = self.alerts.exporting.is_some();
        let skipped = self
            .backend
            .history
            .health
            .alerts_skipped
            .load(Ordering::Relaxed);
        let archive_error = self
            .backend
            .history
            .health
            .alerts_error
            .lock()
            .unwrap()
            .clone();
        let filters = div()
            .flex()
            .gap_3()
            .child(
                div()
                    .h_8()
                    .flex_1()
                    .child(Input::new(&self.alerts.input).id("alerts-search")),
            )
            .child(
                div()
                    .h_8()
                    .w(px(280.))
                    .child(Input::new(&self.alerts.node).id("alerts-node")),
            );
        let severity = div()
            .flex()
            .items_center()
            .gap_2()
            .children(
                [
                    (None, "All"),
                    (Some(0), "Errors"),
                    (Some(1), "Warnings"),
                    (Some(2), "Info"),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, (value, label))| {
                    Button::new(("alerts-severity", index))
                        .label(crate::i18n::tr(label))
                        .selected(self.alerts.query.severity == value)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.alerts.query.severity = value;
                            this.alerts.reset();
                            cx.notify();
                        }))
                }),
            )
            .child(
                Checkbox::new("alerts-unread")
                    .label(crate::i18n::tr("Unread only"))
                    .checked(self.alerts.query.unread)
                    .on_change(cx.listener(|this, value: &bool, _, cx| {
                        this.alerts.query.unread = *value;
                        this.alerts.reset();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("alerts-latest")
                    .label(crate::i18n::tr("Latest"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.alerts.reset();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("alerts-previous")
                    .label(crate::i18n::tr("Previous"))
                    .disabled(self.alerts.older.is_empty() || self.alerts.pending.is_some())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(before) = this.alerts.older.pop() {
                            this.alerts.query.before = before;
                            this.alerts.generation += 1;
                            this.alerts.dirty = true;
                            this.alerts.due = Instant::now();
                            this.alerts.selected = None;
                            this.alerts.detail = None;
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("alerts-older")
                    .label(crate::i18n::tr("Older"))
                    .disabled(
                        !self.alerts.page.has_more
                            || self.alerts.pending.is_some()
                            || self.alerts.older.len() >= 64,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.alerts.older.push(this.alerts.query.before);
                        this.alerts.query.before = this.alerts.page.rows.last().map(|r| r.id);
                        this.alerts.query.ceiling = Some(this.alerts.page.ceiling);
                        this.alerts.generation += 1;
                        this.alerts.dirty = true;
                        this.alerts.due = Instant::now();
                        this.alerts.selected = None;
                        this.alerts.detail = None;
                        cx.notify();
                    })),
            );
        let list = uniform_list(
            "alerts-list",
            self.alerts.page.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let highlights = this.backend.highlighting.snapshot();
                range
                    .map(|index| {
                        let row = &this.alerts.page.rows[index];
                        let id = row.id;
                        let severity = crate::i18n::presentation(&alerts::severity(row.severity));
                        let time = alerts::timestamp(row.received);
                        let read = if row.read {
                            String::new()
                        } else {
                            crate::i18n::tr("Unread")
                        };
                        let cells = [
                            ("severity", severity.as_str()),
                            ("node", row.node.as_str()),
                            ("time", time.as_str()),
                            ("read", read.as_str()),
                            ("summary", row.summary.as_str()),
                        ];
                        let prepared = highlights.prepare_row(
                            crate::highlighting::View::Alerts,
                            &cells,
                            this.alerts.selected == Some(id),
                        );
                        let style = |key: &str, value: &str, cell: Div| {
                            super::highlighting::cell_prepared(
                                &highlights,
                                crate::highlighting::View::Alerts,
                                key,
                                value,
                                prepared,
                                cell,
                            )
                        };
                        ListItem::new(("alert-row", id as u64))
                            .selected(this.alerts.selected == Some(id))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .child(style(
                                                "severity",
                                                &severity,
                                                div().text_sm().child(severity.clone()),
                                            ))
                                            .child(style(
                                                "node",
                                                &row.node,
                                                div().text_sm().child(row.node.clone()),
                                            ))
                                            .child(style(
                                                "time",
                                                &time,
                                                div().text_sm().child(time.clone()),
                                            ))
                                            .child(style(
                                                "read",
                                                &read,
                                                div().text_sm().child(read.clone()),
                                            )),
                                    )
                                    .child(style(
                                        "summary",
                                        &row.summary,
                                        div().text_sm().truncate().child(row.summary.clone()),
                                    )),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.alerts.select(id);
                                cx.notify();
                            }))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .flex_1()
        .min_h_0();
        let mut details = div()
            .w(px(420.))
            .min_w(px(300.))
            .flex()
            .flex_col()
            .gap_2()
            .px_3();
        if let Some(preview) = self.alerts.detail.as_ref() {
            details = details
                .child(div().text_sm().child(format!(
                    "{} · {} · {}",
                    alerts::severity(preview.detail.row.severity),
                    alerts::priority(preview.detail.row.priority),
                    preview.detail.row.kind
                )))
                .child(
                    Button::new("alerts-copy-detail")
                        .label(crate::i18n::tr("Copy complete JSON"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(preview) = this.alerts.detail.as_ref() {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    preview.detail.document.clone(),
                                ));
                            }
                        })),
                )
                .when(preview.shortened, |view| {
                    view.child(div().text_xs().text_color(muted).child(crate::i18n::tr(
                        "Preview shortened. Copy/export retains the complete payload.",
                    )))
                })
                .child(
                    uniform_list(
                        "alerts-detail",
                        preview.lines.len(),
                        cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                            range
                                .map(|index| {
                                    div().h_5().text_xs().truncate().child(
                                        this.alerts.detail.as_ref().unwrap().lines[index].clone(),
                                    )
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .flex_1()
                    .min_h_0(),
                );
        } else {
            details = details.child(div().text_sm().text_color(muted).child(
                if self.alerts.selected.is_some() {
                    crate::i18n::tr("Loading alert details…")
                } else {
                    crate::i18n::tr("Select an alert for details.")
                },
            ));
        }
        let actions = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                Button::new("alerts-read")
                    .label(crate::i18n::tr("Mark read"))
                    .disabled(busy || self.alerts.selected.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(id) = this.alerts.selected {
                            this.alerts.change(&this.backend.history, Change::Read(id));
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("alerts-delete")
                    .label(crate::i18n::tr("Delete selected…"))
                    .disabled(busy || self.alerts.selected.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(id) = this.alerts.selected {
                            this.alerts.confirm = Some(Change::Delete(id));
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("alerts-clear")
                    .label(crate::i18n::tr("Delete matching…"))
                    .disabled(busy || self.alerts.page.rows.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        let mut q = this.alerts.query.clone();
                        q.before = None;
                        q.ceiling = Some(this.alerts.page.ceiling);
                        this.alerts.confirm = Some(Change::Clear(q));
                        cx.notify();
                    })),
            )
            .child(
                Button::new("alerts-copy-page")
                    .label(crate::i18n::tr("Copy page as CSV"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        match alerts::clipboard(&this.alerts.page.rows) {
                            Ok(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
                            Err(error) => this.alerts.error = Some(error.to_string()),
                        }
                        cx.notify();
                    })),
            )
            .child(div().text_xs().text_color(muted).child(crate::i18n::tr(
                "Archive retention: up to 10,000 alerts / 2 MiB plus configured age.",
            )));
        let confirm = div()
            .flex()
            .gap_3()
            .child(div().text_sm().child(
                if matches!(self.alerts.confirm, Some(Change::Clear(_))) {
                    crate::i18n::tr("Delete all matching archived alerts? Newly arriving alerts are kept.")
                } else {
                    crate::i18n::tr("Delete this archived alert? Firewall rules and daemon state are unchanged.")
                },
            ))
            .child(
                Button::new("alerts-delete-confirm")
                    .label(crate::i18n::tr("Confirm delete"))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(change) = this.alerts.confirm.clone() {
                            this.alerts.change(&this.backend.history, change);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("alerts-delete-cancel")
                    .label(crate::i18n::tr("Cancel"))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.alerts.confirm = None;
                        cx.notify();
                    })),
            );
        let exports = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .h_8()
                    .flex_1()
                    .child(Input::new(&self.alerts.path).id("alerts-export-path")),
            )
            .child(
                Checkbox::new("alerts-export-replace")
                    .label(crate::i18n::tr("Replace"))
                    .checked(self.alerts.replace)
                    .disabled(exporting)
                    .on_change(cx.listener(|this, value: &bool, _, cx| {
                        this.alerts.replace = *value;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("alerts-export-csv")
                    .label(crate::i18n::tr("Export CSV"))
                    .disabled(exporting)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.alerts
                            .export(&this.backend.history, alerts::Format::Csv, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("alerts-export-json")
                    .label(crate::i18n::tr("Export JSON"))
                    .disabled(exporting)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.alerts
                            .export(&this.backend.history, alerts::Format::JsonLines, cx);
                        cx.notify();
                    })),
            )
            .child(
                Button::new("alerts-export-stop")
                    .label(crate::i18n::tr("Stop"))
                    .disabled(!exporting)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.alerts.stop.store(true, Ordering::Relaxed);
                        cx.notify();
                    })),
            );
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(filters)
            .child(severity)
            .when_some(self.backend.desktop_notices.error.lock().unwrap().clone(), |view, error| {
                view.child(div().text_sm().child(crate::i18n::tr_args("Desktop notifications: %{error}. Retained alerts remain available here.", &[("error", &(error).to_string())])))
            })
            .when(self.backend.desktop_notices.skipped.load(Ordering::Relaxed) > 0, |view| {
                view.child(div().text_sm().child(crate::i18n::tr_args("%{arg0} desktop notices skipped during bursts; archive recording is independent.", &[("arg0", &(self.backend.desktop_notices.skipped.load(Ordering::Relaxed)).to_string())])))
            })
            .when(skipped > 0, |view| {
                view.child(super::banner(
                    crate::i18n::tr_args("Alert archive incomplete: %{skipped} alert(s) could not be recorded.", &[("skipped", &format!("{}", skipped))]),
                    cx,
                ))
            })
            .when_some(archive_error, |view, error| {
                view.child(super::banner(error, cx))
            })
            .when_some(self.alerts.error.clone(), |view, error| {
                view.child(super::banner(error, cx))
            })
            .when_some(self.alerts.message.clone(), |view, message| {
                view.child(div().text_sm().child(crate::i18n::presentation(&message)))
            })
            .when(self.alerts.page.rows.is_empty(), |view| {
                view.child(div().text_sm().child(crate::i18n::tr("No matching retained alerts.")))
            })
            .child(div().flex_1().min_h_0().flex().child(list).child(details))
            .child(actions)
            .when(self.alerts.confirm.is_some(), |view| view.child(confirm))
            .child(exports)
    }
}
