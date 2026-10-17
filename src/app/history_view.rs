mod nodes;
use super::Dashboard;
use crate::history::{History, Page, Query};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
#[cfg(all(test, feature = "headless-tests"))]
use gpui_kit::component::scroll::ScrollbarHandle;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    scroll::{Scrollbar, ScrollbarMode},
    select::Select,
    table::{DataTable, TableEvent, TableState},
};
use std::sync::{Arc, atomic::Ordering};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

pub(super) struct HistoryView {
    nodes: nodes::State,
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
    table_top: bool,
    selected: Option<i64>,
    marks: Option<Arc<crate::history::selection::Selection>>,
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
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn test_idle(&self) -> bool {
        !self.loading && self.pending.is_none() && !self.dirty
    }

    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn test_tools_closed(&self) -> bool {
        self.tools_window.is_none()
    }

    pub fn new(
        history: &History,
        highlighting: crate::highlighting::Handle,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> Self {
        let options = history.settings.snapshot();
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "Filter executable or destination…",
                    window,
                    cx,
                ))
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
                    highlights: highlighting.snapshot(),
                    selected: None,
                    marks: None,
                    selection_generation: 0,
                    selection_error: None,
                    cache: Default::default(),
                },
                window,
                cx,
            )
            .col_movable(false)
        });
        cx.subscribe(&table, |this, table, event: &TableEvent, cx| {
            if let TableEvent::SelectRow(index) = event {
                this.history.selected = this.history.page.rows.get(*index).map(|row| row.id);
                table.update(cx, |state, _| {
                    state.delegate_mut().selected = this.history.selected
                });
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
            nodes: nodes::State::new(&options.presentation.node, window, cx),
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
            table_top: true,
            selected: None,
            marks: None,
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
        self.table_top = true;
        self.query.before = None;
        self.query.cursor = None;
        self.query.ceiling = None;
        self.page = Page::default();
        self.selected = None;
        self.marks = None;
        self.generation += 1;
        self.dirty = true;
        self.loading = true;
        self.due = Instant::now() + Duration::from_millis(200);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.remember(false);
    }
    fn capture_marks(&mut self) -> anyhow::Result<()> {
        if self.marks.is_none() {
            let selection = crate::history::selection::Selection::capture(
                self.query.clone(),
                self.page.ceiling,
            )?;
            self.query.ceiling = Some(selection.ceiling());
            self.marks = Some(Arc::new(selection));
            self.pending = None;
            self.loading = false;
            self.dirty = false;
        }
        Ok(())
    }
    fn mark(&mut self, id: i64, extend: bool, generation: u64) {
        if generation != self.generation {
            self.error = Some(crate::i18n::tr("History changed; select the event again."));
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            self.capture_marks()?;
            let row = self
                .page
                .rows
                .iter()
                .find(|row| row.id == id)
                .ok_or_else(|| anyhow::anyhow!("History changed; select the event again."))?;
            let selection = Arc::make_mut(self.marks.as_mut().unwrap());
            if extend {
                selection.extend_to(row)
            } else {
                selection.toggle(row)
            }
        })();
        self.error = result.err().map(|error| error.to_string());
    }
    pub(super) fn remember(&mut self, flush: bool) {
        self.memory.record(
            crate::history::settings::Presentation {
                node: self.query.node.clone(),
                search: self.query.search.clone(),
                action: self.query.action.clone(),
                filters: self.query.filters.clone(),
                logic: self.query.logic.clone(),
                sort: self.query.sort,
                widths: self.widths.clone(),
            },
            flush,
        );
    }
    pub fn poll(
        &mut self,
        history: &History,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> bool {
        let mut changed = self.nodes.poll(history, &self.query.node, window, cx);
        if let Some(error) = self
            .table
            .update(cx, |table, _| table.delegate_mut().selection_error.take())
        {
            self.error = Some(error);
            changed = true;
        }
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
                super::history_tools::Message::Mark {
                    id,
                    extend,
                    generation,
                } => {
                    self.mark(id, extend, generation);
                }
                super::history_tools::Message::Sort(Some((field, _)))
                    if indexing && field.needs_metadata() =>
                {
                    self.error = Some(crate::i18n::tr(
                        "Wait for older metadata indexing before sorting this column.",
                    ))
                }
                super::history_tools::Message::Sort(sort) => {
                    self.query.sort = sort;
                    self.reset();
                }
                super::history_tools::Message::Filters(filters, logic) => {
                    self.query.filters = filters;
                    self.query.logic = logic;
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
                                if self.selected.is_some_and(|id| {
                                    !self.page.rows.iter().any(|row| row.id == id)
                                }) {
                                    self.selected = None;
                                }
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
                    self.error = Some(crate::i18n::tr("History worker stopped"));
                    self.pending = None;
                    self.loading = false;
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        let revision = history.health.revision.load(Ordering::Acquire);
        if self.query.before.is_none()
            && self.query.ceiling.is_none()
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
            let scroll_top = self.table_top && !rows.is_empty();
            if scroll_top {
                self.table_top = false;
            }
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
                if scroll_top {
                    table.scroll_to_row(0, cx);
                }
                cx.notify();
            });
        }
        changed
    }

    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn test_table_offset(&self, cx: &App) -> Point<Pixels> {
        let table = self.table.read(cx);
        point(
            table.horizontal_scroll_handle.offset().x,
            table.vertical_scroll_handle.offset().y,
        )
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
            self.history.error = Some(crate::i18n::tr(
                "History is busy; try opening the details again",
            ));
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
                                this.history.error = Some(crate::i18n::tr_args(
                                    "Cannot open connection details: %{error}",
                                    &[("error", &format!("{:#}", error))],
                                ))
                            }
                        }
                    }
                    Ok(None) => {
                        this.history.error = Some(crate::i18n::tr(
                            "This event expired from history; select a newer event",
                        ))
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
        let snapshot = self.backend.highlighting.snapshot();
        let selected = self.history.selected;
        let marks = self.history.marks.clone();
        let generation = self.history.generation;
        let selected_index =
            selected.and_then(|id| self.history.page.rows.iter().position(|row| row.id == id));
        self.history.table.update(cx, |state, cx| {
            state.delegate_mut().highlights = snapshot;
            state.delegate_mut().selected = selected;
            state.delegate_mut().marks = marks;
            state.delegate_mut().selection_generation = generation;
            if state.selected_row() != selected_index {
                if let Some(index) = selected_index {
                    state.set_selected_row(index, cx);
                } else {
                    state.clear_selection(cx);
                }
            }
            let delegate = state.delegate_mut();
            delegate.cache.sync(&delegate.rows, &delegate.highlights);
        });
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let rows = uniform_list(
            "history",
            self.history.page.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let muted = cx.theme().muted_foreground;
                let border = cx.theme().border;
                let highlights = this.backend.highlighting.snapshot();
                range
                    .map(|index| {
                        let row = &this.history.page.rows[index];
                        let time = chrono::DateTime::from_timestamp_nanos(row.unixnano)
                            .format("%m-%d %H:%M:%S")
                            .to_string();
                        let process = if row.process.is_empty() {
                            crate::i18n::tr("Unknown executable")
                        } else {
                            row.process.clone()
                        };
                        let action_color = match row.action.as_str() {
                            "allow" => cx.theme().success,
                            "deny" | "drop" => cx.theme().danger,
                            "reject" => cx.theme().warning,
                            _ => cx.theme().muted_foreground,
                        };
                        let id = row.id;
                        let generation = this.history.generation;
                        let marked = this
                            .history
                            .marks
                            .as_ref()
                            .is_some_and(|selection| selection.contains(row));
                        let destination =
                            crate::history::details::endpoint(&row.destination, row.port as u32);
                        let mut values = if highlights.enabled(crate::highlighting::View::History) {
                            super::highlighting::history_values(row)
                        } else {
                            Vec::new()
                        };
                        for (key, value) in &mut values {
                            if key == "time" {
                                *value = time.clone();
                            }
                            if key == "process" {
                                *value = process.clone();
                            }
                        }
                        values.push(("destination".into(), destination.clone()));
                        let cells = values
                            .iter()
                            .map(|(key, value)| (key.as_str(), value.as_str()))
                            .collect::<Vec<_>>();
                        let prepared = highlights.prepare_row(
                            crate::highlighting::View::History,
                            &cells,
                            this.history.selected == Some(id) || marked,
                        );
                        let style = |key: &str, value: &str, cell: Div| {
                            super::highlighting::cell_prepared(
                                &highlights,
                                crate::highlighting::View::History,
                                key,
                                value,
                                prepared,
                                cell,
                            )
                        };
                        ListItem::new(("history-row", index))
                            .accessibility_label(crate::i18n::tr_args(
                                "%{process} to %{destination}",
                                &[("process", &row.process), ("destination", &row.destination)],
                            ))
                            .selected(this.history.selected == Some(id) || marked)
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
                                        Checkbox::new(("history-mark", id as u64))
                                            .checked(marked)
                                            .on_change(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.history.mark(
                                                    id,
                                                    window.modifiers().shift,
                                                    generation,
                                                );
                                                cx.notify();
                                            })),
                                    )
                                    .child(style(
                                        "time",
                                        &time,
                                        div()
                                            .w(px(110.))
                                            .text_xs()
                                            .text_color(muted)
                                            .child(time.clone()),
                                    ))
                                    .child(style(
                                        "action",
                                        &row.action,
                                        div()
                                            .w(px(55.))
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .when(
                                                this.history.selected != Some(id) && !marked,
                                                |cell| {
                                                    cell.text_color(action_color)
                                                        .bg(action_color.opacity(0.12))
                                                        .rounded_md()
                                                },
                                            )
                                            .text_center()
                                            .py_1()
                                            .child(row.action.clone()),
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(style(
                                                "process",
                                                &process,
                                                div().text_sm().truncate().child(process.clone()),
                                            ))
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_2()
                                                    .child(style(
                                                        "destination",
                                                        &destination,
                                                        div()
                                                            .text_xs()
                                                            .text_color(muted)
                                                            .truncate()
                                                            .child(destination.clone()),
                                                    ))
                                                    .child(style(
                                                        "protocol",
                                                        &row.protocol,
                                                        div()
                                                            .text_xs()
                                                            .text_color(muted)
                                                            .child(row.protocol.clone()),
                                                    ))
                                                    .child(style(
                                                        "reason",
                                                        &row.reason,
                                                        div()
                                                            .text_xs()
                                                            .text_color(muted)
                                                            .truncate()
                                                            .child(row.reason.clone()),
                                                    )),
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
        .min_h_0()
        .pr_3();
        let list = div()
            .id("history-list-viewport")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(rows)
            .child(
                Scrollbar::vertical(&self.history.scroll)
                    .id("history-scrollbar")
                    .mode(ScrollbarMode::Always),
            );
        #[cfg(feature = "headless-tests")]
        let list = list.test_support();
        let table = self.history.table.read(cx);
        let table_vertical = table.vertical_scroll_handle.clone();
        let table_horizontal = table.horizontal_scroll_handle.clone();
        let table_view = div()
            .id("history-table-viewport")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(DataTable::new(&self.history.table).scrollbar_visible(false, false))
            .child(
                div()
                    .absolute()
                    .top(gpui_kit::component::Size::default().table_row_height())
                    .right_0()
                    .bottom_0()
                    .w(Scrollbar::width())
                    .child(
                        Scrollbar::vertical(&table_vertical)
                            .id("history-table-scrollbar")
                            .viewport_from_layout()
                            .mode(ScrollbarMode::Always),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(Scrollbar::width())
                    .child(
                        Scrollbar::horizontal(&table_horizontal)
                            .id("history-table-horizontal-scrollbar")
                            .viewport_from_layout()
                            .mode(ScrollbarMode::Always),
                    ),
            );
        #[cfg(feature = "headless-tests")]
        let table_view = table_view.test_support();
        div().flex_1().min_h_0().flex().flex_col().px_6().pt_4().pb_3().gap_3()
            .child(div().flex().items_center().gap_2().flex_wrap()
                .child(div().w(px(220.)).min_w_0().child(Select::new(&self.history.nodes.picker).id("history-node")))
                .child(div().id("history-search").flex_1().min_w(px(220.)).child(Input::new(&self.history.input)))
                .child(Button::new("history-details").ghost().label(crate::i18n::tr("Details…")).disabled(self.history.selected.is_none()||self.history.detail_loading).on_click(cx.listener(|this,_,window,cx|this.open_history_detail(window,cx))))
                .child(Button::new("history-tools").label(crate::i18n::tr("Filters, columns & export…")).on_click(cx.listener(|this,_,_,cx|{
                    if let Some(handle)=this.history.tools_window {let _=handle.update(cx,|_,window,_|window.activate_window());return;}
                    match super::history_tools::open(this.backend.history.clone(),this.history.query.clone(),this.history.marks.clone(),this.history.tools_tx.clone(),cx){Ok(handle)=>this.history.tools_window=Some(handle),Err(error)=>this.history.error=Some(error.to_string())}cx.notify();
                })))
                .child(Button::new("history-preferences").ghost().label(crate::i18n::tr("History preferences…")).on_click(cx.listener(|this,_,_,cx|{
                    this.open_settings(super::settings::Page::History,cx);
                })))
                .child(Button::new("history-refresh").ghost().label(crate::i18n::tr("Refresh now")).disabled(self.history.pending.is_some()).on_click(cx.listener(|this,_,_,cx|{this.history.generation+=1;this.history.dirty=true;this.history.due=Instant::now();cx.notify();})))
            )
            .child(div().flex().items_center().gap_2()
                .children([("all","All",""),("allow","Allowed","allow"),("deny","Denied","deny"),("reject","Rejected","reject")].into_iter().map(|(id,label,action)| {
                    Button::new(id).ghost().label(crate::i18n::tr(label)).selected(self.history.query.action==action)
                        .on_click(cx.listener(move |this,_,_,cx|{this.history.query.action=action.into();this.history.reset();cx.notify();}))
                })).child(div().text_xs().text_color(muted).child(crate::i18n::tr_args("%{arg0} advanced condition(s)", &[("arg0", &(self.history.query.filters.len()).to_string())]))))
            .child(div().flex().items_center().gap_2().flex_wrap()
                .child(Button::new("history-select-all").ghost().label(crate::i18n::tr("Select all matching")).disabled(self.history.page.rows.is_empty()).on_click(cx.listener(|this,_,_,cx|{
                    match this.history.capture_marks(){Ok(())=>{Arc::make_mut(this.history.marks.as_mut().unwrap()).select_all();this.history.error=None;},Err(error)=>this.history.error=Some(error.to_string())}cx.notify();
                })))
                .child(Button::new("history-clear-selection").ghost().label(crate::i18n::tr("Clear selection")).disabled(this_marks_empty(self.history.marks.as_deref())).on_click(cx.listener(|this,_,_,cx|{this.history.marks=None;cx.notify();})))
                .child(div().text_xs().text_color(muted).child(if self.history.marks.as_ref().is_some_and(|marks|marks.all_matching()){crate::i18n::tr("All captured matches, with individual exclusions")}else if self.history.marks.as_ref().is_some_and(|marks|marks.has_range()){crate::i18n::tr("Captured range and individual marks")}else{crate::i18n::tr_args("%{count} individual marks · Shift-click extends a range",&[("count",&self.history.marks.as_ref().map_or(0,|marks|marks.override_count()).to_string())])})))
            .child(div().text_xs().text_color(muted).child(if self.history.options.refresh_seconds==0{crate::i18n::tr("Manual refresh. History continues to be recorded.")}else{crate::i18n::tr_args("Display refresh: every %{arg0} second(s)", &[("arg0", &format!("{}", self.history.options.refresh_seconds))])}))
            .when_some(self.backend.history.health.metadata_warning(),|v,e|v.child(super::banner(e,cx)))
            .when_some(self.history.error.clone(),|v,e|v.child(super::banner(e,cx)))
            .when_some(self.history.nodes.error.clone(),|v,e|v.child(super::banner(e,cx)))
            .when_some(self.history.memory.error(),|v,e|v.child(super::banner(e,cx)))
            .child(div().flex_1().min_h_0().flex().flex_col().border_1().border_color(border).rounded_lg().overflow_hidden()
                .child(div().px_4().py_2().bg(cx.theme().muted.opacity(0.45)).border_b_1().border_color(border).text_sm().font_weight(FontWeight::MEDIUM).child(crate::i18n::tr("Connection history")))
                .when(self.history.page.rows.is_empty(),|v|v.child(div().p_6().text_color(muted).child(crate::i18n::tr(if self.history.loading {"Loading history…"}else{"No matching events. History appears when the daemon reports traffic."}))))
                .child(if self.history.options.table {table_view.into_any_element()}else{list.into_any_element()}))
            .child(div().flex().justify_between().items_center()
                .child(div().text_xs().text_color(muted).child(crate::i18n::tr_args("%{arg0} events on this page · UTC · %{arg1}", &[("arg0", &(self.history.page.rows.len()).to_string()), ("arg1", &crate::i18n::tr(if self.history.query.ceiling.is_some(){"frozen history"}else{"following latest"}))])))
                .child(div().flex().gap_2()
                    .child(Button::new("latest").ghost().label(crate::i18n::tr("Latest")).on_click(cx.listener(|this,_,_,cx|{this.history.reset();cx.notify();})))
                    .child(Button::new("history-first-page").ghost().label(crate::i18n::tr("First page")).disabled(self.history.loading || (self.history.query.cursor.is_none() && self.history.query.before.is_none()))
                        .on_click(cx.listener(|this,_,_,cx|{
                            this.history.query.cursor=None;this.history.query.before=None;this.history.generation+=1;
                            this.history.dirty=true;this.history.loading=true;this.history.due=Instant::now();
                            this.history.scroll.scroll_to_item(0,ScrollStrategy::Top);cx.notify();
                        })))
                    .child(Button::new("older").ghost().label(crate::i18n::tr("Older")).disabled(!self.history.page.has_more || self.history.loading)
                        .on_click(cx.listener(|this,_,_,cx|{
                            this.history.query.ceiling=Some(this.history.page.ceiling);this.history.query.cursor=this.history.page.next_cursor.clone();this.history.query.before=None;this.history.generation+=1;
                            this.history.dirty=true;this.history.loading=true;this.history.due=Instant::now();
                            this.history.scroll.scroll_to_item(0,ScrollStrategy::Top);cx.notify();
                        })))))
    }
}

fn this_marks_empty(selection: Option<&crate::history::selection::Selection>) -> bool {
    selection.is_none_or(|selection| selection.is_empty())
}
