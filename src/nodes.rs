//! Bounded node metadata and acknowledged daemon control; policy stays daemon-owned.
pub mod preferences;
use crate::{history, protocol};
use anyhow::{Result, ensure};
use prost::Message;
use rusqlite::{Connection, params};
use std::{
    path::Path,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
pub const MAX_NODES: usize = 32;
pub const CONFIG_BYTES: usize = 64 * 1024;
pub const FIREWALL_BYTES: usize = 256 * 1024;
#[derive(Clone)]
pub enum Change {
    Interception(bool),
    Configuration(String),
}
impl Change {
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Configuration(text) = self {
            validate_config(text)?;
        }
        Ok(())
    }
    pub(crate) fn action(&self) -> protocol::Action {
        match self {
            Self::Interception(true) => protocol::Action::EnableInterception,
            Self::Interception(false) => protocol::Action::DisableInterception,
            Self::Configuration(_) => protocol::Action::ChangeConfig,
        }
    }
    pub(crate) fn data(&self) -> &str {
        match self {
            Self::Configuration(text) => text,
            _ => "",
        }
    }
}
pub fn validate_config(text: &str) -> Result<serde_json::Value, String> {
    if text.len() > CONFIG_BYTES {
        return Err("Daemon configuration exceeds 64 KiB".into());
    }
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("Invalid daemon JSON: {error}"))?;
    if !value.is_object() {
        return Err("Daemon configuration must be a JSON object".into());
    }
    Ok(value)
}
pub(crate) struct Saved {
    pub kernel: String,
    pub config: String,
    pub running: bool,
    pub log_level: u32,
    pub firewall: Vec<u8>,
}
impl Saved {
    pub(crate) fn prepare(config: &protocol::ClientConfig) -> Result<Self, String> {
        if config.config.len() > CONFIG_BYTES
            || config
                .system_firewall
                .as_ref()
                .is_some_and(|firewall| firewall.encoded_len() > FIREWALL_BYTES)
        {
            return Err(
                "Node configuration exceeds the 64 KiB JSON / 256 KiB firewall cache limits".into(),
            );
        }
        Ok(Self {
            kernel: history::bounded(&config.version, 256),
            config: config.config.clone(),
            running: config.is_firewall_running,
            log_level: config.log_level,
            firewall: config
                .system_firewall
                .as_ref()
                .map(Message::encode_to_vec)
                .unwrap_or_default(),
        })
    }
    pub fn bytes(&self) -> usize {
        self.kernel.capacity() + self.config.capacity() + self.firewall.capacity() + 512
    }
}
pub(crate) fn schema(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS node_info(name TEXT PRIMARY KEY,session INTEGER NOT NULL,updated INTEGER NOT NULL,kernel TEXT NOT NULL,config TEXT NOT NULL,running INTEGER NOT NULL,log_level INTEGER NOT NULL,firewall BLOB NOT NULL);")
}
pub(crate) fn save(
    db: &mut Connection,
    session: u64,
    node: &str,
    saved: Saved,
    active: &[String],
) -> Result<()> {
    let tx = db.transaction()?;
    tx.execute("INSERT INTO node_info VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(name) DO UPDATE SET session=excluded.session,updated=excluded.updated,kernel=excluded.kernel,config=excluded.config,running=excluded.running,log_level=excluded.log_level,firewall=excluded.firewall",params![node,session,history::now_nanos(),saved.kernel,saved.config,saved.running,saved.log_level,saved.firewall])?;
    let names = {
        let mut query = tx.prepare("SELECT name FROM node_info ORDER BY updated,name")?;
        query
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut excess = names.len().saturating_sub(MAX_NODES);
    for name in names {
        if excess == 0 {
            break;
        }
        if active.contains(&name) {
            continue;
        }
        tx.execute("DELETE FROM node_info WHERE name=?", [&name])?;
        tx.execute("DELETE FROM application_rules WHERE node=?", [&name])?;
        tx.execute("DELETE FROM prompt_rules WHERE node=?", [&name])?;
        excess -= 1;
    }
    // Legacy catalogues had no node metadata and could accumulate names forever.
    // Keep currently loading live nodes, and evict only unreachable cached copies.
    let active = serde_json::to_string(active)?;
    for table in ["application_rules", "prompt_rules"] {
        tx.execute(&format!("DELETE FROM {table} WHERE node NOT IN (SELECT name FROM node_info) AND node NOT IN (SELECT value FROM json_each(?))"),[&active])?;
    }
    ensure!(excess == 0, "Node cache exceeds its 32-node bound");
    tx.commit()?;
    Ok(())
}
pub(crate) fn apply(db: &mut Connection, node: &str, change: &Change) -> Result<()> {
    let affected = match change {
        Change::Interception(running) => db.execute(
            "UPDATE node_info SET running=?,updated=? WHERE name=?",
            params![running, history::now_nanos(), node],
        )?,
        Change::Configuration(config) => db.execute(
            "UPDATE node_info SET config=?,updated=? WHERE name=?",
            params![config, history::now_nanos(), node],
        )?,
    };
    ensure!(
        affected == 1,
        "The node cache is unavailable; reconnect to reload confirmed state"
    );
    Ok(())
}
#[derive(Clone)]
pub struct Row {
    pub session: u64,
    pub name: String,
    pub kernel: String,
    pub running: bool,
    pub log_level: u32,
    pub updated: i64,
}
pub struct Detail {
    pub row: Row,
    pub config: String,
}
fn open(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(250))?;
    db.execute_batch("PRAGMA cache_size=-512; PRAGMA mmap_size=0;")?;
    Ok(db)
}
fn row(record: &rusqlite::Row) -> rusqlite::Result<Row> {
    Ok(Row {
        name: record.get(0)?,
        session: record.get(5)?,
        kernel: record.get(1)?,
        running: record.get(2)?,
        log_level: record.get(3)?,
        updated: record.get(4)?,
    })
}
pub fn page(path: &Path, search: &str) -> Result<Vec<Row>> {
    ensure!(search.len() <= 256, "Node search exceeds 256 bytes");
    let db = open(path)?;
    let mut query=db.prepare("SELECT name,kernel,running,log_level,updated,session FROM node_info WHERE instr(lower(name),lower(?1))>0 OR instr(lower(kernel),lower(?1))>0 ORDER BY name LIMIT 32")?;
    Ok(query
        .query_map([search], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}
pub fn detail(path: &Path, name: &str) -> Result<Detail> {
    ensure!(name.len() <= 128, "Invalid node name");
    let db = open(path)?;
    Ok(db.query_row(
        "SELECT name,kernel,running,log_level,updated,session,config FROM node_info WHERE name=?",
        [name],
        |record| {
            Ok(Detail {
                row: row(record)?,
                config: record.get(6)?,
            })
        },
    )?)
}
pub fn read_permit() -> Option<OwnedSemaphorePermit> {
    static READ: OnceLock<Arc<Semaphore>> = OnceLock::new();
    READ.get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
        .try_acquire_owned()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_cleanup_keeps_active_pending_metadata_and_prunes_unreachable_legacy_rules()
    -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        crate::rules::schema(&db)?;
        schema(&db)?;
        let rule = protocol::Rule {
            name: "fixture".into(),
            ..Default::default()
        };
        for node in ["legacy-orphan", "active-loading", "current"] {
            crate::rules::replace(&mut db, node, std::slice::from_ref(&rule))?;
        }
        let saved = Saved::prepare(&protocol::ClientConfig {
            version: "kernel".into(),
            config: "{}".into(),
            ..Default::default()
        })
        .unwrap();
        save(
            &mut db,
            1,
            "current",
            saved,
            &["current".into(), "active-loading".into()],
        )?;
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM application_rules WHERE node='legacy-orphan'",
                [],
                |row| row.get::<_, usize>(0)
            )?,
            0
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM application_rules", [], |row| row
                .get::<_, usize>(0))?,
            2
        );
        let bad = protocol::ClientConfig {
            config: "x".repeat(CONFIG_BYTES + 1),
            ..Default::default()
        };
        assert!(Saved::prepare(&bad).is_err());
        Ok(())
    }
}
