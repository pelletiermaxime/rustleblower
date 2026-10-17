//! Selection belongs to one immutable disk-backed counter snapshot. Positions
//! remain stable across paging; no keys or counter rows are copied into marks.
use super::{Row, Snapshot};
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
const FILE_BYTES: usize = 16 * 1024 * 1024;
const CLIPBOARD_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Value,
    Hits,
    Reported,
    Nodes,
}
pub const COLUMNS: &[Column] = &[Column::Value, Column::Hits, Column::Reported, Column::Nodes];
impl Column {
    pub fn label(self) -> &'static str {
        match self {
            Self::Value => "Value",
            Self::Hits => "Hits since local reset",
            Self::Reported => "Last reported total",
            Self::Nodes => "Nodes",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::Hits => "hits",
            Self::Reported => "reported",
            Self::Nodes => "nodes",
        }
    }
    fn text(self, row: &Row) -> String {
        match self {
            Self::Value => row.key.clone(),
            Self::Hits => row.hits.to_string(),
            Self::Reported => row.reported.to_string(),
            Self::Nodes => row.nodes.to_string(),
        }
    }
    fn value(self, row: &Row) -> serde_json::Value {
        match self {
            Self::Nodes => row.nodes.into(),
            _ => self.text(row).into(),
        }
    }
}

#[derive(Clone, Default)]
struct Positions {
    all: bool,
    range: Option<(usize, usize)>,
    overrides: BTreeMap<usize, bool>,
    anchor: Option<usize>,
}
impl Positions {
    fn base(&self, index: usize) -> bool {
        self.all
            || self
                .range
                .is_some_and(|(low, high)| (low..=high).contains(&index))
    }
    fn contains(&self, index: usize) -> bool {
        self.overrides
            .get(&index)
            .copied()
            .unwrap_or_else(|| self.base(index))
    }
    fn count(&self, length: usize) -> usize {
        let mut count = if self.all {
            length
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
}

#[derive(Clone)]
pub struct Selection {
    snapshot: Arc<Snapshot>,
    positions: Positions,
}
impl Snapshot {
    pub fn selection(self: &Arc<Self>) -> Selection {
        Selection {
            snapshot: self.clone(),
            positions: Positions::default(),
        }
    }
}
impl Selection {
    pub fn snapshot(&self) -> &Arc<Snapshot> {
        &self.snapshot
    }
    pub fn count(&self) -> usize {
        self.positions.count(self.snapshot.len())
    }
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }
    pub fn contains(&self, index: usize) -> bool {
        index < self.snapshot.len() && self.positions.contains(index)
    }
    pub fn clear(&mut self) {
        self.positions = Positions::default();
    }
    pub fn select_all(&mut self) {
        self.clear();
        self.positions.all = true;
    }
    pub fn toggle(&mut self, index: usize) -> Result<()> {
        ensure!(
            index < self.snapshot.len(),
            "Counter selection is outside the captured view"
        );
        let selected = !self.contains(index);
        let needs_override = selected != self.positions.base(index);
        ensure!(
            !needs_override
                || self.positions.overrides.contains_key(&index)
                || self.positions.overrides.len() < MAX_MARKS,
            "Counter selection is full; select a range or all matching values instead"
        );
        if needs_override {
            self.positions.overrides.insert(index, selected);
        } else {
            self.positions.overrides.remove(&index);
        }
        self.positions.anchor = Some(index);
        Ok(())
    }
    pub fn extend_to(&mut self, index: usize) -> Result<()> {
        ensure!(
            index < self.snapshot.len(),
            "Counter selection is outside the captured view"
        );
        let Some(anchor) = self.positions.anchor else {
            return self.toggle(index);
        };
        let low = anchor.min(index);
        let high = anchor.max(index);
        self.positions.all = false;
        self.positions
            .overrides
            .retain(|&index, _| index < low || index > high);
        self.positions.range = Some((low, high));
        Ok(())
    }
    pub fn copy(
        &self,
        columns: &[Column],
        json: bool,
        stop: &AtomicBool,
    ) -> Result<(String, usize)> {
        ensure!(!self.is_empty(), "Select counter values first");
        let mut output = Vec::new();
        let count = encode(
            &self.snapshot,
            Some(&self.positions),
            columns,
            json,
            stop,
            CLIPBOARD_BYTES,
            &mut output,
        )?;
        Ok((String::from_utf8(output)?, count))
    }
    pub fn save(
        &self,
        columns: &[Column],
        target: &Path,
        json: bool,
        replace: bool,
        stop: &AtomicBool,
    ) -> Result<usize> {
        ensure!(!self.is_empty(), "Select counter values first");
        save(
            &self.snapshot,
            Some(&self.positions),
            columns,
            target,
            json,
            replace,
            stop,
        )
    }
}

fn encode(
    snapshot: &Snapshot,
    positions: Option<&Positions>,
    columns: &[Column],
    json: bool,
    stop: &AtomicBool,
    limit: usize,
    writer: &mut impl Write,
) -> Result<usize> {
    ensure!(
        !columns.is_empty()
            && columns.len() <= COLUMNS.len()
            && columns
                .iter()
                .enumerate()
                .all(|(index, column)| !columns[..index].contains(column)),
        "Choose unique counter columns first"
    );
    let mut bytes = 0;
    let mut count = 0;
    let mut write_line = |line: String| -> Result<()> {
        ensure!(
            !stop.load(Ordering::Relaxed),
            "Statistics output stopped; destination was not replaced"
        );
        ensure!(
            bytes + line.len() <= limit,
            "Statistics output exceeds its size limit"
        );
        writer.write_all(line.as_bytes())?;
        bytes += line.len();
        Ok(())
    };
    if !json {
        write_line(crate::history::export::csv_line(
            columns.iter().map(|column| column.label().to_owned()),
        ))?;
    }
    for start in (0..snapshot.len()).step_by(super::PAGE_SIZE) {
        ensure!(
            !stop.load(Ordering::Relaxed),
            "Statistics output stopped; destination was not replaced"
        );
        for (offset, row) in snapshot.page(start)?.into_iter().enumerate() {
            if positions.is_some_and(|positions| !positions.contains(start + offset)) {
                continue;
            }
            let line = if json {
                let object = columns
                    .iter()
                    .map(|column| (column.key().to_owned(), column.value(&row)))
                    .collect::<serde_json::Map<_, _>>();
                format!("{}\n", serde_json::Value::Object(object))
            } else {
                crate::history::export::csv_line(columns.iter().map(|column| column.text(&row)))
            };
            write_line(line)?;
            count += 1;
        }
    }
    ensure!(
        !stop.load(Ordering::Relaxed),
        "Statistics output stopped; destination was not replaced"
    );
    Ok(count)
}

fn save(
    snapshot: &Snapshot,
    positions: Option<&Positions>,
    columns: &[Column],
    target: &Path,
    json: bool,
    replace: bool,
    stop: &AtomicBool,
) -> Result<usize> {
    crate::history::export::validate_target(&snapshot.database, target, replace)?;
    let mut output = tempfile::NamedTempFile::new_in(target.parent().unwrap())?;
    let count = {
        let mut writer = BufWriter::with_capacity(16 * 1024, &mut output);
        let count = encode(
            snapshot,
            positions,
            columns,
            json,
            stop,
            FILE_BYTES,
            &mut writer,
        )?;
        writer.flush()?;
        count
    };
    output.as_file().sync_all()?;
    ensure!(
        !stop.load(Ordering::Relaxed),
        "Statistics output stopped; destination was not replaced"
    );
    if replace {
        output.persist(target).map_err(|error| error.error)?;
    } else {
        output
            .persist_noclobber(target)
            .map_err(|error| error.error)?;
    }
    Ok(count)
}

pub(super) fn save_all(
    snapshot: &Snapshot,
    target: &Path,
    json: bool,
    replace: bool,
    stop: &AtomicBool,
) -> Result<usize> {
    save(snapshot, None, COLUMNS, target, json, replace, stop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::statistics::{self, Query, Sort};
    use rusqlite::{Connection, params};

    fn fixture(count: usize, long_keys: bool) -> Result<(tempfile::TempDir, Arc<Snapshot>)> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("statistics.sqlite3");
        let mut db = Connection::open(&path)?;
        statistics::schema(&db)?;
        db.execute(
            "INSERT INTO statistics_nodes VALUES('owned-node','1',1,?1)",
            [serde_json::to_string(&statistics::Totals::default())?],
        )?;
        let transaction = db.transaction()?;
        for index in 0..count {
            let key = if long_keys {
                format!("{}-{index:04}", "\"".repeat(3495))
            } else {
                format!("literal,%'\"value-{index:04}")
            };
            transaction.execute(
                "INSERT INTO statistics_counts VALUES('owned-node',0,?1,?2,'0',1,?3)",
                params![key, (index % 13 + 1).to_string(), key.len() + 192],
            )?;
        }
        transaction.commit()?;
        let snapshot = Arc::new(statistics::capture(
            &path,
            Query {
                sort: Sort::Name,
                ..Default::default()
            },
        )?);
        Ok((dir, snapshot))
    }

    #[test]
    fn cross_page_ranges_exclusions_and_columns_stay_on_the_captured_disk_snapshot() -> Result<()> {
        let (dir, snapshot) = fixture(650, false)?;
        for sort in [Sort::Most, Sort::Least, Sort::Name, Sort::ReverseName] {
            let snapshot = Arc::new(statistics::capture(
                &dir.path().join("statistics.sqlite3"),
                Query {
                    sort,
                    ..Default::default()
                },
            )?);
            let rows = (0..snapshot.len())
                .step_by(statistics::PAGE_SIZE)
                .map(|offset| snapshot.page(offset))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let mut selection = snapshot.selection();
            selection.toggle(10)?;
            selection.extend_to(430)?;
            selection.toggle(250)?;
            selection.toggle(600)?;
            assert_eq!(selection.count(), 421);
            let (text, count) = selection.copy(
                &[Column::Value, Column::Hits],
                true,
                &AtomicBool::new(false),
            )?;
            assert_eq!(count, 421);
            let values = text
                .lines()
                .map(serde_json::from_str::<serde_json::Value>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let expected = rows
                .iter()
                .enumerate()
                .filter(|(index, _)| ((10..=430).contains(index) && *index != 250) || *index == 600)
                .map(|(_, row)| row)
                .collect::<Vec<_>>();
            for (value, row) in values.iter().zip(expected) {
                assert_eq!(value.as_object().unwrap().len(), 2);
                assert_eq!(value["value"], row.key);
                assert_eq!(value["hits"], row.hits.to_string());
            }
            selection.extend_to(620)?;
            assert_eq!(selection.count(), 21); // Shift replaces the old range from the last ordinary mark,600.
            assert!(selection.contains(600));
            assert!(selection.contains(620));
            assert!(!selection.contains(430));
        }
        let mut selection = snapshot.selection();
        selection.select_all();
        selection.toggle(0)?;
        selection.toggle(649)?;
        let before = selection.copy(COLUMNS, true, &AtomicBool::new(false))?;
        let db = Connection::open(dir.path().join("statistics.sqlite3"))?;
        db.execute("DELETE FROM statistics_counts", [])?;
        assert_eq!(
            statistics::capture(&dir.path().join("statistics.sqlite3"), Query::default())?.len(),
            0
        );
        assert_eq!(
            selection.copy(COLUMNS, true, &AtomicBool::new(false))?,
            before
        );
        assert_eq!(before.1, 648);
        let target = dir.path().join("selected.csv");
        assert_eq!(
            selection.save(
                &[Column::Value, Column::Nodes],
                &target,
                false,
                false,
                &AtomicBool::new(false)
            )?,
            648
        );
        let text = std::fs::read_to_string(&target)?;
        assert!(text.starts_with("\"Value\",\"Nodes\"\r\n"));
        assert!(text.contains("literal,%'\"\"value"));
        Ok(())
    }

    #[test]
    fn mark_limit_refusal_is_atomic_and_range_select_all_do_not_expand_keys() -> Result<()> {
        let (_, snapshot) = fixture(3000, false)?;
        let mut selection = snapshot.selection();
        for index in 0..MAX_MARKS {
            selection.toggle(index)?;
        }
        assert!(selection.toggle(MAX_MARKS).is_err());
        assert_eq!(selection.count(), MAX_MARKS);
        assert!(!selection.contains(MAX_MARKS));
        assert!(selection.toggle(snapshot.len()).is_err());
        assert_eq!(selection.count(), MAX_MARKS);
        selection.select_all();
        assert_eq!(selection.positions.overrides.len(), 0);
        assert_eq!(selection.count(), 3000);
        selection.toggle(2999)?;
        assert_eq!(selection.count(), 2999);
        selection.clear();
        selection.toggle(0)?;
        selection.extend_to(2999)?;
        assert_eq!(selection.count(), 3000);
        assert_eq!(selection.positions.overrides.len(), 0);
        selection.extend_to(2)?;
        assert_eq!(selection.count(), 3);
        Ok(())
    }

    #[test]
    fn stopped_and_invalid_outputs_preserve_old_files_and_clipboard_has_its_own_cap() -> Result<()>
    {
        let (dir, snapshot) = fixture(599, true)?;
        let mut selection = snapshot.selection();
        selection.select_all();
        assert!(
            selection
                .copy(COLUMNS, false, &AtomicBool::new(false))
                .is_err()
        );
        let target = dir.path().join("selected.csv");
        assert_eq!(
            selection.save(COLUMNS, &target, false, false, &AtomicBool::new(false))?,
            599
        );
        let existing = std::fs::read(&target)?;
        assert!(existing.len() > CLIPBOARD_BYTES);
        assert!(
            selection
                .save(COLUMNS, &target, false, false, &AtomicBool::new(false))
                .is_err()
        );
        assert!(
            selection
                .save(COLUMNS, &target, true, true, &AtomicBool::new(true))
                .is_err()
        );
        assert!(
            selection
                .save(
                    &[Column::Value, Column::Value],
                    &target,
                    true,
                    true,
                    &AtomicBool::new(false)
                )
                .is_err()
        );
        assert!(
            selection
                .save(&[], &target, true, true, &AtomicBool::new(false))
                .is_err()
        );
        assert_eq!(std::fs::read(&target)?, existing);
        assert!(
            selection
                .save(
                    COLUMNS,
                    &dir.path().join("statistics.sqlite3"),
                    true,
                    true,
                    &AtomicBool::new(false)
                )
                .is_err()
        );
        Ok(())
    }
}
