//! Disk-backed rule pages and bounded, acknowledged daemon commands.
pub mod checksums;
pub mod distribute;
pub mod document;
pub mod export_nodes;
pub mod transfer;
use crate::{
    history::History,
    protocol::{self, Action, Notification, NotificationReply, Rule},
};
use prost::Message;
use rusqlite::{Connection, params};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

pub const MAX_RULE_BYTES: usize = 64 * 1024;
pub const MAX_RULES: usize = 10_000;
pub const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
const MAX_COMMANDS: usize = 16;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Default)]
pub struct Query {
    pub node: String,
    pub after: String,
    pub search: String,
    pub temporary: Option<bool>,
}

#[derive(Default)]
pub struct Page {
    pub rules: Vec<Rule>,
    pub has_more: bool,
    pub awaiting: std::collections::BTreeSet<String>,
}

pub(crate) fn schema(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS application_rules (
        node TEXT NOT NULL, name TEXT NOT NULL, wire BLOB NOT NULL,
        process TEXT NOT NULL, description TEXT NOT NULL, duration TEXT NOT NULL,
        PRIMARY KEY(node,name));
        CREATE TABLE IF NOT EXISTS prompt_rules (
        node TEXT NOT NULL, name TEXT NOT NULL, wire BLOB NOT NULL,
        process TEXT NOT NULL, description TEXT NOT NULL, duration TEXT NOT NULL,
        PRIMARY KEY(node,name));",
    )
}

pub(crate) fn valid(rule: &Rule) -> Result<(), String> {
    if rule.name.is_empty() || rule.name.len() > 1024 {
        return Err("Rule name must contain 1–1024 bytes".into());
    }
    if rule.name.contains(['/', '\0']) {
        return Err("Rule names cannot contain a slash or a null character".into());
    }
    if rule.encoded_len() > MAX_RULE_BYTES || memory_bytes(rule) > 256 * 1024 {
        return Err("Rule exceeds the 64 KiB limit".into());
    }
    Ok(())
}

pub(crate) fn memory_bytes(rule: &Rule) -> usize {
    fn operator(op: &protocol::Operator) -> usize {
        op.r#type.capacity()
            + op.operand.capacity()
            + op.data.capacity()
            + op.list.capacity() * std::mem::size_of::<protocol::Operator>()
            + op.list.iter().map(operator).sum::<usize>()
    }
    std::mem::size_of::<Rule>()
        + rule.name.capacity()
        + rule.description.capacity()
        + rule.action.capacity()
        + rule.duration.capacity()
        + rule.operator.as_ref().map_or(0, operator)
}

pub fn executable(rule: &Rule) -> Option<&str> {
    fn find(op: &protocol::Operator) -> Option<&str> {
        if op.operand == "process.path" {
            return Some(&op.data);
        }
        op.list.iter().find_map(find)
    }
    rule.operator.as_ref().and_then(find)
}

fn put(db: &Connection, node: &str, rule: &Rule) -> anyhow::Result<()> {
    db.execute("INSERT INTO application_rules(node,name,wire,process,description,duration) VALUES(?,?,?,?,?,?)
        ON CONFLICT(node,name) DO UPDATE SET wire=excluded.wire,process=excluded.process,description=excluded.description,duration=excluded.duration",
        params![node, rule.name, rule.encode_to_vec(), executable(rule).unwrap_or(""), rule.description, rule.duration])?;
    db.execute(
        "DELETE FROM prompt_rules WHERE node=? AND name=?",
        params![node, rule.name],
    )?;
    Ok(())
}

pub(crate) fn replace(db: &mut Connection, node: &str, rules: &[Rule]) -> anyhow::Result<()> {
    let tx = db.transaction()?;
    tx.execute("DELETE FROM application_rules WHERE node=?", [node])?;
    tx.execute("DELETE FROM prompt_rules WHERE node=?", [node])?;
    for rule in rules {
        put(&tx, node, rule)?;
    }
    tx.commit()?;
    Ok(())
}

pub(crate) fn apply(
    db: &mut Connection,
    node: &str,
    action: Action,
    rule: &Rule,
) -> anyhow::Result<()> {
    let tx = db.transaction()?;
    if action == Action::DeleteRule {
        tx.execute(
            "DELETE FROM application_rules WHERE node=? AND name=?",
            params![node, rule.name],
        )?;
        tx.execute(
            "DELETE FROM prompt_rules WHERE node=? AND name=?",
            params![node, rule.name],
        )?;
    } else {
        let mut rule = rule.clone();
        if action == Action::EnableRule {
            rule.enabled = true;
        }
        if action == Action::DisableRule {
            rule.enabled = false;
        }
        put(&tx, node, &rule)?;
    }
    let (count, bytes): (i64, i64) = tx.query_row(
        "SELECT count(*),coalesce(sum(length(wire)),0) FROM (SELECT node,wire FROM application_rules UNION ALL SELECT node,wire FROM prompt_rules) WHERE node=?",
        [node],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    anyhow::ensure!(
        count <= MAX_RULES as i64 && bytes <= MAX_CATALOG_BYTES as i64,
        "Rule catalogue exceeds its bounded capacity"
    );
    tx.commit()?;
    Ok(())
}

pub(crate) fn query(db: &Connection, query: &Query) -> anyhow::Result<Page> {
    let mut statement = db.prepare_cached("WITH catalogue AS (SELECT *,0 AS awaiting FROM application_rules UNION ALL SELECT *,1 AS awaiting FROM prompt_rules) SELECT wire,awaiting FROM catalogue WHERE node=?1 AND name>?2
        AND (?3='' OR instr(lower(name),lower(?3))>0 OR instr(lower(process),lower(?3))>0 OR instr(lower(description),lower(?3))>0)
        AND (?4 IS NULL OR (duration!='always')=?4) ORDER BY name LIMIT 201")?;
    let records = statement.query_map(
        params![query.node, query.after, query.search, query.temporary],
        |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, bool>(1)?)),
    )?;
    let mut rules = Vec::new();
    let mut awaiting = std::collections::BTreeSet::new();
    for record in records {
        let (wire, pending) = record?;
        let rule = Rule::decode(wire.as_slice())?;
        if pending {
            awaiting.insert(rule.name.clone());
        }
        rules.push(rule);
    }
    let has_more = rules.len() > 200;
    rules.truncate(200);
    awaiting.retain(|name| rules.iter().any(|rule| &rule.name == name));
    Ok(Page {
        rules,
        has_more,
        awaiting,
    })
}

pub(crate) fn propose(db: &mut Connection, node: &str, rule: &Rule) -> anyhow::Result<()> {
    valid(rule).map_err(anyhow::Error::msg)?;
    let tx = db.transaction()?;
    let confirmed: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM application_rules WHERE node=? AND name=?)",
        params![node, rule.name],
        |row| row.get(0),
    )?;
    if confirmed {
        return Ok(());
    }
    let count: i64 = tx.query_row("SELECT count(*) FROM prompt_rules", [], |row| row.get(0))?;
    anyhow::ensure!(
        count < 128,
        "At most 128 popup rules can await daemon observation"
    );
    tx.execute("INSERT OR REPLACE INTO prompt_rules(node,name,wire,process,description,duration) VALUES(?,?,?,?,?,?)",params![node,rule.name,rule.encode_to_vec(),executable(rule).unwrap_or(""),rule.description,rule.duration])?;
    let (count,bytes):(i64,i64)=tx.query_row("SELECT count(*),coalesce(sum(length(wire)),0) FROM (SELECT node,wire FROM application_rules UNION ALL SELECT node,wire FROM prompt_rules) WHERE node=?",[node],|row|Ok((row.get(0)?,row.get(1)?)))?;
    anyhow::ensure!(
        count <= MAX_RULES as i64 && bytes <= MAX_CATALOG_BYTES as i64,
        "Popup rule catalogue exceeds its bounded capacity"
    );
    tx.commit()?;
    Ok(())
}
pub(crate) fn confirm_observation(
    db: &mut Connection,
    node: &str,
    rule: &Rule,
) -> anyhow::Result<bool> {
    let proposed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM prompt_rules WHERE node=? AND name=?)",
        params![node, rule.name],
        |row| row.get(0),
    )?;
    if !proposed {
        return Ok(false);
    }
    valid(rule).map_err(anyhow::Error::msg)?;
    apply(db, node, Action::ChangeRule, rule)?;
    Ok(true)
}

/// Connection generations are independent per node, including catalogue readiness.
#[derive(Default)]
pub struct Catalogues(Mutex<BTreeMap<u64, Catalogue>>);
struct Catalogue {
    node: String,
    ready: bool,
    uncertain: bool,
}
impl Catalogues {
    pub(crate) fn begin(&self, session: u64, node: &str) -> Result<(), String> {
        let mut entries = self.0.lock().unwrap();
        entries.retain(|id, entry| *id == session || entry.node != node);
        if !entries.contains_key(&session) && entries.len() >= crate::nodes::MAX_NODES {
            return Err("At most 32 nodes can be connected".into());
        }
        entries.insert(
            session,
            Catalogue {
                node: node.into(),
                ready: false,
                uncertain: false,
            },
        );
        Ok(())
    }
    pub fn ready(&self, session: u64) -> bool {
        self.0
            .lock()
            .unwrap()
            .get(&session)
            .is_some_and(|entry| entry.ready)
    }
    pub fn uncertain(&self, session: u64) -> bool {
        self.0
            .lock()
            .unwrap()
            .get(&session)
            .is_some_and(|entry| entry.uncertain)
    }
    pub(crate) fn current(&self, session: u64, node: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .get(&session)
            .is_some_and(|entry| entry.node == node)
    }
    pub(crate) fn saved(&self, session: u64, node: &str) {
        if let Some(entry) = self.0.lock().unwrap().get_mut(&session)
            && entry.node == node
        {
            entry.ready = true;
            entry.uncertain = false;
        }
    }
    pub(crate) fn invalidate(&self, session: u64) {
        if let Some(entry) = self.0.lock().unwrap().get_mut(&session) {
            entry.ready = false;
            entry.uncertain = true;
        }
    }
    pub(crate) fn mark_uncertain(&self, session: u64) {
        if let Some(entry) = self.0.lock().unwrap().get_mut(&session) {
            entry.uncertain = true;
        }
    }
    pub(crate) fn finish(&self, session: u64) {
        self.0.lock().unwrap().remove(&session);
    }
    pub(crate) fn names(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .values()
            .map(|entry| entry.node.clone())
            .collect()
    }
}
#[derive(Clone)]
enum Change {
    Rule(Action, Rule),
    Node(crate::nodes::Change),
}
struct Pending {
    session: u64,
    node: String,
    change: Change,
    deadline: Instant,
    reply: oneshot::Sender<Result<(), String>>,
}
struct Channel {
    node: String,
    sender: Option<mpsc::Sender<Result<Notification, tonic::Status>>>,
    firewall: Option<(u32, [u8; 32])>,
    range_version: bool,
    range_observed: bool,
    restore_pause: bool,
    restoring_pause: bool,
}
#[derive(Default)]
struct State {
    sessions: BTreeMap<u64, Channel>,
    pending: BTreeMap<u64, Pending>,
    firewall_operation: Option<(u64, u64)>,
    rule_operations: BTreeMap<u64, u64>,
}
/// A reviewed copy owns this node's UI command path while each ACK is saved.
/// Permission replies and telemetry do not use this reservation.
pub(crate) struct RulesLease {
    control: Arc<Control>,
    session: u64,
    token: u64,
}
impl RulesLease {
    pub(crate) fn submit(
        &self,
        rule: Rule,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        valid(&rule)?;
        if rule.operator.is_none() {
            return Err("A copied rule needs a match operator".into());
        }
        self.control.queue_reserved(
            self.session,
            Change::Rule(Action::ChangeRule, rule),
            None,
            None,
            Some(self.token),
        )
    }
    pub(crate) fn delete(
        &self,
        name: String,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        let rule = Rule {
            name,
            ..Default::default()
        };
        valid(&rule)?;
        self.control.queue_reserved(
            self.session,
            Change::Rule(Action::DeleteRule, rule),
            None,
            None,
            Some(self.token),
        )
    }
}
impl Drop for RulesLease {
    fn drop(&mut self) {
        let mut state = self.control.state.lock().unwrap();
        if state.rule_operations.get(&self.session) == Some(&self.token) {
            state.rule_operations.remove(&self.session);
        }
    }
}
/// One bounded multi-step firewall operation, independent of its originating window.
pub(crate) struct FirewallLease {
    control: Arc<Control>,
    session: u64,
    token: u64,
}
impl FirewallLease {
    pub(crate) fn submit(
        &self,
        change: crate::firewall::Change,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        crate::firewall::validate(&change.config)?;
        self.control.queue_reserved(
            self.session,
            Change::Node(crate::nodes::Change::Firewall(change)),
            Some(self.token),
            None,
            None,
        )
    }
}
impl Drop for FirewallLease {
    fn drop(&mut self) {
        let mut state = self.control.state.lock().unwrap();
        if state.firewall_operation == Some((self.session, self.token)) {
            state.firewall_operation = None;
        }
    }
}
pub(crate) type PauseRestore = (u64, String, oneshot::Receiver<Result<(), String>>);

pub struct Control {
    next: AtomicU64,
    state: Mutex<State>,
    catalogues: Arc<Catalogues>,
}
impl Control {
    pub(crate) fn new(catalogues: Arc<Catalogues>) -> Self {
        Self {
            next: AtomicU64::new(0),
            state: Mutex::new(State::default()),
            catalogues,
        }
    }
    pub(crate) fn start_session(&self, session: u64, node: String) {
        let mut state = self.state.lock().unwrap();
        let replaced = state
            .sessions
            .iter()
            .filter_map(|(id, channel)| (*id == session || channel.node == node).then_some(*id))
            .collect::<Vec<_>>();
        for id in replaced {
            Self::finish(
                &mut state,
                id,
                "Daemon reconnected before confirming the change; check its state",
            );
            self.catalogues.finish(id);
        }
        state.sessions.insert(
            session,
            Channel {
                node,
                sender: None,
                firewall: None,
                range_version: false,
                range_observed: false,
                restore_pause: false,
                restoring_pause: false,
            },
        );
    }
    /// Range was added after the 1.8 release while master still reported 1.8.0.
    /// A 1.8 version string alone cannot distinguish the older operator set.
    /// Existing reported range rules establish support for a backport/build.
    pub(crate) fn report_range_version(&self, session: u64, version: &str) {
        let version = version.strip_prefix('v').unwrap_or(version);
        let release = version.split([' ', '-', '+']).next().unwrap_or("");
        let parts = release.split('.').collect::<Vec<_>>();
        let supported = if parts.len() == 3 {
            let parsed = parts
                .iter()
                .map(|part| {
                    (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                        .then(|| part.parse::<u32>().ok())
                        .flatten()
                })
                .collect::<Option<Vec<_>>>();
            parsed.is_some_and(|v| v[0] > 1 || v[0] == 1 && v[1] >= 9)
        } else {
            false
        };
        if let Some(channel) = self.state.lock().unwrap().sessions.get_mut(&session) {
            channel.range_version = supported;
        }
    }
    pub(crate) fn report_range_inventory(&self, session: u64, rules: &[Rule]) {
        if let Some(channel) = self.state.lock().unwrap().sessions.get_mut(&session) {
            channel.range_observed = rules.iter().any(document::has_range);
        }
    }
    pub fn range_supported(&self, session: u64) -> bool {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&session)
            .is_some_and(|channel| channel.range_version || channel.range_observed)
    }
    pub(crate) fn report_interception(&self, session: u64, running: bool) {
        if let Some(channel) = self.state.lock().unwrap().sessions.get_mut(&session) {
            channel.restore_pause = running;
        }
    }
    pub fn node_busy(&self, session: u64) -> bool {
        let state = self.state.lock().unwrap();
        state.rule_operations.contains_key(&session)
            || state
                .sessions
                .get(&session)
                .is_some_and(|c| c.restoring_pause)
            || state
                .pending
                .values()
                .any(|p| p.session == session && matches!(p.change, Change::Node(_)))
    }
    pub fn pause_restoring(&self, session: u64) -> bool {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&session)
            .is_some_and(|c| c.restoring_pause)
    }
    pub(crate) fn finish_pause_restore(&self, session: u64) {
        if let Some(channel) = self.state.lock().unwrap().sessions.get_mut(&session) {
            channel.restoring_pause = false;
        }
    }
    pub(crate) fn restore_pauses(&self, history: &History, capacity: usize) -> Vec<PauseRestore> {
        let sessions = {
            let state = self.state.lock().unwrap();
            state
                .sessions
                .iter()
                .filter_map(|(id, c)| {
                    (c.restore_pause
                        && c.sender.is_some()
                        && history.health.interception.may_restore(&c.node, *id)
                        && self.catalogues.ready(*id)
                        && !self.catalogues.uncertain(*id)
                        && !state.pending.values().any(|p| p.session == *id))
                    .then_some((*id, c.node.clone()))
                })
                .take(capacity)
                .collect::<Vec<_>>()
        };
        sessions
            .into_iter()
            .filter_map(|(session, node)| {
                self.queue_reserved(
                    session,
                    Change::Node(crate::nodes::Change::Interception(false)),
                    None,
                    Some(&history.health.interception),
                    None,
                )
                .ok()
                .map(|reply| (session, node, reply))
            })
            .collect()
    }
    pub(crate) fn report_firewall(
        &self,
        session: u64,
        config: Option<&protocol::SysFirewall>,
        settings: &str,
    ) {
        if let Some(channel) = self.state.lock().unwrap().sessions.get_mut(&session) {
            channel.firewall = config
                .filter(|_| crate::firewall::backend_name(settings) == "nftables")
                .map(|config| {
                    (
                        config.version,
                        crate::firewall::fingerprint(&prost::Message::encode_to_vec(config)),
                    )
                });
        }
    }
    fn finish(state: &mut State, session: u64, error: &str) {
        if state
            .firewall_operation
            .is_some_and(|(id, _)| id == session)
        {
            state.firewall_operation = None;
        }
        state.sessions.remove(&session);
        state.rule_operations.remove(&session);
        let ids = state
            .pending
            .iter()
            .filter_map(|(id, pending)| (pending.session == session).then_some(*id))
            .collect::<Vec<_>>();
        for id in ids {
            let _ = state
                .pending
                .remove(&id)
                .unwrap()
                .reply
                .send(Err(error.into()));
        }
    }
    pub(crate) fn attach(
        &self,
        session: u64,
        sender: mpsc::Sender<Result<Notification, tonic::Status>>,
    ) {
        if let Some(channel) = self.state.lock().unwrap().sessions.get_mut(&session) {
            channel.sender = Some(sender);
        }
    }
    pub(crate) fn disconnect(&self, session: u64) {
        Self::finish(
            &mut self.state.lock().unwrap(),
            session,
            "Daemon disconnected before confirming the change; check its state",
        );
        self.catalogues.finish(session);
    }
    pub fn refresh(&self, session: u64) -> Result<(), String> {
        let state = self.state.lock().unwrap();
        if state.rule_operations.contains_key(&session) {
            return Err("Stop this node's rule copy before refreshing".into());
        }
        if state
            .firewall_operation
            .is_some_and(|(id, _)| id == session)
        {
            return Err("Wait for this node's system-firewall operation before refreshing".into());
        }
        let channel = state
            .sessions
            .get(&session)
            .ok_or("This daemon is disconnected")?;
        if state
            .pending
            .values()
            .any(|pending| pending.session == session)
        {
            return Err("Wait for outstanding daemon commands before refreshing".into());
        }
        channel
            .sender
            .as_ref()
            .ok_or("Connect this daemon's notification stream before refreshing")?
            .try_send(Err(tonic::Status::unavailable(
                "Rustleblower requested a fresh daemon inventory",
            )))
            .map_err(|_| "The notification stream is busy; try refreshing again".to_string())?;
        self.catalogues.invalidate(session);
        Ok(())
    }
    pub(crate) fn task_command(
        &self,
        session: u64,
        action: Action,
        data: String,
    ) -> Result<u64, String> {
        let state = self.state.lock().unwrap();
        let channel = state
            .sessions
            .get(&session)
            .ok_or("This daemon is disconnected")?;
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        channel
            .sender
            .as_ref()
            .ok_or("Connect this daemon's notification stream first")?
            .try_send(Ok(Notification {
                id,
                r#type: action as i32,
                data,
                client_name: channel.node.clone(),
                server_name: "Rustleblower".into(),
                ..Default::default()
            }))
            .map_err(|_| "The daemon stream is busy; task command was not queued".to_string())?;
        Ok(id)
    }
    pub fn submit(
        &self,
        session: u64,
        action: Action,
        rule: Rule,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        valid(&rule)?;
        if !matches!(
            action,
            Action::ChangeRule | Action::DeleteRule | Action::EnableRule | Action::DisableRule
        ) {
            return Err("Unsupported rule action".into());
        }
        if action != Action::DeleteRule && rule.operator.is_none() {
            return Err("This rule has no match operator; it cannot be applied".into());
        }
        self.queue(session, Change::Rule(action, rule))
    }
    pub fn submit_node(
        &self,
        session: u64,
        change: crate::nodes::Change,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        change.validate()?;
        self.queue(session, Change::Node(change))
    }
    pub fn firewall_current(&self, session: u64, expected: [u8; 32]) -> bool {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&session)
            .is_some_and(|channel| channel.firewall == Some((1, expected)))
    }
    pub fn firewall_busy(&self, session: u64) -> bool {
        self.state
            .lock()
            .unwrap()
            .firewall_operation
            .is_some_and(|(id, _)| id == session)
    }
    pub(crate) fn reserve_firewall(
        self: &Arc<Self>,
        session: u64,
        expected: [u8; 32],
    ) -> Result<FirewallLease, String> {
        let mut state = self.state.lock().unwrap();
        if state.rule_operations.contains_key(&session) {
            return Err("Wait for this node's reviewed rule copy to finish".into());
        }
        if state.firewall_operation.is_some() {
            return Err("Wait for the current system-firewall operation to finish".into());
        }
        let channel = state
            .sessions
            .get(&session)
            .ok_or("Daemon changed; select the node again")?;
        if channel.firewall != Some((1, expected))
            || channel.sender.is_none()
            || self.catalogues.uncertain(session)
        {
            return Err(
                "A fresh connected nftables format-1 report matching this review is required"
                    .into(),
            );
        }
        if state.pending.values().any(|p| p.session == session) {
            return Err(
                "Wait for this node's pending commands before starting the firewall operation"
                    .into(),
            );
        }
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        state.firewall_operation = Some((session, token));
        Ok(FirewallLease {
            control: self.clone(),
            session,
            token,
        })
    }
    fn queue(
        &self,
        session: u64,
        change: Change,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        self.queue_reserved(session, change, None, None, None)
    }
    pub(crate) fn reserve_rules(self: &Arc<Self>, session: u64) -> Result<RulesLease, String> {
        let mut state = self.state.lock().unwrap();
        let channel = state
            .sessions
            .get(&session)
            .ok_or("Destination disconnected")?;
        if channel.sender.is_none()
            || channel.restoring_pause
            || !self.catalogues.ready(session)
            || self.catalogues.uncertain(session)
        {
            return Err("Destination needs a ready confirmed connection".into());
        }
        if state.rule_operations.contains_key(&session)
            || state
                .firewall_operation
                .is_some_and(|(id, _)| id == session)
            || state.pending.values().any(|p| p.session == session)
        {
            return Err("Wait for this node's pending operation before copying rules".into());
        }
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        state.rule_operations.insert(session, token);
        Ok(RulesLease {
            control: self.clone(),
            session,
            token,
        })
    }
    fn queue_reserved(
        &self,
        session: u64,
        change: Change,
        token: Option<u64>,
        restore: Option<&crate::nodes::interception::State>,
        rule_token: Option<u64>,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        let mut state = self.state.lock().unwrap();
        match (state.rule_operations.get(&session), rule_token) {
            (Some(active), Some(token)) if *active == token => {}
            (_, Some(_)) => {
                return Err("The rule copy's captured connection ended; rebuild its review".into());
            }
            (Some(_), None) => {
                return Err("Wait for this node's reviewed rule copy to finish".into());
            }
            _ => {}
        }
        match (state.firewall_operation,token) {
            (Some((id,active)),Some(token)) if id==session&&active==token=>{},
            (_,Some(_))=>return Err("The firewall operation's captured connection ended; reconnect to verify its last accepted stage".into()),
            (Some((id,_)),None) if id==session=>return Err("Wait for this node's system-firewall operation to finish".into()),
            _=>{},
        }
        let channel = state
            .sessions
            .get(&session)
            .ok_or("Daemon changed; select the node again")?;
        if let Some(preference) = restore
            && (!channel.restore_pause || !preference.may_restore(&channel.node, session))
        {
            return Err("Saved pause restoration was superseded".into());
        }
        if restore.is_none() && channel.restoring_pause && matches!(change, Change::Node(_)) {
            return Err("Wait for the saved pause restoration to be confirmed and saved".into());
        }
        if let Change::Rule(Action::ChangeRule | Action::EnableRule, rule) = &change
            && document::has_range(rule)
        {
            document::validate_edit(rule)?;
            if !channel.range_version && !channel.range_observed {
                return Err("This daemon has not established port-range support. OpenSnitch 1.9 or a reported existing range rule is required; 1.8 builds are ambiguous. The rule was not sent.".into());
            }
        }
        if let Change::Node(crate::nodes::Change::Firewall(change)) = &change {
            match channel.firewall {
                Some((1, fingerprint)) if fingerprint == change.expected => {}
                Some((1, _)) => {
                    return Err(
                        "The firewall changed since review; reload it before applying".into(),
                    );
                }
                _ => {
                    return Err(
                        "A fresh nftables format-1 firewall report is required before changing it"
                            .into(),
                    );
                }
            }
        }
        if matches!(change, Change::Rule(..)) && !self.catalogues.ready(session) {
            return Err(
                "Waiting for the daemon's rule inventory to be saved; reconnect if loading failed"
                    .into(),
            );
        }
        if matches!(change, Change::Node(_)) && self.catalogues.uncertain(session) {
            return Err(
                "Node state is uncertain; reconnect to refresh before changing it again".into(),
            );
        }
        if state.pending.len() >= MAX_COMMANDS {
            return Err("Wait for the pending daemon changes to finish".into());
        }
        if state.pending.values().any(|pending| {
            pending.session == session
                && (matches!(change, Change::Node(_)) || matches!(pending.change, Change::Node(_)))
        }) {
            return Err("Wait for this node's current command before changing its configuration or interception".into());
        }
        let sender = channel
            .sender
            .as_ref()
            .ok_or("Waiting for the daemon's command channel")?;
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let mut notification = Notification {
            id,
            client_name: channel.node.clone(),
            server_name: "Rustleblower".into(),
            ..Default::default()
        };
        match &change {
            Change::Rule(action, rule) => {
                notification.r#type = *action as i32;
                notification.rules = vec![rule.clone()];
            }
            Change::Node(change) => {
                notification.r#type = change.action() as i32;
                notification.data = change.data().into();
                if let crate::nodes::Change::Firewall(change) = change {
                    notification.sys_firewall = Some(*change.config.clone());
                }
            }
        }
        sender
            .try_send(Ok(notification))
            .map_err(|_| "Daemon command channel is busy or disconnected")?;
        let node = channel.node.clone();
        if matches!(change, Change::Node(crate::nodes::Change::Interception(_))) {
            let channel = state.sessions.get_mut(&session).unwrap();
            channel.restore_pause = false;
            channel.restoring_pause = restore.is_some();
        }
        let (reply, rx) = oneshot::channel();
        state.pending.insert(
            id,
            Pending {
                session,
                node,
                change,
                deadline: Instant::now() + COMMAND_TIMEOUT,
                reply,
            },
        );
        Ok(rx)
    }
    pub(crate) fn complete(&self, session: u64, reply: NotificationReply, history: &History) {
        let mut state = self.state.lock().unwrap();
        if !state.sessions.contains_key(&session)
            || !state
                .pending
                .get(&reply.id)
                .is_some_and(|pending| pending.session == session)
        {
            return;
        }
        let pending = state.pending.remove(&reply.id).unwrap();
        if Instant::now() >= pending.deadline {
            self.catalogues.invalidate(session);
            let _ = pending.reply.send(Err(
                "Daemon confirmation arrived after the timeout; reconnect to refresh its state"
                    .into(),
            ));
            return;
        }
        if reply.code != protocol::NotificationReplyCode::Ok as i32 {
            self.catalogues.mark_uncertain(session);
            let _=pending.reply.send(Err(format!("Daemon reported a rule or node command error: {}. Its live state may have changed; reconnect to verify.",crate::history::bounded(&reply.data,512))));
            return;
        }
        match pending.change {
            Change::Rule(action, rule) => {
                history.apply_rule(session, pending.node, action, rule, pending.reply)
            }
            Change::Node(change) => {
                // Configuration reload can replace the firewall backend/path.
                // Its prior firewall report must not authorize a later overwrite.
                if matches!(change, crate::nodes::Change::Configuration(_))
                    && let Some(channel) = state.sessions.get_mut(&session)
                {
                    channel.firewall = None;
                }
                if let crate::nodes::Change::Firewall(change) = &change
                    && let Some(channel) = state.sessions.get_mut(&session)
                {
                    channel.firewall = Some((
                        change.config.version,
                        crate::firewall::fingerprint(&prost::Message::encode_to_vec(
                            change.config.as_ref(),
                        )),
                    ));
                }
                history.apply_node(session, pending.node, change, pending.reply)
            }
        }
    }
    pub(crate) fn expire(&self, now: Instant) {
        let mut state = self.state.lock().unwrap();
        let expired = state
            .pending
            .iter()
            .filter_map(|(id, pending)| (now >= pending.deadline).then_some(*id))
            .collect::<Vec<_>>();
        for id in expired {
            let pending = state.pending.remove(&id).unwrap();
            self.catalogues.invalidate(pending.session);
            let _=pending.reply.send(Err("Daemon has not confirmed the change; reconnect to refresh its state before retrying".into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Rule {
        Rule {
            name: "fixture".into(),
            action: "allow".into(),
            duration: "always".into(),
            operator: Some(protocol::Operator::default()),
            ..Default::default()
        }
    }

    #[test]
    fn rule_names_cannot_escape_the_daemons_rule_directory() {
        for name in ["../escape", "/absolute", "nested/rule", "bad\0name"] {
            let mut rule = sample();
            rule.name = name.into();
            assert!(valid(&rule).unwrap_err().contains("slash"));
        }
        let mut rule = sample();
        rule.name = "allow.example application".into();
        assert!(valid(&rule).is_ok());
    }

    #[tokio::test]
    async fn pending_commands_are_bounded_expire_and_require_a_current_inventory() {
        let catalogue = Arc::new(Catalogues::default());
        let control = Control::new(catalogue.clone());
        control.start_session(1, "fixture".into());
        let (tx, mut rx) = mpsc::channel(32);
        control.attach(1, tx);
        assert!(control.submit(1, Action::ChangeRule, sample()).is_err());
        catalogue.begin(1, "fixture").unwrap();
        catalogue.saved(1, "fixture");
        let mut replies = Vec::new();
        for _ in 0..MAX_COMMANDS {
            replies.push(control.submit(1, Action::ChangeRule, sample()).unwrap());
            rx.recv().await.unwrap().unwrap();
        }
        assert!(control.submit(1, Action::ChangeRule, sample()).is_err());
        control.expire(Instant::now() + COMMAND_TIMEOUT + Duration::from_secs(1));
        for reply in replies {
            assert!(reply.await.unwrap().unwrap_err().contains("not confirmed"));
        }
        assert!(
            control.submit(1, Action::ChangeRule, sample()).is_err(),
            "a timed-out change leaves the daemon state unknown until its inventory is reloaded"
        );
        control.disconnect(1);
        assert!(control.submit(1, Action::ChangeRule, sample()).is_err());
    }
}
