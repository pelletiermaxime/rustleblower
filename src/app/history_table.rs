use crate::history::{Row, query::Field};
use gpui::{prelude::*, *};
use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use std::sync::{Arc, mpsc::SyncSender};
pub(super) struct Rows {
    pub rows: Arc<Vec<Row>>,
    pub columns: Vec<Field>,
    pub sort: Option<(Field, bool)>,
    pub widths: Vec<(Field, Pixels)>,
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
        let mut column = Column::new(field.key(), field.label()).sortable();
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
