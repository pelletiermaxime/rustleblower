//! Native style application and one bounded reload action. Paint only reads a
//! captured immutable generation; profile files belong to the backend worker.
use crate::highlighting::{Applied, Command, Handle, PreparedRow, Snapshot, View};
use gpui::{prelude::*, *};

#[derive(Default)]
struct Reload {
    pending: bool,
    message: Option<String>,
}
impl Global for Reload {}
pub(super) fn pending(cx: &App) -> bool {
    cx.try_global::<Reload>().is_some_and(|state| state.pending)
}
pub(super) fn message(cx: &App) -> Option<String> {
    cx.try_global::<Reload>()
        .and_then(|state| state.message.clone())
        .filter(|message| !message.is_empty())
}
pub(super) fn reload(handle: &Handle, cx: &mut App) {
    if pending(cx) {
        return;
    }
    cx.set_global(Reload {
        pending: true,
        message: None,
    });
    let reply = handle.request(Command::Reload);
    cx.spawn(async move |cx| {
        let result = reply.await;
        cx.update(|cx| {
            let message = match result {
                Ok(Ok(snapshot)) => {
                    if snapshot.warning.is_some() {
                        String::new()
                    } else {
                        crate::i18n::tr("Table highlighting reloaded")
                    }
                }
                Ok(Err(error)) => error,
                Err(_) => crate::i18n::tr("Table highlighting is unavailable"),
            };
            cx.set_global(Reload {
                pending: false,
                message: Some(crate::history::bounded(&message, 1024)),
            });
            cx.refresh_windows();
        });
    })
    .detach();
}
/// Stable caller-supplied keys and displayed cell strings are the matching
/// contract. Selection is evaluated before any configured style is applied.
pub(super) fn cell_prepared(
    snapshot: &Snapshot,
    view: View,
    key: &str,
    value: &str,
    row: Option<PreparedRow>,
    cell: Div,
) -> Div {
    match row.and_then(|row| snapshot.style_prepared(view, key, value, row)) {
        Some(style) => apply(cell, style),
        None => cell,
    }
}
fn apply(mut cell: Div, style: Applied) -> Div {
    if let Some(color) = style.color {
        cell = cell.text_color(rgba(u32::from_be_bytes(color)));
    }
    if let Some(color) = style.bgcolor {
        cell = cell.bg(rgba(u32::from_be_bytes(color)));
    }
    if style.horizontal_center || style.vertical_center {
        cell = cell.flex();
    }
    if style.horizontal_center {
        cell = cell.justify_center();
    }
    if style.vertical_center {
        cell = cell.items_center();
    }
    if let Some([horizontal, vertical]) = style.margins {
        cell = cell.px(px(horizontal as f32)).py(px(vertical as f32));
    }
    cell.overflow_hidden()
}
pub(super) fn statistics(kind: crate::statistics::Kind) -> View {
    use crate::statistics::Kind;
    match kind {
        Kind::Applications => View::Applications,
        Kind::Hosts => View::Hosts,
        Kind::Addresses => View::Addresses,
        Kind::Ports => View::Ports,
        Kind::Users => View::Users,
        Kind::Protocols => View::Protocols,
    }
}

/// All native History fields participate in row matching, including columns
/// currently hidden in the table or card presentation. No metadata read occurs.
pub(super) fn history_values(row: &crate::history::Row) -> Vec<(String, String)> {
    crate::history::query::FIELDS
        .iter()
        .map(|field| (field.key(), field.text(row)))
        .collect()
}

const HISTORY_CACHE_ROWS: usize = 64;
#[derive(Default)]
pub(super) struct HistoryCache {
    rows: std::sync::Weak<Vec<crate::history::Row>>,
    snapshot: std::sync::Weak<Snapshot>,
    entries: std::collections::VecDeque<CachedHistory>,
}
pub(super) struct CachedHistory {
    index: usize,
    selected: bool,
    pub values: Vec<(String, String)>,
    pub prepared: Option<PreparedRow>,
}
impl HistoryCache {
    pub fn sync(
        &mut self,
        rows: &std::sync::Arc<Vec<crate::history::Row>>,
        snapshot: &std::sync::Arc<Snapshot>,
    ) {
        let current_rows = std::sync::Arc::downgrade(rows);
        let current_snapshot = std::sync::Arc::downgrade(snapshot);
        if !self.rows.ptr_eq(&current_rows) || !self.snapshot.ptr_eq(&current_snapshot) {
            self.entries.clear();
            self.rows = current_rows;
            self.snapshot = current_snapshot;
        }
    }
    pub fn row(
        &mut self,
        rows: &std::sync::Arc<Vec<crate::history::Row>>,
        snapshot: &std::sync::Arc<Snapshot>,
        index: usize,
        selected: bool,
    ) -> &CachedHistory {
        self.sync(rows, snapshot);
        let existing = self.entries.iter().position(|row| row.index == index);
        let position = if let Some(position) =
            existing.filter(|position| self.entries[*position].selected == selected)
        {
            position
        } else {
            if let Some(position) = existing {
                self.entries.remove(position);
            }
            if self.entries.len() >= HISTORY_CACHE_ROWS {
                self.entries.pop_front();
            }
            let values = history_values(&rows[index]);
            let displayed = values
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect::<Vec<_>>();
            let prepared = snapshot.prepare_row(View::History, &displayed, selected);
            self.entries.push_back(CachedHistory {
                index,
                selected,
                values,
                prepared,
            });
            self.entries.len() - 1
        };
        &self.entries[position]
    }
}

#[cfg(test)]
mod tests {
    use super::{HISTORY_CACHE_ROWS, HistoryCache};
    use crate::highlighting::{Profile, Rule, Settings, Snapshot, Style, View};
    use std::sync::Arc;
    fn row(id: i64, process: &str) -> crate::history::Row {
        crate::history::Row {
            id,
            unixnano: 0,
            action: "deny".into(),
            protocol: "tcp".into(),
            process: process.into(),
            destination: "192.0.2.1".into(),
            port: 443,
            reason: String::new(),
            node: "owned".into(),
            metadata: Default::default(),
        }
    }
    fn snapshot(color: &str) -> Arc<Snapshot> {
        let settings = Settings {
            profiles: vec![Profile {
                view: View::History,
                enabled: true,
                cells: vec![Rule {
                    cols: vec!["process".into()],
                    text: vec!["after".into()],
                    style: Style {
                        bgcolor: Some(color.into()),
                        ..Default::default()
                    },
                }],
                rows: vec![],
            }],
        };
        Arc::new(Snapshot {
            ready: true,
            revision: None,
            profiles: settings.compile().unwrap(),
            settings,
            warning: None,
        })
    }
    #[test]
    fn history_cache_tracks_page_style_generations_selection_and_bound() {
        let first = Arc::new(vec![row(1, "before")]);
        let red = snapshot("red");
        let mut cache = HistoryCache::default();
        let before = cache.row(&first, &red, 0, false);
        assert!(
            red.style_prepared(View::History, "process", "before", before.prepared.unwrap())
                .is_none()
        );
        // Same database id, new metadata/page generation must replace strings.
        let changed = Arc::new(vec![row(1, "after")]);
        let after = cache.row(&changed, &red, 0, false);
        assert_eq!(
            after
                .values
                .iter()
                .find(|(key, _)| key == "process")
                .unwrap()
                .1,
            "after"
        );
        assert_eq!(
            red.style_prepared(View::History, "process", "after", after.prepared.unwrap())
                .unwrap()
                .bgcolor,
            Some([255, 0, 0, 255])
        );
        // Both revisions are None: Arc generation, not revision alone, matters.
        let blue = snapshot("blue");
        let recolored = cache.row(&changed, &blue, 0, false);
        assert_eq!(
            blue.style_prepared(
                View::History,
                "process",
                "after",
                recolored.prepared.unwrap()
            )
            .unwrap()
            .bgcolor,
            Some([0, 0, 255, 255])
        );
        let selected = cache.row(&changed, &blue, 0, true);
        assert!(
            blue.style_prepared(
                View::History,
                "process",
                "after",
                selected.prepared.unwrap()
            )
            .is_none()
        );
        let page = Arc::new((0..200).map(|id| row(id, "after")).collect());
        for index in 0..200 {
            cache.row(&page, &blue, index, false);
        }
        assert_eq!(cache.entries.len(), HISTORY_CACHE_ROWS);
        cache.sync(&Arc::new(Vec::new()), &blue);
        assert!(cache.entries.is_empty());
    }
}
