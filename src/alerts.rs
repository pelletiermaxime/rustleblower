//! Bounded daemon-alert snapshots, distinct from connection history and policy.
use crate::{
    history,
    protocol::{self, alert::Data},
};
use anyhow::{Result, ensure};
use prost::Message;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

pub const MAX_WIRE: usize = 64 * 1024;
pub const MAX_RETAINED: usize = 256 * 1024;
pub const MAX_DOCUMENT: usize = 512 * 1024;
pub const MAX_STORAGE: u64 = 2 * 1024 * 1024;
pub const MAX_ROWS: usize = 10_000;
pub const PAGE_SIZE: usize = 200;
pub(crate) const PREPARE_BYTES: usize = 4 * 1024 * 1024;

pub(crate) struct Prepared {
    pub wire: Vec<u8>,
    pub document: String,
    pub summary: String,
    pub kind: String,
    pub severity: i32,
    pub priority: i32,
    pub cause: i32,
    pub action: i32,
    pub source_id: String,
}
impl Prepared {
    pub fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.wire.capacity()
            + self.document.capacity()
            + self.summary.capacity()
            + self.kind.capacity()
            + self.source_id.capacity()
            + 256
    }
    pub fn storage_bytes(&self, node: &str) -> usize {
        192 + node.len()
            + self.wire.len()
            + self.document.len()
            + self.summary.len()
            + self.kind.len()
            + self.source_id.len()
    }
}

fn map_bytes(map: &std::collections::HashMap<String, String>) -> usize {
    map.capacity()
        .saturating_mul(2 * std::mem::size_of::<String>() + 16)
        .saturating_add(
            map.iter()
                .map(|(k, v)| k.capacity() + v.capacity())
                .sum::<usize>(),
        )
}
fn strings(values: &[String], capacity: usize) -> usize {
    capacity.saturating_mul(std::mem::size_of::<String>())
        + values.iter().map(String::capacity).sum::<usize>()
}
fn tree(values: &Vec<protocol::StringInt>) -> usize {
    values.capacity() * std::mem::size_of::<protocol::StringInt>()
        + values.iter().map(|p| p.key.capacity()).sum::<usize>()
}
fn retained(alert: &protocol::Alert) -> usize {
    std::mem::size_of::<protocol::Alert>()
        + match &alert.data {
            None => 0,
            Some(Data::Text(v)) => v.capacity(),
            Some(Data::Conn(c)) => {
                c.protocol.capacity()
                    + c.src_ip.capacity()
                    + c.dst_ip.capacity()
                    + c.dst_host.capacity()
                    + c.process_path.capacity()
                    + c.process_cwd.capacity()
                    + strings(&c.process_args, c.process_args.capacity())
                    + map_bytes(&c.process_env)
                    + map_bytes(&c.process_checksums)
                    + tree(&c.process_tree)
            }
            Some(Data::Proc(p)) => {
                p.comm.capacity()
                    + p.path.capacity()
                    + p.cwd.capacity()
                    + strings(&p.args, p.args.capacity())
                    + map_bytes(&p.env)
                    + map_bytes(&p.checksums)
                    + tree(&p.process_tree)
            }
            Some(Data::Rule(r)) => crate::rules::memory_bytes(r),
            Some(Data::Fwrule(r)) => {
                r.table.capacity()
                    + r.chain.capacity()
                    + r.uuid.capacity()
                    + r.description.capacity()
                    + r.parameters.capacity()
                    + r.target.capacity()
                    + r.target_parameters.capacity()
                    + r.expressions.capacity() * std::mem::size_of::<protocol::Expressions>()
                    + r.expressions
                        .iter()
                        .filter_map(|e| e.statement.as_ref())
                        .map(|s| {
                            std::mem::size_of::<protocol::Statement>()
                                + s.op.capacity()
                                + s.name.capacity()
                                + s.values.capacity()
                                    * std::mem::size_of::<protocol::StatementValues>()
                                + s.values
                                    .iter()
                                    .map(|v| v.key.capacity() + v.value.capacity())
                                    .sum::<usize>()
                        })
                        .sum::<usize>()
            }
        }
}
fn operator(op: &protocol::Operator, depth: usize) -> Result<Value> {
    ensure!(depth <= 16, "Alert rule nesting exceeds 16 levels");
    let mut value = serde_json::Map::new();
    for (key, text) in [
        ("type", &op.r#type),
        ("operand", &op.operand),
        ("data", &op.data),
    ] {
        value.insert(key.into(), Value::String(text.clone()));
    }
    value.insert("sensitive".into(), Value::Bool(op.sensitive));
    value.insert(
        "list".into(),
        Value::Array(
            op.list
                .iter()
                .map(|child| operator(child, depth + 1))
                .collect::<Result<Vec<_>>>()?,
        ),
    );
    Ok(Value::Object(value))
}

fn payload(alert: &protocol::Alert) -> Result<(&'static str, String, Value)> {
    Ok(match &alert.data {
        None => ("none", "No payload reported".into(), Value::Null),
        Some(Data::Text(t)) => ("text", history::bounded(t, 512), json!({"text":t})),
        Some(Data::Conn(c)) => (
            "connection",
            format!(
                "{} → {}",
                c.process_path,
                history::details::endpoint(
                    if c.dst_host.is_empty() {
                        &c.dst_ip
                    } else {
                        &c.dst_host
                    },
                    c.dst_port
                )
            ),
            json!({"protocol":c.protocol,"src_ip":c.src_ip,"src_port":c.src_port,"dst_ip":c.dst_ip,"dst_host":c.dst_host,"dst_port":c.dst_port,"user_id":c.user_id,"process_id":c.process_id,"process_path":c.process_path,"process_cwd":c.process_cwd,"process_args":c.process_args,"process_env":c.process_env,"process_checksums":c.process_checksums,"process_tree":c.process_tree.iter().map(|p|json!({"key":p.key,"value":p.value})).collect::<Vec<_>>()}),
        ),
        Some(Data::Proc(p)) => (
            "process",
            format!("{} · PID {}", p.path, p.pid),
            json!({"pid":p.pid,"ppid":p.ppid,"uid":p.uid,"comm":p.comm,"path":p.path,"args":p.args,"env":p.env,"cwd":p.cwd,"checksums":p.checksums,"io_reads":p.io_reads,"io_writes":p.io_writes,"net_reads":p.net_reads,"net_writes":p.net_writes,"process_tree":p.process_tree.iter().map(|p|json!({"key":p.key,"value":p.value})).collect::<Vec<_>>()}),
        ),
        Some(Data::Rule(r)) => (
            "rule",
            format!("Rule {} · {}", r.name, r.action),
            json!({"created":r.created,"name":r.name,"description":r.description,"enabled":r.enabled,"precedence":r.precedence,"nolog":r.nolog,"action":r.action,"duration":r.duration,"operator":r.operator.as_ref().map(|op|operator(op,0)).transpose()?}),
        ),
        Some(Data::Fwrule(r)) => (
            "firewall-rule",
            format!("Firewall {} · {}", r.chain, r.description),
            json!({"table":r.table,"chain":r.chain,"uuid":r.uuid,"enabled":r.enabled,"position":r.position,"description":r.description,"parameters":r.parameters,"target":r.target,"target_parameters":r.target_parameters,"expressions":r.expressions.iter().map(|e|json!({"statement":e.statement.as_ref().map(|s|json!({"op":s.op,"name":s.name,"values":s.values.iter().map(|v|json!({"key":v.key,"value":v.value})).collect::<Vec<_>>()}))})).collect::<Vec<_>>()}),
        ),
    })
}
pub(crate) fn summary(alert: &protocol::Alert) -> String {
    let text = match &alert.data {
        Some(Data::Text(text)) => history::bounded(text, 512),
        Some(Data::Conn(c)) => format!(
            "{} → {}",
            history::bounded(&c.process_path, 240),
            history::bounded(
                if c.dst_host.is_empty() {
                    &c.dst_ip
                } else {
                    &c.dst_host
                },
                240
            )
        ),
        Some(Data::Proc(p)) => format!("{} · PID {}", history::bounded(&p.path, 400), p.pid),
        Some(Data::Rule(r)) => format!(
            "Rule {} · {}",
            history::bounded(&r.name, 400),
            history::bounded(&r.action, 32)
        ),
        Some(Data::Fwrule(r)) => format!(
            "Firewall {} · {}",
            history::bounded(&r.chain, 100),
            history::bounded(&r.description, 360)
        ),
        None => "No payload reported".into(),
    };
    let text = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>();
    if text.trim().is_empty() {
        "Empty alert".into()
    } else {
        text
    }
}
pub(crate) fn prepare(alert: protocol::Alert) -> Result<Prepared> {
    ensure!(
        alert.encoded_len() <= MAX_WIRE,
        "Alert exceeds 64 KiB encoded"
    );
    ensure!(
        retained(&alert) <= MAX_RETAINED,
        "Alert exceeds 256 KiB retained allocations"
    );
    let (kind, _, data) = payload(&alert)?;
    let summary = summary(&alert);
    let mut envelope = serde_json::Map::new();
    envelope.insert("source_id".into(), Value::String(alert.id.to_string()));
    for (key, number) in [
        ("severity", alert.r#type),
        ("priority", alert.priority),
        ("cause", alert.what),
        ("action", alert.action),
    ] {
        envelope.insert(key.into(), Value::from(number));
    }
    envelope.insert("kind".into(), Value::String(kind.into()));
    envelope.insert("payload".into(), data);
    let document = serde_json::to_string(&Value::Object(envelope))?;
    ensure!(document.len() <= MAX_DOCUMENT, "Alert JSON exceeds 512 KiB");
    Ok(Prepared {
        wire: alert.encode_to_vec(),
        document,
        summary: history::bounded(&summary, 512),
        kind: kind.into(),
        severity: alert.r#type,
        priority: alert.priority,
        cause: alert.what,
        action: alert.action,
        source_id: alert.id.to_string(),
    })
}
pub fn severity(value: i32) -> String {
    match value {
        0 => "Error".into(),
        1 => "Warning".into(),
        2 => "Info".into(),
        v => format!("Unknown ({v})"),
    }
}
pub fn priority(value: i32) -> String {
    match value {
        0 => "Low".into(),
        1 => "Normal".into(),
        2 => "High".into(),
        v => format!("Unknown ({v})"),
    }
}

pub(crate) fn schema(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS alerts(id INTEGER PRIMARY KEY AUTOINCREMENT,received INTEGER NOT NULL,node TEXT NOT NULL,source_id TEXT NOT NULL,severity INTEGER NOT NULL,priority INTEGER NOT NULL,cause INTEGER NOT NULL,action INTEGER NOT NULL,kind TEXT NOT NULL,summary TEXT NOT NULL,document TEXT NOT NULL,payload BLOB NOT NULL,read INTEGER NOT NULL DEFAULT 0,bytes INTEGER NOT NULL);CREATE INDEX IF NOT EXISTS alerts_received ON alerts(received,id);CREATE INDEX IF NOT EXISTS alerts_node ON alerts(node,id);")
}
pub(crate) fn storage_bytes(db: &Connection) -> Result<u64> {
    Ok(
        db.query_row("SELECT coalesce(sum(bytes),0) FROM alerts", [], |r| {
            r.get(0)
        })?,
    )
}
pub(crate) fn prune(db: &Connection, days: u32) -> Result<usize> {
    Ok(db.execute(
        "DELETE FROM alerts WHERE received<?",
        [history::now_seconds() - i64::from(days) * 86400],
    )?)
}
pub(crate) fn insert(
    db: &mut Connection,
    node: &str,
    record: Prepared,
    days: u32,
) -> Result<usize> {
    let tx = db.transaction()?;
    let mut evicted = prune(&tx, days)?;
    let bytes = record.storage_bytes(node) as u64;
    ensure!(bytes <= MAX_STORAGE, "Alert exceeds archive budget");
    let (mut count, mut used): (usize, u64) = tx.query_row(
        "SELECT count(*),coalesce(sum(bytes),0) FROM alerts",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    while count >= MAX_ROWS || used + bytes > MAX_STORAGE {
        let oldest: (i64, u64) =
            tx.query_row("SELECT id,bytes FROM alerts ORDER BY id LIMIT 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
        tx.execute("DELETE FROM alerts WHERE id=?", [oldest.0])?;
        count -= 1;
        used = used.saturating_sub(oldest.1);
        evicted += 1;
    }
    tx.execute("INSERT INTO alerts(received,node,source_id,severity,priority,cause,action,kind,summary,document,payload,bytes) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![history::now_seconds(),node,record.source_id,record.severity,record.priority,record.cause,record.action,record.kind,record.summary,record.document,record.wire,bytes])?;
    tx.commit()?;
    Ok(evicted)
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Query {
    pub search: String,
    pub node: String,
    pub severity: Option<i32>,
    pub unread: bool,
    pub before: Option<i64>,
    pub ceiling: Option<i64>,
}
impl Query {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.search.len() <= 256 && self.node.len() <= 128,
            "Alert filters exceed their text bounds"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub id: i64,
    pub received: i64,
    pub node: String,
    pub source_id: String,
    pub severity: i32,
    pub priority: i32,
    pub cause: i32,
    pub action: i32,
    pub kind: String,
    pub summary: String,
    pub read: bool,
}
#[derive(Default)]
pub struct Page {
    pub rows: Vec<Row>,
    pub has_more: bool,
    pub ceiling: i64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Detail {
    pub row: Row,
    pub document: String,
}
fn open(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(250))?;
    db.execute_batch("PRAGMA cache_size=-512;PRAGMA mmap_size=0;")?;
    Ok(db)
}
const COLUMNS: &str = "id,received,node,source_id,severity,priority,cause,action,kind,summary,read";
const WHERE: &str = "id<=?1 AND (?2 IS NULL OR id<?2) AND (?3='' OR node=?3) AND (?4 IS NULL OR severity=?4) AND (?5=0 OR read=0) AND (instr(lower(summary),lower(?6))>0 OR instr(lower(node),lower(?6))>0 OR instr(lower(document),lower(?6))>0)";
fn row(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        received: r.get(1)?,
        node: r.get(2)?,
        source_id: r.get(3)?,
        severity: r.get(4)?,
        priority: r.get(5)?,
        cause: r.get(6)?,
        action: r.get(7)?,
        kind: r.get(8)?,
        summary: r.get(9)?,
        read: r.get(10)?,
    })
}
fn query(db: &Connection, q: &Query) -> Result<Page> {
    q.validate()?;
    let ceiling =
        q.ceiling
            .unwrap_or(db.query_row("SELECT coalesce(max(id),0) FROM alerts", [], |r| r.get(0))?);
    let mut s = db.prepare(&format!(
        "SELECT {COLUMNS} FROM alerts WHERE {WHERE} ORDER BY id DESC LIMIT 201"
    ))?;
    let mut rows = s
        .query_map(
            params![ceiling, q.before, q.node, q.severity, q.unread, q.search],
            row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = rows.len() > PAGE_SIZE;
    rows.truncate(PAGE_SIZE);
    Ok(Page {
        rows,
        has_more,
        ceiling,
    })
}
pub fn page(path: &Path, q: &Query) -> Result<Page> {
    query(&open(path)?, q)
}
pub fn detail(path: &Path, id: i64) -> Result<Option<Detail>> {
    let db = open(path)?;
    Ok(db
        .query_row(
            &format!("SELECT {COLUMNS},document FROM alerts WHERE id=?"),
            [id],
            |r| {
                Ok(Detail {
                    row: row(r)?,
                    document: r.get(11)?,
                })
            },
        )
        .optional()?)
}
#[derive(Clone)]
pub enum Change {
    Read(i64),
    Delete(i64),
    Clear(Query),
}
pub(crate) fn change(db: &Connection, change: Change) -> Result<usize> {
    Ok(match change {
        Change::Read(id) => db.execute("UPDATE alerts SET read=1 WHERE id=?", [id])?,
        Change::Delete(id) => db.execute("DELETE FROM alerts WHERE id=?", [id])?,
        Change::Clear(q) => {
            q.validate()?;
            ensure!(
                q.ceiling.is_some(),
                "Capture an alert view before clearing it"
            );
            db.execute(
                &format!("DELETE FROM alerts WHERE {WHERE}"),
                params![q.ceiling, q.before, q.node, q.severity, q.unread, q.search],
            )?
        }
    })
}

pub const MAX_EXPORT: usize = 16 * 1024 * 1024;
pub fn clipboard(rows: &[Row]) -> Result<String> {
    let mut result = history::export::csv_line(
        [
            "Received UTC",
            "Node",
            "Severity",
            "Priority",
            "Cause",
            "Payload",
            "Summary",
            "Read",
        ]
        .map(str::to_string),
    );
    for r in rows {
        let line = history::export::csv_line([
            timestamp(r.received),
            r.node.clone(),
            severity(r.severity),
            priority(r.priority),
            r.cause.to_string(),
            r.kind.clone(),
            r.summary.clone(),
            r.read.to_string(),
        ]);
        ensure!(
            result.len() + line.len() <= 1024 * 1024,
            "Alert page exceeds clipboard limit"
        );
        result.push_str(&line);
    }
    Ok(result)
}
pub fn timestamp(value: i64) -> String {
    chrono::DateTime::from_timestamp(value, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| value.to_string())
}
#[derive(Clone, Copy)]
pub enum Format {
    Csv,
    JsonLines,
}
pub struct Report {
    pub rows: usize,
    pub bytes: usize,
}
pub fn export(
    database: &Path,
    q: &Query,
    target: &Path,
    format: Format,
    replace: bool,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<Report> {
    use std::{
        io::{BufWriter, Write},
        sync::atomic::Ordering,
    };
    q.validate()?;
    history::export::validate_target(database, target, replace)?;
    let mut captured = Vec::new();
    let mut retained = 0;
    {
        let db = open(database)?;
        db.execute_batch("BEGIN")?;
        let ceiling = q.ceiling.unwrap_or(db.query_row(
            "SELECT coalesce(max(id),0) FROM alerts",
            [],
            |r| r.get(0),
        )?);
        let mut statement = db.prepare(&format!(
            "SELECT {COLUMNS},document FROM alerts WHERE {WHERE} ORDER BY id DESC LIMIT 10001"
        ))?;
        let mut rows = statement.query(params![
            ceiling, q.before, q.node, q.severity, q.unread, q.search
        ])?;
        while let Some(record) = rows.next()? {
            ensure!(!stop.load(Ordering::Relaxed), "Alert export stopped");
            let detail = Detail {
                row: row(record)?,
                document: record.get(11)?,
            };
            retained += std::mem::size_of::<Detail>()
                + detail.document.capacity()
                + detail.row.node.capacity()
                + detail.row.kind.capacity()
                + detail.row.source_id.capacity()
                + detail.row.summary.capacity();
            ensure!(
                retained <= 4 * 1024 * 1024 && captured.len() < MAX_ROWS,
                "Alert snapshot exceeds its 4 MiB / 10,000-row bound"
            );
            captured.push(detail);
        }
    } // Release the live reader before filesystem output; slow disks cannot pin WAL.
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Export directory missing"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    let mut bytes = 0;
    {
        let mut output = BufWriter::with_capacity(16 * 1024, temp.as_file_mut());
        if matches!(format, Format::Csv) {
            let line = history::export::csv_line(
                [
                    "Received UTC",
                    "Node",
                    "Severity",
                    "Priority",
                    "Cause",
                    "Action",
                    "Payload",
                    "Source ID",
                    "Read",
                    "Summary",
                    "Document JSON",
                ]
                .map(str::to_string),
            );
            bytes += line.len();
            output.write_all(line.as_bytes())?;
        }
        for detail in &captured {
            ensure!(!stop.load(Ordering::Relaxed), "Alert export stopped");
            let r = &detail.row;
            let line = match format {
                Format::Csv => history::export::csv_line([
                    timestamp(r.received),
                    r.node.clone(),
                    severity(r.severity),
                    priority(r.priority),
                    r.cause.to_string(),
                    r.action.to_string(),
                    r.kind.clone(),
                    r.source_id.clone(),
                    r.read.to_string(),
                    r.summary.clone(),
                    detail.document.clone(),
                ]),
                Format::JsonLines => {
                    let mut value = serde_json::to_value(detail)?;
                    value["document"] = serde_json::from_str(&detail.document)?;
                    let mut text = serde_json::to_string(&value)?;
                    text.push('\n');
                    text
                }
            };
            ensure!(
                bytes + line.len() <= MAX_EXPORT,
                "Alert export exceeds 16 MiB; narrow the filters"
            );
            output.write_all(line.as_bytes())?;
            bytes += line.len();
        }
        output.flush()?;
    }
    temp.as_file().sync_all()?;
    ensure!(!stop.load(Ordering::Relaxed), "Alert export stopped");
    history::export::validate_target(database, target, replace)?;
    if replace {
        temp.persist(target)?;
    } else {
        temp.persist_noclobber(target)?;
    }
    Ok(Report {
        rows: captured.len(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(id: u64, text: &str) -> Prepared {
        prepare(protocol::Alert {
            id,
            action: 2,
            data: Some(Data::Text(text.into())),
            ..Default::default()
        })
        .unwrap()
    }
    #[test]
    fn caps_preserve_complete_documents_and_reject_oversized_allocations() {
        let mut text = String::with_capacity(MAX_RETAINED + 1);
        text.push('x');
        assert!(
            prepare(protocol::Alert {
                data: Some(Data::Text(text)),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            prepare(protocol::Alert {
                data: Some(Data::Text("x".repeat(MAX_WIRE))),
                ..Default::default()
            })
            .is_err()
        );
        let snapshot = record(u64::MAX, &"Ω".repeat(10000));
        assert!(snapshot.summary.len() <= 512);
        let line = record(1, "first\nsecond\r\nline");
        assert!(!line.summary.contains(char::is_control));
        assert!(line.document.contains("first\\nsecond"));
        assert!(snapshot.document.contains(&"Ω".repeat(10000)));
        assert_eq!(snapshot.source_id, u64::MAX.to_string());
        let mut op = protocol::Operator::default();
        for _ in 0..18 {
            op = protocol::Operator {
                list: vec![op],
                ..Default::default()
            };
        }
        assert!(
            prepare(protocol::Alert {
                data: Some(Data::Rule(protocol::Rule {
                    operator: Some(op),
                    ..Default::default()
                })),
                ..Default::default()
            })
            .is_err()
        );
    }
    #[test]
    fn retention_pages_and_captured_mutation_scope() -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        schema(&db)?;
        for id in 0..40 {
            insert(&mut db, "node", record(id, &"x".repeat(60000)), 7)?;
        }
        assert!(storage_bytes(&db)? <= MAX_STORAGE);
        let count: usize = db.query_row("SELECT count(*) FROM alerts", [], |r| r.get(0))?;
        assert!(count < 40);
        assert_eq!(
            db.query_row(
                "SELECT source_id FROM alerts ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get::<_, String>(0)
            )?,
            "39"
        );
        db.execute("UPDATE alerts SET received=0", [])?;
        assert_eq!(prune(&db, 7)?, count);
        for id in 0..220 {
            insert(
                &mut db,
                if id % 2 == 0 { "alpha" } else { "beta" },
                record(id, &format!("message {id}")),
                7,
            )?;
        }
        let first = query(&db, &Query::default())?;
        assert_eq!(first.rows.len(), 200);
        assert!(first.has_more);
        insert(&mut db, "alpha", record(999, "newest"), 7)?;
        let older = query(
            &db,
            &Query {
                before: first.rows.last().map(|r| r.id),
                ceiling: Some(first.ceiling),
                ..Default::default()
            },
        )?;
        assert_eq!(older.rows.len(), 20);
        let scope = Query {
            node: "alpha".into(),
            ceiling: Some(first.ceiling),
            ..Default::default()
        };
        assert_eq!(change(&db, Change::Clear(scope))?, 110);
        assert_eq!(
            query(
                &db,
                &Query {
                    node: "alpha".into(),
                    ..Default::default()
                }
            )?
            .rows
            .len(),
            1
        );
        assert!(change(&db, Change::Clear(Query::default())).is_err());
        Ok(())
    }
}
