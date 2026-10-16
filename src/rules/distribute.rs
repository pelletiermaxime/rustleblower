//! Reviewed cross-node copies. One shared source, one transient target inventory,
//! bounded latest progress, sequential ACKs; no SQLite snapshot spans an RPC wait.
use super::{
    document,
    transfer::{self, ExportScope},
};
use crate::{backend::Handle, protocol::Rule};
use anyhow::{Result, bail, ensure};
use prost::Message;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    pub session: u64,
    pub name: String,
}
#[derive(Clone)]
pub struct Target {
    pub destination: Destination,
    pub copies: usize,
    pub existing: usize,
    pub identical: usize,
    pub awaiting: usize,
    pub blocked: Option<String>,
    pub rename_removals: usize,
    digest: [u8; 32],
    config: [u8; 32],
}
pub struct Review {
    pub source: String,
    pub rules: Arc<Vec<Rule>>,
    pub targets: Vec<Target>,
    pub replace: bool,
    pub rename_from: Option<String>,
    hashes: Vec<[u8; 32]>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Ready,
    Running,
    Complete,
    Stopped,
    Failed(String),
}
impl Status {
    pub fn label(&self) -> String {
        match self {
            Self::Ready => "Ready".into(),
            Self::Running => "Waiting for confirmation".into(),
            Self::Complete => "Complete".into(),
            Self::Stopped => "Stopped".into(),
            Self::Failed(error) => format!("Failed: {error}"),
        }
    }
}
#[derive(Clone)]
pub struct TargetProgress {
    pub next: usize,
    pub confirmed: usize,
    pub skipped: usize,
    pub removed: usize,
    remove_done: bool,
    pub status: Status,
    digest: [u8; 32],
}
#[derive(Clone)]
pub struct Progress {
    pub targets: Vec<TargetProgress>,
}
impl Review {
    pub fn progress(&self) -> Progress {
        Progress {
            targets: self
                .targets
                .iter()
                .map(|t| TargetProgress {
                    next: 0,
                    confirmed: 0,
                    skipped: 0,
                    removed: 0,
                    remove_done: t.rename_removals == 0,
                    status: Status::Ready,
                    digest: t.digest,
                })
                .collect(),
        }
    }
}
impl Progress {
    pub fn confirmed(&self) -> usize {
        self.targets.iter().map(|p| p.confirmed).sum()
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Entry {
    wire: [u8; 32],
    awaiting: bool,
    bytes: usize,
}
type Inventory = BTreeMap<String, Entry>;
fn open(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(250))?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-512; PRAGMA mmap_size=0;")?;
    Ok(db)
}
fn metadata(db: &Connection, dest: &Destination) -> Result<[u8; 32]> {
    let (session, bytes): (u64, usize) = db.query_row(
        "SELECT session,length(CAST(config AS BLOB)) FROM node_info WHERE name=?",
        [&dest.name],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(
        session == dest.session,
        "Destination reconnected; rebuild the review"
    );
    ensure!(
        bytes <= crate::nodes::CONFIG_BYTES,
        "Destination configuration exceeds its limit"
    );
    let config: String = db.query_row(
        "SELECT config FROM node_info WHERE name=?",
        [&dest.name],
        |r| r.get(0),
    )?;
    Ok(Sha256::digest(config.as_bytes()).into())
}
fn admit_inventory(db: &Connection, node: &str) -> Result<()> {
    let sql = "SELECT name,wire,0 AS awaiting FROM application_rules WHERE node=?1 UNION ALL SELECT name,wire,1 FROM prompt_rules WHERE node=?1";
    let(count,bytes,max_wire,names,max_name):(usize,usize,usize,usize,usize)=db.query_row(&format!("SELECT count(*),coalesce(sum(length(wire)),0),coalesce(max(length(wire)),0),coalesce(sum(length(CAST(name AS BLOB))),0),coalesce(max(length(CAST(name AS BLOB))),0) FROM ({sql})"),[node],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    ensure!(
        count <= super::MAX_RULES
            && bytes <= super::MAX_CATALOG_BYTES
            && max_wire <= super::MAX_RULE_BYTES
            && names <= super::MAX_CATALOG_BYTES
            && max_name <= 1024,
        "Rule inventory exceeds its bounded capacity"
    );
    Ok(())
}
fn inventory(db: &Connection, node: &str) -> Result<Inventory> {
    admit_inventory(db, node)?;
    let sql = "SELECT name,wire,0 AS awaiting FROM application_rules WHERE node=?1 UNION ALL SELECT name,wire,1 FROM prompt_rules WHERE node=?1";
    let mut result = Inventory::new();
    let mut query = db.prepare(sql)?;
    let mut rows = query.query([node])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        let wire: Vec<u8> = row.get(1)?;
        ensure!(!name.is_empty(), "Destination has an empty rule name");
        ensure!(
            result
                .insert(
                    name,
                    Entry {
                        bytes: wire.len(),
                        wire: Sha256::digest(&wire).into(),
                        awaiting: row.get(2)?
                    }
                )
                .is_none(),
            "Destination has duplicate confirmed/proposed names"
        );
    }
    Ok(result)
}
fn fingerprint(map: &Inventory, config: [u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(config);
    for (name, entry) in map {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update(entry.wire);
        hash.update([u8::from(entry.awaiting)]);
    }
    hash.finalize().into()
}
fn identity(backend: &Handle, dest: &Destination) -> Result<()> {
    let current = backend
        .node(&dest.name)
        .ok_or_else(|| anyhow::anyhow!("Destination disconnected; rebuild the review"))?;
    ensure!(
        current.id == dest.session && current.notifications,
        "Destination connection changed; rebuild the review"
    );
    ensure!(
        backend.history.health.catalogues.ready(dest.session)
            && !backend.history.health.catalogues.uncertain(dest.session),
        "Destination needs a fresh confirmed inventory; reconnect and rebuild the review"
    );
    Ok(())
}
fn active(backend: &Handle, dest: &Destination) -> Result<()> {
    identity(backend, dest)?;
    ensure!(
        !backend.control.node_busy(dest.session) && !backend.control.firewall_busy(dest.session),
        "Destination has a pending node operation; retry after it finishes"
    );
    Ok(())
}
fn skips(entry: Option<&Entry>, hash: [u8; 32], replace: bool) -> bool {
    entry.is_some_and(|e| e.awaiting || e.wire == hash || !replace)
}
/// Call on an I/O executor. Source and every destination use one short read snapshot.
pub fn prepare(
    backend: &Handle,
    scope: &ExportScope,
    targets: Vec<Destination>,
    replace: bool,
) -> Result<Review> {
    ensure!(
        !targets.is_empty() && targets.len() <= crate::nodes::MAX_NODES,
        "Choose between 1 and 32 destination nodes"
    );
    let mut unique = std::collections::BTreeSet::new();
    for t in &targets {
        ensure!(
            t.name != scope.query.node,
            "The source is not a copy destination"
        );
        ensure!(
            t.name.len() <= 128 && unique.insert(t.name.clone()),
            "Duplicate or invalid destination"
        );
        active(backend, t)?;
    }
    let mut db = open(backend.history.database_path())?;
    let tx = db.transaction()?;
    admit_inventory(&tx, &scope.query.node)?;
    let mut rules = Vec::new();
    let mut allocated = 0;
    transfer::visit_snapshot(&tx, scope, |rule, awaiting| {
        ensure!(
            !awaiting,
            "Source rule awaits daemon observation; refresh before copying"
        );
        super::valid(rule).map_err(anyhow::Error::msg)?;
        ensure!(
            rule.operator.is_some(),
            "A source rule has no match operator; refresh its inventory"
        );
        allocated += super::memory_bytes(rule);
        ensure!(allocated <= 8 * 1024 * 1024, "Source review exceeds 8 MiB");
        rules.push(rule.clone());
        Ok(())
    })?;
    ensure!(
        !rules.is_empty(),
        "No confirmed source rules match this scope"
    );
    rules.sort_by(|a, b| a.name.cmp(&b.name));
    let review = prepare_targets(
        backend,
        &tx,
        scope.query.node.clone(),
        Arc::new(rules),
        targets,
        replace,
        None,
    )?;
    tx.commit()?;
    Ok(review)
}
/// Rules supplied by the native editor or a bounded import; no source-node write
/// or source-node exclusion is implied. Validate before acquiring target data.
pub fn prepare_rules(
    backend: &Handle,
    label: &str,
    rules: Arc<Vec<Rule>>,
    targets: Vec<Destination>,
    replace: bool,
    rename_from: Option<String>,
) -> Result<Review> {
    ensure!(
        !label.is_empty() && label.len() <= 256,
        "Invalid rule source label"
    );
    ensure!(
        !rules.is_empty() && rules.len() <= super::MAX_RULES,
        "Provide 1–10,000 rules"
    );
    let mut bytes = 0;
    let mut allocated = 0;
    let mut names = std::collections::BTreeSet::new();
    for rule in rules.iter() {
        document::validate_edit(rule).map_err(anyhow::Error::msg)?;
        bytes += rule.encoded_len();
        allocated += super::memory_bytes(rule);
        ensure!(
            bytes <= super::MAX_CATALOG_BYTES && allocated <= 8 * 1024 * 1024,
            "Supplied rules exceed the 2-MiB wire / 8-MiB memory limit"
        );
        ensure!(names.insert(&rule.name), "Duplicate supplied rule name");
    }
    if let Some(old) = &rename_from {
        ensure!(
            rules.len() == 1 && old != &rules[0].name,
            "Rename needs one rule and a different original name"
        );
        super::valid(&Rule {
            name: old.clone(),
            ..Default::default()
        })
        .map_err(anyhow::Error::msg)?;
    }
    let mut db = open(backend.history.database_path())?;
    let tx = db.transaction()?;
    let review = prepare_targets(
        backend,
        &tx,
        label.to_string(),
        rules,
        targets,
        replace,
        rename_from,
    )?;
    tx.commit()?;
    Ok(review)
}
fn prepare_targets(
    backend: &Handle,
    db: &Connection,
    label: String,
    rules: Arc<Vec<Rule>>,
    targets: Vec<Destination>,
    replace: bool,
    rename_from: Option<String>,
) -> Result<Review> {
    ensure!(
        !targets.is_empty() && targets.len() <= crate::nodes::MAX_NODES,
        "Choose between 1 and 32 destination nodes"
    );
    let mut names = std::collections::BTreeSet::new();
    for target in &targets {
        ensure!(
            !target.name.is_empty() && target.name.len() <= 128 && names.insert(&target.name),
            "Duplicate or invalid destination"
        );
        active(backend, target)?;
    }
    let hashes: Vec<[u8; 32]> = rules
        .iter()
        .map(|r| Sha256::digest(r.encode_to_vec()).into())
        .collect();
    let mut reviewed = Vec::new();
    for destination in targets {
        let config = metadata(db, &destination)?;
        let map = inventory(db, &destination.name)?;
        let (mut copies, mut existing, mut identical, mut awaiting) = (0, 0, 0, 0);
        let mut blocked = None;
        let mut projected_count = map.len();
        let mut projected_bytes = map.values().map(|e| e.bytes).sum::<usize>();
        for (rule, hash) in rules.iter().zip(&hashes) {
            match map.get(&rule.name) {
                Some(e) if e.awaiting => awaiting += 1,
                Some(e) if e.wire == *hash => identical += 1,
                Some(_) if !replace => existing += 1,
                _ => {
                    copies += 1;
                    if let Some(old) = map.get(&rule.name) {
                        projected_bytes -= old.bytes;
                    } else {
                        projected_count += 1;
                    }
                    projected_bytes += rule.encoded_len();
                    if document::has_range(rule)
                        && !backend.control.range_supported(destination.session)
                    {
                        blocked = Some(
                            "Port-range support is not established for this destination".into(),
                        );
                    }
                }
            }
        }
        if projected_count > super::MAX_RULES || projected_bytes > super::MAX_CATALOG_BYTES {
            blocked = Some(
                "Copied rules would exceed the destination's 10,000-rule / 2-MiB cache limit"
                    .into(),
            );
        }
        let rename_removals = if let Some(old) = &rename_from {
            match map.get(old) {
                Some(entry) if entry.awaiting => {
                    blocked = Some(
                        "The original name awaits daemon observation; refresh before renaming"
                            .into(),
                    );
                    0
                }
                Some(_) => {
                    if let Some(new) = map.get(&rules[0].name)
                        && new.wire != hashes[0]
                        && !replace
                    {
                        blocked=Some("Renaming would conflict with the new name; explicitly allow replacement first".into());
                    }
                    1
                }
                None => 0,
            }
        } else {
            0
        };
        reviewed.push(Target {
            digest: fingerprint(&map, config),
            config,
            destination,
            copies,
            existing,
            identical,
            awaiting,
            blocked,
            rename_removals,
        });
    }
    Ok(Review {
        source: label,
        rules,
        targets: reviewed,
        replace,
        rename_from,
        hashes,
    })
}
fn before_send(path: &Path, target: &Target, name: &str, expected: Option<&Entry>) -> Result<()> {
    let mut db = open(path)?;
    let tx = db.transaction()?;
    ensure!(
        metadata(&tx, &target.destination)? == target.config,
        "Destination configuration changed; rebuild the review"
    );
    let (count,max):(usize,usize)=tx.query_row("SELECT count(*),coalesce(max(length(wire)),0) FROM (SELECT wire FROM application_rules WHERE node=?1 AND name=?2 UNION ALL SELECT wire FROM prompt_rules WHERE node=?1 AND name=?2)",params![target.destination.name,name],|r|Ok((r.get(0)?,r.get(1)?)))?;
    ensure!(
        count <= 1 && max <= super::MAX_RULE_BYTES,
        "Destination rule exceeds its bound or has conflicting observation state"
    );
    let row:Option<(Vec<u8>,bool)>=tx.query_row("SELECT wire,0 FROM application_rules WHERE node=?1 AND name=?2 UNION ALL SELECT wire,1 FROM prompt_rules WHERE node=?1 AND name=?2",params![target.destination.name,name],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let current = row.map(|(wire, awaiting)| Entry {
        bytes: wire.len(),
        wire: Sha256::digest(wire).into(),
        awaiting,
    });
    ensure!(
        current.as_ref() == expected,
        "Destination rule changed since review; rebuild before overwriting it"
    );
    Ok(())
}
fn current_inventory(path: &Path, target: &Target, expected: [u8; 32]) -> Result<Inventory> {
    let mut db = open(path)?;
    let tx = db.transaction()?;
    let config = metadata(&tx, &target.destination)?;
    let map = inventory(&tx, &target.destination.name)?;
    ensure!(
        fingerprint(&map, config) == expected,
        "Destination inventory changed since review; rebuild the review"
    );
    tx.commit()?;
    Ok(map)
}
/// Run on a background executor. Only the current destination's compact hashes
/// are retained; connections/transactions are dropped before waiting for ACK.
pub async fn run(
    backend: Handle,
    review: Arc<Review>,
    mut progress: Progress,
    stop: Arc<AtomicBool>,
    updates: watch::Sender<Progress>,
) -> Progress {
    if progress.targets.len() != review.targets.len() {
        return review.progress();
    }
    for (index, target) in review.targets.iter().enumerate() {
        if stop.load(Ordering::Acquire) {
            break;
        }
        if progress.targets[index].status == Status::Complete {
            continue;
        }
        let mut expected_map = None;
        let result = async {
            if let Some(reason) = &target.blocked {
                bail!("{reason}");
            }
            active(&backend, &target.destination)?;
            let lease = backend
                .control
                .reserve_rules(target.destination.session)
                .map_err(anyhow::Error::msg)?;
            backend
                .history
                .fence()
                .map_err(anyhow::Error::msg)?
                .await
                .map_err(|_| anyhow::anyhow!("Rule cache stopped before its copy fence"))?;
            let map = current_inventory(
                backend.history.database_path(),
                target,
                progress.targets[index].digest,
            )?;
            expected_map = Some(map);
            let map = expected_map.as_mut().unwrap();
            progress.targets[index].status = Status::Running;
            updates.send_replace(progress.clone());
            while progress.targets[index].next < review.rules.len() {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let cursor = progress.targets[index].next;
                let rule = &review.rules[cursor];
                let hash = review.hashes[cursor];
                if skips(map.get(&rule.name), hash, review.replace) {
                    progress.targets[index].skipped += 1;
                    progress.targets[index].next += 1;
                    continue;
                }
                identity(&backend, &target.destination)?;
                before_send(
                    backend.history.database_path(),
                    target,
                    &rule.name,
                    map.get(&rule.name),
                )?;
                let reply = lease.submit(rule.clone()).map_err(anyhow::Error::msg)?;
                reply
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!("Command ended without confirmation; reconnect to verify")
                    })?
                    .map_err(anyhow::Error::msg)?;
                map.insert(
                    rule.name.clone(),
                    Entry {
                        wire: hash,
                        awaiting: false,
                        bytes: rule.encoded_len(),
                    },
                );
                let p = &mut progress.targets[index];
                p.next += 1;
                p.confirmed += 1;
                updates.send_replace(progress.clone());
            }
            if !progress.targets[index].remove_done && !stop.load(Ordering::Acquire) {
                let old = review
                    .rename_from
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Rename review is incomplete"))?;
                let new = &review.rules[0];
                ensure!(
                    map.get(&new.name)
                        .is_some_and(|e| !e.awaiting && e.wire == review.hashes[0]),
                    "The replacement rule is not confirmed; the original was retained"
                );
                identity(&backend, &target.destination)?;
                before_send(backend.history.database_path(), target, old, map.get(old))?;
                let reply = lease.delete(old.clone()).map_err(anyhow::Error::msg)?;
                reply
                    .await
                    .map_err(|_| {
                        anyhow::anyhow!(
                            "Replacement confirmed, but original removal ended without confirmation"
                        )
                    })?
                    .map_err(|error| {
                        anyhow::anyhow!(
                            "Replacement confirmed, but original removal failed: {error}"
                        )
                    })?;
                map.remove(old);
                progress.targets[index].removed += 1;
                progress.targets[index].remove_done = true;
                updates.send_replace(progress.clone());
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Some(map) = expected_map {
            progress.targets[index].digest = fingerprint(&map, target.config);
        }
        progress.targets[index].status = match result {
            Err(error) => Status::Failed(crate::history::bounded(&format!("{error:#}"), 1024)),
            Ok(())
                if progress.targets[index].next == review.rules.len()
                    && progress.targets[index].remove_done =>
            {
                Status::Complete
            }
            Ok(()) => Status::Stopped,
        };
        updates.send_replace(progress.clone());
    }
    progress
}
