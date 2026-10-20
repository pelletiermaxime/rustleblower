//! Native Kit table over one bounded worker-prepared application page.
use super::Dashboard;
use crate::bandwidth::{self, Client, Query, Sort, Status, View};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::Button,
    input::{Input, InputEvent, InputState},
    table::{Column, ColumnSort, DataTable, TableDelegate, TableState},
};
use std::sync::{
    Arc,
    mpsc::{Receiver, SyncSender, sync_channel},
};

pub(super) struct BandwidthView {
    pub input: Entity<InputState>,
    pub client: Option<Client>,
    pub snapshot: Arc<View>,
    pub query: Query,
    table: Entity<TableState<Rows>>,
    sorts: Receiver<(Sort, bool)>,
    message: Option<String>,
    active: bool,
    #[cfg(all(test, feature = "headless-tests"))]
    pub launch_fixture: Option<bandwidth::launcher::Launch>,
}
impl BandwidthView {
    pub fn new(window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder("Filter applications…", window, cx))
                .validate(|value, _| value.len() <= 256)
        });
        cx.subscribe(&input, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.bandwidth.query.search = input.read(cx).value().to_string();
                this.bandwidth.query.offset = 0;
                this.bandwidth.request();
                cx.notify();
            }
        })
        .detach();
        let (sender, sorts) = sync_channel(4);
        let snapshot = Arc::new(View::default());
        let table = cx.new(|cx| {
            TableState::new(
                Rows {
                    view: snapshot.clone(),
                    sorts: sender,
                },
                window,
                cx,
            )
        });
        Self {
            input,
            client: None,
            snapshot,
            query: Query::default(),
            table,
            sorts,
            message: None,
            active: false,
            #[cfg(all(test, feature = "headless-tests"))]
            launch_fixture: None,
        }
    }
    fn request(&self) {
        if let Some(client) = &self.client {
            client.query(self.query.clone());
        }
    }
    pub fn poll(&mut self, cx: &mut Context<Dashboard>) -> bool {
        let active = self
            .client
            .as_ref()
            .is_some_and(|client| !client.finished());
        let mut changed = active != self.active;
        self.active = active;
        while let Ok((sort, ascending)) = self.sorts.try_recv() {
            self.query.sort = sort;
            self.query.ascending = ascending;
            self.query.offset = 0;
            self.request();
            changed = true;
        }
        if let Some(view) = self.client.as_mut().and_then(Client::snapshot)
            && !Arc::ptr_eq(&view, &self.snapshot)
        {
            self.snapshot = view;
            if self.query.offset > 0 && self.query.offset >= self.snapshot.matches {
                self.query.offset = 0;
                self.request();
            }
            self.table.update(cx, |table, cx| {
                table.delegate_mut().view = self.snapshot.clone();
                table.refresh(cx);
            });
            changed = true;
        }
        changed
    }
    pub fn stop(&self) {
        if let Some(client) = &self.client {
            client.stop();
        }
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub fn test_query_ready(&self) -> bool {
        self.client.is_none()
            || (self.snapshot.query == self.query && self.snapshot.status != Status::Starting)
    }
}
impl Dashboard {
    pub(super) fn render_bandwidth(&mut self, cx: &mut Context<Self>) -> Div {
        let snapshot = self.bandwidth.snapshot.clone();
        let active = self
            .bandwidth
            .client
            .as_ref()
            .is_some_and(|c| !c.finished());
        let pending = snapshot.query != self.bandwidth.query;
        let muted = cx.theme().tokens.muted_foreground;
        let total = |value| {
            if snapshot.started_ms.is_some() {
                bandwidth::bytes(value)
            } else {
                "—".into()
            }
        };
        div().flex().flex_col().flex_1().min_h_0().gap_3().px_4().py_3()
            .child(div().flex().flex_wrap().items_center().gap_2()
                .child(div().flex_1().text_lg().font_weight(FontWeight::SEMIBOLD).child(crate::i18n::tr("Data usage")))
                .child(Button::new("bandwidth-start").label(crate::i18n::tr("Start monitoring")).disabled(active)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let directory = this.backend.history.database_path().parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
                        let uid = unsafe { libc::getuid() };
                        #[cfg(all(test, feature = "headless-tests"))]
                        let client = if let Some(launch) = this.bandwidth.launch_fixture.clone() {
                            Client::start_fixture(directory, uid, Some(launch))
                        } else { Client::start(directory, uid) };
                        #[cfg(not(all(test, feature = "headless-tests")))]
                        let client = Client::start(directory, uid);
                        this.bandwidth.client = Some(client);
                        this.bandwidth.query.offset = 0; this.bandwidth.request();
                        this.bandwidth.snapshot = Arc::new(View { status: Status::Starting,
                            query: this.bandwidth.query.clone(), ..Default::default() });
                        this.bandwidth.table.update(cx, |table, cx| {
                            table.delegate_mut().view = this.bandwidth.snapshot.clone(); table.refresh(cx);
                        });
                        this.bandwidth.message = None; cx.notify();
                    })))
                .child(Button::new("bandwidth-stop").label(crate::i18n::tr("Stop")).disabled(!active)
                    .on_click(cx.listener(|this, _, _, cx| { this.bandwidth.stop(); cx.notify(); })))
                .child(Button::new("bandwidth-address").label(crate::i18n::tr("Copy collector address"))
                    .disabled(!active || snapshot.endpoint.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(path) = &this.bandwidth.snapshot.endpoint {
                            cx.write_to_clipboard(ClipboardItem::new_string(path.to_string_lossy().into_owned()));
                            this.bandwidth.message = Some(crate::i18n::tr("Collector address copied")); cx.notify();
                        }
                    }))))
            .child(div().text_sm().child(crate::i18n::tr(snapshot.status.label())))
            .when(snapshot.status == Status::Waiting, |view| view.child(div().text_sm().text_color(muted)
                .child(crate::i18n::tr("The local collector is starting. Usage appears when its first measurement arrives."))))
            .when(snapshot.status == Status::Authorizing, |view| view.child(div().text_sm().text_color(muted)
                .child(crate::i18n::tr("Approve the collector in the system authentication dialog. The app never receives your password."))))
            .when_some(snapshot.warning.clone(), |view, warning| view.child(super::banner(warning, cx)))
            .when_some(self.bandwidth.message.clone(), |view, message| view.child(div().text_sm().text_color(muted).child(message)))
            .child(div().flex().flex_wrap().gap_4().text_sm()
                .child(format!("{}: {}", crate::i18n::tr("Downloaded"), total(snapshot.received)))
                .child(format!("{}: {}", crate::i18n::tr("Uploaded"), total(snapshot.sent)))
                .child(div().text_color(muted).child(crate::i18n::tr("Totals since monitoring started"))))
            .child(Input::new(&self.bandwidth.input).id("bandwidth-search"))
            .child(div().flex_1().min_h_0().child(DataTable::new(&self.bandwidth.table)))
            .child(div().flex().items_center().gap_2()
                .child(Button::new("bandwidth-previous").label(crate::i18n::tr("Previous page"))
                    .disabled(pending || self.bandwidth.query.offset == 0)
                    .on_click(cx.listener(|this, _, _, cx| { this.bandwidth.query.offset = this.bandwidth.query.offset.saturating_sub(bandwidth::PAGE); this.bandwidth.request(); cx.notify(); })))
                .child(Button::new("bandwidth-next").label(crate::i18n::tr("Next page"))
                    .disabled(pending || self.bandwidth.query.offset + snapshot.rows.len() >= snapshot.matches)
                    .on_click(cx.listener(|this, _, _, cx| { this.bandwidth.query.offset += bandwidth::PAGE; this.bandwidth.request(); cx.notify(); })))
                .child(div().text_sm().text_color(muted).child(if pending { crate::i18n::tr("Updating…") }
                    else { format!("{} {}", snapshot.matches, crate::i18n::tr("applications")) })))
            .child(div().text_xs().text_color(muted).child(crate::i18n::tr("TCP/UDP socket payload bytes. Network headers and retransmission overhead are excluded.")))
    }
}
struct Rows {
    view: Arc<View>,
    sorts: SyncSender<(Sort, bool)>,
}
impl TableDelegate for Rows {
    fn columns_count(&self, _: &App) -> usize {
        5
    }
    fn rows_count(&self, _: &App) -> usize {
        self.view.rows.len()
    }
    fn column(&self, index: usize, _: &App) -> Column {
        let labels = [
            "Application",
            "Download rate",
            "Upload rate",
            "Downloaded",
            "Uploaded",
        ];
        let mut column =
            Column::new(format!("usage-{index}"), crate::i18n::tr(labels[index])).sortable();
        column.width = px(if index == 0 { 280. } else { 120. });
        column.min_width = px(80.);
        column.movable = false;
        if self.view.query.sort.index() == index {
            column.sort = Some(if self.view.query.ascending {
                ColumnSort::Ascending
            } else {
                ColumnSort::Descending
            });
        }
        column
    }
    fn perform_sort(
        &mut self,
        index: usize,
        order: ColumnSort,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        // The four-slot queue is drained each UI tick; never block input.
        if self
            .sorts
            .try_send((Sort::column(index), order == ColumnSort::Ascending))
            .is_err()
        {
            cx.notify();
        }
    }
    fn cell_text(&self, row: usize, column: usize, _: &App) -> String {
        let Some(row) = self.view.rows.get(row) else {
            return String::new();
        };
        match column {
            0 => row.path.clone().unwrap_or_else(|| {
                format!("{} ({})", row.name, crate::i18n::tr("path unavailable"))
            }),
            1 => bandwidth::rate(row.download),
            2 => bandwidth::rate(row.upload),
            3 => bandwidth::bytes(row.received),
            _ => bandwidth::bytes(row.sent),
        }
    }
    fn fit_column(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Option<Pixels> {
        Some(super::table_layout::fit_column(
            self, index, false, window, cx,
        ))
    }
    fn render_td(
        &mut self,
        row: usize,
        column: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .truncate()
            .text_sm()
            .child(self.cell_text(row, column, cx))
    }
}
