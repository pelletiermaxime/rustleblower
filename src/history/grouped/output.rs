//! Captured group selection and streaming output. No expanded Select-all ID list.
use super::{Column, Group, Snapshot};
use crate::history::export::{self, Format, Report};
use anyhow::{Result, ensure};
use std::{
    collections::BTreeMap,
    io::{BufWriter, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
pub const MAX_MARKS: usize = 1024;
pub const MAX_COLUMNS: usize = 21;

#[derive(Clone)]
pub struct Selection {
    snapshot: Arc<Snapshot>,
    sort: (Column, bool),
    all: bool,
    range: Option<(usize, usize)>,
    overrides: BTreeMap<usize, bool>,
    anchor: Option<usize>,
}
impl Snapshot {
    pub fn selection(self: &Arc<Self>, sort: (Column, bool)) -> Selection {
        Selection {
            snapshot: self.clone(),
            sort,
            all: false,
            range: None,
            overrides: BTreeMap::new(),
            anchor: None,
        }
    }
    pub fn export(self: &Arc<Self>, sort: (Column, bool), columns: Vec<Column>) -> Result<Export> {
        Export::new(self.clone(), sort, columns, None)
    }
}
impl Selection {
    fn base(&self, index: usize) -> bool {
        self.all
            || self
                .range
                .is_some_and(|(low, high)| (low..=high).contains(&index))
    }
    pub fn contains(&self, index: usize) -> bool {
        index < self.snapshot.groups
            && self
                .overrides
                .get(&index)
                .copied()
                .unwrap_or_else(|| self.base(index))
    }
    pub fn count(&self) -> usize {
        let mut count = if self.all {
            self.snapshot.groups
        } else {
            self.range.map_or(0, |(low, high)| high - low + 1)
        };
        for (&index, &selected) in &self.overrides {
            match (selected, self.base(index)) {
                (true, false) => count += 1,
                (false, true) => count -= 1,
                _ => {}
            }
        }
        count
    }
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }
    pub fn clear(&mut self) {
        self.all = false;
        self.range = None;
        self.overrides.clear();
        self.anchor = None;
    }
    pub fn select_all(&mut self) {
        self.clear();
        self.all = true;
    }
    pub fn toggle(&mut self, index: usize) -> Result<()> {
        ensure!(
            index < self.snapshot.groups,
            "Grouped selection is outside the captured view"
        );
        let selected = !self.contains(index);
        let needs_override = selected != self.base(index);
        ensure!(
            !needs_override
                || self.overrides.contains_key(&index)
                || self.overrides.len() < MAX_MARKS,
            "Grouped selection is full; select a range or all matching groups instead"
        );
        if needs_override {
            self.overrides.insert(index, selected);
        } else {
            self.overrides.remove(&index);
        }
        self.anchor = Some(index);
        Ok(())
    }
    pub fn extend_to(&mut self, index: usize) -> Result<()> {
        ensure!(
            index < self.snapshot.groups,
            "Grouped selection is outside the captured view"
        );
        let Some(anchor) = self.anchor else {
            return self.toggle(index);
        };
        let low = anchor.min(index);
        let high = anchor.max(index);
        self.all = false;
        self.overrides
            .retain(|&index, selected| *selected && (index < low || index > high));
        self.range = Some((low, high));
        Ok(())
    }
    pub fn export(&self, columns: Vec<Column>) -> Result<Export> {
        ensure!(!self.is_empty(), "Select retained history groups first");
        Export::new(
            self.snapshot.clone(),
            self.sort,
            columns,
            Some(self.clone()),
        )
    }
}
#[derive(Clone)]
pub struct Export {
    snapshot: Arc<Snapshot>,
    sort: (Column, bool),
    columns: Vec<Column>,
    selection: Option<Selection>,
}
impl Export {
    pub fn with_columns(&self, columns: Vec<Column>) -> Result<Self> {
        Self::new(
            self.snapshot.clone(),
            self.sort,
            columns,
            self.selection.clone(),
        )
    }
    fn new(
        snapshot: Arc<Snapshot>,
        sort: (Column, bool),
        columns: Vec<Column>,
        selection: Option<Selection>,
    ) -> Result<Self> {
        ensure!(
            !columns.is_empty()
                && columns.len() <= MAX_COLUMNS
                && columns
                    .iter()
                    .enumerate()
                    .all(|(index, column)| !columns[..index].contains(column)),
            "Choose unique grouped-history columns first"
        );
        Ok(Self {
            snapshot,
            sort,
            columns,
            selection,
        })
    }
    fn write(
        &self,
        format: Format,
        stop: Arc<AtomicBool>,
        limit: usize,
        writer: &mut impl Write,
    ) -> Result<Report> {
        let db = self.snapshot.open(stop.clone())?;
        let direction = if self.sort.1 { "ASC" } else { "DESC" };
        let mut statement = db.prepare(&format!(
            "SELECT {},hits FROM grouped_detail ORDER BY {} {direction},id {direction}",
            super::COLUMNS,
            self.sort.0.sort_sql()
        ))?;
        let mut records = statement.query([])?;
        let mut report = Report { rows: 0, bytes: 0 };
        let mut index = 0;
        let mut line = |text: String| -> Result<()> {
            ensure!(
                !stop.load(Ordering::Relaxed),
                "Grouped history output stopped; destination was not replaced"
            );
            ensure!(
                report.bytes + text.len() <= limit,
                "Grouped history output exceeds its size limit"
            );
            writer.write_all(text.as_bytes())?;
            report.bytes += text.len();
            Ok(())
        };
        if matches!(format, Format::Csv) {
            line(export::csv_line(
                self.columns.iter().map(|column| column.label().to_owned()),
            ))?;
        }
        while let Some(record) = records.next()? {
            let selected = self
                .selection
                .as_ref()
                .is_none_or(|selection| selection.contains(index));
            index += 1;
            if !selected {
                continue;
            }
            let group = Group {
                event: crate::history::read_row(record)?,
                hits: record.get(10)?,
            };
            line(encode(&group, &self.columns, format)?)?;
            report.rows += 1;
        }
        ensure!(
            !stop.load(Ordering::Relaxed),
            "Grouped history output stopped; destination was not replaced"
        );
        Ok(report)
    }
    pub fn copy(&self, format: Format, stop: Arc<AtomicBool>) -> Result<(String, Report)> {
        let mut output = Vec::new();
        let report = self.write(format, stop, 4 * 1024 * 1024, &mut output)?;
        Ok((String::from_utf8(output)?, report))
    }
    pub fn save(
        &self,
        target: &Path,
        format: Format,
        replace: bool,
        stop: Arc<AtomicBool>,
    ) -> Result<Report> {
        export::validate_target(&self.snapshot.database, target, replace)?;
        let mut output = tempfile::NamedTempFile::new_in(target.parent().unwrap())?;
        let report = {
            let mut writer = BufWriter::with_capacity(16 * 1024, &mut output);
            let report = self.write(format, stop.clone(), export::MAX_BYTES, &mut writer)?;
            writer.flush()?;
            report
        };
        output.as_file().sync_all()?;
        ensure!(
            !stop.load(Ordering::Relaxed),
            "Grouped history output stopped; destination was not replaced"
        );
        if replace {
            output.persist(target).map_err(|error| error.error)?;
        } else {
            output
                .persist_noclobber(target)
                .map_err(|error| error.error)?;
        }
        Ok(report)
    }
}
fn encode(group: &Group, columns: &[Column], format: Format) -> Result<String> {
    let text = |column: &Column| match column {
        Column::Hits => group.hits.to_string(),
        Column::Event(field) => field.text(&group.event),
    };
    match format {
        Format::Csv => Ok(export::csv_line(columns.iter().map(text))),
        Format::JsonLines => {
            let values = columns
                .iter()
                .map(|column| match column {
                    Column::Hits => (
                        "retained_hits".to_owned(),
                        serde_json::Value::from(group.hits),
                    ),
                    Column::Event(field) => (field.key(), serde_json::Value::String(text(column))),
                })
                .collect::<serde_json::Map<_, _>>();
            Ok(format!("{}\n", serde_json::Value::Object(values)))
        }
    }
}
