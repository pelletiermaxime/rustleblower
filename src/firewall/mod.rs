//! Bounded system-firewall snapshots and edits; enforcement remains daemon-owned.
pub mod document;
mod guard;
pub mod helpers;
pub mod policy;
pub mod suggestions;
pub mod transfer;
use crate::{nodes, protocol};
use anyhow::{Context, Result, ensure};
use prost::Message;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const MAX_CHAINS: usize = 256;
pub const MAX_RULES: usize = 4096;
pub const MAX_EXPRESSIONS: usize = 8192;
pub const MAX_VALUES: usize = 16384;
pub const MAX_JSON: usize = 1024 * 1024;
pub const MAX_HEAP: usize = 2 * 1024 * 1024;
pub const PAGE: usize = 200;

pub fn read_job() -> Option<OwnedSemaphorePermit> {
    static JOB: OnceLock<Arc<Semaphore>> = OnceLock::new();
    JOB.get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
        .try_acquire_owned()
        .ok()
}

pub fn fingerprint(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
pub fn backend_name(config: &str) -> String {
    #[derive(serde::Deserialize)]
    struct Report {
        #[serde(rename = "Firewall")]
        firewall: Option<String>,
    }
    if config.len() > nodes::CONFIG_BYTES {
        return "unreported".into();
    }
    serde_json::from_str::<Report>(config)
        .ok()
        .and_then(|v| v.firewall)
        .map(|v| crate::history::bounded(&v, 128))
        .unwrap_or_else(|| "unreported".into())
}

pub fn validate(config: &protocol::SysFirewall) -> Result<(), String> {
    check(config, true)
}
pub fn admit(config: &protocol::SysFirewall) -> Result<(), String> {
    check(config, false)
}
fn check(config: &protocol::SysFirewall, editing: bool) -> Result<(), String> {
    if editing && config.version != 1 {
        return Err(
            "Only reported system-firewall format 1 can be edited; refresh or update this node"
                .into(),
        );
    }
    if config.encoded_len() > nodes::FIREWALL_BYTES
        || config.system_rules.len() > 256
        || memory_bytes(config) > MAX_HEAP
    {
        return Err("System firewall exceeds the 256 KiB snapshot/group limit".into());
    }
    let (mut chains, mut rules, mut expressions, mut values) = (0, 0, 0, 0);
    let mut identities = BTreeSet::new();
    for group in &config.system_rules {
        for chain in &group.chains {
            chains += 1;
            if editing
                && (chain.name.is_empty() || chain.table.is_empty() || chain.family.is_empty())
            {
                return Err("Every chain needs a name, table and family".into());
            }
            if editing && !identities.insert((&chain.table, &chain.family, &chain.name)) {
                return Err("Duplicate chain identity (table, family, name)".into());
            }
        }
        for rule in group
            .rule
            .iter()
            .chain(group.chains.iter().flat_map(|c| &c.rules))
        {
            rules += 1;
            expressions += rule.expressions.len();
            for expression in &rule.expressions {
                let Some(statement) = &expression.statement else {
                    if editing {
                        return Err("An expression needs its statement".into());
                    }
                    continue;
                };
                if editing && statement.name.is_empty() {
                    return Err("Every expression needs a statement name".into());
                }
                values += statement.values.len();
            }
        }
    }
    if chains > MAX_CHAINS
        || rules > MAX_RULES
        || expressions > MAX_EXPRESSIONS
        || values > MAX_VALUES
    {
        return Err(
            "System firewall exceeds the bounded chain/rule/expression/value limits".into(),
        );
    }
    Ok(())
}

pub fn memory_bytes(config: &protocol::SysFirewall) -> usize {
    fn rule(r: &protocol::FwRule) -> usize {
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
                    s.op.capacity()
                        + s.name.capacity()
                        + s.values.capacity() * std::mem::size_of::<protocol::StatementValues>()
                        + s.values
                            .iter()
                            .map(|v| v.key.capacity() + v.value.capacity())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
    std::mem::size_of::<protocol::SysFirewall>()
        + config.system_rules.capacity() * std::mem::size_of::<protocol::FwChains>()
        + config
            .system_rules
            .iter()
            .map(|group| {
                group.rule.as_ref().map(rule).unwrap_or(0)
                    + group.chains.capacity() * std::mem::size_of::<protocol::FwChain>()
                    + group
                        .chains
                        .iter()
                        .map(|c| {
                            c.name.capacity()
                                + c.table.capacity()
                                + c.family.capacity()
                                + c.priority.capacity()
                                + c.r#type.capacity()
                                + c.hook.capacity()
                                + c.policy.capacity()
                                + c.rules.capacity() * std::mem::size_of::<protocol::FwRule>()
                                + c.rules.iter().map(rule).sum::<usize>()
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
}

#[derive(Clone)]
pub struct Change {
    pub expected: [u8; 32],
    pub config: Box<protocol::SysFirewall>,
}
impl Change {
    pub fn new(expected: [u8; 32], config: protocol::SysFirewall) -> Result<Self, String> {
        validate(&config)?;
        Ok(Self {
            expected,
            config: Box::new(config),
        })
    }
    pub fn bytes(&self) -> usize {
        // Covers decoded repeated-message allocations as well as encoded bytes.
        memory_bytes(&self.config) + 512
    }
}

pub struct Snapshot {
    pub node: String,
    pub session: u64,
    pub updated: i64,
    pub reported: bool,
    pub backend: String,
    pub config: protocol::SysFirewall,
    pub fingerprint: [u8; 32],
}

pub(crate) fn schema(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS firewall_accepted(node TEXT PRIMARY KEY REFERENCES node_info(name) ON DELETE CASCADE, fingerprint BLOB NOT NULL);")
}

pub fn read(path: &Path, node: &str) -> Result<Snapshot> {
    ensure!(node.len() <= 128, "Invalid node name");
    let db = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(250))?;
    db.execute_batch("PRAGMA cache_size=-512; PRAGMA mmap_size=0;")?;
    let (session, updated, len, bytes, accepted,config): (u64, i64, usize, Vec<u8>, bool,String) = db.query_row(
        "SELECT session,updated,length(firewall),substr(firewall,1,?2),EXISTS(SELECT 1 FROM firewall_accepted a WHERE a.node=n.name),substr(config,1,65537) FROM node_info n WHERE name=?1",
        params![node, nodes::FIREWALL_BYTES],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    )?;
    ensure!(
        len <= nodes::FIREWALL_BYTES,
        "Cached firewall exceeds 256 KiB"
    );
    let backend = backend_name(&config);
    let config = protocol::SysFirewall::decode(bytes.as_slice())
        .context("Cannot decode reported firewall")?;
    Ok(Snapshot {
        node: node.into(),
        session,
        updated,
        reported: !accepted,
        backend,
        fingerprint: fingerprint(&bytes),
        config,
    })
}

pub(crate) fn apply(db: &mut Connection, node: &str, change: &Change) -> Result<()> {
    let tx = db.transaction()?;
    let (len, before): (usize, Vec<u8>) = tx.query_row(
        "SELECT length(firewall),substr(firewall,1,?2) FROM node_info WHERE name=?1",
        params![node, nodes::FIREWALL_BYTES],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(
        len <= nodes::FIREWALL_BYTES,
        "Cached firewall exceeds 256 KiB; reconnect"
    );
    ensure!(
        fingerprint(&before) == change.expected,
        "The cached firewall changed before the acknowledged configuration could be saved; reconnect"
    );
    let bytes = change.config.encode_to_vec();
    tx.execute(
        "UPDATE node_info SET firewall=?,updated=? WHERE name=?",
        params![bytes, crate::history::now_nanos(), node],
    )?;
    tx.execute("INSERT INTO firewall_accepted VALUES(?1,?2) ON CONFLICT(node) DO UPDATE SET fingerprint=excluded.fingerprint",params![node,fingerprint(&bytes).as_slice()])?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_oversized_cache_is_rejected_by_bounded_reads() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("cache.sqlite3");
        let mut db = Connection::open(&path)?;
        crate::nodes::schema(&db)?;
        db.execute(
            "INSERT INTO node_info VALUES('owned',1,1,'','{}',0,0,zeroblob(?))",
            [nodes::FIREWALL_BYTES + 1],
        )?;
        assert!(
            read(&path, "owned")
                .err()
                .unwrap()
                .to_string()
                .contains("256 KiB")
        );
        let config = protocol::SysFirewall {
            version: 1,
            ..Default::default()
        };
        let change = Change::new([0; 32], config).map_err(anyhow::Error::msg)?;
        assert!(
            apply(&mut db, "owned", &change)
                .unwrap_err()
                .to_string()
                .contains("256 KiB")
        );
        assert_eq!(
            db.query_row("SELECT length(firewall) FROM node_info", [], |r| r
                .get::<_, usize>(0))?,
            nodes::FIREWALL_BYTES + 1
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM firewall_accepted", [], |r| r
                .get::<_, usize>(0))?,
            0
        );
        Ok(())
    }
}
