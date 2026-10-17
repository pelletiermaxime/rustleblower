//! Bounded daemon-reported counters, separate from retained connection events.
use crate::{history, protocol};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
    sync::{Arc, OnceLock, atomic::AtomicBool},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
pub const MAX_KEYS: usize = 10_000;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_NODES: usize = 32;
pub const PAGE_SIZE: usize = 200;

mod selection;
pub use selection::{COLUMNS, Column, Selection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Applications,
    Hosts,
    Addresses,
    Ports,
    Users,
    Protocols,
}
pub const KINDS: &[Kind] = &[
    Kind::Applications,
    Kind::Hosts,
    Kind::Addresses,
    Kind::Ports,
    Kind::Users,
    Kind::Protocols,
];
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Applications => "Applications",
            Self::Hosts => "Hosts",
            Self::Addresses => "Addresses",
            Self::Ports => "Ports",
            Self::Users => "Users",
            Self::Protocols => "Protocols",
        }
    }
    pub fn field(self) -> history::query::Field {
        use history::query::Field;
        match self {
            Self::Applications => Field::Process,
            Self::Hosts => Field::Host,
            Self::Addresses => Field::DestinationIp,
            Self::Ports => Field::Port,
            Self::Users => Field::User,
            Self::Protocols => Field::Protocol,
        }
    }
    fn limit(self) -> usize {
        match self {
            Self::Applications => 4096,
            Self::Hosts => 1024,
            Self::Addresses => 64,
            Self::Ports | Self::Users => 16,
            Self::Protocols => 32,
        }
    }
    fn code(self) -> i64 {
        self as i64
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Totals {
    pub version: String,
    pub uptime: u64,
    pub rules: u64,
    pub connections: u64,
    pub accepted: u64,
    pub dropped: u64,
    pub ignored: u64,
    pub dns: u64,
    pub rule_hits: u64,
    pub rule_misses: u64,
}
struct Entry {
    kind: Kind,
    key: String,
    count: u64,
}
pub(crate) struct Prepared {
    entries: Vec<Entry>,
    totals: Totals,
    _permit: OwnedSemaphorePermit,
}
pub(crate) fn prepare(mut stats: protocol::Statistics, budget: Arc<Semaphore>) -> Result<Prepared> {
    let maps = [
        (Kind::Applications, std::mem::take(&mut stats.by_executable)),
        (Kind::Hosts, std::mem::take(&mut stats.by_host)),
        (Kind::Addresses, std::mem::take(&mut stats.by_address)),
        (Kind::Ports, std::mem::take(&mut stats.by_port)),
        (Kind::Users, std::mem::take(&mut stats.by_uid)),
        (Kind::Protocols, std::mem::take(&mut stats.by_proto)),
    ];
    let count: usize = maps.iter().map(|(_, map)| map.len()).sum();
    ensure!(count <= MAX_KEYS, "Counter report exceeds 10,000 keys");
    let bytes = std::mem::size_of::<Prepared>()
        + count * std::mem::size_of::<Entry>()
        + maps
            .iter()
            .map(|(_, map)| {
                map.capacity() * (std::mem::size_of::<String>() + 8 + 1)
                    + map.keys().map(String::capacity).sum::<usize>()
            })
            .sum::<usize>();
    ensure!(
        bytes <= MAX_BYTES,
        "Counter report exceeds its 2 MiB allocation limit"
    );
    for (kind, map) in &maps {
        ensure!(
            map.keys().all(|key| key.len() <= kind.limit()),
            "Counter key exceeds its field limit"
        );
    }
    let permit = budget
        .try_acquire_many_owned(bytes as u32)
        .map_err(|_| anyhow::anyhow!("Statistics could not reserve the shared ingestion budget"))?;
    let totals = Totals {
        version: history::bounded(&stats.daemon_version, 64),
        uptime: stats.uptime,
        rules: stats.rules,
        connections: stats.connections,
        accepted: stats.accepted,
        dropped: stats.dropped,
        ignored: stats.ignored,
        dns: stats.dns_responses,
        rule_hits: stats.rule_hits,
        rule_misses: stats.rule_misses,
    };
    let mut entries = Vec::with_capacity(count);
    for (kind, map) in maps {
        for (key, count) in map {
            entries.push(Entry { kind, key, count });
        }
    }
    Ok(Prepared {
        entries,
        totals,
        _permit: permit,
    })
}
pub(crate) fn schema(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS statistics_nodes(node TEXT PRIMARY KEY,uptime TEXT NOT NULL,updated INTEGER NOT NULL,totals TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS statistics_counts(node TEXT NOT NULL REFERENCES statistics_nodes(node) ON DELETE CASCADE,kind INTEGER NOT NULL,key TEXT NOT NULL,count TEXT NOT NULL,baseline TEXT NOT NULL,updated INTEGER NOT NULL,bytes INTEGER NOT NULL,PRIMARY KEY(node,kind,key));
    CREATE INDEX IF NOT EXISTS statistics_updated ON statistics_counts(updated);")
}
pub(crate) fn storage_bytes(db: &Connection) -> rusqlite::Result<u64> {
    db.query_row("SELECT coalesce(sum(bytes),0)+(SELECT coalesce(sum(length(totals)+length(node)+128),0) FROM statistics_nodes) FROM statistics_counts",[],|row|row.get(0))
}
fn row_bytes(node: &str, key: &str) -> usize {
    (node.len() + key.len()) * 2 + 192
}
pub(crate) fn anticipated(db: &Connection, node: &str, sample: &Prepared) -> Result<u64> {
    let mut additional = 0usize;
    let mut exists = db.prepare_cached(
        "SELECT EXISTS(SELECT 1 FROM statistics_counts WHERE node=? AND kind=? AND key=?)",
    )?;
    for entry in &sample.entries {
        if !exists.query_row(params![node, entry.kind.code(), entry.key], |row| {
            row.get::<_, bool>(0)
        })? {
            additional += row_bytes(node, &entry.key);
        }
    }
    Ok((storage_bytes(db)? + additional as u64 + 1024)
        .min(MAX_BYTES as u64 + MAX_NODES as u64 * 1024))
}
pub(crate) fn apply(db: &mut Connection, node: &str, sample: Prepared, days: u32) -> Result<usize> {
    let tx = db.transaction()?;
    let now = history::now_nanos();
    let uptime = format!("{:020}", sample.totals.uptime);
    let previous: Option<String> = tx
        .query_row(
            "SELECT uptime FROM statistics_nodes WHERE node=?",
            [node],
            |row| row.get(0),
        )
        .optional()?;
    // A daemon restart begins a fresh counter lifetime, independent of history.
    if previous.as_ref().is_some_and(|old| old > &uptime) {
        tx.execute("DELETE FROM statistics_counts WHERE node=?", [node])?;
    }
    tx.execute("INSERT INTO statistics_nodes VALUES(?1,?2,?3,?4) ON CONFLICT(node) DO UPDATE SET uptime=excluded.uptime,updated=excluded.updated,totals=excluded.totals",params![node,uptime,now,serde_json::to_string(&sample.totals)?])?;
    {
        let mut put=tx.prepare_cached("INSERT INTO statistics_counts VALUES(?1,?2,?3,?4,'00000000000000000000',?5,?6) ON CONFLICT(node,kind,key) DO UPDATE SET baseline=CASE WHEN excluded.count<statistics_counts.count THEN '00000000000000000000' ELSE statistics_counts.baseline END,count=excluded.count,updated=excluded.updated,bytes=excluded.bytes")?;
        for entry in sample.entries {
            put.execute(params![
                node,
                entry.kind.code(),
                entry.key,
                format!("{:020}", entry.count),
                now,
                row_bytes(node, &entry.key)
            ])?;
        }
    }
    let mut evicted = tx.execute(
        "DELETE FROM statistics_counts WHERE updated<?",
        [now - i64::from(days) * 86400 * 1_000_000_000],
    )?;
    tx.execute(
        "DELETE FROM statistics_nodes WHERE updated<?",
        [now - i64::from(days) * 86400 * 1_000_000_000],
    )?;
    let before_nodes: usize =
        tx.query_row("SELECT count(*) FROM statistics_counts", [], |row| {
            row.get(0)
        })?;
    tx.execute("DELETE FROM statistics_nodes WHERE node IN (SELECT node FROM statistics_nodes ORDER BY updated DESC,node LIMIT -1 OFFSET ?)",[MAX_NODES])?;
    let (count, bytes): (usize, usize) = tx.query_row(
        "SELECT count(*),coalesce(sum(bytes),0) FROM statistics_counts",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    evicted += before_nodes.saturating_sub(count);
    if count > MAX_KEYS || bytes > MAX_BYTES {
        let needed:usize=tx.query_row("SELECT n FROM (SELECT row_number() OVER (ORDER BY updated,count,node,kind,key) n,sum(bytes) OVER (ORDER BY updated,count,node,kind,key ROWS UNBOUNDED PRECEDING) b FROM statistics_counts) WHERE n>=? AND b>=? ORDER BY n LIMIT 1",params![count.saturating_sub(MAX_KEYS),bytes.saturating_sub(MAX_BYTES)],|row|row.get(0))?;
        evicted+=tx.execute("DELETE FROM statistics_counts WHERE (node,kind,key) IN (SELECT node,kind,key FROM statistics_counts ORDER BY updated,count,node,kind,key LIMIT ?)",[needed])?;
    }
    tx.commit()?;
    Ok(evicted)
}
#[derive(Clone)]
pub struct Reset {
    pub kind: Kind,
    pub node: String,
    pub key: Option<String>,
    pub clear_history: bool,
}
#[derive(Debug)]
pub struct ResetResult {
    pub counters: usize,
    pub events: usize,
}
pub(crate) fn reset(db: &mut Connection, reset: &Reset) -> Result<ResetResult> {
    ensure!(
        reset.node.len() <= 128
            && reset
                .key
                .as_ref()
                .is_none_or(|key| key.len() <= reset.kind.limit()),
        "Invalid statistics reset scope"
    );
    ensure!(
        !reset.clear_history || reset.key.is_some(),
        "Removing history requires a selected value"
    );
    let tx = db.transaction()?;
    let counters=tx.execute("UPDATE statistics_counts SET baseline=count WHERE kind=?1 AND (?2='' OR node=?2) AND (?3 IS NULL OR key=?3)",params![reset.kind.code(),reset.node,reset.key])?;
    let events = if reset.clear_history {
        let field = reset.kind.field().sql();
        tx.execute(
            &format!("DELETE FROM events WHERE (?1='' OR node=?1) AND CAST({field} AS TEXT)=?2"),
            params![reset.node, reset.key],
        )?
    } else {
        0
    };
    tx.commit()?;
    Ok(ResetResult { counters, events })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Most,
    Least,
    Name,
    ReverseName,
}
pub const SORTS: &[(Sort, &str)] = &[
    (Sort::Most, "Most hits"),
    (Sort::Least, "Fewest hits"),
    (Sort::Name, "Name A–Z"),
    (Sort::ReverseName, "Name Z–A"),
];
#[derive(Clone)]
pub struct Query {
    pub kind: Kind,
    pub node: String,
    pub search: String,
    pub sort: Sort,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            kind: Kind::Applications,
            node: String::new(),
            search: String::new(),
            sort: Sort::Most,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Row {
    pub key: String,
    pub hits: u128,
    pub reported: u128,
    pub nodes: u32,
}
#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub updated: i64,
    pub totals: Totals,
}
pub struct Snapshot {
    file: tempfile::NamedTempFile,
    database: std::path::PathBuf,
    offsets: Vec<u64>,
    pub query: Query,
    pub nodes: Vec<Node>,
    pub top: Vec<Row>,
    pub total: u128,
    pub captured: i64,
}
impl Snapshot {
    pub fn len(&self) -> usize {
        self.offsets.len()
    }
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }
    pub fn page(&self, start: usize) -> Result<Vec<Row>> {
        let mut file = std::fs::File::open(self.file.path())?;
        let mut rows = Vec::new();
        for offset in self.offsets.iter().skip(start).take(PAGE_SIZE) {
            file.seek(SeekFrom::Start(*offset))?;
            rows.push(read_row(&mut file)?);
        }
        Ok(rows)
    }
    pub fn save(
        &self,
        target: &Path,
        json: bool,
        replace: bool,
        stop: &AtomicBool,
    ) -> Result<usize> {
        selection::save_all(self, target, json, replace, stop)
    }
}
pub fn capture_permit() -> Option<OwnedSemaphorePermit> {
    static READ: OnceLock<Arc<Semaphore>> = OnceLock::new();
    READ.get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
        .try_acquire_owned()
        .ok()
}
pub fn capture(path: &Path, query: Query) -> Result<Snapshot> {
    ensure!(
        query.search.len() <= 256 && query.node.len() <= 128,
        "Statistics filters exceed their limits"
    );
    let (mut data, nodes) = {
        let db = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(Duration::from_millis(250))?;
        db.execute_batch("PRAGMA cache_size=-512; PRAGMA mmap_size=0;")?;
        db.execute_batch("BEGIN")?;
        let mut statement=db.prepare("SELECT key,count,baseline FROM statistics_counts WHERE kind=?1 AND (?2='' OR node=?2) AND instr(lower(key),lower(?3))>0")?;
        let mut rows = statement.query(params![query.kind.code(), query.node, query.search])?;
        let mut data: BTreeMap<String, (u128, u128, u32)> = BTreeMap::new();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let key: String = row.get(0)?;
            let count = row.get::<_, String>(1)?.parse::<u64>()?;
            let baseline = row.get::<_, String>(2)?.parse::<u64>()?;
            if !data.contains_key(&key) {
                bytes += key.len();
                ensure!(
                    data.len() < MAX_KEYS && bytes <= MAX_BYTES,
                    "Statistics cache exceeds its bounded capacity"
                );
            }
            let value = data.entry(key).or_default();
            value.0 += u128::from(count.saturating_sub(baseline));
            value.1 += u128::from(count);
            value.2 += 1;
        }
        let mut statement =
            db.prepare("SELECT node,updated,totals FROM statistics_nodes ORDER BY node LIMIT 32")?;
        let nodes = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .map(|result| {
                let (name, updated, json) = result?;
                Ok(Node {
                    name,
                    updated,
                    totals: serde_json::from_str(&json)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        (
            data.into_iter()
                .map(|(key, (hits, reported, nodes))| Row {
                    key,
                    hits,
                    reported,
                    nodes,
                })
                .collect::<Vec<_>>(),
            nodes,
        )
    };
    let total = data.iter().map(|row| row.hits).sum();
    let mut top: Vec<Row> = Vec::new();
    for row in &data {
        if top.len() < 10
            || top.last().is_some_and(|last| {
                row.hits > last.hits || (row.hits == last.hits && row.key < last.key)
            })
        {
            top.push(row.clone());
            top.sort_by(|a, b| b.hits.cmp(&a.hits).then(a.key.cmp(&b.key)));
            top.truncate(10);
        }
    }
    match query.sort {
        Sort::Most => data.sort_by(|a, b| b.hits.cmp(&a.hits).then(a.key.cmp(&b.key))),
        Sort::Least => data.sort_by(|a, b| a.hits.cmp(&b.hits).then(a.key.cmp(&b.key))),
        Sort::Name => {}
        Sort::ReverseName => data.reverse(),
    }
    let mut file = tempfile::NamedTempFile::new()?;
    let mut offsets = Vec::with_capacity(data.len());
    let mut bytes = 0u64;
    {
        let mut writer = BufWriter::with_capacity(16 * 1024, file.as_file_mut());
        for row in data {
            offsets.push(bytes);
            let key = row.key.as_bytes();
            writer.write_all(&(key.len() as u32).to_le_bytes())?;
            writer.write_all(key)?;
            writer.write_all(&row.hits.to_le_bytes())?;
            writer.write_all(&row.reported.to_le_bytes())?;
            writer.write_all(&row.nodes.to_le_bytes())?;
            bytes += 4 + key.len() as u64 + 16 + 16 + 4;
            ensure!(
                bytes <= 4 * 1024 * 1024,
                "Statistics view exceeds its 4 MiB disk snapshot limit"
            );
        }
        writer.flush()?;
    }
    Ok(Snapshot {
        file,
        database: path.to_owned(),
        offsets,
        query,
        nodes,
        top,
        total,
        captured: history::now_seconds(),
    })
}
fn read_row(file: &mut std::fs::File) -> Result<Row> {
    let mut length = [0; 4];
    file.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    ensure!(length <= 4096, "Invalid counter key length");
    let mut key = vec![0; length];
    file.read_exact(&mut key)?;
    let mut hits = [0; 16];
    let mut reported = [0; 16];
    let mut nodes = [0; 4];
    file.read_exact(&mut hits)?;
    file.read_exact(&mut reported)?;
    file.read_exact(&mut nodes)?;
    Ok(Row {
        key: String::from_utf8(key)?,
        hits: u128::from_le_bytes(hits),
        reported: u128::from_le_bytes(reported),
        nodes: u32::from_le_bytes(nodes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(keys: impl IntoIterator<Item = (String, u64)>, uptime: u64) -> Prepared {
        prepare(
            protocol::Statistics {
                uptime,
                by_executable: keys.into_iter().collect(),
                ..Default::default()
            },
            Arc::new(Semaphore::new(8 * 1024 * 1024)),
        )
        .unwrap()
    }
    #[test]
    fn cache_bounds_evict_old_keys_nodes_and_expired_reports() -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        db.execute_batch("PRAGMA foreign_keys=ON")?;
        schema(&db)?;
        apply(
            &mut db,
            "a",
            sample((0..6000).map(|i| (format!("a{i}"), 1)), 1),
            7,
        )?;
        assert!(
            apply(
                &mut db,
                "b",
                sample((0..6000).map(|i| (format!("b{i}"), 2)), 1),
                7
            )? >= 2000
        );
        let (count, bytes): (usize, usize) = db.query_row(
            "SELECT count(*),sum(bytes) FROM statistics_counts",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert!(count <= MAX_KEYS && bytes <= MAX_BYTES);
        let oldest: usize = db.query_row(
            "SELECT count(*) FROM statistics_counts WHERE node='a'",
            [],
            |row| row.get(0),
        )?;
        assert!(oldest < 6000);
        apply(
            &mut db,
            "long",
            sample((0..300).map(|i| (format!("{:0>4000}", i), 3)), 1),
            7,
        )?;
        assert!(storage_bytes(&db)? <= MAX_BYTES as u64 + 32 * 1024);
        for i in 0..34 {
            apply(
                &mut db,
                &format!("node-{i}"),
                sample([("/bin/test".into(), 1)], 1),
                7,
            )?;
        }
        assert_eq!(
            db.query_row("SELECT count(*) FROM statistics_nodes", [], |row| row
                .get::<_, usize>(0))?,
            MAX_NODES
        );
        db.execute("UPDATE statistics_counts SET updated=0", [])?;
        db.execute("UPDATE statistics_nodes SET updated=0", [])?;
        apply(&mut db, "fresh", sample([("/bin/fresh".into(), 1)], 1), 7)?;
        assert_eq!(
            db.query_row("SELECT count(*) FROM statistics_counts", [], |row| row
                .get::<_, usize>(0))?,
            1
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM statistics_nodes", [], |row| row
                .get::<_, usize>(0))?,
            1
        );
        Ok(())
    }
    #[test]
    fn counter_reports_reserve_the_shared_budget_and_reject_oversized_allocations() {
        assert!(
            prepare(
                protocol::Statistics {
                    by_executable: (0..10001).map(|i| (i.to_string(), 1)).collect(),
                    ..Default::default()
                },
                Arc::new(Semaphore::new(8 * 1024 * 1024))
            )
            .is_err()
        );
        let budget = Arc::new(Semaphore::new(8 * 1024 * 1024));
        let start = budget.available_permits();
        let frame = prepare(
            protocol::Statistics {
                by_executable: [("/bin/a".into(), 1)].into(),
                ..Default::default()
            },
            budget.clone(),
        )
        .unwrap();
        assert!(budget.available_permits() < start);
        drop(frame);
        assert_eq!(budget.available_permits(), start);
        assert!(prepare(protocol::Statistics::default(), Arc::new(Semaphore::new(0))).is_err());
        let mut oversized = std::collections::HashMap::with_capacity(100000);
        oversized.insert("/bin/a".into(), 1);
        assert!(
            prepare(
                protocol::Statistics {
                    by_executable: oversized,
                    ..Default::default()
                },
                budget
            )
            .is_err()
        );
    }
}
