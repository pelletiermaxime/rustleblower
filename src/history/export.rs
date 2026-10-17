//! Bounded streaming export from a private SQLite backup, never the live writer.
use super::{Query, Row, query::Field};
use anyhow::{Result, ensure};
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use std::{
    io::{BufWriter, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
pub const MAX_BYTES: usize = 256 * 1024 * 1024;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    JsonLines,
}
pub struct Report {
    pub rows: usize,
    pub bytes: usize,
}

pub(crate) fn csv_line(values: impl IntoIterator<Item = String>) -> String {
    let mut line = String::new();
    for (index, value) in values.into_iter().enumerate() {
        if index > 0 {
            line.push(',');
        }
        line.push('"');
        line.push_str(&value.replace('"', "\"\""));
        line.push('"');
    }
    line.push_str("\r\n");
    line
}
pub fn page_csv(rows: &[Row], columns: &[Field]) -> Result<String> {
    ensure!(
        !columns.is_empty() && columns.len() <= super::query::FIELDS.len(),
        "Choose history columns first"
    );
    let mut text = csv_line(columns.iter().map(|field| field.label().to_string()));
    for row in rows {
        let line = csv_line(columns.iter().map(|field| field.text(row)));
        ensure!(
            text.len() + line.len() <= 4 * 1024 * 1024,
            "Page clipboard export exceeds 4 MiB; choose fewer columns"
        );
        text.push_str(&line);
    }
    Ok(text)
}

pub(crate) fn validate_target(database: &Path, target: &Path, replace: bool) -> Result<()> {
    ensure!(target.is_absolute(), "Choose an absolute export path");
    ensure!(
        target.file_name().is_some() && !target.is_dir(),
        "Choose a file path for export"
    );
    ensure!(
        !target.exists() || target.is_file(),
        "Export destination must be a regular file"
    );
    let normalized = |path: &Path| -> std::io::Result<std::path::PathBuf> {
        Ok(path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()?
            .join(path.file_name().unwrap_or_default()))
    };
    let target_path = normalized(target)?;
    let target_resolved = target.canonicalize().ok();
    for base in [normalized(database)?, database.canonicalize()?] {
        for suffix in [
            "",
            "-wal",
            "-shm",
            ".history.json",
            ".preferences.json",
            ".appearance.json",
            ".window.json",
            ".updates.json",
            ".reputation.json",
            ".raw-downloads.json",
            ".highlighting.json",
            ".catalogue-policy.json",
            ".live-view.json",
            ".views.json",
            ".raw-downloads-journal.json",
        ] {
            let protected = std::path::PathBuf::from(format!("{}{suffix}", base.display()));
            ensure!(
                target_path != protected
                    && !(target_resolved.is_some()
                        && target_resolved == protected.canonicalize().ok()),
                "Export path is used by Rustleblower; choose a separate file"
            );
        }
    }
    ensure!(
        replace || !target.exists(),
        "Export destination already exists; choose another file or explicitly allow replacement"
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum Scope<'a> {
    All(&'a Query),
    Selected(&'a super::selection::Selection),
}
impl Scope<'_> {
    fn sql(self, destination: &Connection) -> Result<(String, Vec<rusqlite::types::Value>)> {
        match self {
            Self::Selected(selection) => selection.sql(),
            Self::All(query) => {
                let ceiling =
                    destination.query_row("SELECT coalesce(max(id),0) FROM events", [], |row| {
                        row.get(0)
                    })?;
                let mut query = query.clone();
                query.before = None;
                query.cursor = None;
                query.ceiling = Some(ceiling);
                let (sql, values) = super::query::sql(&query, ceiling, 0)?;
                Ok((
                    sql.strip_suffix(" LIMIT 0")
                        .expect("fixed history query limit")
                        .to_owned(),
                    values,
                ))
            }
        }
    }
    fn required(self) -> std::collections::BTreeSet<i64> {
        match self {
            Self::All(_) => Default::default(),
            Self::Selected(selection) => selection.required_ids(),
        }
    }
}

pub(super) fn snapshot(
    database: &Path,
    stop: &AtomicBool,
) -> Result<(tempfile::NamedTempFile, Connection)> {
    // Each backup step releases the live reader. Slow output encoding/writes then
    // hold no live history transaction or WAL checkpoint open.
    ensure!(
        database.metadata()?.len() <= 4 * 1024 * 1024 * 1024,
        "History database exceeds the 4 GiB snapshot source limit"
    );
    let snapshot = tempfile::NamedTempFile::new()?;
    let source = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    source.busy_timeout(Duration::from_millis(250))?;
    let mut destination = Connection::open(snapshot.path())?;
    source.execute_batch("PRAGMA cache_size=-512; PRAGMA mmap_size=0;")?;
    destination.execute_batch("PRAGMA cache_size=-512; PRAGMA mmap_size=0;")?;
    let started = Instant::now();
    {
        let backup = Backup::new(&source, &mut destination)?;
        loop {
            ensure!(
                !stop.load(Ordering::Relaxed),
                "Export stopped; destination was not replaced"
            );
            ensure!(
                started.elapsed() < Duration::from_secs(15),
                "History changed too quickly to snapshot within 15 seconds; try a quieter moment"
            );
            match backup.step(128)? {
                StepResult::Done => break,
                StepResult::More => {}
                StepResult::Busy | StepResult::Locked => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => anyhow::bail!("Unexpected SQLite backup state"),
            }
        }
    }
    drop(source);
    let page_size: u64 = destination.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    ensure!(
        (512..=65536).contains(&page_size) && page_size.is_power_of_two(),
        "Unsupported history snapshot page size"
    );
    destination.pragma_update(
        None,
        "max_page_count",
        8 * 1024 * 1024 * 1024u64 / page_size,
    )?;
    let mut after = 0;
    while super::backfill_chunk(&mut destination, &mut after)? {
        ensure!(
            !stop.load(Ordering::Relaxed),
            "Export stopped; destination was not replaced"
        );
    }
    destination.execute_batch(
        "PRAGMA query_only=ON; PRAGMA cache_size=-512; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;",
    )?;
    Ok((snapshot, destination))
}

pub fn save(
    database: &Path,
    query: &Query,
    columns: &[Field],
    format: Format,
    target: &Path,
    replace: bool,
    stop: &AtomicBool,
) -> Result<Report> {
    save_scope(
        database,
        Scope::All(query),
        columns,
        format,
        target,
        replace,
        stop,
    )
}

pub fn save_selection(
    database: &Path,
    selection: &super::selection::Selection,
    columns: &[Field],
    format: Format,
    target: &Path,
    replace: bool,
    stop: &AtomicBool,
) -> Result<Report> {
    ensure!(
        !selection.is_empty(),
        "Select history events before exporting"
    );
    save_scope(
        database,
        Scope::Selected(selection),
        columns,
        format,
        target,
        replace,
        stop,
    )
}
fn save_scope(
    database: &Path,
    scope: Scope<'_>,
    columns: &[Field],
    format: Format,
    target: &Path,
    replace: bool,
    stop: &AtomicBool,
) -> Result<Report> {
    validate_target(database, target, replace)?;
    ensure!(
        !columns.is_empty() && columns.len() <= super::query::FIELDS.len(),
        "Choose history columns first"
    );
    let (_snapshot, destination) = snapshot(database, stop)?;
    let (sql, values) = scope.sql(&destination)?;
    let mut required = scope.required();
    let mut statement = destination.prepare(&sql)?;
    let mut records = statement.query(rusqlite::params_from_iter(values))?;
    let parent = target.parent().unwrap();
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    let mut report = Report { rows: 0, bytes: 0 };
    {
        let mut writer = BufWriter::with_capacity(16 * 1024, &mut output);
        if matches!(format, Format::Csv) {
            let header = csv_line(columns.iter().map(|field| field.label().to_string()));
            writer.write_all(header.as_bytes())?;
            report.bytes += header.len();
        }
        while let Some(record) = records.next()? {
            ensure!(
                !stop.load(Ordering::Relaxed),
                "Export stopped; destination was not replaced"
            );
            let row = super::read_row(record)?;
            required.remove(&row.id);
            let line = encode(&row, columns, format)?;
            ensure!(
                report.bytes + line.len() <= MAX_BYTES,
                "Export exceeds 256 MiB; narrow the history filters and try again"
            );
            writer.write_all(line.as_bytes())?;
            report.bytes += line.len();
            report.rows += 1;
        }
        writer.flush()?;
    }
    ensure!(
        !stop.load(Ordering::Relaxed),
        "Export stopped; destination was not replaced"
    );
    ensure!(
        required.is_empty(),
        "Selected history events are no longer retained; select them again"
    );
    output.as_file().sync_all()?;
    if replace {
        output.persist(target).map_err(|error| error.error)?;
    } else {
        output
            .persist_noclobber(target)
            .map_err(|error| error.error)?;
    }
    Ok(report)
}

/// Copy every selected retained row, not merely the visible page. The caller
/// runs this on its background executor and receives at most4MiB plus one row.
pub fn copy_selection(
    database: &Path,
    selection: &super::selection::Selection,
    columns: &[Field],
    format: Format,
    stop: &AtomicBool,
) -> Result<(String, Report)> {
    ensure!(
        !selection.is_empty(),
        "Select history events before copying"
    );
    ensure!(
        !columns.is_empty() && columns.len() <= super::query::FIELDS.len(),
        "Choose history columns first"
    );
    let (_snapshot, destination) = snapshot(database, stop)?;
    let (sql, values) = selection.sql()?;
    let mut statement = destination.prepare(&sql)?;
    let mut rows = statement.query(rusqlite::params_from_iter(values))?;
    let mut required = selection.required_ids();
    let mut text = if format == Format::Csv {
        csv_line(columns.iter().map(|field| field.label().to_owned()))
    } else {
        String::new()
    };
    let mut report = Report { rows: 0, bytes: 0 };
    while let Some(record) = rows.next()? {
        ensure!(!stop.load(Ordering::Relaxed), "History copy stopped");
        let row = super::read_row(record)?;
        required.remove(&row.id);
        let line = encode(&row, columns, format)?;
        ensure!(
            text.len() + line.len() <= 4 * 1024 * 1024,
            "Selected history exceeds 4 MiB; export to a file or select fewer events"
        );
        text.push_str(&line);
        report.rows += 1;
    }
    ensure!(
        required.is_empty(),
        "Selected history events are no longer retained; select them again"
    );
    ensure!(!stop.load(Ordering::Relaxed), "History copy stopped");
    report.bytes = text.len();
    Ok((text, report))
}
fn encode(row: &Row, columns: &[Field], format: Format) -> Result<String> {
    match format {
        Format::Csv => Ok(csv_line(columns.iter().map(|field| field.text(row)))),
        Format::JsonLines => {
            let object: serde_json::Map<_, _> = columns
                .iter()
                .map(|field| (field.key(), serde_json::Value::String(field.text(row))))
                .collect();
            Ok(format!("{}\n", serde_json::to_string(&object)?))
        }
    }
}
