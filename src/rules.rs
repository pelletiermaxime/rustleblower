//! Disk-backed rule pages and bounded, acknowledged daemon commands.
pub mod checksums;
pub mod document;
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
}
#[derive(Default)]
struct State {
    sessions: BTreeMap<u64, Channel>,
    pending: BTreeMap<u64, Pending>,
}
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
        state
            .sessions
            .insert(session, Channel { node, sender: None });
    }
    fn finish(state: &mut State, session: u64, error: &str) {
        state.sessions.remove(&session);
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
    fn queue(
        &self,
        session: u64,
        change: Change,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        let mut state = self.state.lock().unwrap();
        let channel = state
            .sessions
            .get(&session)
            .ok_or("Daemon changed; select the node again")?;
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
            }
        }
        sender
            .try_send(Ok(notification))
            .map_err(|_| "Daemon command channel is busy or disconnected")?;
        let node = channel.node.clone();
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
