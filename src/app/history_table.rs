use crate::history::{Row, query::Field};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme,
    checkbox::Checkbox,
    table::{Column, ColumnSort, TableDelegate, TableState},
};
use std::sync::{Arc, mpsc::SyncSender};
pub(super) struct Rows {
    pub rows: Arc<Vec<Row>>,
    pub columns: Vec<Field>,
    pub sort: Option<(Field, bool)>,
    pub widths: Vec<(Field, Pixels)>,
    pub highlights: Arc<crate::highlighting::Snapshot>,
    pub selected: Option<i64>,
    pub marks: Option<Arc<crate::history::selection::Selection>>,
    pub selection_generation: u64,
    pub selection_error: Option<String>,
    pub cache: super::highlighting::HistoryCache,
    pub sender: SyncSender<super::history_tools::Message>,
}
impl TableDelegate for Rows {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }
    fn column(&self, index: usize, _: &App) -> Column {
        let field = self.columns[index];
        let mut column = Column::new(field.key(), crate::i18n::tr(field.label())).sortable();
        column.width = self
            .widths
            .iter()
            .find(|(key, _)| *key == field)
            .map_or(px(field.width()), |(_, width)| *width);
        column.min_width = px(40.);
        column.max_width = px(2000.);
        column.movable = false;
        if let Some((sorted, ascending)) = self.sort
            && sorted == field
        {
            column.sort = Some(if ascending {
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
        let field = self.columns[index];
        let sort = match sort {
            ColumnSort::Default => None,
            ColumnSort::Ascending => Some((field, true)),
            ColumnSort::Descending => Some((field, false)),
        };
        let _ = self
            .sender
            .try_send(super::history_tools::Message::Sort(sort));
    }
    fn cell_text(&self, row: usize, column: usize, _: &App) -> String {
        self.columns[column].text(&self.rows[row])
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
        div().id(("row", row)).when(
            self.marks
                .as_ref()
                .is_some_and(|selection| selection.contains(&self.rows[row])),
            |row| row.bg(cx.theme().tokens.table_active),
        )
    }
    fn render_td(
        &mut self,
        row: usize,
        column: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let field = self.columns[column];
        let id = self.rows[row].id;
        let marked = self
            .marks
            .as_ref()
            .is_some_and(|selection| selection.contains(&self.rows[row]));
        let selected = self.selected == Some(id) || marked;
        let cell = if !self.highlights.enabled(crate::highlighting::View::History) || selected {
            div()
                .truncate()
                .text_sm()
                .child(field.text(&self.rows[row]))
        } else {
            let cached = self.cache.row(&self.rows, &self.highlights, row, false);
            let key = field.key();
            let value = cached
                .values
                .iter()
                .find(|(column, _)| column == &key)
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            super::highlighting::cell_prepared(
                &self.highlights,
                crate::highlighting::View::History,
                &key,
                &value,
                cached.prepared,
                div().truncate().text_sm().child(value.clone()),
            )
        };
        if column != 0 {
            return cell;
        }
        let generation = self.selection_generation;
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Checkbox::new(("history-mark", id as u64))
                    .checked(marked)
                    .on_change(cx.listener(move |table, _, window, cx| {
                        cx.stop_propagation();
                        let delegate = table.delegate_mut();
                        if delegate
                            .sender
                            .try_send(super::history_tools::Message::Mark {
                                id,
                                generation,
                                extend: window.modifiers().shift,
                            })
                            .is_err()
                        {
                            delegate.selection_error = Some(crate::i18n::tr(
                                "History selection input was not queued; try again.",
                            ));
                        }
                        cx.notify();
                    })),
            )
            .child(cell.flex_1().min_w_0())
    }
}
