use crate::protocol::Event;
use anyhow::{Result, ensure};
use prost::Message as _;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Metadata {
    pub schema: u8,
    pub available: bool,
    pub source: String,
    pub source_port: Option<u32>,
    pub destination_ip: String,
    pub host: String,
    pub user: Option<u32>,
    pub pid: Option<u32>,
    pub command: String,
    pub cwd: String,
    pub rule: String,
    pub duration: String,
    pub logged: Option<bool>,
    pub priority: Option<bool>,
    pub truncated: bool,
}
impl Metadata {
    pub fn from_event(event: &Event) -> Self {
        let mut metadata = Self {
            schema: 1,
            available: true,
            ..Default::default()
        };
        if let Some(c) = &event.connection {
            metadata.source = super::bounded(&c.src_ip, 64);
            metadata.source_port = Some(c.src_port);
            metadata.destination_ip = super::bounded(&c.dst_ip, 64);
            metadata.host = super::bounded(&c.dst_host, 1024);
            metadata.user = Some(c.user_id);
            metadata.pid = Some(c.process_id);
            metadata.cwd = super::bounded(&c.process_cwd, 4096);
            for (index, arg) in c.process_args.iter().take(64).enumerate() {
                if index > 0 && metadata.command.len() < 8192 {
                    metadata.command.push(' ');
                }
                let room = 8192 - metadata.command.len();
                metadata.command.push_str(&super::bounded(arg, room));
                if arg.len() > room {
                    metadata.truncated = true;
                    break;
                }
            }
            metadata.truncated |= c.process_cwd.len() > 4096
                || c.process_args.len() > 64
                || c.src_ip.len() > 64
                || c.dst_ip.len() > 64
                || c.dst_host.len() > 1024;
        }
        if let Some(rule) = &event.rule {
            metadata.rule = super::bounded(&rule.name, 1024);
            metadata.duration = super::bounded(&rule.duration, 64);
            metadata.logged = Some(!rule.nolog);
            metadata.priority = Some(rule.precedence);
            metadata.truncated |= rule.name.len() > 1024 || rule.duration.len() > 64;
        }
        metadata
    }
}

const PENDING: &str = "metadata IS NULL OR coalesce(json_extract(metadata,'$.schema'),0)<1";
pub(super) fn ensure_ready(db: &Connection, query: &super::Query) -> Result<()> {
    if !query.filters.iter().any(|f| f.field.needs_metadata())
        && !query.sort.is_some_and(|(field, _)| field.needs_metadata())
    {
        return Ok(());
    }
    // The owned writer/private export connection records completion once.
    // Avoid scanning every retained JSON record on each refresh/page after the
    // upgrade; all later production inserts already carry the current schema.
    let cached: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE type='table' AND name='rustleblower_metadata_ready')", [], |row| row.get(0))?;
    if cached {
        return Ok(());
    }
    let pending: bool = db.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM events WHERE {PENDING})"),
        [],
        |row| row.get(0),
    )?;
    ensure!(
        !pending,
        "Older history metadata is still indexing; retry this metadata filter or sort after indexing finishes"
    );
    Ok(())
}

/// Upgrade both NULL legacy summaries and the older already-indexed JSON rows.
/// One transaction consumes at most 32 bounded detail/metadata records and
/// updates the retained-byte estimate by its actual serialized size delta.
pub(super) fn backfill_chunk(db: &mut Connection, after: &mut i64) -> Result<bool> {
    type LegacyRecord = (i64, Option<String>, Option<Vec<u8>>, u32);
    let pending: Vec<LegacyRecord> = {
        let sql = format!(
            "SELECT e.id,e.metadata,d.payload,coalesce(d.truncated,0) FROM events e LEFT JOIN event_details d ON d.event_id=e.id WHERE e.id>? AND ({PENDING}) ORDER BY e.id LIMIT 32"
        );
        let mut statement = db.prepare(&sql)?;
        statement
            .query_map([*after], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<rusqlite::Result<_>>()?
    };
    if pending.is_empty() {
        db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS rustleblower_metadata_ready(schema INTEGER NOT NULL); INSERT INTO rustleblower_metadata_ready SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM rustleblower_metadata_ready);")?;
        return Ok(false);
    }
    let tx = db.transaction()?;
    let mut additional: i64 = 0;
    for (id, previous, wire, flags) in pending {
        ensure!(
            previous.as_ref().is_none_or(|s| s.len() <= 32 * 1024),
            "Legacy metadata exceeds the bounded migration record size"
        );
        let decoded = wire.and_then(|wire| Event::decode(wire.as_slice()).ok());
        let json = if let Some(previous) = &previous {
            // Preserve prior indexed values and unknown JSON fields. Only the
            // newly introduced cwd/schema and truncation marker are updated.
            let mut value: serde_json::Value = serde_json::from_str(previous)?;
            let object = value
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("Legacy metadata is not an object"))?;
            let cwd = decoded
                .as_ref()
                .and_then(|e| e.connection.as_ref())
                .map_or("", |c| c.process_cwd.as_str());
            object.insert("cwd".into(), super::bounded(cwd, 4096).into());
            object.insert("schema".into(), 1.into());
            let truncated = object
                .get("truncated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || flags != 0
                || cwd.len() > 4096;
            object.insert("truncated".into(), truncated.into());
            serde_json::to_string(&value)?
        } else {
            let mut metadata = decoded
                .as_ref()
                .map(Metadata::from_event)
                .unwrap_or_default();
            metadata.schema = 1;
            metadata.truncated |= flags != 0;
            serde_json::to_string(&metadata)?
        };
        let delta = json.len() as i64 - previous.as_ref().map_or(-24, |s| s.len() as i64);
        if tx.execute(&format!("UPDATE events SET metadata=?1,stored_bytes=stored_bytes+?2 WHERE id=?3 AND ({PENDING})"), params![json,delta,id])? == 1 { additional += delta; }
        *after = id;
    }
    tx.execute("UPDATE usage SET bytes=bytes+?", [additional])?;
    tx.commit()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cwd_indexing_is_bounded_resumable_and_metadata_queries_report_not_ready() -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        db.execute_batch("CREATE TABLE events(id INTEGER PRIMARY KEY,metadata TEXT,stored_bytes INTEGER); CREATE TABLE event_details(event_id INTEGER,payload BLOB,truncated INTEGER); CREATE TABLE usage(bytes INTEGER); INSERT INTO usage VALUES(35000);")?;
        let event = Event {
            connection: Some(crate::protocol::Connection {
                process_cwd: "é".repeat(3000),
                ..Default::default()
            }),
            ..Default::default()
        };
        for id in 1..=35 {
            db.execute("INSERT INTO events VALUES(?, '{\"available\":true,\"command\":\"preserve\"}',1000)", [id])?;
            db.execute(
                "INSERT INTO event_details VALUES(?,?,0)",
                params![id, event.encode_to_vec()],
            )?;
        }
        let query = super::super::Query {
            filters: vec![super::super::query::Filter {
                field: super::super::query::Field::Cwd,
                comparison: super::super::query::Comparison::Present,
                value: String::new(),
            }],
            ..Default::default()
        };
        assert!(
            ensure_ready(&db, &query)
                .unwrap_err()
                .to_string()
                .contains("indexing")
        );
        ensure_ready(&db, &super::super::Query::default())?;
        let mut after = 0;
        assert!(backfill_chunk(&mut db, &mut after)?);
        assert_eq!(after, 32);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM events WHERE json_extract(metadata,'$.schema')=1",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            32
        );
        assert!(ensure_ready(&db, &query).is_err());
        after = 0; // Restart resumes by the per-row marker, not the old cursor.
        assert!(backfill_chunk(&mut db, &mut after)?);
        assert!(!backfill_chunk(&mut db, &mut after)?);
        ensure_ready(&db, &query)?;
        let metadata: String =
            db.query_row("SELECT metadata FROM events WHERE id=1", [], |r| r.get(0))?;
        let metadata: Metadata = serde_json::from_str(&metadata)?;
        assert_eq!(metadata.cwd.len(), 4096);
        assert!(metadata.truncated);
        assert_eq!(metadata.command, "preserve");
        assert_eq!(
            db.query_row("SELECT bytes FROM usage", [], |r| r.get::<_, i64>(0))?,
            db.query_row("SELECT sum(stored_bytes) FROM events", [], |r| r
                .get::<_, i64>(0))?
        );
        Ok(())
    }
}
