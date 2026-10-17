//! Local, bounded usage snapshots. No daemon commands or permission-path locks.
pub(crate) mod launcher;
mod worker;
pub use worker::Client;

use anyhow::{Result, ensure};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const MAX_FRAME: usize = 2 * 1024 * 1024;
pub const MAX_RECORDS: usize = 4096;
pub const PAGE: usize = 100;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Stopped,
    Starting,
    Waiting,
    Authorizing,
    Cancelled,
    Ended,
    Live,
    Stale,
    Disconnected,
    Unavailable(String),
}
impl Status {
    pub fn label(&self) -> &str {
        match self {
            Self::Stopped => "Monitoring stopped",
            Self::Starting => "Starting local receiver…",
            Self::Waiting => "Waiting for the local collector…",
            Self::Authorizing => "Waiting for system authorization…",
            Self::Cancelled => "Authorization cancelled",
            Self::Ended => "Monitoring session ended",
            Self::Live => "Monitoring",
            Self::Stale => "Collector is not updating; rates unavailable",
            Self::Disconnected => "Collector disconnected; rates unavailable",
            Self::Unavailable(error) => error,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    Application,
    #[default]
    Download,
    Upload,
    Received,
    Sent,
}
impl Sort {
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn column(index: usize) -> Self {
        match index {
            0 => Self::Application,
            1 => Self::Download,
            2 => Self::Upload,
            3 => Self::Received,
            _ => Self::Sent,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub search: String,
    pub sort: Sort,
    pub ascending: bool,
    pub offset: usize,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            search: String::new(),
            sort: Sort::Download,
            ascending: false,
            offset: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Application {
    pub name: String,
    pub path: Option<String>,
    pub received: u128,
    pub sent: u128,
    pub download: Option<f64>,
    pub upload: Option<f64>,
}
#[derive(Clone, Debug, Default)]
pub struct View {
    pub revision: u64,
    pub query: Query,
    pub status: Status,
    pub endpoint: Option<std::path::PathBuf>,
    pub started_ms: Option<u64>,
    pub received: u128,
    pub sent: u128,
    pub matches: usize,
    pub rows: Vec<Application>,
    pub warning: Option<String>,
}

#[derive(Deserialize)]
struct Report {
    version: u8,
    started_ms: u64,
    sample_ms: u64,
    rows: Vec<Record>,
    lost_operations: u64,
    truncated: bool,
}
#[derive(Deserialize)]
struct Record {
    pid: u32,
    uid: u32,
    start_ns: u64,
    inode: u64,
    device: u32,
    mount_id: u32,
    #[serde(default)]
    exec_id: u64,
    comm_hex: String,
    executable_hex: Option<String>,
    sent: u64,
    received: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    pid: u32,
    uid: u32,
    start: u64,
    inode: u64,
    device: u32,
    mount: u32,
    exec_id: u64,
    comm: Vec<u8>,
}
#[derive(Clone)]
struct Sample {
    sent: u64,
    received: u64,
    sent_delta: u64,
    received_delta: u64,
    comm: String,
    path: Option<String>,
}
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum AppIdentity {
    Known {
        device: u32,
        mount: u32,
        inode: u64,
        path: String,
    },
    Unattributed(Identity),
}
pub(super) struct Engine {
    uid: u32,
    started: Option<u64>,
    sample: Option<u64>,
    seconds: Option<f64>,
    records: BTreeMap<Identity, Sample>,
    losses: u64,
    truncated: bool,
}
fn display_bytes(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}
impl Engine {
    pub fn new(uid: u32) -> Self {
        Self {
            uid,
            started: None,
            sample: None,
            seconds: None,
            records: BTreeMap::new(),
            losses: 0,
            truncated: false,
        }
    }
    pub fn ingest(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(bytes.len() <= MAX_FRAME, "Usage snapshot exceeds 2 MiB");
        let report: Report = serde_json::from_slice(bytes)?;
        ensure!(report.version == 1, "Unsupported usage snapshot version");
        ensure!(
            report.rows.len() <= MAX_RECORDS,
            "Usage snapshot exceeds 4,096 records"
        );
        ensure!(
            report.sample_ms >= report.started_ms,
            "Usage timestamp precedes collector startup"
        );
        ensure!(
            self.started.is_none_or(|s| s == report.started_ms),
            "Collector session changed within a connection"
        );
        ensure!(
            self.sample.is_none_or(|s| report.sample_ms > s),
            "Usage timestamp did not advance"
        );
        ensure!(
            report.lost_operations >= self.losses,
            "Collector loss counter moved backwards"
        );
        let mut prepared = Vec::with_capacity(report.rows.len());
        let mut identities = BTreeSet::new();
        for row in report.rows {
            ensure!(
                row.uid == self.uid && row.pid > 0 && row.pid <= i32::MAX as u32,
                "Usage record has an unexpected process owner"
            );
            ensure!(
                row.start_ns > 0 && row.inode > 0 && row.mount_id > 0,
                "Usage record lacks executable identity"
            );
            ensure!(row.comm_hex.len() <= 32, "Process name exceeds 16 bytes");
            let comm = hex::decode(row.comm_hex)?;
            let path = row
                .executable_hex
                .map(|text| {
                    ensure!(text.len() <= 1024, "Executable path exceeds 512 bytes");
                    let bytes = hex::decode(text)?;
                    ensure!(
                        bytes.first() == Some(&b'/') && !bytes.contains(&0),
                        "Executable path is not absolute"
                    );
                    Ok::<_, anyhow::Error>(display_bytes(&bytes))
                })
                .transpose()?;
            let identity = Identity {
                pid: row.pid,
                uid: row.uid,
                start: row.start_ns,
                inode: row.inode,
                device: row.device,
                mount: row.mount_id,
                exec_id: row.exec_id,
                comm: comm.clone(),
            };
            ensure!(
                identities.insert(identity.clone()),
                "Usage snapshot contains a duplicate identity"
            );
            let previous = self.records.get(&identity);
            ensure!(
                previous.is_none_or(|p| row.sent >= p.sent && row.received >= p.received),
                "Usage counters moved backwards"
            );
            let sample = Sample {
                sent: row.sent,
                received: row.received,
                sent_delta: row.sent - previous.map_or(0, |p| p.sent),
                received_delta: row.received - previous.map_or(0, |p| p.received),
                comm: display_bytes(&comm),
                path: path.or_else(|| previous.and_then(|p| p.path.clone())),
            };
            prepared.push((identity, sample));
        }
        let added = prepared
            .iter()
            .filter(|(id, _)| !self.records.contains_key(id))
            .count();
        ensure!(
            self.records.len() + added <= MAX_RECORDS,
            "Usage session exceeds 4,096 identities"
        );
        if !report.truncated {
            ensure!(
                self.records.keys().all(|id| identities.contains(id)),
                "Collector omitted previously reported counters"
            );
        }
        // Validate the whole frame before publishing any part of it.
        for sample in self.records.values_mut() {
            sample.sent_delta = 0;
            sample.received_delta = 0;
        }
        for (id, sample) in prepared {
            self.records.insert(id, sample);
        }
        // A missing record spans more than one interval. Do not turn that gap
        // into zero traffic, or divide its later catch-up bytes by one interval.
        self.seconds = self
            .sample
            .filter(|_| {
                !self.truncated && !report.truncated && self.losses == report.lost_operations
            })
            .map(|p| (report.sample_ms - p) as f64 / 1000.);
        self.started = Some(report.started_ms);
        self.sample = Some(report.sample_ms);
        self.losses = report.lost_operations;
        self.truncated = report.truncated;
        Ok(())
    }
    pub fn view(
        &self,
        query: &Query,
        status: Status,
        endpoint: Option<std::path::PathBuf>,
        revision: u64,
    ) -> Arc<View> {
        let live = status == Status::Live;
        let seconds = self.seconds.filter(|_| live);
        let mut apps = BTreeMap::<AppIdentity, Application>::new();
        let mut received = 0u128;
        let mut sent = 0u128;
        for (id, record) in &self.records {
            // A recycled inode at another path must not inherit an old app's
            // label. Unknown paths remain unattributed process records.
            let identity = match &record.path {
                Some(path) => AppIdentity::Known {
                    device: id.device,
                    mount: id.mount,
                    inode: id.inode,
                    path: path.clone(),
                },
                None => AppIdentity::Unattributed(id.clone()),
            };
            let app = apps.entry(identity).or_insert_with(|| Application {
                name: record
                    .path
                    .as_deref()
                    .and_then(|p| p.rsplit('/').next())
                    .unwrap_or(&record.comm)
                    .to_owned(),
                path: record.path.clone(),
                received: 0,
                sent: 0,
                download: seconds.map(|_| 0.),
                upload: seconds.map(|_| 0.),
            });
            app.received += record.received as u128;
            app.sent += record.sent as u128;
            received += record.received as u128;
            sent += record.sent as u128;
            if let Some(seconds) = seconds {
                *app.download.as_mut().unwrap() += record.received_delta as f64 / seconds;
                *app.upload.as_mut().unwrap() += record.sent_delta as f64 / seconds;
            }
        }
        let search = query.search.to_lowercase();
        let mut apps = apps
            .into_values()
            .filter(|app| {
                app.name.to_lowercase().contains(&search)
                    || app
                        .path
                        .as_ref()
                        .is_some_and(|p| p.to_lowercase().contains(&search))
            })
            .collect::<Vec<_>>();
        apps.sort_by(|a, b| {
            let order = match query.sort {
                Sort::Application => a.name.cmp(&b.name),
                Sort::Download => a
                    .download
                    .unwrap_or(-1.)
                    .total_cmp(&b.download.unwrap_or(-1.)),
                Sort::Upload => a.upload.unwrap_or(-1.).total_cmp(&b.upload.unwrap_or(-1.)),
                Sort::Received => a.received.cmp(&b.received),
                Sort::Sent => a.sent.cmp(&b.sent),
            };
            let order = if query.ascending {
                order
            } else {
                order.reverse()
            };
            order.then(a.name.cmp(&b.name)).then(a.path.cmp(&b.path))
        });
        let matches = apps.len();
        let rows = apps
            .into_iter()
            .skip(query.offset.min(MAX_RECORDS))
            .take(PAGE)
            .collect();
        let warning = (self.losses > 0 || self.truncated).then(|| {
            format!(
                "Incomplete measurements: {} lost operations{}",
                self.losses,
                if self.truncated {
                    "; snapshot truncated"
                } else {
                    ""
                }
            )
        });
        Arc::new(View {
            revision,
            query: query.clone(),
            status,
            endpoint,
            started_ms: self.started,
            received,
            sent,
            matches,
            rows,
            warning,
        })
    }
}

pub fn bytes(value: u128) -> String {
    let mut amount = value as f64;
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut unit = 0;
    while amount >= 1024. && unit < units.len() - 1 {
        amount /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{amount:.1} {}", units[unit])
    }
}
pub fn rate(value: Option<f64>) -> String {
    value
        .map(|value| format!("{}/s", bytes(value.round() as u128)))
        .unwrap_or_else(|| "—".into())
}

#[cfg(test)]
mod tests;
