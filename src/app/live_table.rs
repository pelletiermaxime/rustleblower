//! Pinned Kit table adapter; cells read only one immutable socket snapshot.
use crate::live::{
    model::Sockets,
    presentation::{Column as SettingColumn, Sort},
    table::{Capture, Selection},
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme,
    checkbox::Checkbox,
    table::{Column, ColumnSort, TableDelegate, TableState},
};
use std::{
    collections::VecDeque,
    sync::{Arc, mpsc::SyncSender},
};
pub(super) enum Message {
    Sort(Sort),
    Mark {
        capture: Capture,
        position: usize,
        extend: bool,
    },
}
struct Cached {
    index: usize,
    selected: bool,
    values: Vec<(&'static str, String)>,
    prepared: Option<crate::highlighting::PreparedRow>,
}
pub(super) struct Rows {
    pub data: Option<Arc<Sockets>>,
    pub order: Arc<Vec<usize>>,
    pub offset: usize,
    pub columns: Vec<SettingColumn>,
    pub sort: Sort,
    pub selected: Option<usize>,
    pub marks: Arc<Selection>,
    pub capture: Capture,
    pub highlights: Arc<crate::highlighting::Snapshot>,
    pub sender: SyncSender<Message>,
    cache: VecDeque<Cached>,
    cache_data: std::sync::Weak<Sockets>,
    cache_highlights: std::sync::Weak<crate::highlighting::Snapshot>,
    pub error: Option<String>,
}
impl Rows {
    pub fn new(
        sender: SyncSender<Message>,
        highlights: Arc<crate::highlighting::Snapshot>,
    ) -> Self {
        let settings = crate::live::presentation::Settings::default();
        Self {
            data: None,
            order: Arc::new(Vec::new()),
            offset: 0,
            columns: settings.columns,
            sort: settings.sort,
            selected: None,
            marks: Arc::new(Selection::default()),
            capture: Capture {
                session: 0,
                revision: 0,
            },
            highlights,
            sender,
            cache: VecDeque::new(),
            cache_data: Default::default(),
            cache_highlights: Default::default(),
            error: None,
        }
    }
    fn cached(&mut self, index: usize, selected: bool) -> &Cached {
        let data = self.data.as_ref().expect("table rows require snapshot");
        let weak = Arc::downgrade(data);
        let highlights = Arc::downgrade(&self.highlights);
        if !weak.ptr_eq(&self.cache_data) || !highlights.ptr_eq(&self.cache_highlights) {
            self.cache.clear();
            self.cache_data = weak;
            self.cache_highlights = highlights;
        }
        let existing = self.cache.iter().position(|row| row.index == index);
        let position =
            if let Some(position) = existing.filter(|i| self.cache[*i].selected == selected) {
                position
            } else {
                if let Some(position) = existing {
                    self.cache.remove(position);
                }
                if self.cache.len() >= 64 {
                    self.cache.pop_front();
                }
                let row = &data.rows[index];
                let mut values = crate::live::presentation::FIELDS
                    .into_iter()
                    .map(|field| (field.key(), field.text(row)))
                    .collect::<Vec<_>>();
                values.push(("title", row.title.clone()));
                values.push(("endpoints", row.endpoints.clone()));
                let strings = values
                    .iter()
                    .map(|(key, value)| (*key, value.as_str()))
                    .collect::<Vec<_>>();
                let prepared = self.highlights.prepare_row(
                    crate::highlighting::View::LiveSockets,
                    &strings,
                    selected,
                );
                self.cache.push_back(Cached {
                    index,
                    selected,
                    values,
                    prepared,
                });
                self.cache.len() - 1
            };
        &self.cache[position]
    }
}
impl TableDelegate for Rows {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.order
            .len()
            .saturating_sub(self.offset)
            .min(crate::live::model::PAGE)
    }
    fn column(&self, index: usize, _: &App) -> Column {
        let setting = self.columns[index];
        let mut column =
            Column::new(setting.field.key(), crate::i18n::tr(setting.field.label())).sortable();
        column.width = px(setting.width as f32);
        column.min_width = px(40.);
        column.max_width = px(2000.);
        column.movable = false;
        if setting.field == self.sort.field {
            column.sort = Some(if self.sort.ascending {
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
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        let sort = Sort {
            field: self.columns[index].field,
            // Kit cycles Ascending through Default. This view always has a
            // sort, so Default must return to Descending rather than pin Asc.
            ascending: sort == ColumnSort::Ascending,
        };
        if self.sender.try_send(Message::Sort(sort)).is_err() {
            self.error = Some(crate::i18n::tr(
                "Live table input was not queued; try again.",
            ));
        }
    }
    fn cell_text(&self, row: usize, column: usize, _: &App) -> String {
        self.data
            .as_ref()
            .and_then(|data| {
                self.order
                    .get(self.offset + row)
                    .and_then(|index| data.rows.get(*index))
            })
            .map(|row| self.columns[column].field.text(row))
            .unwrap_or_default()
    }
    fn fit_column(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Option<Pixels> {
        Some(super::table_layout::fit_column(
            self,
            index,
            index == 0,
            window,
            cx,
        ))
    }
    fn render_tr(
        &mut self,
        row: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let marked = self
            .order
            .get(self.offset + row)
            .is_some_and(|index| self.marks.contains(*index));
        div()
            .id(("row", row))
            .when(marked, |row| row.bg(cx.theme().tokens.table_active))
    }
    fn render_td(
        &mut self,
        row: usize,
        column: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let index = self.order[self.offset + row];
        let field = self.columns[column].field;
        let marked = self.marks.contains(index);
        let selected = marked || self.selected == Some(index);
        let snapshot = self.highlights.clone();
        let cached = self.cached(index, selected);
        let value = cached
            .values
            .iter()
            .find(|(key, _)| *key == field.key())
            .map(|(_, value)| value.as_str())
            .unwrap_or_default();
        let cell = super::highlighting::cell_prepared(
            &snapshot,
            crate::highlighting::View::LiveSockets,
            field.key(),
            value,
            cached.prepared,
            div().truncate().text_sm().child(value.to_owned()),
        );
        if column != 0 {
            return cell;
        }
        let capture = self.capture;
        let position = self.offset + row;
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Checkbox::new(("live-mark", index as u64))
                    .checked(marked)
                    .on_change(cx.listener(move |table, _, window, cx| {
                        cx.stop_propagation();
                        let delegate = table.delegate_mut();
                        if delegate
                            .sender
                            .try_send(Message::Mark {
                                capture,
                                position,
                                extend: window.modifiers().shift,
                            })
                            .is_err()
                        {
                            delegate.error = Some(crate::i18n::tr(
                                "Live table input was not queued; try again.",
                            ));
                        }
                        cx.notify();
                    })),
            )
            .child(cell.flex_1().min_w_0())
    }
}
