use super::Dashboard;
use crate::history::{History, Page, Query};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable,
    button::Button,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    table::{DataTable, TableEvent, TableState},
};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

pub(super) struct HistoryView {
    pub(super) input: Entity<InputState>,
    pub(super) page: Page,
    pub(super) query: Query,
    pending: Option<(u64, oneshot::Receiver<Result<Page, String>>)>,
    generation: u64,
    dirty: bool,
    due: Instant,
    revision: u64,
    pub(super) error: Option<String>,
    loading: bool,
    scroll: UniformListScrollHandle,
    selected: Option<i64>,
    detail_loading: bool,
    pub(super) detail_window: Option<AnyWindowHandle>,
    actions_tx: std::sync::mpsc::SyncSender<super::connection_details::OpenRule>,
    pub(super) actions_rx: std::sync::mpsc::Receiver<super::connection_details::OpenRule>,
    processes_tx: std::sync::mpsc::SyncSender<super::connection_details::OpenProcess>,
    pub(super) processes_rx: std::sync::mpsc::Receiver<super::connection_details::OpenProcess>,
    table: Entity<TableState<super::history_table::Rows>>,
    options: crate::history::settings::Options,
    pub(super) memory: super::history_memory::Client,
    widths: Vec<(crate::history::query::Field, f32)>,
    tools_window: Option<AnyWindowHandle>,
    pub(super) preferences_window: Option<AnyWindowHandle>,
    tools_tx: std::sync::mpsc::SyncSender<super::history_tools::Message>,
    tools_rx: std::sync::mpsc::Receiver<super::history_tools::Message>,
    indexing: bool,
}

impl HistoryView {
    pub fn new(history: &History, window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let options = history.settings.snapshot();
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .placeholder("Filter executable or destination…")
                .validate(|text, _| text.len() <= 256);
            input.set_value(options.presentation.search.clone(), window, cx);
            input
        });
        cx.subscribe(&input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.history.query.search = crate::history::bounded(&input.read(cx).value(), 256);
                this.history.reset();
                cx.notify();
            }
        })
        .detach();
        let (actions_tx, actions_rx) = std::sync::mpsc::sync_channel(4);
        let (processes_tx, processes_rx) = std::sync::mpsc::sync_channel(4);
        let (tools_tx, tools_rx) = std::sync::mpsc::sync_channel(4);
        let widths = options.presentation.widths.clone();
        let table = cx.new(|cx| {
            TableState::new(
                super::history_table::Rows {
                    rows: Default::default(),
                    columns: options.columns.clone(),
                    sort: options.presentation.sort,
                    widths: widths
                        .iter()
                        .map(|(field, width)| (*field, px(*width)))
                        .collect(),
                    sender: tools_tx.clone(),
                },
                window,
                cx,
            )
            .col_movable(false)
        });
        cx.subscribe(&table, |this, table, event: &TableEvent, cx| {
            if let TableEvent::SelectRow(index) = event {
                this.history.selected = this.history.page.rows.get(*index).map(|row| row.id);
                cx.notify();
            }
            if let TableEvent::ColumnWidthsChanged(widths) = event {
                table.update(cx, |table, _| {
                    let delegate = table.delegate_mut();
                    delegate.widths = delegate
                        .columns
                        .iter()
                        .copied()
                        .zip(
                            widths
                                .iter()
                                .copied()
                                .map(|width| width.clamp(px(40.), px(2000.))),
                        )
                        .collect();
                    for (field, width) in &delegate.widths {
                        let width = (*field, (*width).into());
                        if let Some(old) = this.history.widths.iter_mut().find(|(f, _)| f == field)
                        {
                            *old = width;
                        } else {
                            this.history.widths.push(width);
                        }
                    }
                });
                this.history.remember(false);
            }
        })
        .detach();
        Self {
            input,
            page: Page::default(),
            query: options.presentation.query(),
            pending: None,
            generation: 0,
            dirty: true,
            due: Instant::now(),
            revision: 0,
            error: None,
            loading: true,
            scroll: UniformListScrollHandle::new(),
            selected: None,
            detail_loading: false,
            detail_window: None,
            actions_tx,
            actions_rx,
            processes_tx,
            processes_rx,
            table,
            memory: super::history_memory::Client::start(history.settings.clone(), cx),
            widths,
            options,
            tools_window: None,
            preferences_window: None,
            tools_tx,
            tools_rx,
            indexing: history.health.indexing.load(Ordering::Acquire),
        }
    }
    pub(super) fn drill(
        &mut self,
        filters: Vec<crate::history::query::Filter>,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) {
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.query = Query {
            filters,
            ..Default::default()
        };
        self.reset();
    }
    fn reset(&mut self) {
        self.query.before = None;
        self.query.cursor = None;
        self.query.ceiling = None;
        self.page = Page::default();
        self.selected = None;
        self.generation += 1;
        self.dirty = true;
        self.loading = true;
        self.due = Instant::now() + Duration::from_millis(200);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.remember(false);
    }
    pub(super) fn remember(&mut self, flush: bool) {
        self.memory.record(
            crate::history::settings::Presentation {
                search: self.query.search.clone(),
                action: self.query.action.clone(),
                filters: self.query.filters.clone(),
                sort: self.query.sort,
                widths: self.widths.clone(),
            },
            flush,
        );
    }
    pub fn poll(
        &mut self,
        history: &History,
        _window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> bool {
        let mut changed = false;
        let indexing = history.health.indexing.load(Ordering::Acquire);
        if indexing != self.indexing {
            self.indexing = indexing;
            if !indexing {
                self.dirty = true;
            }
            changed = true;
        }
        while let Ok(message) = self.tools_rx.try_recv() {
            match message {
                super::history_tools::Message::Sort(Some((field, _)))
                    if indexing && field.needs_metadata() =>
                {
                    self.error =
                        Some("Wait for older metadata indexing before sorting this column.".into())
                }
                super::history_tools::Message::Sort(sort) => {
                    self.query.sort = sort;
                    self.reset();
                }
                super::history_tools::Message::Filters(filters) => {
                    self.query.filters = filters;
                    self.reset();
                }
            }
            changed = true;
        }
        if let Some(window) = self.tools_window
            && window.update(cx, |_, _, _| {}).is_err()
        {
            self.tools_window = None;
        }
        if let Some(handle) = self.preferences_window
            && handle.update(cx, |_, _, _| {}).is_err()
        {
            self.preferences_window = None;
        }
        let options = history.settings.snapshot();
        if options.refresh_seconds != self.options.refresh_seconds {
            self.generation += 1;
            self.pending = None;
            self.loading = false;
            self.dirty = options.refresh_seconds > 0;
            self.due = Instant::now();
        }
        if options != self.options {
            self.options = options;
            changed = true;
        }
        if let Some((generation, rx)) = self.pending.as_mut() {
            match rx.try_recv() {
                Ok(result) => {
                    if *generation == self.generation {
                        match result {
                            Ok(page) => {
                                self.page = page;
                                self.error = None;
                            }
                            Err(e) => self.error = Some(e),
                        }
                        self.loading = false;
                        changed = true;
                    }
                    self.pending = None;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.error = Some("History worker stopped".into());
                    self.pending = None;
                    self.loading = false;
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        let revision = history.health.revision.load(Ordering::Acquire);
        if self.query.before.is_none()
            && self.query.cursor.is_none()
            && self.options.refresh_seconds > 0
            && revision != self.revision
            && Instant::now() >= self.due
        {
            self.dirty = true;
        }
        if self.dirty
            && self.pending.is_none()
            && Instant::now() >= self.due
            && let Some(rx) = history.query(self.query.clone())
        {
            self.pending = Some((self.generation, rx));
            self.dirty = false;
            self.revision = revision;
            self.due =
                Instant::now() + Duration::from_secs(u64::from(self.options.refresh_seconds));
        }
        if changed {
            let rows = self.page.rows.clone();
            let columns = self.options.columns.clone();
            let sort = self.query.sort;
            let widths = self
                .widths
                .iter()
                .map(|(field, width)| (*field, px(*width)))
                .collect();
            self.table.update(cx, |table, cx| {
                let delegate = table.delegate_mut();
                delegate.rows = rows;
                delegate.columns = columns;
                delegate.sort = sort;
                delegate.widths = widths;
                table.refresh(cx);
                cx.notify();
            });
        }
        changed
    }
}

impl Dashboard {
    fn open_history_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.detail_loading {
            return;
        }
        let Some(id) = self.history.selected else {
            return;
        };
        let Some(reply) = self.backend.history.detail(id) else {
            self.history.error = Some("History is busy; try opening the details again".into());
            cx.notify();
            return;
        };
        self.history.detail_loading = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = reply
                .await
                .unwrap_or_else(|_| Err("History worker stopped".into()));
            let _ = this.update_in(cx, |this, _, cx| {
                this.history.detail_loading = false;
                match result {
                    Ok(Some(detail)) => {
                        if let Some(old) = this.history.detail_window.take() {
                            let _ = old.update(cx, |_, window, _| window.remove_window());
                        }
                        match super::connection_details::open(
                            this.backend.clone(),
                            detail,
                            this.history.actions_tx.clone(),
                            this.history.processes_tx.clone(),
                            cx,
                        ) {
                            Ok(window) => this.history.detail_window = Some(window),
                            Err(error) => {
                                this.history.error =
                                    Some(format!("Cannot open connection details: {error:#}"))
                            }
                        }
                    }
                    Ok(None) => {
                        this.history.error =
                            Some("This event expired from history; select a newer event".into())
                    }
                    Err(error) => this.history.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn render_history(&mut self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let rows = uniform_list(
            "history",
            self.history.page.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let muted = cx.theme().muted_foreground;
                let border = cx.theme().border;
                range
                    .map(|index| {
                        let row = &this.history.page.rows[index];
                        let time = chrono::DateTime::from_timestamp_nanos(row.unixnano)
                            .format("%m-%d %H:%M:%S")
                            .to_string();
                        let process = if row.process.is_empty() {
                            "Unknown executable".into()
                        } else {
                            row.process.clone()
                        };
                        let action_color = if row.action == "allow" {
                            cx.theme().success
                        } else {
                            cx.theme().danger
                        };
                        let id = row.id;
                        ListItem::new(("history-row", index))
                            .accessibility_label(format!("{} to {}", row.process, row.destination))
                            .selected(this.history.selected == Some(id))
                            .w_full()
                            .h(px(62.))
                            .px_4()
                            .py_2()
                            .border_b_1()
                            .border_color(border)
                            .flex()
                            .gap_4()
                            .items_center()
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .gap_4()
                                    .items_center()
                                    .child(
                                        div().w(px(110.)).text_xs().text_color(muted).child(time),
                                    )
                                    .child(
                                        div()
                                            .w(px(55.))
                                            .text_sm()
                                            .text_color(action_color)
                                            .child(row.action.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(div().text_sm().truncate().child(process))
                                            .child(
                                                div().text_xs().text_color(muted).truncate().child(
                                                    format!(
                                                        "{} · {} · {}",
                                                        crate::history::details::endpoint(
                                                            &row.destination,
                                                            row.port as u32
                                                        ),
                                                        row.protocol,
                                                        row.reason
                                                    ),
                                                ),
                                            ),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.history.selected = Some(id);
                                cx.notify();
                            }))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .w_full()
        .track_scroll(&self.history.scroll)
        .flex_1()
        .min_h_0();
        div().flex_1().min_h_0().flex().flex_col().px_6().pb_3().gap_3()
            .child(div().flex().items_center().gap_2()
                .child(div().id("history-search").flex_1().child(Input::new(&self.history.input)))
                .child(Button::new("history-details").label("Details…").disabled(self.history.selected.is_none()||self.history.detail_loading).on_click(cx.listener(|this,_,window,cx|this.open_history_detail(window,cx))))
                .child(Button::new("history-tools").label("Filters, columns & export…").on_click(cx.listener(|this,_,_,cx|{
                    if let Some(handle)=this.history.tools_window {let _=handle.update(cx,|_,window,_|window.activate_window());return;}
                    match super::history_tools::open(this.backend.history.clone(),this.history.query.clone(),this.history.tools_tx.clone(),cx){Ok(handle)=>this.history.tools_window=Some(handle),Err(error)=>this.history.error=Some(error.to_string())}cx.notify();
                })))
                .child(Button::new("history-preferences").label("History preferences…").on_click(cx.listener(|this,_,_,cx|{
                    if let Some(handle)=this.history.preferences_window{let _=handle.update(cx,|_,window,_|window.activate_window());return;}
                    match super::history_preferences::open(this.backend.history.clone(),cx){Ok(handle)=>this.history.preferences_window=Some(handle),Err(error)=>this.history.error=Some(error.to_string())}cx.notify();
                })))
                .child(Button::new("history-refresh").label("Refresh now").disabled(self.history.pending.is_some()).on_click(cx.listener(|this,_,_,cx|{this.history.generation+=1;this.history.dirty=true;this.history.due=Instant::now();cx.notify();})))
            )
            .child(div().flex().items_center().gap_2()
                .children([("all","All",""),("allow","Allowed","allow"),("deny","Denied","deny"),("reject","Rejected","reject")].into_iter().map(|(id,label,action)| {
                    Button::new(id).label(label).selected(self.history.query.action==action)
                        .on_click(cx.listener(move |this,_,_,cx|{this.history.query.action=action.into();this.history.reset();cx.notify();}))
                })).child(div().text_xs().text_color(muted).child(format!("{} advanced condition(s)",self.history.query.filters.len()))))
            .child(div().text_xs().text_color(muted).child(if self.history.options.refresh_seconds==0{"Manual refresh. History continues to be recorded.".into()}else{format!("Display refresh: every {} second(s)",self.history.options.refresh_seconds)}))
            .when_some(self.backend.history.health.metadata_warning(),|v,e|v.child(super::banner(e,cx)))
            .when_some(self.history.error.clone(),|v,e|v.child(super::banner(e,cx)))
            .when_some(self.history.memory.error(),|v,e|v.child(super::banner(e,cx)))
            .child(div().flex_1().min_h_0().flex().flex_col().border_1().border_color(border).rounded_lg().overflow_hidden()
                .child(div().px_4().py_2().text_sm().font_weight(FontWeight::MEDIUM).child("Connection history"))
                .when(self.history.page.rows.is_empty(),|v|v.child(div().p_6().text_color(muted).child(if self.history.loading {"Loading history…"}else{"No matching events. History appears when the daemon reports traffic."})))
                .child(if self.history.options.table {div().flex_1().min_h_0().child(DataTable::new(&self.history.table)).into_any_element()}else{rows.into_any_element()}))
            .child(div().flex().justify_between().items_center()
                .child(div().text_xs().text_color(muted).child(format!("{} events on this page · UTC · {}",self.history.page.rows.len(),if self.history.query.before.is_none()&&self.history.query.cursor.is_none(){"following latest"}else{"earlier history"})))
                .child(div().flex().gap_2()
                    .child(Button::new("latest").label("Latest").on_click(cx.listener(|this,_,_,cx|{this.history.reset();cx.notify();})))
                    .child(Button::new("older").label("Older").disabled(!self.history.page.has_more || self.history.loading)
                        .on_click(cx.listener(|this,_,_,cx|{
                            this.history.query.ceiling=Some(this.history.page.ceiling);this.history.query.cursor=this.history.page.next_cursor.clone();this.history.query.before=None;this.history.generation+=1;
                            this.history.dirty=true;this.history.loading=true;this.history.due=Instant::now();
                            this.history.scroll.scroll_to_item(0,ScrollStrategy::Top);cx.notify();
                        })))))
    }
}
