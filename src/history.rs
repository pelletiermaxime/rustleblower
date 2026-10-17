//! One SQLite writer thread; exports use independent read-only snapshots.
//! Neither gRPC nor GPUI waits for disk I/O.
pub mod details;
pub mod export;
pub mod grouped;
pub mod metadata;
pub mod query;
pub mod selection;
pub mod settings;
pub mod storage;
use crate::{
    protocol::{Action, Event, Rule},
    rules,
};
use anyhow::{Context, Result};
use prost::Message;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::{
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};

pub const QUEUE_BYTES: usize = 8 * 1024 * 1024;
pub const PAGE_SIZE: usize = 200;
const MAX_BATCH_EVENTS: usize = 4096;

#[derive(Clone, Debug)]
pub struct Row {
    pub id: i64,
    pub unixnano: i64,
    pub action: String,
    pub protocol: String,
    pub process: String,
    pub destination: String,
    pub port: i64,
    pub reason: String,
    pub node: String,
    pub metadata: metadata::Metadata,
}

#[derive(Clone, Default)]
pub struct Query {
    pub node: String,
    pub before: Option<i64>,
    pub search: String,
    pub action: String,
    pub filters: Vec<query::Filter>,
    pub logic: query::Logic,
    pub sort: Option<(query::Field, bool)>,
    pub cursor: Option<query::Cursor>,
    pub ceiling: Option<i64>,
}

#[derive(Default)]
pub struct Page {
    pub rows: Arc<Vec<Row>>,
    pub has_more: bool,
    pub next_cursor: Option<query::Cursor>,
    pub ceiling: i64,
}

#[derive(Default)]
pub struct Health {
    pub dropped: AtomicU64,
    pub queue_full: AtomicU64,
    pub event_limit: AtomicU64,
    pub storage_failed: AtomicU64,
    pub connection_lost: AtomicU64,
    pub stored: AtomicU64,
    pub revision: AtomicU64,
    pub catalogues: Arc<rules::Catalogues>,
    pub nodes_revision: AtomicU64,
    pub interception: crate::nodes::interception::State,
    pub alerts_revision: AtomicU64,
    pub alerts_skipped: AtomicU64,
    pub alerts_evicted: AtomicU64,
    pub alerts_error: Mutex<Option<String>>,
    pub error: Mutex<Option<String>>,
    pub indexing: AtomicBool,
    pub metadata_error: Mutex<Option<String>>,
    pub statistics_revision: AtomicU64,
    pub statistics_skipped: AtomicU64,
    pub statistics_evicted: AtomicU64,
    pub statistics_error: Mutex<Option<String>>,
}

#[derive(Clone, Copy)]
pub(crate) enum LossReason {
    QueueFull,
    EventLimit,
    Storage,
    Connection,
}

impl Health {
    pub fn metadata_warning(&self) -> Option<String> {
        self.metadata_error.lock().unwrap().clone().or_else(|| self.indexing.load(Ordering::Acquire).then(||"Indexing older history metadata; advanced filters and columns are incomplete until it finishes.".into()))
    }
    pub(crate) fn record_loss(&self, reason: LossReason, count: usize) {
        let counter = match reason {
            LossReason::QueueFull => &self.queue_full,
            LossReason::EventLimit => &self.event_limit,
            LossReason::Storage => &self.storage_failed,
            LossReason::Connection => &self.connection_lost,
        };
        counter.fetch_add(count as u64, Ordering::Relaxed);
        self.dropped.fetch_add(count as u64, Ordering::Relaxed);
    }

    pub fn warning(&self) -> Option<String> {
        let causes: Vec<_> = [
            (&self.queue_full, "while recording couldn't keep up"),
            (&self.event_limit, "that exceeded event limits"),
            (&self.storage_failed, "because they couldn't be saved"),
            (&self.connection_lost, "during a daemon disconnect"),
        ]
        .into_iter()
        .filter_map(|(counter, cause)| {
            let count = counter.load(Ordering::Relaxed);
            (count > 0).then(|| format!("{count} {cause}"))
        })
        .collect();
        (!causes.is_empty()).then(|| format!("History missed events: {}.", causes.join("; ")))
    }
}

struct Record {
    fingerprint: [u8; 32],
    unixnano: i64,
    action: String,
    protocol: String,
    process: String,
    destination: String,
    port: u32,
    detail: Vec<u8>,
    truncated: u32,
    observed_rule: Option<Box<Rule>>,
    observed_name: Option<String>,
    metadata: String,
}

impl Record {
    fn from_event(mut event: Event) -> Self {
        let fingerprint = fingerprint(&mut event);
        let observed_rule = event
            .rule
            .as_ref()
            .filter(|rule| {
                rule.name.starts_with("rustleblower.prompt.") && rules::valid(rule).is_ok()
            })
            .cloned()
            .map(Box::new);
        let observed_name = event
            .rule
            .as_ref()
            .filter(|rule| !rule.name.is_empty() && rule.name.len() <= 1024)
            .map(|rule| rule.name.clone());
        let con = event.connection.as_ref();
        let rule = event.rule.as_ref();
        let action = rule.map_or(String::new(), |r| bounded(&r.action, 32));
        let protocol = con.map_or(String::new(), |c| bounded(&c.protocol, 32));
        let process = con.map_or(String::new(), |c| bounded(&c.process_path, 4096));
        let destination = con.map_or(String::new(), |c| {
            bounded(
                if c.dst_host.is_empty() {
                    &c.dst_ip
                } else {
                    &c.dst_host
                },
                1024,
            )
        });
        let port = con.map_or(0, |c| c.dst_port);
        let unixnano = event.unixnano;
        let mut metadata = metadata::Metadata::from_event(&event);
        let (detail, truncated) = details::encode(event);
        metadata.truncated |= truncated != 0;
        let metadata = serde_json::to_string(&metadata).expect("metadata strings always serialize");
        Self {
            fingerprint,
            unixnano,
            action,
            protocol,
            process,
            destination,
            port,
            detail,
            truncated,
            observed_rule,
            observed_name,
            metadata,
        }
    }
    fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.action.capacity()
            + self.protocol.capacity()
            + self.process.capacity()
            + self.destination.capacity()
            + self.detail.capacity()
            + self.metadata.capacity()
            + self.observed_name.as_ref().map_or(0, String::capacity)
            + self
                .observed_rule
                .as_ref()
                .map_or(0, |rule| rules::memory_bytes(rule))
    }
}

// Protobuf map iteration is not stable after decoding. Remove those maps from
// the wire portion, then hash sorted, length-framed entries to deduplicate replay.
fn fingerprint(event: &mut Event) -> [u8; 32] {
    let maps = event.connection.as_mut().map(|c| {
        (
            std::mem::take(&mut c.process_env),
            std::mem::take(&mut c.process_checksums),
        )
    });
    let mut hash = Sha256::new();
    hash.update(event.encode_to_vec());
    if let Some((env, checksums)) = maps {
        for (index, map) in [&env, &checksums].into_iter().enumerate() {
            if map.is_empty() {
                continue;
            }
            hash.update([index as u8]);
            hash.update((map.len() as u64).to_le_bytes());
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(key, _)| *key);
            for (key, value) in entries {
                for text in [key, value] {
                    hash.update((text.len() as u64).to_le_bytes());
                    hash.update(text.as_bytes());
                }
            }
        }
        if let Some(c) = event.connection.as_mut() {
            c.process_env = env;
            c.process_checksums = checksums;
        }
    }
    hash.finalize().into()
}

pub(crate) struct Batch {
    events: Vec<Record>,
    reason: String,
    _permit: OwnedSemaphorePermit,
}

impl Batch {
    pub(crate) fn len(&self) -> usize {
        self.events.len()
    }
}

enum Command {
    Fence(oneshot::Sender<()>),
    PausePreference {
        session: u64,
        node: String,
        remember: bool,
        running: bool,
        expected: Option<bool>,
        reply: oneshot::Sender<std::result::Result<(), String>>,
    },
    Alert {
        node: String,
        record: Box<crate::alerts::Prepared>,
        _permit: OwnedSemaphorePermit,
    },
    AlertChange(
        crate::alerts::Change,
        oneshot::Sender<std::result::Result<usize, String>>,
    ),
    SaveNode {
        session: u64,
        node: String,
        saved: Box<crate::nodes::Saved>,
        _permit: OwnedSemaphorePermit,
    },
    ApplyNode {
        session: u64,
        node: String,
        change: crate::nodes::Change,
        reply: oneshot::Sender<std::result::Result<(), String>>,
        _permit: OwnedSemaphorePermit,
    },

    Events {
        node: String,
        batch: Batch,
    },
    ReplaceRules {
        session: u64,
        node: String,
        rules: Vec<Rule>,
        _permit: OwnedSemaphorePermit,
    },
    QueryRules(
        rules::Query,
        oneshot::Sender<std::result::Result<rules::Page, String>>,
    ),
    ApplyRule {
        session: u64,
        node: String,
        action: Action,
        rule: Box<Rule>,
        reply: oneshot::Sender<std::result::Result<(), String>>,
        _permit: OwnedSemaphorePermit,
    },
    ProposeRule {
        session: u64,
        node: String,
        rule: Box<Rule>,
        _permit: OwnedSemaphorePermit,
    },
    Query(
        Box<Query>,
        oneshot::Sender<std::result::Result<Page, String>>,
    ),
    Detail(
        i64,
        oneshot::Sender<std::result::Result<Option<details::Detail>, String>>,
    ),
    StatisticsReset(
        crate::statistics::Reset,
        oneshot::Sender<std::result::Result<crate::statistics::ResetResult, String>>,
    ),
    StoragePreview(
        storage::Config,
        u64,
        oneshot::Sender<Result<storage::Preview, String>>,
    ),
    StorageApply(
        storage::Config,
        u64,
        oneshot::Sender<Result<storage::Applied, String>>,
    ),
    Barrier(oneshot::Sender<()>),
}

#[derive(Clone)]
pub struct History {
    tx: mpsc::SyncSender<Command>,
    budget: Arc<Semaphore>,
    pub health: Arc<Health>,
    pub settings: Arc<settings::Store>,
    pub storage: Arc<storage::State>,
    statistics: Arc<Mutex<std::collections::VecDeque<(String, crate::statistics::Prepared)>>>,
    pub(crate) database: std::path::PathBuf,
}

pub struct Worker {
    pub history: History,
    join: Option<thread::JoinHandle<()>>,
}

impl Worker {
    pub fn start(path: &Path, days: u32, mib: u32) -> Result<Self> {
        Self::start_with_overrides(path, days, mib, Default::default())
    }
    pub fn start_with_overrides(
        path: &Path,
        days: u32,
        mib: u32,
        overrides: storage::Overrides,
    ) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut db = open(path, mib)?;
        let saved = storage::initialize(&db, days, mib, overrides)?;
        let (mut days, mut mib) = (saved.config.days, saved.config.mib);
        maintain(&mut db, days, mib)?;
        storage::cap(&db, mib)?;
        let storage = Arc::new(storage::State::new(saved));
        let storage_worker = storage.clone();
        let (tx, rx) = mpsc::sync_channel(32);
        let health = Arc::new(Health::default());
        health.interception.synchronize(&db);
        health.indexing.store(true, Ordering::Release);
        let statistics = Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(
            crate::nodes::MAX_NODES,
        )));
        let history = History {
            tx,
            budget: Arc::new(Semaphore::new(QUEUE_BYTES)),
            health: health.clone(),
            settings: Arc::new(settings::Store::open_for_app(path)),
            storage,
            statistics: statistics.clone(),
            database: path.to_owned(),
        };
        let join = thread::Builder::new()
            .name("rustleblower-history".into())
            .spawn(move || {
                let mut indexed_after=0;
                let mut next_cleanup=std::time::Instant::now()+std::time::Duration::from_secs(u64::from(storage_worker.snapshot().config.cleanup_minutes)*60);
                loop {
                    let command=match rx.recv_timeout(std::time::Duration::from_millis(10)) {Ok(command)=>Some(command),Err(mpsc::RecvTimeoutError::Timeout)=>None,Err(mpsc::RecvTimeoutError::Disconnected)=>break};
                    let samples={let mut queue=statistics.lock().unwrap();if matches!(command.as_ref(),Some(Command::Barrier(_))|Some(Command::StatisticsReset(..))){queue.drain(..).collect::<Vec<_>>()}else{queue.pop_front().into_iter().collect::<Vec<_>>()}};
                    for (node,sample) in samples {
                        let result=(||->Result<usize>{let current=crate::statistics::storage_bytes(&db)?;let anticipated=crate::statistics::anticipated(&db,&node,&sample)?;let changes=db.total_changes();maintain_reserved(&mut db,days,mib,anticipated.saturating_sub(current))?;if db.total_changes()!=changes{health.revision.fetch_add(1,Ordering::Release);}crate::statistics::apply(&mut db,&node,sample,days)})();
                        match result {Ok(evicted)=>{health.statistics_evicted.fetch_add(evicted as u64,Ordering::Relaxed);health.statistics_revision.fetch_add(1,Ordering::Release);*health.statistics_error.lock().unwrap()=None;},Err(error)=>{*health.statistics_error.lock().unwrap()=Some(format!("Statistics report could not be cached: {error:#}"));}}
                    }
                    if let Some(command)=command { match command {
                        Command::Alert{node,record,_permit}=>{
                            let result=(||->Result<usize>{let changes=db.total_changes();let current=crate::alerts::storage_bytes(&db)?;let anticipated=(current+record.storage_bytes(&node) as u64).min(crate::alerts::MAX_STORAGE);maintain_reserved(&mut db,days,mib,anticipated.saturating_sub(current))?;if db.total_changes()!=changes{health.revision.fetch_add(1,Ordering::Release);}crate::alerts::insert(&mut db,&node,*record,days)})();
                            match result{Ok(evicted)=>{health.alerts_evicted.fetch_add(evicted as u64,Ordering::Relaxed);health.alerts_revision.fetch_add(1,Ordering::Release);*health.alerts_error.lock().unwrap()=None;},Err(error)=>{health.alerts_skipped.fetch_add(1,Ordering::Relaxed);*health.alerts_error.lock().unwrap()=Some(format!("Alert could not be saved: {error:#}"));}}
                        }
                        Command::AlertChange(change,reply)=>{let result=crate::alerts::change(&db,change).map_err(|error|format!("{error:#}"));if result.is_ok(){health.alerts_revision.fetch_add(1,Ordering::Release);}let _=reply.send(result);}

                        Command::Events { node, batch } => {
                            let mut written = 0;
                            for chunk in batch.events.chunks(128) {
                                if let Err(e) = insert(&mut db, &node, chunk, &batch.reason) {
                                    health.record_loss(LossReason::Storage, batch.len() - written);
                                    *health.error.lock().unwrap() =
                                        Some(format!("History could not be saved: {e:#}"));
                                    break;
                                }
                                written += chunk.len();
                                let mut catalogue_error=None;
                                if batch.reason=="observed" {for record in chunk {if let Some(rule)=&record.observed_rule && let Err(error)=rules::confirm_observation(&mut db,&node,rule){catalogue_error=Some(format!("Daemon observation could not update the rules catalogue: {error:#}"));}}}
                                health
                                    .stored
                                    .fetch_add(chunk.len() as u64, Ordering::Relaxed);
                                health.revision.fetch_add(1, Ordering::Release);
                                // These rows are already committed. A maintenance failure
                                // is an error, but does not mean that their write was lost.
                                let maintenance_error = maintain(&mut db, days, mib)
                                    .err()
                                    .map(|e| format!("History cleanup failed: {e:#}"));
                                *health.error.lock().unwrap() = maintenance_error.or(catalogue_error);
                            }
                        }
                        Command::Query(query, reply) => {
                            let _ =
                                reply.send(query_page(&db, &query).map_err(|e| format!("{e:#}")));
                        }
                        Command::Detail(id,reply)=>{let _=reply.send(query_detail(&db,id).map_err(|e|format!("{e:#}")));}
                        Command::ReplaceRules { session, node, rules, _permit } => {
                            if !health.catalogues.current(session,&node){continue;}
                            match rules::replace(&mut db, &node, &rules) {
                                Ok(()) => { health.catalogues.saved(session,&node); health.revision.fetch_add(1, Ordering::Release); }
                                Err(e) => { *health.error.lock().unwrap() = Some(format!("Rules catalogue could not be saved: {e:#}")); }
                            }
                        }
                        Command::QueryRules(query, reply) => {
                            let _ = reply.send(rules::query(&db, &query).map_err(|e| format!("{e:#}")));
                        }
                        Command::ApplyRule { session,node, action, rule, reply, _permit } => {
                            if !health.catalogues.current(session,&node){let _=reply.send(Err("Daemon changed before its catalogue could be saved".into()));continue;}
                            let result = rules::apply(&mut db, &node, action, &rule).map_err(|e| format!("Daemon confirmed the change, but its local catalogue could not be saved: {e:#}"));
                            if result.is_ok() { health.revision.fetch_add(1, Ordering::Release); }
                            else { health.catalogues.invalidate(session); }
                            let _ = reply.send(result);
                        }
                        Command::ProposeRule{session,node,rule,_permit}=>{
                            if health.catalogues.current(session,&node)&&health.catalogues.ready(session) {
                                match rules::propose(&mut db,&node,&rule){Ok(())=>{health.revision.fetch_add(1,Ordering::Release);},Err(error)=>{*health.error.lock().unwrap()=Some(format!("Popup reply was sent, but its pending rule could not be cached: {error:#}"));}}
                            }
                        }
                        Command::PausePreference{session,node,remember,running,expected,reply}=>{
                            let result=if health.catalogues.current(session,&node) && health.catalogues.ready(session) && !health.catalogues.uncertain(session) {
                                crate::nodes::interception::set(&mut db,session,&node,remember,running,expected).map_err(|error|format!("{error:#}"))
                            }else{Err("A fresh confirmed node connection is required to save pause preferences".into())};
                            if result.is_ok(){health.interception.synchronize(&db);health.nodes_revision.fetch_add(1,Ordering::Release);}
                            let _=reply.send(result);
                        }
                        Command::SaveNode{session,node,saved,_permit}=>{
                            if health.catalogues.current(session,&node){match crate::nodes::save(&mut db,session,&node,*saved,&health.catalogues.names()){Ok(())=>{health.interception.synchronize(&db);health.nodes_revision.fetch_add(1,Ordering::Release);},Err(error)=>{*health.error.lock().unwrap()=Some(format!("Node metadata could not be saved: {error:#}"));}}}
                        }
                        Command::ApplyNode{session,node,change,reply,_permit}=>{
                            let result=if health.catalogues.current(session,&node){crate::nodes::apply(&mut db,&node,&change).map_err(|error|format!("Daemon confirmed the command but its local node cache failed: {error:#}"))}else{Err("Daemon changed before the confirmed node state could be saved; reconnect".into())};
                            if result.is_ok(){health.interception.synchronize(&db);health.nodes_revision.fetch_add(1,Ordering::Release);}else{health.catalogues.mark_uncertain(session);}
                            let _=reply.send(result);
                        }
                        Command::StatisticsReset(scope,reply)=>{
                            let result=crate::statistics::reset(&mut db,&scope).map_err(|error|format!("{error:#}"));
                            if result.is_ok(){health.statistics_revision.fetch_add(1,Ordering::Release);health.revision.fetch_add(1,Ordering::Release);}
                            let _=reply.send(result);
                        }
                        Command::StoragePreview(candidate,expected,reply)=>{let _=reply.send(storage::preview(&db,candidate,expected).map_err(|e|format!("{e:#}")));}
                        Command::StorageApply(candidate,expected,reply)=>{
                            let result=storage::apply(&mut db,candidate,expected).map_err(|e|format!("{e:#}"));
                            if let Ok(applied)=&result {days=applied.snapshot.config.days;mib=applied.snapshot.config.mib;storage_worker.publish(applied.snapshot);health.revision.fetch_add(1,Ordering::Release);health.alerts_revision.fetch_add(1,Ordering::Release);health.statistics_revision.fetch_add(1,Ordering::Release);next_cleanup=std::time::Instant::now()+std::time::Duration::from_secs(u64::from(applied.snapshot.config.cleanup_minutes)*60);}
                            let _=reply.send(result);
                        }
                        Command::Fence(reply) => {let _=reply.send(());}
                        Command::Barrier(reply) => {
                            let _ = db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
                            let _ = reply.send(());
                        }
                    } }
                    if std::time::Instant::now()>=next_cleanup {
                        let changes=db.total_changes();
                        if let Err(e)=maintain(&mut db,days,mib).and_then(|_|storage::cap(&db,mib).map(|_|())) {*health.error.lock().unwrap()=Some(format!("Scheduled history cleanup failed: {e:#}"));}
                        if changes!=db.total_changes(){health.revision.fetch_add(1,Ordering::Release);health.alerts_revision.fetch_add(1,Ordering::Release);health.statistics_revision.fetch_add(1,Ordering::Release);}
                        next_cleanup=std::time::Instant::now()+std::time::Duration::from_secs(u64::from(storage_worker.snapshot().config.cleanup_minutes)*60);
                    }
                    if health.indexing.load(Ordering::Acquire) {
                        match backfill_chunk(&mut db,&mut indexed_after).and_then(|more| {if more {maintain(&mut db,days,mib)?;} Ok(more)}) {
                            Ok(true)=>{health.revision.fetch_add(1,Ordering::Release);},
                            Ok(false)=>{health.indexing.store(false,Ordering::Release);health.revision.fetch_add(1,Ordering::Release);},
                            Err(error)=>{*health.metadata_error.lock().unwrap()=Some(format!("Older history metadata could not be indexed: {error:#}. Restart to retry; basic history and permission controls remain available."));health.indexing.store(false,Ordering::Release);}
                        }
                    }
                }
                let _ = db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
            })?;
        Ok(Self {
            history,
            join: Some(join),
        })
    }

    /// Stop after the server and every history handle have been dropped.
    pub fn join(mut self) {
        drop(self.history);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl History {
    pub(crate) fn alert(&self, node: String, alert: crate::protocol::Alert) {
        let result = (|| -> Result<()> {
            let mut permit = self
                .budget
                .clone()
                .try_acquire_many_owned(crate::alerts::PREPARE_BYTES as u32)
                .map_err(|_| anyhow::anyhow!("Shared ingestion budget is full"))?;
            let record = crate::alerts::prepare(alert)?;
            let needed = record.memory_bytes() + node.capacity();
            anyhow::ensure!(
                needed <= crate::alerts::PREPARE_BYTES,
                "Alert allocations exceed preparation budget"
            );
            drop(permit.split(permit.num_permits() - needed));
            self.tx
                .try_send(Command::Alert {
                    node,
                    record: Box::new(record),
                    _permit: permit,
                })
                .map_err(|_| anyhow::anyhow!("Alert writer queue is full or stopped"))?;
            Ok(())
        })();
        if let Err(error) = result {
            self.health.alerts_skipped.fetch_add(1, Ordering::Relaxed);
            *self.health.alerts_error.lock().unwrap() =
                Some(format!("Alert not recorded: {error:#}"));
        }
    }
    pub fn change_alert(
        &self,
        change: crate::alerts::Change,
    ) -> Result<oneshot::Receiver<std::result::Result<usize, String>>, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .try_send(Command::AlertChange(change, tx))
            .map_err(|_| "Alert writer queue is full; retry".to_string())?;
        Ok(rx)
    }

    pub(crate) fn prepare_statistics(
        &self,
        stats: crate::protocol::Statistics,
    ) -> Option<crate::statistics::Prepared> {
        match crate::statistics::prepare(stats, self.budget.clone()) {
            Ok(sample) => Some(sample),
            Err(error) => {
                self.health
                    .statistics_skipped
                    .fetch_add(1, Ordering::Relaxed);
                *self.health.statistics_error.lock().unwrap() =
                    Some(format!("Statistics report skipped: {error:#}"));
                None
            }
        }
    }
    pub(crate) fn enqueue_statistics(&self, node: String, sample: crate::statistics::Prepared) {
        let node = bounded(&node, 128);
        let mut queue = self.statistics.lock().unwrap();
        if let Some((_, previous)) = queue.iter_mut().find(|(name, _)| name == &node) {
            *previous = sample;
            self.health
                .statistics_skipped
                .fetch_add(1, Ordering::Relaxed);
        } else if queue.len() < crate::nodes::MAX_NODES {
            queue.push_back((node, sample));
        } else {
            self.health
                .statistics_skipped
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn reset_statistics(
        &self,
        scope: crate::statistics::Reset,
    ) -> Option<oneshot::Receiver<std::result::Result<crate::statistics::ResetResult, String>>>
    {
        let (tx, rx) = oneshot::channel();
        self.tx.try_send(Command::StatisticsReset(scope, tx)).ok()?;
        Some(rx)
    }
    pub(crate) fn propose_rule(&self, session: u64, node: String, rule: Rule) -> bool {
        let bytes = rules::memory_bytes(&rule) + 128;
        let Ok(permit) = self
            .budget
            .clone()
            .try_acquire_many_owned(bytes.min(u32::MAX as usize) as u32)
        else {
            return false;
        };
        self.tx
            .try_send(Command::ProposeRule {
                session,
                node,
                rule: Box::new(rule),
                _permit: permit,
            })
            .is_ok()
    }
    pub(crate) fn save_node(
        &self,
        session: u64,
        node: String,
        saved: crate::nodes::Saved,
    ) -> std::result::Result<(), String> {
        let permit = self
            .budget
            .clone()
            .try_acquire_many_owned(saved.bytes() as u32)
            .map_err(|_| "Node cache ingestion is busy")?;
        self.tx
            .try_send(Command::SaveNode {
                session,
                node,
                saved: Box::new(saved),
                _permit: permit,
            })
            .map_err(|_| "Node cache worker is busy; reconnect".into())
    }
    pub(crate) fn apply_node(
        &self,
        session: u64,
        node: String,
        change: crate::nodes::Change,
        reply: oneshot::Sender<std::result::Result<(), String>>,
    ) {
        let bytes = change.bytes();
        let Ok(permit) = self.budget.clone().try_acquire_many_owned(bytes as u32) else {
            self.health.catalogues.mark_uncertain(session);
            let _ = reply.send(Err(
                "Node command was confirmed but the local cache is busy; reconnect".into(),
            ));
            return;
        };
        if let Err(error) = self.tx.try_send(Command::ApplyNode {
            session,
            node,
            change,
            reply,
            _permit: permit,
        }) {
            self.health.catalogues.mark_uncertain(session);
            let command = match error {
                mpsc::TrySendError::Full(command) | mpsc::TrySendError::Disconnected(command) => {
                    command
                }
            };
            if let Command::ApplyNode { reply, .. } = command {
                let _ = reply.send(Err(
                    "Node command was confirmed but its local cache is unavailable; reconnect"
                        .into(),
                ));
            }
        }
    }
    pub fn remember_interception(
        &self,
        session: u64,
        node: String,
        remember: bool,
        running: bool,
        expected: Option<bool>,
    ) -> Result<oneshot::Receiver<std::result::Result<(), String>>, String> {
        if node.is_empty() || node.len() > 128 {
            return Err("Invalid node name".into());
        }
        let (tx, rx) = oneshot::channel();
        self.tx
            .try_send(Command::PausePreference {
                session,
                node,
                remember,
                running,
                expected,
                reply: tx,
            })
            .map_err(|_| "Pause preference worker is busy; try again".to_string())?;
        Ok(rx)
    }
    pub fn database_path(&self) -> &Path {
        &self.database
    }
    pub(crate) fn replace_rules(&self, session: u64, node: String, rules: Vec<Rule>) {
        let result = (|| -> std::result::Result<(), String> {
            if !self.health.catalogues.current(session, &node) {
                self.health.catalogues.begin(session, &node)?;
            }
            if rules.len() > rules::MAX_RULES
                || rules.iter().map(Message::encoded_len).sum::<usize>() > rules::MAX_CATALOG_BYTES
            {
                return Err("Daemon rule inventory exceeds the supported size".into());
            }
            for rule in &rules {
                rules::valid(rule)?;
            }
            let bytes = rules.iter().map(rules::memory_bytes).sum::<usize>()
                + 128
                + (rules.capacity() - rules.len()) * std::mem::size_of::<Rule>();
            let permit = self
                .budget
                .clone()
                .try_acquire_many_owned(
                    bytes
                        .try_into()
                        .map_err(|_| "Rule inventory is too large")?,
                )
                .map_err(|_| "History worker is busy; reconnect to reload the rule inventory")?;
            self.tx
                .try_send(Command::ReplaceRules {
                    session,
                    node,
                    rules,
                    _permit: permit,
                })
                .map_err(|_| "History worker is busy; reconnect to reload the rule inventory")?;
            Ok(())
        })();
        if let Err(error) = result {
            *self.health.error.lock().unwrap() = Some(error);
        }
    }

    pub fn query_rules(
        &self,
        mut query: rules::Query,
    ) -> Option<oneshot::Receiver<std::result::Result<rules::Page, String>>> {
        let (tx, rx) = oneshot::channel();
        if let Err(error) = rules::query::Predicate::new(&query) {
            let _ = tx.send(Err(error.to_string()));
            return Some(rx);
        }
        query.filters.shrink_to_fit();
        query.logic.groups.shrink_to_fit();
        self.tx.try_send(Command::QueryRules(query, tx)).ok()?;
        Some(rx)
    }

    pub(crate) fn apply_rule(
        &self,
        session: u64,
        node: String,
        action: Action,
        rule: Rule,
        reply: oneshot::Sender<std::result::Result<(), String>>,
    ) {
        let permit = self
            .budget
            .clone()
            .try_acquire_many_owned((rules::memory_bytes(&rule) + 128) as u32);
        let Ok(permit) = permit else {
            self.health.catalogues.invalidate(session);
            let _ = reply.send(Err("Daemon confirmed the change, but the local catalogue is busy; reconnect to reload it".into()));
            return;
        };
        if let Err(error) = self.tx.try_send(Command::ApplyRule {
            session,
            node,
            action,
            rule: Box::new(rule),
            reply,
            _permit: permit,
        }) {
            self.health.catalogues.invalidate(session);
            let command = match error {
                mpsc::TrySendError::Full(command) | mpsc::TrySendError::Disconnected(command) => {
                    command
                }
            };
            if let Command::ApplyRule { reply, .. } = command {
                let _ = reply.send(Err("Daemon confirmed the change, but the local catalogue is unavailable; reconnect to reload it".into()));
            }
        }
    }

    pub fn ingest(&self, node: String, events: Vec<Event>, reason: &str) {
        if let Some(batch) = self.prepare(events, reason) {
            self.enqueue(node, batch);
        }
    }

    // Early connection reports share exactly the same budget as queued writes.
    pub(crate) fn prepare(&self, events: Vec<Event>, reason: &str) -> Option<Batch> {
        if events.is_empty() {
            return None;
        }
        let count = events.len();
        if count > MAX_BATCH_EVENTS {
            self.health.record_loss(LossReason::EventLimit, count);
            return None;
        }
        let reason = bounded(reason, 32);
        let base = count * std::mem::size_of::<Record>() + 128 + reason.capacity();
        if base > QUEUE_BYTES {
            self.health.record_loss(LossReason::EventLimit, count);
            return None;
        }
        let Ok(mut permit) = self
            .budget
            .clone()
            .try_acquire_many_owned(base.max(1) as u32)
        else {
            self.health.record_loss(LossReason::QueueFull, count);
            return None;
        };
        let mut bytes = base;
        let mut records = Vec::with_capacity(count);
        for event in events {
            let mut record = Record::from_event(event);
            if reason != "observed" {
                record.observed_rule = None;
            }
            let extra = record.memory_bytes() - std::mem::size_of::<Record>();
            bytes += extra;
            if bytes > QUEUE_BYTES {
                self.health.record_loss(LossReason::EventLimit, count);
                return None;
            }
            let Ok(extra_permit) = self.budget.clone().try_acquire_many_owned(extra as u32) else {
                self.health.record_loss(LossReason::QueueFull, count);
                return None;
            };
            permit.merge(extra_permit);
            records.push(record);
        }
        Some(Batch {
            events: records,
            reason,
            _permit: permit,
        })
    }

    pub(crate) fn enqueue(&self, node: String, batch: Batch) {
        let count = batch.len();
        match self.tx.try_send(Command::Events {
            node: bounded(&node, 128),
            batch,
        }) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                self.health.record_loss(LossReason::QueueFull, count)
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.health.record_loss(LossReason::Storage, count);
                *self.health.error.lock().unwrap() = Some("History worker stopped".into());
            }
        }
    }

    pub fn query(
        &self,
        mut query: Query,
    ) -> Option<oneshot::Receiver<std::result::Result<Page, String>>> {
        query.search = bounded(&query.search, 256);
        let (tx, rx) = oneshot::channel();
        if let Err(error) = query::sql(&query, 0, 1) {
            let _ = tx.send(Err(error.to_string()));
            return Some(rx);
        }
        query.filters.shrink_to_fit();
        query.logic.groups.shrink_to_fit();
        self.tx.try_send(Command::Query(Box::new(query), tx)).ok()?;
        Some(rx)
    }
    pub fn detail(
        &self,
        id: i64,
    ) -> Option<oneshot::Receiver<std::result::Result<Option<details::Detail>, String>>> {
        let (tx, rx) = oneshot::channel();
        self.tx.try_send(Command::Detail(id, tx)).ok()?;
        Some(rx)
    }

    pub fn preview_storage(
        &self,
        candidate: storage::Config,
        expected: u64,
    ) -> Option<oneshot::Receiver<Result<storage::Preview, String>>> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .try_send(Command::StoragePreview(candidate, expected, tx))
            .ok()?;
        Some(rx)
    }
    pub fn apply_storage(
        &self,
        candidate: storage::Config,
        expected: u64,
    ) -> Option<oneshot::Receiver<Result<storage::Applied, String>>> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .try_send(Command::StorageApply(candidate, expected, tx))
            .ok()?;
        Some(rx)
    }
    pub async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        // Used for shutdown/tests, never in an RPC or UI callback.
        let sender = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            sender
                .send(Command::Barrier(tx))
                .map_err(|_| anyhow::anyhow!("History worker stopped before flush"))
        })
        .await??;
        rx.await?;
        Ok(())
    }
    pub(crate) fn fence(&self) -> Result<oneshot::Receiver<()>, String> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .try_send(Command::Fence(tx))
            .map_err(|_| "Rule cache worker is busy; retry the copy".to_string())?;
        Ok(rx)
    }
}

fn open(path: &Path, mib: u32) -> Result<Connection> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    drop(file);
    let mut db = Connection::open(path).context("open history database")?;
    db.busy_timeout(std::time::Duration::from_millis(500))?;
    // Reserve one quarter of the disk target for WAL and a write batch.
    db.execute_batch(&format!(
        "PRAGMA foreign_keys=ON; PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
        PRAGMA cache_size=-2048; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;
        PRAGMA max_page_count={}; PRAGMA journal_size_limit=1048576;",
        u64::from(mib) * 1024 * 1024 / 4096 * 3 / 4
    ))?;
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    anyhow::ensure!(
        version <= 11,
        "History schema {version} is newer than this application supports"
    );
    let tx = db.transaction()?;
    tx.execute_batch("
        CREATE TABLE IF NOT EXISTS events (
          id INTEGER PRIMARY KEY AUTOINCREMENT, node TEXT NOT NULL, fingerprint BLOB NOT NULL,
          unixnano INTEGER NOT NULL, received INTEGER NOT NULL, action TEXT NOT NULL,
          protocol TEXT NOT NULL, process TEXT NOT NULL, destination TEXT NOT NULL,
          port INTEGER NOT NULL, reason TEXT NOT NULL, stored_bytes INTEGER NOT NULL, metadata TEXT,
          UNIQUE(node,fingerprint));
        CREATE INDEX IF NOT EXISTS events_time ON events(received,id);
        CREATE INDEX IF NOT EXISTS events_process ON events(process,id);
        CREATE INDEX IF NOT EXISTS events_action ON events(action,id);
        CREATE INDEX IF NOT EXISTS events_node ON events(node,id);
        CREATE INDEX IF NOT EXISTS events_destination ON events(destination,id);
        CREATE TABLE IF NOT EXISTS usage (id INTEGER PRIMARY KEY CHECK(id=1), bytes INTEGER NOT NULL);
        INSERT OR IGNORE INTO usage VALUES(1,0);
        CREATE TRIGGER IF NOT EXISTS event_added AFTER INSERT ON events BEGIN UPDATE usage SET bytes=bytes+new.stored_bytes WHERE id=1; END;
        CREATE TRIGGER IF NOT EXISTS event_removed AFTER DELETE ON events BEGIN UPDATE usage SET bytes=bytes-old.stored_bytes WHERE id=1; END;
        CREATE TABLE IF NOT EXISTS event_details (
          event_id INTEGER PRIMARY KEY REFERENCES events(id) ON DELETE CASCADE,
          payload BLOB NOT NULL CHECK(length(payload)<=24576), truncated INTEGER NOT NULL);
        ")?;
    rules::schema(&tx)?;
    crate::nodes::schema(&tx)?;
    let has_metadata: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('events') WHERE name='metadata')",
        [],
        |row| row.get(0),
    )?;
    if !has_metadata {
        tx.execute_batch("ALTER TABLE events ADD COLUMN metadata TEXT;")?;
    }
    crate::statistics::schema(&tx)?;
    crate::alerts::schema(&tx)?;
    storage::schema(&tx)?;
    tx.pragma_update(None, "user_version", 11)?;
    tx.commit()?;
    Ok(db)
}

// NULL/metadata schema markers let interrupted indexing resume on next launch.
// Runs in the worker between commands, not before the RPC listener can start.
pub(crate) fn backfill_chunk(db: &mut Connection, after: &mut i64) -> Result<bool> {
    metadata::backfill_chunk(db, after)
}

fn insert(db: &mut Connection, node: &str, events: &[Record], reason: &str) -> Result<()> {
    let tx = db.transaction()?;
    {
        let mut statement=tx.prepare_cached("INSERT OR IGNORE INTO events(node,fingerprint,unixnano,received,action,protocol,process,destination,port,reason,stored_bytes,metadata) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")?;
        for event in events {
            let size = event.memory_bytes() + node.len() + reason.len() + 256;
            let changed = statement.execute(params![
                node,
                event.fingerprint.as_slice(),
                event.unixnano,
                now_seconds(),
                event.action,
                event.protocol,
                event.process,
                event.destination,
                event.port,
                reason,
                size as i64,
                event.metadata
            ])?;
            if changed == 1 {
                tx.execute(
                    "INSERT INTO event_details(event_id,payload,truncated) VALUES(?,?,?)",
                    params![tx.last_insert_rowid(), event.detail, event.truncated],
                )?;
                // Only actual daemon telemetry advances the local observation clock.
                // The INSERT SELECT can create no row outside the bounded catalogue.
                if reason == "observed"
                    && let Some(name) = &event.observed_name
                {
                    tx.execute("INSERT INTO rule_observations(node,name,received) SELECT node,name,?3 FROM application_rules WHERE node=?1 AND name=?2 UNION SELECT node,name,?3 FROM prompt_rules WHERE node=?1 AND name=?2 ON CONFLICT(node,name) DO UPDATE SET received=excluded.received", params![node,name,now_seconds()])?;
                    rules::stamp_observation(&tx, node, name)?;
                }
            }
        }
    }
    tx.commit()?;
    Ok(())
}

fn maintain(db: &mut Connection, days: u32, mib: u32) -> Result<()> {
    maintain_reserved(db, days, mib, 0)
}
fn maintain_reserved(db: &mut Connection, days: u32, mib: u32, reserved: u64) -> Result<()> {
    prune_reserved(db, days, mib, reserved)?;
    db.execute_batch("PRAGMA incremental_vacuum(64); PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}
fn prune_reserved(db: &Connection, days: u32, mib: u32, reserved: u64) -> Result<()> {
    let cutoff_nanos = now_nanos() - i64::from(days) * 86400 * 1_000_000_000;
    db.execute(
        "DELETE FROM statistics_counts WHERE updated<?",
        [cutoff_nanos],
    )?;
    db.execute(
        "DELETE FROM statistics_nodes WHERE updated<?",
        [cutoff_nanos],
    )?;
    crate::alerts::prune(db, days)?;
    let cutoff = now_seconds() - i64::from(days) * 86400;
    let limit = (u64::from(mib) * 1024 * 1024 / 4).saturating_sub(
        crate::statistics::storage_bytes(db)?
            .saturating_add(crate::alerts::storage_bytes(db)?)
            .saturating_add(reserved),
    ) as i64;
    loop {
        let deleted = db.execute(
            "DELETE FROM events WHERE id IN (SELECT id FROM events WHERE received < ? LIMIT 512)",
            [cutoff],
        )?;
        let bytes: i64 = db.query_row("SELECT bytes FROM usage WHERE id=1", [], |r| r.get(0))?;
        if bytes > limit {
            // Delete only the prefix needed to meet the budget. A fixed 512-row
            // eviction can wipe an entire small history now that details vary
            // in size. The scan and each deletion remain capped at 512 records.
            let mut removed = 0;
            let mut cutoff = None;
            {
                let mut statement =
                    db.prepare_cached("SELECT id,stored_bytes FROM events ORDER BY id LIMIT 512")?;
                let mut rows = statement.query([])?;
                while let Some(row) = rows.next()? {
                    cutoff = Some(row.get::<_, i64>(0)?);
                    removed += row.get::<_, i64>(1)?;
                    if removed >= bytes - limit {
                        break;
                    }
                }
            }
            if let Some(cutoff) = cutoff {
                db.execute("DELETE FROM events WHERE id<=?", [cutoff])?;
            } else {
                break;
            }
        } else if deleted == 0 {
            break;
        }
    }
    Ok(())
}

pub(crate) fn query_page(db: &Connection, query: &Query) -> Result<Page> {
    metadata::ensure_ready(db, query)?;
    let ceiling = match query.ceiling {
        Some(id) => id,
        None => db.query_row("SELECT coalesce(max(id),0) FROM events", [], |row| {
            row.get(0)
        })?,
    };
    let (sql, values) = query::sql(query, ceiling, PAGE_SIZE + 1)?;
    let mut statement = db.prepare_cached(&sql)?;
    let mut records = statement.query(rusqlite::params_from_iter(values))?;
    let mut rows = Vec::new();
    let mut cursor = None;
    while let Some(record) = records.next()? {
        if rows.len() == PAGE_SIZE {
            return Ok(Page {
                rows: Arc::new(rows),
                has_more: true,
                next_cursor: cursor,
                ceiling,
            });
        }
        let row = read_row(record)?;
        cursor = Some(query::Cursor {
            id: row.id,
            value: record.get(10)?,
        });
        rows.push(row);
    }
    Ok(Page {
        rows: Arc::new(rows),
        has_more: false,
        next_cursor: cursor,
        ceiling,
    })
}

pub(crate) fn read_row(record: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let json: String = record.get(9)?;
    let metadata = serde_json::from_str(&json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(Row {
        id: record.get(0)?,
        unixnano: record.get(1)?,
        action: record.get(2)?,
        protocol: record.get(3)?,
        process: record.get(4)?,
        destination: record.get(5)?,
        port: record.get(6)?,
        reason: record.get(7)?,
        node: record.get(8)?,
        metadata,
    })
}

fn query_detail(db: &Connection, id: i64) -> Result<Option<details::Detail>> {
    use rusqlite::OptionalExtension;
    let detail=db.query_row("SELECT e.id,e.unixnano,e.action,e.protocol,e.process,e.destination,e.port,e.reason,e.node,coalesce(e.metadata,'{}'),d.payload,d.truncated FROM events e LEFT JOIN event_details d ON d.event_id=e.id WHERE e.id=?",[id],|r|{
        let row=read_row(r)?;
        let wire:Option<Vec<u8>>=r.get(10)?;
        Ok((row,wire,r.get::<_,Option<u32>>(11)?.unwrap_or(0)))
    }).optional()?;
    detail
        .map(|(row, wire, truncated)| {
            Ok(details::Detail {
                row,
                event: wire
                    .map(|wire| Event::decode(wire.as_slice()))
                    .transpose()?,
                truncated,
            })
        })
        .transpose()
}

pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn bounded(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

pub fn now_nanos() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol;

    #[tokio::test]
    async fn partial_write_failure_counts_only_unwritten_events_and_reports_recovery() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("history.sqlite3");
        let worker = Worker::start(&path, 7, 16)?;
        let db = Connection::open(&path)?;
        db.execute_batch("CREATE TRIGGER fail_second_chunk BEFORE INSERT ON events WHEN new.unixnano >= 128 BEGIN SELECT RAISE(ABORT, 'test storage failure'); END;")?;
        worker.history.ingest(
            "local".into(),
            (0..130)
                .map(|unixnano| Event {
                    unixnano,
                    ..Default::default()
                })
                .collect(),
            "observed",
        );
        worker.history.flush().await?;
        let health = &worker.history.health;
        assert_eq!(
            db.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))?,
            128
        );
        assert_eq!(health.stored.load(Ordering::Relaxed), 128);
        assert_eq!(health.dropped.load(Ordering::Relaxed), 2);
        assert_eq!(health.storage_failed.load(Ordering::Relaxed), 2);
        assert!(
            health
                .warning()
                .unwrap()
                .contains("2 because they couldn't be saved")
        );
        assert!(
            health
                .error
                .lock()
                .unwrap()
                .as_deref()
                .unwrap()
                .contains("test storage failure")
        );
        db.execute_batch("DROP TRIGGER fail_second_chunk")?;
        worker.history.ingest(
            "local".into(),
            vec![Event {
                unixnano: 999,
                ..Default::default()
            }],
            "observed",
        );
        worker.history.flush().await?;
        assert_eq!(health.stored.load(Ordering::Relaxed), 129);
        assert_eq!(health.dropped.load(Ordering::Relaxed), 2);
        assert!(
            health.error.lock().unwrap().is_none(),
            "recovery clears the active storage error, not the past loss"
        );
        drop(db);
        worker.join();
        Ok(())
    }

    #[test]
    fn prepared_batches_share_the_queue_memory_budget_and_release_it_on_drop() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let worker = Worker::start(&dir.path().join("history.sqlite3"), 7, 16)?;
        let events: Vec<_> = (0..256)
            .map(|unixnano| Event {
                unixnano,
                connection: Some(protocol::Connection {
                    process_path: "p".repeat(4096),
                    dst_host: "h".repeat(1024),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        let mut batches = Vec::new();
        while let Some(batch) = worker.history.prepare(events.clone(), "observed") {
            batches.push(batch);
        }
        assert!(!batches.is_empty() && batches.len() < 8);
        assert!(worker.history.budget.available_permits() < QUEUE_BYTES);
        assert_eq!(
            worker.history.health.queue_full.load(Ordering::Relaxed),
            256
        );
        drop(batches);
        assert_eq!(worker.history.budget.available_permits(), QUEUE_BYTES);
        assert!(worker.history.prepare(events, "observed").is_some());
        worker.join();
        Ok(())
    }

    #[test]
    fn durable_deduplicated_paged_and_filtered() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("history.sqlite3");
        let mut db = open(&path, 16)?;
        let events: Vec<_> = (1..=230)
            .map(|i| Event {
                unixnano: i,
                connection: Some(protocol::Connection {
                    process_path: if i % 2 == 0 {
                        "/usr/bin/test%"
                    } else {
                        "/usr/bin/other"
                    }
                    .into(),
                    dst_ip: "1.2.3.4".into(),
                    ..Default::default()
                }),
                rule: Some(protocol::Rule {
                    action: "deny".into(),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        let records: Vec<_> = events.clone().into_iter().map(Record::from_event).collect();
        insert(&mut db, "local", &records, "observed")?;
        let records: Vec<_> = events.clone().into_iter().map(Record::from_event).collect();
        insert(&mut db, "local", &records, "observed")?;
        drop(db);
        let mut db = open(&path, 16)?;
        let page = query_page(&db, &Query::default())?;
        assert_eq!(page.rows.len(), 200);
        assert!(page.has_more);
        let older = query_page(
            &db,
            &Query {
                before: page.rows.last().map(|r| r.id),
                ..Default::default()
            },
        )?;
        assert_eq!(older.rows.len(), 30);
        assert!(!older.has_more);
        assert_eq!(
            query_page(
                &db,
                &Query {
                    search: "test%".into(),
                    ..Default::default()
                }
            )?
            .rows
            .len(),
            115
        );
        assert!(
            query_page(
                &db,
                &Query {
                    action: "allow".into(),
                    ..Default::default()
                }
            )?
            .rows
            .is_empty()
        );
        db.execute("UPDATE events SET received=0", [])?;
        maintain(&mut db, 1, 16)?;
        assert!(query_page(&db, &Query::default())?.rows.is_empty());
        Ok(())
    }

    #[test]
    fn retention_keeps_large_records_and_sqlite_files_bounded() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("history.sqlite3");
        let mut db = open(&path, 16)?;
        for batch in 0..24 {
            let records: Vec<_> = (0..128)
                .map(|i| {
                    Record::from_event(Event {
                        unixnano: batch * 128 + i,
                        connection: Some(protocol::Connection {
                            process_path: "x".repeat(4096),
                            dst_host: "h".repeat(1024),
                            process_env: [("SECRET".into(), "not persisted".into())].into(),
                            ..Default::default()
                        }),
                        rule: Some(protocol::Rule {
                            action: "deny".into(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    })
                })
                .collect();
            insert(&mut db, "fixture", &records, "observed")?;
            maintain(&mut db, 7, 16)?;
        }
        let retained: i64 = db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
        assert!(retained > 0 && retained < 3072);
        let bytes: i64 = db.query_row("SELECT bytes FROM usage", [], |r| r.get(0))?;
        assert!(bytes <= 4 * 1024 * 1024);
        let files_bytes: u64 = std::fs::read_dir(dir.path())?
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum();
        assert!(
            files_bytes <= 16 * 1024 * 1024,
            "SQLite files used {files_bytes} bytes"
        );
        assert_eq!(std::fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
        Ok(())
    }
    #[test]
    fn legacy_history_migrates_without_inventing_missing_details_and_cleans_children() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("history.sqlite3");
        let db = open(&path, 16)?;
        db.execute_batch("DROP TABLE event_details; PRAGMA user_version=1;")?;
        db.execute("INSERT INTO events(node,fingerprint,unixnano,received,action,protocol,process,destination,port,reason,stored_bytes) VALUES('old',zeroblob(32),1,?,'deny','tcp','/bin/old','old.example',443,'observed',512)",[now_seconds()])?;
        drop(db);
        let mut db = open(&path, 16)?;
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?,
            11
        );
        let old = query_detail(&db, 1)?.unwrap();
        assert!(old.event.is_none());
        assert_eq!(old.row.node, "old");
        assert!(old.draft(details::DraftKind::Application).is_err());
        insert(
            &mut db,
            "new",
            &[Record::from_event(Event {
                unixnano: 2,
                ..Default::default()
            })],
            "observed",
        )?;
        assert_eq!(
            db.query_row("SELECT count(*) FROM event_details", [], |row| row
                .get::<_, i64>(0))?,
            1
        );
        db.execute("UPDATE events SET received=0", [])?;
        maintain(&mut db, 1, 16)?;
        assert_eq!(
            db.query_row("SELECT count(*) FROM event_details", [], |row| row
                .get::<_, i64>(0))?,
            0
        );
        assert_eq!(
            db.query_row("SELECT bytes FROM usage", [], |row| row.get::<_, i64>(0))?,
            0
        );
        assert!(query_detail(&db, 1)?.is_none());
        Ok(())
    }
}
