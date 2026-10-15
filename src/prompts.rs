//! Pending decisions are independent of telemetry and SQLite. No lock spans an await.
pub mod choice;
pub mod placement;
use crate::{
    config::Decision,
    protocol::{Connection, Rule},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
use tokio::sync::{oneshot, watch};

pub const MAX_PENDING: usize = 32;
pub const MAX_PROMPT_BYTES: usize = 64 * 1024;
pub const MAX_PROMPT_MEMORY: usize = 256 * 1024;
pub fn valid_connection(connection: &Connection) -> bool {
    use prost::Message;
    let strings = [
        &connection.protocol,
        &connection.src_ip,
        &connection.dst_ip,
        &connection.dst_host,
        &connection.process_path,
        &connection.process_cwd,
    ]
    .iter()
    .map(|text| text.capacity())
    .sum::<usize>();
    let maps = [&connection.process_env, &connection.process_checksums]
        .iter()
        .map(|map| {
            map.capacity() * (2 * std::mem::size_of::<String>() + 1)
                + map
                    .iter()
                    .map(|(key, value)| key.capacity() + value.capacity())
                    .sum::<usize>()
        })
        .sum::<usize>();
    let bytes = std::mem::size_of::<Connection>()
        + strings
        + maps
        + connection.process_args.capacity() * std::mem::size_of::<String>()
        + connection
            .process_args
            .iter()
            .map(String::capacity)
            .sum::<usize>()
        + connection.process_tree.capacity() * std::mem::size_of::<crate::protocol::StringInt>()
        + connection
            .process_tree
            .iter()
            .map(|item| item.key.capacity())
            .sum::<usize>();
    bytes <= MAX_PROMPT_MEMORY && connection.encoded_len() <= MAX_PROMPT_BYTES
}

#[derive(Clone, Default)]
pub struct Initial {
    pub choice: choice::Choice,
    pub expanded: bool,
    pub unavailable: Vec<String>,
    pub fallback: Option<Decision>,
    pub position: placement::Position,
}

#[derive(Clone)]
pub struct Prompt {
    pub id: u64,
    pub session: u64,
    pub connection: Arc<Connection>,
    pub deadline: Instant,
    /// Upper bound captured from the daemon's RPC; interaction cannot extend it.
    pub hard_deadline: Instant,
    pub reviewing: bool,
    pub initial: Initial,
}

pub struct Resolution {
    pub decision: Decision,
    pub reason: &'static str,
    pub rule: Option<Box<Rule>>,
}

struct Pending {
    prompt: Prompt,
    reply: oneshot::Sender<Resolution>,
    deadline_changed: watch::Sender<Instant>,
}

#[derive(Default)]
pub struct Prompts {
    next: AtomicU64,
    pending: Mutex<BTreeMap<u64, Pending>>,
}

impl Prompts {
    pub fn insert(
        &self,
        session: u64,
        connection: Connection,
        deadline: Instant,
    ) -> Option<(Prompt, oneshot::Receiver<Resolution>)> {
        self.insert_with_defaults(session, connection, deadline, Initial::default())
    }

    pub(crate) fn insert_with_defaults(
        &self,
        session: u64,
        connection: Connection,
        deadline: Instant,
        initial: Initial,
    ) -> Option<(Prompt, oneshot::Receiver<Resolution>)> {
        self.insert_timed(session, connection, deadline, deadline, initial)
    }

    pub(crate) fn insert_timed(
        &self,
        session: u64,
        connection: Connection,
        deadline: Instant,
        hard_deadline: Instant,
        initial: Initial,
    ) -> Option<(Prompt, oneshot::Receiver<Resolution>)> {
        if !valid_connection(&connection) {
            return None;
        }
        let mut pending = self.pending.lock().unwrap();
        if pending.len() >= MAX_PENDING {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let prompt = Prompt {
            id,
            session,
            connection: Arc::new(connection),
            deadline: deadline.min(hard_deadline),
            hard_deadline,
            reviewing: false,
            initial,
        };
        let (reply, receiver) = oneshot::channel();
        let (deadline_changed, _) = watch::channel(prompt.deadline);
        pending.insert(
            id,
            Pending {
                prompt: prompt.clone(),
                reply,
                deadline_changed,
            },
        );
        Some((prompt, receiver))
    }

    /// Pause the ordinary countdown for this request only. The RPC limit remains
    /// finite; stale input cannot revive an expired or replacement request.
    pub fn review(&self, id: u64) -> bool {
        let mut pending = self.pending.lock().unwrap();
        let Some(item) = pending.get_mut(&id) else {
            return false;
        };
        if item.prompt.reviewing || Instant::now() >= item.prompt.deadline {
            return false;
        }
        item.prompt.reviewing = true;
        item.prompt.deadline = item.prompt.hard_deadline;
        item.deadline_changed.send_replace(item.prompt.deadline);
        true
    }

    /// Wait independently of UI rendering, telemetry and storage. Watch retains
    /// only one deadline and handles review both before and after subscription.
    pub(crate) async fn wait_for_expiry(&self, id: u64) {
        let changed = self
            .pending
            .lock()
            .unwrap()
            .get(&id)
            .map(|item| item.deadline_changed.subscribe());
        let Some(mut changed) = changed else {
            return std::future::pending().await;
        };
        loop {
            let deadline = *changed.borrow_and_update();
            tokio::select! {
                biased;
                result = changed.changed() => {
                    if result.is_err() {
                        return std::future::pending().await;
                    }
                }
                _ = tokio::time::sleep_until(deadline.into()) => {
                    // Linearize against review/choose: once this deadline has
                    // elapsed, neither can extend or answer this request.
                    if self.pending.lock().unwrap().get(&id)
                        .is_some_and(|item| Instant::now() >= item.prompt.deadline) {
                        return;
                    }
                }
            }
        }
    }

    pub fn snapshot(&self) -> Vec<Prompt> {
        self.pending
            .lock()
            .unwrap()
            .values()
            .map(|p| p.prompt.clone())
            .collect()
    }

    pub fn decide(&self, id: u64, decision: Decision) -> bool {
        let mut pending = self.pending.lock().unwrap();
        let Some(item) = pending.get(&id) else {
            return false;
        };
        if Instant::now() >= item.prompt.deadline {
            return false;
        }
        pending
            .remove(&id)
            .unwrap()
            .reply
            .send(Resolution {
                decision,
                reason: "decision",
                rule: None,
            })
            .is_ok()
    }
    pub fn choose(
        &self,
        id: u64,
        decision: Decision,
        choice: &choice::Choice,
    ) -> Result<bool, String> {
        let prompt = {
            let pending = self.pending.lock().unwrap();
            let Some(item) = pending.get(&id) else {
                return Ok(false);
            };
            item.prompt.clone()
        };
        if Instant::now() >= prompt.deadline {
            return Ok(false);
        }
        let rule = choice::rule(&prompt.connection, decision, id, choice)?;
        let mut pending = self.pending.lock().unwrap();
        if !pending
            .get(&id)
            .is_some_and(|item| Instant::now() < item.prompt.deadline)
        {
            return Ok(false);
        }
        Ok(pending
            .remove(&id)
            .unwrap()
            .reply
            .send(Resolution {
                decision,
                reason: "decision",
                rule: Some(Box::new(rule)),
            })
            .is_ok())
    }

    pub fn remove(&self, id: u64) {
        self.pending.lock().unwrap().remove(&id);
    }

    pub fn finish_session(&self, session: u64, decision: Decision, reason: &'static str) {
        let mut pending = self.pending.lock().unwrap();
        let ids: Vec<_> = pending
            .iter()
            .filter(|(_, p)| p.prompt.session == session)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(p) = pending.remove(&id) {
                let _ = p.reply.send(Resolution {
                    decision: p.prompt.initial.fallback.unwrap_or(decision),
                    reason,
                    rule: None,
                });
            }
        }
    }
}

/// Dropping an RPC future (client deadline/cancellation) immediately removes its prompt.
pub struct PendingGuard(pub Arc<Prompts>, pub u64);
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.0.remove(self.1);
    }
}

pub fn once_rule(connection: &Connection, decision: Decision, id: u64) -> Rule {
    choice::rule(connection, decision, id, &choice::Choice::default())
        .expect("the bounded connection-only fallback is a valid once rule")
}
