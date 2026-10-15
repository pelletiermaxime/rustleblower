//! Bounded review of reported executable hashes against confirmed daemon rules.
use super::{MAX_RULES, document, memory_bytes, valid};
use crate::protocol::{Connection, Rule};
use anyhow::{Context as _, Result, ensure};
use prost::Message;
use rusqlite::{Connection as Database, OpenFlags, OptionalExtension, params};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const PAGE_RULES: usize = 16;
pub const PAGE_BYTES: usize = 512 * 1024;
const HASHES: [(&str, usize); 2] = [("process.hash.md5", 32), ("process.hash.sha1", 40)];

#[derive(Clone)]
pub struct Context {
    pub process: String,
    pub hashes: BTreeMap<String, String>,
}
impl Context {
    pub fn from_connection(connection: &Connection) -> Option<Self> {
        if connection.process_path.is_empty() || connection.process_path.len() > 4096 {
            return None;
        }
        let hashes: BTreeMap<_, _> = HASHES
            .into_iter()
            .filter_map(|(key, size)| {
                connection
                    .process_checksums
                    .get(key)
                    .filter(|hash| {
                        hash.len() == size && hash.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                    .map(|hash| (key.to_string(), hash.clone()))
            })
            .collect();
        (!hashes.is_empty()).then(|| Self {
            process: connection.process_path.clone(),
            hashes,
        })
    }
}

#[derive(Clone)]
pub struct Difference {
    pub operand: String,
    pub expected: String,
    pub reported: String,
}
#[derive(Clone)]
pub struct Change {
    pub rule: Rule,
    pub differences: Vec<Difference>,
}
#[derive(Default)]
pub struct Page {
    pub changes: Vec<Change>,
    pub after: String,
    pub has_more: bool,
}

/// One read/prepare worker across probes and the review window. UI callers use
/// try-acquire before spawning, so obsolete prompts cannot accumulate I/O tasks.
pub fn permit() -> Option<OwnedSemaphorePermit> {
    static SCANS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SCANS
        .get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
        .try_acquire_owned()
        .ok()
}

pub fn differences(rule: &Rule, context: &Context) -> Vec<Difference> {
    let Some(op) = rule.operator.as_ref() else {
        return Vec::new();
    };
    if !rule.enabled
        || op.r#type != "list"
        || !op.list.iter().any(|child| {
            child.r#type == "simple"
                && child.operand == "process.path"
                && if child.sensitive {
                    child.data == context.process
                } else {
                    child.data.eq_ignore_ascii_case(&context.process)
                }
        })
    {
        return Vec::new();
    }
    // The 1.7 protocol carries a flat AND group. Never interpret a nested group
    // or replace regex/list-file predicates as an exact checksum condition.
    if op
        .list
        .iter()
        .any(|child| !child.list.is_empty() || child.r#type == "list")
    {
        return Vec::new();
    }
    op.list
        .iter()
        .filter_map(|child| {
            let reported = context.hashes.get(&child.operand)?;
            let matches = if child.sensitive {
                child.data == *reported
            } else {
                child.data.eq_ignore_ascii_case(reported)
            };
            (child.r#type == "simple" && !matches).then(|| Difference {
                operand: child.operand.clone(),
                expected: child.data.clone(),
                reported: reported.clone(),
            })
        })
        .collect()
}

pub fn updated(rule: &Rule, context: &Context) -> Result<Rule> {
    ensure!(
        !differences(rule, context).is_empty(),
        "This rule no longer has a reported checksum mismatch"
    );
    let mut changed = rule.clone();
    let op = changed.operator.as_mut().unwrap();
    for child in &mut op.list {
        if child.r#type == "simple"
            && let Some(reported) = context.hashes.get(&child.operand)
        {
            child.data = reported.clone();
        }
    }
    op.data = serde_json::to_string(
        &op.list
            .iter()
            .map(document::operator_json)
            .collect::<Vec<_>>(),
    )?;
    document::validate_edit(&changed).map_err(anyhow::Error::msg)?;
    Ok(changed)
}

fn open(path: &Path) -> Result<Database> {
    let db = Database::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(250))?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-512; PRAGMA mmap_size=0;")?;
    Ok(db)
}

pub fn page(path: &Path, node: &str, context: &Context, after: &str) -> Result<Page> {
    let db = open(path)?;
    let mut statement = db.prepare("SELECT name,wire FROM application_rules WHERE node=?1 AND process=?2 COLLATE NOCASE AND name>?3 ORDER BY name LIMIT ?4")?;
    let records = statement.query_map(
        params![node, context.process, after, MAX_RULES as i64 + 1],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
    )?;
    let mut page = Page::default();
    let mut bytes = 0;
    for (index, record) in records.enumerate() {
        ensure!(
            index < MAX_RULES,
            "The rule catalogue exceeds its bounded capacity"
        );
        let (name, wire) = record?;
        let rule = Rule::decode(wire.as_slice())?;
        valid(&rule).map_err(anyhow::Error::msg)?;
        let differences = differences(&rule, context);
        if !differences.is_empty() {
            let retained = memory_bytes(&rule)
                + differences.capacity() * std::mem::size_of::<Difference>()
                + differences
                    .iter()
                    .map(|d| d.operand.capacity() + d.expected.capacity() + d.reported.capacity())
                    .sum::<usize>();
            if page.changes.len() == PAGE_RULES || bytes + retained > PAGE_BYTES {
                ensure!(
                    !page.changes.is_empty(),
                    "This checksum rule exceeds the review page's memory budget"
                );
                page.has_more = true;
                break;
            }
            bytes += retained;
            page.changes.push(Change { rule, differences });
        }
        page.after = name;
    }
    Ok(page)
}

pub fn prepare(path: &Path, node: &str, expected: &Rule, context: &Context) -> Result<Rule> {
    let db = open(path)?;
    let wire: Option<Vec<u8>> = db
        .query_row(
            "SELECT wire FROM application_rules WHERE node=? AND name=?",
            params![node, expected.name],
            |row| row.get(0),
        )
        .optional()?;
    let current = Rule::decode(
        wire.context("The rule was removed; reload this review")?
            .as_slice(),
    )?;
    ensure!(
        current == *expected,
        "The rule changed since review; reload before updating its checksum"
    );
    updated(&current, context)
}
