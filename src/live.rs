//! Owned, bounded daemon read tasks. Streaming replies never enter policy commands.
pub mod model;
use crate::{
    protocol::{self, Action, NotificationReply},
    rules::Control,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub const MAX_TASKS: usize = 4;
pub const MAX_UPDATE: usize = 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Sockets {
        seconds: u16,
        state: u8,
        proto: u8,
        family: u8,
    },
    Process {
        seconds: u16,
        pid: u32,
    },
    Resources {
        seconds: u16,
        node: String,
    },
}
impl Kind {
    pub fn seconds(&self) -> u16 {
        match self {
            Self::Sockets { seconds, .. }
            | Self::Process { seconds, .. }
            | Self::Resources { seconds, .. } => *seconds,
        }
    }
    fn key(&self) -> String {
        match self {
            Self::Sockets { .. } => "sockets-monitor".into(),
            Self::Process { pid, .. } => format!("pid-monitor-{pid}"),
            Self::Resources { node, .. } => format!("node-monitor-{node}"),
        }
    }
    fn data(&self) -> Result<String, String> {
        if !(1..=60).contains(&self.seconds()) {
            return Err("Task interval must be 1–60 seconds".into());
        }
        let interval = format!("{}s", self.seconds());
        Ok(match self {
            Self::Sockets{state,proto,family,..}=>serde_json::json!({"name":"sockets-monitor","data":{"interval":interval,"state":state,"proto":proto,"family":family}}),
            Self::Process{pid,..}=>{if *pid==0 || *pid>i32::MAX as u32{return Err("Choose a positive Linux PID".into());} serde_json::json!({"name":"pid-monitor","data":{"interval":interval,"pid":pid.to_string()}})},
            Self::Resources{node,..}=>{if node.is_empty()||node.len()>128{return Err("Choose a connected node".into());}serde_json::json!({"name":"node-monitor","data":{"interval":interval,"node":node}})}
        }.to_string())
    }
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub raw: Option<Arc<str>>,
    pub received: Option<Instant>,
    pub revision: u64,
    pub running: bool,
    pub status: String,
    pub error: Option<String>,
}
struct Entry {
    session: u64,
    kind: Kind,
    data: String,
    update: watch::Sender<Snapshot>,
    last: Instant,
    stopping: bool,
}
pub struct Tasks {
    control: Arc<Control>,
    entries: Mutex<BTreeMap<u64, Entry>>,
}
pub struct Lease {
    tasks: Arc<Tasks>,
    id: u64,
    updates: watch::Receiver<Snapshot>,
}
impl Lease {
    pub fn snapshot(&self) -> Snapshot {
        self.updates.borrow().clone()
    }
    pub fn stop(&self) {
        self.tasks.stop(self.id);
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.stop();
    }
}
impl Tasks {
    pub fn new(control: Arc<Control>) -> Self {
        Self {
            control,
            entries: Mutex::new(BTreeMap::new()),
        }
    }
    pub fn start(self: &Arc<Self>, session: u64, kind: Kind) -> Result<Lease, String> {
        let data = kind.data()?;
        let mut entries = self.entries.lock().unwrap();
        if entries.len() >= MAX_TASKS {
            return Err("At most four live inspections may run at once".into());
        }
        if entries
            .values()
            .any(|e| e.session == session && e.kind.key() == kind.key())
        {
            return Err("This task is already running or waiting for its stop command".into());
        }
        let id = self
            .control
            .task_command(session, Action::TaskStart, data.clone())?;
        let (update, updates) = watch::channel(Snapshot {
            running: true,
            status: "Waiting for the first daemon update".into(),
            ..Default::default()
        });
        entries.insert(
            id,
            Entry {
                session,
                kind,
                data,
                update,
                last: Instant::now(),
                stopping: false,
            },
        );
        Ok(Lease {
            tasks: self.clone(),
            id,
            updates,
        })
    }
    fn stop_locked(&self, entries: &mut BTreeMap<u64, Entry>, id: u64) {
        let Some(entry) = entries.get_mut(&id) else {
            return;
        };
        entry.stopping = true;
        let mut value = entry.update.borrow().clone();
        value.running = false;
        match self
            .control
            .task_command(entry.session, Action::TaskStop, entry.data.clone())
        {
            Ok(_) => {
                value.status = "Stop requested; this daemon does not acknowledge task stops".into();
                entry.update.send_replace(value);
                entries.remove(&id);
            }
            Err(error) => {
                value.status = format!("Waiting to queue the task stop; retrying: {error}");
                entry.update.send_replace(value);
            }
        }
    }
    fn stop(&self, id: u64) {
        self.stop_locked(&mut self.entries.lock().unwrap(), id);
    }
    pub(crate) fn disconnect(&self, session: u64) {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, entry| {
            if entry.session != session {
                return true;
            }
            let mut value = entry.update.borrow().clone();
            value.running = false;
            value.status = "Daemon disconnected; live data is stale".into();
            entry.update.send_replace(value);
            false
        });
    }
    pub(crate) fn complete(
        &self,
        session: u64,
        reply: NotificationReply,
    ) -> Option<NotificationReply> {
        let mut entries = self.entries.lock().unwrap();
        let Some(entry) = entries.get_mut(&reply.id).filter(|e| e.session == session) else {
            return Some(reply);
        };
        if entry.stopping {
            return None;
        }
        let mut value = entry.update.borrow().clone();
        if reply.code != protocol::NotificationReplyCode::Ok as i32 || reply.data.len() > MAX_UPDATE
        {
            value.error = Some(if reply.data.len() > MAX_UPDATE {
                "Live update exceeds 1 MiB; narrow the daemon filters".into()
            } else {
                crate::history::bounded(&reply.data, 512)
            });
            value.status = "Live inspection failed; last good data is retained".into();
            value.running = false;
            entry.update.send_replace(value);
            self.stop_locked(&mut entries, reply.id);
        } else if !reply.data.is_empty() {
            entry.last = Instant::now();
            value.received = Some(entry.last);
            value.revision += 1;
            value.status = "Live daemon data".into();
            value.raw = Some(Arc::from(reply.data.into_boxed_str()));
            value.error = None;
            entry.update.send_replace(value);
        }
        None
    }
    pub(crate) fn maintain(&self, now: Instant) {
        let mut entries = self.entries.lock().unwrap();
        let ids = entries
            .iter()
            .filter_map(|(id, e)| {
                (e.stopping
                    || now.saturating_duration_since(e.last)
                        > Duration::from_secs(u64::from(e.kind.seconds()) * 3 + 10))
                .then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in ids {
            let entry = entries.get_mut(&id).unwrap();
            if !entry.stopping {
                let mut value = entry.update.borrow().clone();
                value.error = Some("The daemon stopped reporting this task".into());
                entry.update.send_replace(value);
            }
            self.stop_locked(&mut entries, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_tasks_stop_retry_expiry_and_invalid_configs() {
        let control = Arc::new(Control::new(Arc::new(crate::rules::Catalogues::default())));
        control.start_session(1, "fixture".into());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        control.attach(1, tx);
        let tasks = Arc::new(Tasks::new(control));
        assert!(
            tasks
                .start(1, Kind::Process { seconds: 5, pid: 0 })
                .is_err()
        );
        assert!(
            tasks
                .start(1, Kind::Process { seconds: 0, pid: 1 })
                .is_err()
        );
        assert!(rx.try_recv().is_err());
        let mut leases = Vec::new();
        for pid in 1..=4 {
            let lease = tasks.start(1, Kind::Process { seconds: 1, pid }).unwrap();
            assert_eq!(rx.try_recv().unwrap().unwrap().r#type, 13);
            leases.push(lease);
        }
        assert!(
            tasks
                .start(1, Kind::Process { seconds: 1, pid: 5 })
                .is_err()
        );
        // Saturate the shared output channel, then Drop must retain an owned stop retry.
        tasks
            .control
            .task_command(1, Action::TaskStart, "{}".into())
            .unwrap();
        leases[0].stop();
        assert!(!leases[0].snapshot().running);
        assert!(leases[0].snapshot().status.contains("retrying"));
        assert_eq!(tasks.entries.lock().unwrap().len(), 4);
        rx.try_recv().unwrap().unwrap();
        tasks.maintain(Instant::now());
        let stop = rx.try_recv().unwrap().unwrap();
        assert_eq!(stop.r#type, 14);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&stop.data).unwrap()["data"]["pid"],
            "1"
        );
        assert_eq!(tasks.entries.lock().unwrap().len(), 3);
        tasks.maintain(Instant::now() + Duration::from_secs(30));
        assert!(leases[1].snapshot().error.is_some());
        assert!(!leases[1].snapshot().running);
        tasks.disconnect(1);
        assert!(tasks.entries.lock().unwrap().is_empty());
        assert!(leases[2].snapshot().status.contains("disconnected"));
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::*;
    #[test]
    fn four_live_tasks_leave_all_sixteen_policy_confirmation_slots_available() {
        let catalogues = Arc::new(crate::rules::Catalogues::default());
        let control = Arc::new(Control::new(catalogues.clone()));
        control.start_session(1, "fixture".into());
        catalogues.begin(1, "fixture").unwrap();
        catalogues.saved(1, "fixture");
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        control.attach(1, tx);
        let tasks = Arc::new(Tasks::new(control.clone()));
        let leases = (1..=4)
            .map(|pid| tasks.start(1, Kind::Process { seconds: 5, pid }).unwrap())
            .collect::<Vec<_>>();
        let rule = |name: String| protocol::Rule {
            name,
            action: "deny".into(),
            duration: "always".into(),
            enabled: true,
            operator: Some(protocol::Operator {
                r#type: "simple".into(),
                operand: "process.path".into(),
                data: "/bin/fixture".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut replies = Vec::new();
        for index in 0..16 {
            replies.push(
                control
                    .submit(1, Action::ChangeRule, rule(format!("fixture-{index}")))
                    .unwrap(),
            );
        }
        assert!(
            control
                .submit(1, Action::ChangeRule, rule("excess".into()))
                .unwrap_err()
                .contains("pending daemon changes")
        );
        let first = rx.try_recv().unwrap().unwrap();
        assert!(
            tasks
                .complete(
                    1,
                    NotificationReply {
                        id: first.id,
                        data: "{}".into(),
                        ..Default::default()
                    }
                )
                .is_none()
        );
        assert_eq!(leases[0].snapshot().revision, 1);
        assert_eq!(replies.len(), 16);
    }
}
