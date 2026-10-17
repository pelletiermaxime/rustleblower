//! Bounded daemon-counter pages; capture/file work stays on the background pool.
use super::Dashboard;
use crate::{
    history::History,
    statistics::{self, KINDS, Query, Row, SORTS, Snapshot},
};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath,
    button::Button,
    chart::BarChart,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
type Picker<T> = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<T>>>>;
type Load = oneshot::Receiver<Result<(Arc<Snapshot>, Vec<Row>), String>>;

pub(super) struct StatisticsView {
    pub input: Entity<InputState>,
    pub query: Query,
    pub page: Vec<Row>,
    pub snapshot: Option<Arc<Snapshot>>,
    node: Picker<String>,
    sort: Picker<statistics::Sort>,
    node_names: Vec<String>,
    node_subscription: Option<Subscription>,
    generation: u64,
    pending: Option<(u64, Load)>,
    dirty: bool,
    due: Instant,
    revision: u64,
    offset: usize,
    frozen: bool,
    selected: Option<usize>,
    marks: Option<Arc<statistics::Selection>>,
    chart: bool,
    confirm: Option<bool>,
    clear_history: bool,
    reset: Option<oneshot::Receiver<Result<statistics::ResetResult, String>>>,
    error: Option<String>,
    message: Option<String>,
    export_window: Option<AnyWindowHandle>,
}
impl StatisticsView {
    pub fn new(window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "Filter cached counter values…",
                    window,
                    cx,
                ))
                .validate(|value, _| value.len() <= 256)
        });
        cx.subscribe(&input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.statistics.query.search = input.read(cx).value().to_string();
                this.statistics.invalidate();
                cx.notify();
            }
        })
        .detach();
        let node = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![crate::i18n::LocalizedOption::new(
                    String::new(),
                    "All nodes",
                )]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let sort = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    SORTS
                        .iter()
                        .map(|(sort, label)| crate::i18n::LocalizedOption::new(*sort, *label))
                        .collect::<Vec<_>>(),
                ),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
            .searchable(true)
        });
        cx.subscribe(
            &sort,
            |this,
             state,
             _: &SelectEvent<SearchableVec<crate::i18n::LocalizedOption<statistics::Sort>>>,
             cx| {
                if let Some(sort) = state.read(cx).selected_value().copied()
                    && sort != this.statistics.query.sort
                {
                    this.statistics.query.sort = sort;
                    this.statistics.invalidate();
                    cx.notify();
                }
            },
        )
        .detach();
        Self {
            input,
            query: Query::default(),
            page: Vec::new(),
            snapshot: None,
            node,
            sort,
            node_names: Vec::new(),
            node_subscription: None,
            generation: 0,
            pending: None,
            dirty: true,
            due: Instant::now(),
            revision: 0,
            offset: 0,
            frozen: false,
            selected: None,
            marks: None,
            chart: false,
            confirm: None,
            clear_history: false,
            reset: None,
            error: None,
            message: None,
            export_window: None,
        }
    }
    fn invalidate(&mut self) {
        self.generation += 1;
        self.dirty = true;
        self.due = Instant::now() + Duration::from_millis(200);
        self.offset = 0;
        self.frozen = false;
        self.snapshot = None;
        self.page.clear();
        self.selected = None;
        self.marks = None;
        self.confirm = None;
    }
    fn capture_marks(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.dirty && self.pending.is_none(),
            "Wait for the counter page to finish loading"
        );
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Wait for the counter page to finish loading"))?;
        if self
            .marks
            .as_ref()
            .is_none_or(|selection| !Arc::ptr_eq(selection.snapshot(), snapshot))
        {
            self.marks = Some(Arc::new(snapshot.selection()));
        }
        self.frozen = true;
        Ok(())
    }
    fn mark(&mut self, index: usize, extend: bool, generation: u64) {
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                generation == self.generation,
                "Counters changed; select the value again."
            );
            self.capture_marks()?;
            let selection = Arc::make_mut(self.marks.as_mut().unwrap());
            if extend {
                selection.extend_to(index)
            } else {
                selection.toggle(index)
            }
        })();
        self.error = result.err().map(|error| error.to_string());
    }
    fn update_nodes(
        &mut self,
        snapshot: &Snapshot,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) {
        let mut names = snapshot
            .nodes
            .iter()
            .map(|node| node.name.clone())
            .collect::<Vec<_>>();
        if !self.query.node.is_empty() && !names.contains(&self.query.node) {
            names.push(self.query.node.clone());
        }
        names.sort();
        if names == self.node_names {
            return;
        }
        self.node_names = names;
        let mut labels = vec![crate::i18n::LocalizedOption::new(
            String::new(),
            "All nodes",
        )];
        labels.extend(self.node_names.iter().map(|name| {
            let value = name.clone();
            let title = name.clone();
            crate::i18n::LocalizedOption::with_title(value, move || {
                crate::i18n::tr_args("Node: %{name}", &[("name", &title)])
            })
        }));
        let selected = self
            .node_names
            .iter()
            .position(|name| *name == self.query.node)
            .map_or(0, |index| index + 1);
        self.node = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(labels),
                Some(IndexPath::new(selected)),
                window,
                cx,
            )
            .searchable(true)
        });
        self.node_subscription = Some(cx.subscribe(
            &self.node,
            |this,
             state,
             _: &SelectEvent<SearchableVec<crate::i18n::LocalizedOption<String>>>,
             cx| {
                if let Some(label) = state.read(cx).selected_value() {
                    let node = label.clone();
                    if node != this.statistics.query.node {
                        this.statistics.query.node = node;
                        this.statistics.invalidate();
                        cx.notify();
                    }
                }
            },
        ));
    }
    pub fn poll(
        &mut self,
        history: &History,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> bool {
        let mut changed = false;
        if let Some(handle) = self.export_window
            && handle.update(cx, |_, _, _| {}).is_err()
        {
            self.export_window = None;
        }
        if let Some(rx) = self.reset.as_mut() {
            match rx.try_recv() {
                Ok(result) => {
                    self.reset = None;
                    self.confirm = None;
                    match result {
                        Ok(report) => {
                            self.message = Some(crate::i18n::tr_args(
                                "Reset %{arg0} cached counters; removed %{arg1} retained history events. Daemon counters and firewall policy were not changed.",
                                &[
                                    ("arg0", &format!("{}", report.counters)),
                                    ("arg1", &format!("{}", report.events)),
                                ],
                            ));
                            self.error = None;
                            self.invalidate();
                        }
                        Err(error) => self.error = Some(error),
                    }
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.reset = None;
                    self.error = Some(crate::i18n::tr("Statistics worker stopped"));
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        if let Some((generation, rx)) = self.pending.as_mut() {
            match rx.try_recv() {
                Ok(result) => {
                    let current = *generation == self.generation;
                    self.pending = None;
                    if current {
                        match result {
                            Ok((snapshot, page)) => {
                                self.update_nodes(&snapshot, window, cx);
                                if self.marks.as_ref().is_some_and(|selection| {
                                    !Arc::ptr_eq(selection.snapshot(), &snapshot)
                                }) {
                                    self.marks = None;
                                }
                                self.snapshot = Some(snapshot);
                                self.page = page;
                                self.selected = None;
                                self.error = None;
                            }
                            Err(error) => self.error = Some(error),
                        }
                        changed = true;
                    }
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending = None;
                    self.error = Some(crate::i18n::tr("Statistics reader stopped"));
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        let revision = history.health.statistics_revision.load(Ordering::Acquire);
        if !self.frozen && revision != self.revision && Instant::now() >= self.due {
            self.dirty = true;
            self.snapshot = None;
            self.offset = 0;
        }
        if self.dirty
            && self.pending.is_none()
            && Instant::now() >= self.due
            && let Some(permit) = statistics::capture_permit()
        {
            let snapshot = self.snapshot.clone();
            let query = self.query.clone();
            let database = history.database.clone();
            let offset = self.offset;
            let (tx, rx) = oneshot::channel();
            cx.background_executor()
                .spawn(async move {
                    let _permit = permit;
                    let result = (|| -> anyhow::Result<_> {
                        let snapshot = match snapshot {
                            Some(snapshot) => snapshot,
                            None => Arc::new(statistics::capture(&database, query)?),
                        };
                        let page = snapshot.page(offset)?;
                        Ok((snapshot, page))
                    })();
                    let _ = tx.send(result.map_err(|error| format!("{error:#}")));
                })
                .detach();
            self.pending = Some((self.generation, rx));
            self.dirty = false;
            self.revision = revision;
            self.due = Instant::now() + Duration::from_secs(1);
        }
        changed
    }
}
impl Dashboard {
    fn statistics_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self
            .statistics
            .selected
            .and_then(|index| self.statistics.page.get(index))
        else {
            return;
        };
        let field = self.statistics.query.kind.field();
        let filter = crate::history::query::Filter {
            field,
            comparison: if row.key.is_empty() {
                crate::history::query::Comparison::Missing
            } else {
                crate::history::query::Comparison::Exact
            },
            value: row.key.clone(),
        };
        if let Err(error) = filter.validate() {
            self.statistics.error = Some(crate::i18n::tr_args(
                "This daemon counter label cannot be matched to history: %{error}",
                &[("error", &format!("{}", error))],
            ));
            return;
        }
        let mut filters = vec![filter];
        if !self.statistics.query.node.is_empty() {
            filters.push(crate::history::query::Filter {
                field: crate::history::query::Field::Node,
                comparison: crate::history::query::Comparison::Exact,
                value: self.statistics.query.node.clone(),
            });
        }
        self.history.drill(filters, window, cx);
        self.screen = super::Screen::History;
        cx.notify();
    }
    fn statistics_reset(&mut self, cx: &mut Context<Self>) {
        let Some(all) = self.statistics.confirm else {
            return;
        };
        let key = if all {
            None
        } else {
            self.statistics
                .selected
                .and_then(|index| self.statistics.page.get(index))
                .map(|row| row.key.clone())
        };
        if !all && key.is_none() {
            return;
        }
        let scope = statistics::Reset {
            kind: self.statistics.query.kind,
            node: self.statistics.query.node.clone(),
            key,
            clear_history: !all && self.statistics.clear_history,
        };
        match self.backend.history.reset_statistics(scope) {
            Some(reply) => self.statistics.reset = Some(reply),
            None => {
                self.statistics.error =
                    Some(crate::i18n::tr("Statistics worker is busy; try again"))
            }
        }
        cx.notify();
    }
    pub(super) fn render_statistics(&mut self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let selected = self.statistics.selected.is_some();
        let busy = self.statistics.reset.is_some();
        let mut body=div().flex_1().min_h_0().px_6().pb_3().flex().flex_col().gap_2()
            .child(TabBar::new("statistics-kinds").selected_index(KINDS.iter().position(|kind|*kind==self.statistics.query.kind).unwrap()).on_click(cx.listener(|this,index:&usize,window,cx|{this.statistics.query.kind=KINDS[*index];this.statistics.input.update(cx,|input,cx|input.set_value("",window,cx));this.statistics.query.search.clear();this.statistics.invalidate();cx.notify();})).children(KINDS.iter().map(|kind|Tab::new().label(crate::i18n::tr(kind.label())))))
            .child(div().flex().gap_3().child(div().id("statistics-search").h_8().flex_1().child(Input::new(&self.statistics.input))).child(div().w(px(220.)).h_8().flex_shrink_0().child(Select::new(&self.statistics.node).id("statistics-node").w_full())).child(div().w(px(160.)).h_8().flex_shrink_0().child(Select::new(&self.statistics.sort).id("statistics-sort").w_full())))
            .child(div().text_xs().text_color(muted).child(crate::i18n::tr("Daemon-reported counters can include traffic outside retained history. Keys may be evicted or restart from zero; cached values show their last report.")))
            .when_some(self.statistics.error.clone(),|body,error|body.child(super::banner(error,cx)))
            .when_some(self.statistics.message.clone(),|body,message|body.child(div().text_sm().child(crate::i18n::presentation(&message))))
            .when_some(self.backend.history.health.statistics_error.lock().unwrap().clone(),|body,error|body.child(super::banner(error,cx)));
        if let Some(snapshot) = self.statistics.snapshot.as_ref() {
            let totals = snapshot
                .nodes
                .iter()
                .filter(|node| snapshot.query.node.is_empty() || snapshot.query.node == node.name)
                .fold(
                    (0u128, 0u128, 0u128),
                    |(connections, accepted, dropped), node| {
                        (
                            connections + u128::from(node.totals.connections),
                            accepted + u128::from(node.totals.accepted),
                            dropped + u128::from(node.totals.dropped),
                        )
                    },
                );
            body=body.child(div().flex().gap_3().items_center().child(div().flex_1().text_sm().child(crate::i18n::tr_args("Daemon totals: %{arg0} connections · %{arg1} accepted · %{arg2} dropped. Cached matches: %{arg3} values · %{arg4} hits since local reset.", &[("arg0", &(totals.0).to_string()), ("arg1", &(totals.1).to_string()), ("arg2", &(totals.2).to_string()), ("arg3", &(snapshot.len()).to_string()), ("arg4", &(snapshot.total).to_string())]))).child(Checkbox::new("statistics-chart").label(crate::i18n::tr("Show chart")).checked(self.statistics.chart).on_change(cx.listener(|this,value:&bool,_,cx|{this.statistics.chart= *value;cx.notify();}))));
            if self.statistics.chart && !snapshot.top.is_empty() {
                let data = snapshot
                    .top
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        (
                            format!(
                                "{} {}",
                                index + 1,
                                crate::history::bounded(
                                    row.key.rsplit('/').next().unwrap_or(&row.key),
                                    18
                                )
                            ),
                            row.hits,
                        )
                    })
                    .collect::<Vec<_>>();
                body = body.child(
                    div().h(px(130.)).flex_shrink_0().child(
                        BarChart::new(data)
                            .id("statistics-bars")
                            .appear(false)
                            .band(|(label, _)| label.clone())
                            .value(|(_, hits)| *hits as f64)
                            .label(|(_, hits)| hits.to_string())
                            .value_axis(true),
                    ),
                );
            }
        }
        let rows = uniform_list(
            "statistics-rows",
            self.statistics.page.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let highlights = this.backend.highlighting.snapshot();
                let view = super::highlighting::statistics(this.statistics.query.kind);
                range
                    .map(|index| {
                        let row = &this.statistics.page[index];
                        let position = this.statistics.offset + index;
                        let generation = this.statistics.generation;
                        let marked = this
                            .statistics
                            .marks
                            .as_ref()
                            .is_some_and(|selection| selection.contains(position));
                        let mark_disabled = this.statistics.dirty
                            || this.statistics.pending.is_some()
                            || this.statistics.snapshot.is_none();
                        let key = if row.key.is_empty() {
                            crate::i18n::tr("Not reported")
                        } else {
                            row.key.clone()
                        };
                        let hits = row.hits.to_string();
                        let nodes = crate::i18n::tr_args(
                            "%{arg0} node(s)",
                            &[("arg0", &row.nodes.to_string())],
                        );
                        let cells = [
                            ("key", key.as_str()),
                            ("hits", hits.as_str()),
                            ("nodes", nodes.as_str()),
                        ];
                        let prepared = highlights.prepare_row(
                            view,
                            &cells,
                            this.statistics.selected == Some(index) || marked,
                        );
                        let style = |column: &str, value: &str, cell: Div| {
                            super::highlighting::cell_prepared(
                                &highlights,
                                view,
                                column,
                                value,
                                prepared,
                                cell,
                            )
                        };
                        ListItem::new(("statistics-row", index))
                            .selected(this.statistics.selected == Some(index) || marked)
                            .w_full()
                            .h(px(48.))
                            .px_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .flex()
                                    .gap_4()
                                    .items_center()
                                    .child(
                                        Checkbox::new(("statistics-mark", position))
                                            .checked(marked)
                                            .disabled(mark_disabled)
                                            .on_change(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.statistics.mark(
                                                    position,
                                                    window.modifiers().shift,
                                                    generation,
                                                );
                                                cx.notify();
                                            })),
                                    )
                                    .child(style(
                                        "key",
                                        &key,
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .truncate()
                                            .child(key.clone()),
                                    ))
                                    .child(style(
                                        "hits",
                                        &hits,
                                        div().w(px(210.)).text_sm().child(hits.clone()),
                                    ))
                                    .child(style(
                                        "nodes",
                                        &nodes,
                                        div()
                                            .w(px(100.))
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(nodes.clone()),
                                    )),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.statistics.selected = Some(index);
                                this.statistics.confirm = None;
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
                .flex()
                .items_center()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("statistics-select-all")
                        .label(crate::i18n::tr("Select all matching"))
                        .disabled(
                            self.statistics.dirty
                                || self.statistics.pending.is_some()
                                || self
                                    .statistics
                                    .snapshot
                                    .as_ref()
                                    .is_none_or(|snapshot| snapshot.is_empty()),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            match this.statistics.capture_marks() {
                                Ok(()) => {
                                    Arc::make_mut(this.statistics.marks.as_mut().unwrap())
                                        .select_all();
                                    this.statistics.error = None;
                                }
                                Err(error) => this.statistics.error = Some(error.to_string()),
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("statistics-clear-selection")
                        .label(crate::i18n::tr("Clear selection"))
                        .disabled(
                            self.statistics
                                .marks
                                .as_ref()
                                .is_none_or(|selection| selection.is_empty()),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.statistics.marks = None;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(crate::i18n::tr_args(
                            "%{count} selected counter values · Shift-click extends a range",
                            &[(
                                "count",
                                &self
                                    .statistics
                                    .marks
                                    .as_ref()
                                    .map_or(0, |selection| selection.count())
                                    .to_string(),
                            )],
                        )),
                ),
        );
        body = body.child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .border_1()
                .border_color(border)
                .rounded_lg()
                .overflow_hidden()
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .gap_4()
                        .text_xs()
                        .text_color(muted)
                        .child(
                            div()
                                .flex_1()
                                .child(crate::i18n::tr(self.statistics.query.kind.label())),
                        )
                        .child(
                            div()
                                .w(px(210.))
                                .child(crate::i18n::tr("Hits since local reset")),
                        )
                        .child(div().w(px(100.)).child(crate::i18n::tr("Nodes"))),
                )
                .when(self.statistics.page.is_empty(), |body| {
                    body.child(div().p_4().child(
                        if self.statistics.pending.is_some() || self.statistics.dirty {
                            crate::i18n::tr("Loading statistics…")
                        } else {
                            crate::i18n::tr("No matching daemon counters.")
                        },
                    ))
                })
                .child(rows),
        );
        if let Some(all) = self.statistics.confirm {
            let key = self
                .statistics
                .selected
                .and_then(|index| self.statistics.page.get(index))
                .map(|row| crate::history::bounded(&row.key, 256))
                .unwrap_or_default();
            body=body.child(div().text_sm().child(if all{crate::i18n::tr_args("Reset all cached %{arg0} counts for %{arg1}? This sets a local baseline; daemon counters stay unchanged.", &[("arg0", &crate::i18n::tr(self.statistics.query.kind.label()).to_string()), ("arg1", &(if self.statistics.query.node.is_empty(){crate::i18n::tr("all cached nodes")}else{self.statistics.query.node.clone()}).to_string())])}else{crate::i18n::tr_args("Reset cached counts for %{key}? The next report shows only increments beyond the new local baseline.", &[("key", &format!("{:?}", key))])}))
                .when(!all,|body|body.child(Checkbox::new("statistics-clear-history").label(crate::i18n::tr("Also remove retained history for this value")).checked(self.statistics.clear_history).disabled(busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.statistics.clear_history= *value;cx.notify();}))))
                .child(div().flex().gap_3().child(Button::new("statistics-reset-confirm").label(crate::i18n::tr("Confirm reset")).disabled(busy).on_click(cx.listener(|this,_,_,cx|this.statistics_reset(cx)))).child(Button::new("statistics-reset-cancel").label(crate::i18n::tr("Cancel")).disabled(busy).on_click(cx.listener(|this,_,_,cx|{this.statistics.confirm=None;cx.notify();}))));
        }
        let count = self
            .statistics
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.len());
        body.child(div().flex().justify_between().items_center().child(div().text_xs().text_color(muted).child(crate::i18n::tr_args("%{arg0}–%{arg1} of %{arg2} cached values · %{arg3} · %{arg4} skipped/coalesced samples · %{arg5} evicted keys", &[("arg0", &(if count==0{0}else{self.statistics.offset+1}).to_string()), ("arg1", &((self.statistics.offset+self.statistics.page.len()).min(count)).to_string()), ("arg2", &(count).to_string()), ("arg3", (if self.statistics.frozen{"frozen view"}else{"following reports"})), ("arg4", &(self.backend.history.health.statistics_skipped.load(Ordering::Relaxed)).to_string()), ("arg5", &(self.backend.history.health.statistics_evicted.load(Ordering::Relaxed)).to_string())])))
            .child(div().flex().gap_2().child(Button::new("statistics-latest").label(crate::i18n::tr("Latest")).on_click(cx.listener(|this,_,_,cx|{this.statistics.invalidate();this.statistics.due=Instant::now();cx.notify();}))).child(Button::new("statistics-previous").label(crate::i18n::tr("Previous page")).disabled(self.statistics.pending.is_some()||self.statistics.offset==0).on_click(cx.listener(|this,_,_,cx|{this.statistics.offset=this.statistics.offset.saturating_sub(statistics::PAGE_SIZE);this.statistics.generation+=1;this.statistics.frozen=true;this.statistics.dirty=true;this.statistics.due=Instant::now();this.statistics.selected=None;cx.notify();}))).child(Button::new("statistics-older").label(crate::i18n::tr("Next page")).disabled(self.statistics.pending.is_some()||self.statistics.offset+self.statistics.page.len()>=count).on_click(cx.listener(|this,_,_,cx|{this.statistics.offset+=statistics::PAGE_SIZE;this.statistics.generation+=1;this.statistics.frozen=true;this.statistics.dirty=true;this.statistics.due=Instant::now();this.statistics.selected=None;cx.notify();})))))
            .child(div().flex().gap_3().flex_wrap().child(Button::new("statistics-history").label(crate::i18n::tr("Show matching history")).disabled(!selected||busy).on_click(cx.listener(|this,_,window,cx|this.statistics_history(window,cx))))
                .child(Button::new("statistics-reset-selected").label(crate::i18n::tr("Reset focused value…")).disabled(!selected||busy).on_click(cx.listener(|this,_,_,cx|{this.statistics.confirm=Some(false);this.statistics.clear_history=false;cx.notify();})))
                .child(Button::new("statistics-reset-all").label(crate::i18n::tr("Reset category…")).disabled(count==0||busy).on_click(cx.listener(|this,_,_,cx|{this.statistics.confirm=Some(true);this.statistics.clear_history=false;cx.notify();})))
                .child(Button::new("statistics-export").label(crate::i18n::tr("Export counters…")).disabled(self.statistics.snapshot.is_none()).on_click(cx.listener(|this,_,_,cx|{if let Some(handle)=this.statistics.export_window{let _=handle.update(cx,|_,window,_|window.activate_window());return;}else if let Some(snapshot)=this.statistics.snapshot.clone(){match export_window(snapshot,this.statistics.offset,this.statistics.marks.clone(),cx){Ok(handle)=>this.statistics.export_window=Some(handle),Err(error)=>this.statistics.error=Some(error.to_string())}}cx.notify();}))))
    }
}

fn export_window(
    snapshot: Arc<Snapshot>,
    offset: usize,
    selection: Option<Arc<statistics::Selection>>,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(820.), px(600.)),
                cx,
            ))),
            window_min_size: Some(size(px(700.), px(520.))),
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Rustleblower — statistics export").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| Export::new(snapshot, offset, selection, window, cx)),
    )?;
    crate::i18n::register_title(handle, "Rustleblower — statistics export", cx);
    Ok(handle)
}
struct Export {
    selection: Option<Arc<statistics::Selection>>,
    columns: Vec<statistics::Column>,
    snapshot: Arc<Snapshot>,
    offset: usize,
    path: Entity<InputState>,
    json: bool,
    replace: bool,
    busy: bool,
    stop: Arc<AtomicBool>,
    message: Option<String>,
    error: Option<String>,
}
impl Export {
    fn new(
        snapshot: Arc<Snapshot>,
        offset: usize,
        selection: Option<Arc<statistics::Selection>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let busy = weak.upgrade().is_some_and(|view| view.read(cx).busy);
            if busy {
                window.minimize_window();
            }
            !busy
        });
        Self {
            selection,
            columns: statistics::COLUMNS.to_vec(),
            snapshot,
            offset,
            path: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(crate::i18n::placeholder(
                        "/absolute/path/statistics.csv",
                        window,
                        cx,
                    ))
                    .validate(|text, _| text.len() <= 4096)
            }),
            json: false,
            replace: false,
            busy: false,
            stop: Arc::new(AtomicBool::new(false)),
            message: None,
            error: None,
        }
    }
    fn save(&mut self, copy: bool, selected: bool, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.snapshot.clone();
        let selection = if selected {
            self.selection
                .clone()
                .filter(|selection| !selection.is_empty())
        } else {
            None
        };
        if selected && selection.is_none() {
            self.error = Some(crate::i18n::tr(
                "Select counter values, then reopen the export window.",
            ));
            cx.notify();
            return;
        }
        let columns = self.columns.clone();
        let target = PathBuf::from(self.path.read(cx).value().as_ref());
        let json = self.json;
        let replace = self.replace;
        let stop = self.stop.clone();
        let offset = self.offset;
        self.stop.store(false, Ordering::Relaxed);
        self.busy = true;
        self.error = None;
        self.message = None;
        let task = cx.background_executor().spawn(async move {
            if let Some(selection) = selection {
                if copy {
                    let (text, count) = selection.copy(&columns, json, &stop)?;
                    Ok((count, Some(text)))
                } else {
                    Ok((
                        selection.save(&columns, &target, json, replace, &stop)?,
                        None,
                    ))
                }
            } else if copy {
                let rows = snapshot.page(offset)?;
                let mut text = crate::history::export::csv_line([
                    "Value".into(),
                    "Hits since local reset".into(),
                    "Last reported total".into(),
                    "Nodes".into(),
                ]);
                for row in rows {
                    text.push_str(&crate::history::export::csv_line([
                        row.key,
                        row.hits.to_string(),
                        row.reported.to_string(),
                        row.nodes.to_string(),
                    ]));
                }
                anyhow::ensure!(
                    text.len() <= 4 * 1024 * 1024,
                    "Clipboard export exceeds 4 MiB"
                );
                Ok::<_, anyhow::Error>((0, Some(text)))
            } else {
                Ok((snapshot.save(&target, json, replace, &stop)?, None))
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                if this.stop.load(Ordering::Relaxed) {
                    this.error = Some(crate::i18n::tr("Counter output stopped"));
                    cx.notify();
                    return;
                }
                match result {
                    Ok((count, Some(text))) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        this.message = Some(if selected {
                            crate::i18n::tr_args(
                                "Copied %{count} selected counter values.",
                                &[("count", &count.to_string())],
                            )
                        } else {
                            crate::i18n::tr("Counter page copied as CSV.")
                        });
                        this.error = None;
                    }
                    Ok((count, None)) => {
                        this.message = Some(crate::i18n::tr_args(
                            "Exported %{count} cached counter values.",
                            &[("count", &format!("{}", count))],
                        ));
                        this.error = None;
                    }
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_new_path(Path::new("."), Some("rustleblower-statistics.csv"));
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = prompt.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(Some(path))) => this.path.update(cx, |input, cx| {
                        input.set_value(path.to_string_lossy().to_string(), window, cx)
                    }),
                    Ok(Ok(None)) => {}
                    other => {
                        this.error = Some(crate::i18n::tr_args(
                            "Cannot choose export path: %{other}",
                            &[("other", &format!("{:?}", other))],
                        ))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
impl Render for Export {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel=div().id("statistics-export-panel").flex_1().min_h_0().overflow_y_scroll().flex().flex_col().gap_3()
    .child(div().text_sm().child(crate::i18n::tr_args("%{arg0} · %{arg1} cached values · captured at %{arg2} UTC. Export preserves this view even if new daemon reports arrive.", &[("arg0", &crate::i18n::tr(self.snapshot.query.kind.label())), ("arg1", &(self.snapshot.len()).to_string()), ("arg2", &(chrono::DateTime::from_timestamp(self.snapshot.captured,0).unwrap().format("%Y-%m-%d %H:%M:%S")).to_string())])))
    .child(div().h_8().flex_shrink_0().child(Input::new(&self.path).id("statistics-export-path").disabled(self.busy))).child(Button::new("statistics-export-browse").label(crate::i18n::tr("Choose file…")).disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.browse(window,cx))))
    .child(Checkbox::new("statistics-export-json").label(crate::i18n::tr("JSON Lines instead of CSV")).checked(self.json).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.json= *value;cx.notify();})))
    .child(Checkbox::new("statistics-export-replace").label(crate::i18n::tr("Replace an existing export file")).checked(self.replace).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.replace= *value;cx.notify();})))
    .child(div().flex().gap_3().child(Button::new("statistics-export-save").label(crate::i18n::tr("Export all cached matches")).disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.save(false,false,window,cx))))
        .child(Button::new("statistics-export-copy").label(crate::i18n::tr("Copy this counter page")).disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.save(true,false,window,cx)))))
    .child(div().text_sm().child(crate::i18n::tr("Selected values and the counter snapshot were captured when this window opened. Reopen to capture a changed selection.")))
    .child(div().flex().flex_wrap().gap_3().children(statistics::COLUMNS.iter().enumerate().map(|(index,column)|{
        let column=*column;
        Checkbox::new(("statistics-export-column",index)).label(crate::i18n::tr(column.label())).checked(self.columns.contains(&column)).disabled(self.busy).on_change(cx.listener(move|this,checked:&bool,_,cx|{
            if *checked {if !this.columns.contains(&column){this.columns.push(column);}}else{this.columns.retain(|value|*value!=column);}cx.notify();
        }))
    })))
    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(crate::i18n::tr("Chosen columns and format apply to selected output.")))
    .child(div().flex().gap_3().flex_wrap()
        .child(Button::new("statistics-export-selected").label(crate::i18n::tr("Export selected values")).disabled(self.busy||self.columns.is_empty()||self.selection.as_ref().is_none_or(|selection|selection.is_empty())).on_click(cx.listener(|this,_,window,cx|this.save(false,true,window,cx))))
        .child(Button::new("statistics-copy-selected").label(crate::i18n::tr("Copy selected values")).disabled(self.busy||self.columns.is_empty()||self.selection.as_ref().is_none_or(|selection|selection.is_empty())).on_click(cx.listener(|this,_,window,cx|this.save(true,true,window,cx)))))
    .when_some(self.message.clone(),|body,text|body.child(div().text_sm().child(text))).when_some(self.error.clone(),|body,error|body.child(super::banner(error,cx)))
    ;
        #[cfg(feature = "headless-tests")]
        let panel = panel.test_support();
        div()
            .size_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .text_xl()
                    .child(crate::i18n::tr("Export daemon counters")),
            )
            .child(panel)
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .justify_end()
                    .gap_3()
                    .child(
                        Button::new("statistics-export-stop")
                            .label(crate::i18n::tr("Stop"))
                            .disabled(!self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.stop.store(true, Ordering::Relaxed);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("statistics-export-close")
                            .label(crate::i18n::tr("Close"))
                            .disabled(self.busy)
                            .on_click(|_, window, _| window.remove_window()),
                    ),
            )
    }
}

impl Drop for Export {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
