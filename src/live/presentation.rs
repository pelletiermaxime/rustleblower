//! Socket presentation is local UI state; restoring it never starts a task.
mod store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub use store::{Command, Handle, Snapshot, Worker};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Field {
    Comm,
    Process,
    State,
    SourcePort,
    SourceIp,
    DestinationIp,
    DestinationPort,
    Protocol,
    Uid,
    Pid,
    Family,
    Interface,
    Metadata,
}
pub const FIELDS: [Field; 13] = [
    Field::Comm,
    Field::Process,
    Field::State,
    Field::SourcePort,
    Field::SourceIp,
    Field::DestinationIp,
    Field::DestinationPort,
    Field::Protocol,
    Field::Uid,
    Field::Pid,
    Field::Family,
    Field::Interface,
    Field::Metadata,
];
impl Field {
    pub fn key(self) -> &'static str {
        match self {
            Self::Comm => "comm",
            Self::Process => "process",
            Self::State => "state",
            Self::SourcePort => "source-port",
            Self::SourceIp => "source-ip",
            Self::DestinationIp => "destination-ip",
            Self::DestinationPort => "destination-port",
            Self::Protocol => "protocol",
            Self::Uid => "uid",
            Self::Pid => "pid",
            Self::Family => "family",
            Self::Interface => "interface",
            Self::Metadata => "metadata",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Comm => "Command",
            Self::Process => "Process",
            Self::State => "State",
            Self::SourcePort => "Source port",
            Self::SourceIp => "Source IP",
            Self::DestinationIp => "Destination IP",
            Self::DestinationPort => "Destination port",
            Self::Protocol => "Protocol",
            Self::Uid => "UID",
            Self::Pid => "PID",
            Self::Family => "Family",
            Self::Interface => "Interface",
            Self::Metadata => "Metadata",
        }
    }
    pub fn width(self) -> u16 {
        match self {
            Self::Process | Self::Metadata => 280,
            Self::SourceIp | Self::DestinationIp => 180,
            Self::Comm => 140,
            _ => 110,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sort {
    pub field: Field,
    pub ascending: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub field: Field,
    pub width: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub seconds: u16,
    pub protocol: u16,
    pub family: u16,
    pub state: u8,
    pub search: String,
    pub sort: Sort,
    pub columns: Vec<Column>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            seconds: 5,
            protocol: 0,
            family: 0,
            state: 0,
            search: String::new(),
            sort: Sort {
                field: Field::Process,
                ascending: true,
            },
            columns: FIELDS
                .into_iter()
                .map(|field| Column {
                    field,
                    width: field.width(),
                })
                .collect(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        ensure!(
            [1, 2, 5, 10, 30, 60].contains(&self.seconds),
            "Unsupported live interval"
        );
        ensure!(
            [0, 1, 2, 6, 17, 33, 58, 132, 136, 255].contains(&self.protocol),
            "Unsupported socket protocol"
        );
        ensure!(
            [0, 2, 10, 17, 44].contains(&self.family) && self.state <= 12,
            "Unsupported socket family or state"
        );
        ensure!(self.search.len() <= 256, "Live search exceeds 256 bytes");
        ensure!(
            !self.columns.is_empty() && self.columns.len() <= 13,
            "Choose 1–13 socket columns"
        );
        let mut seen = std::collections::BTreeSet::new();
        ensure!(
            self.columns
                .iter()
                .all(|c| (40..=2000).contains(&c.width) && seen.insert(c.field)),
            "Invalid or duplicate socket columns"
        );
        Ok(())
    }
}
pub fn path_for(database: &Path) -> PathBuf {
    let mut path = database.as_os_str().to_owned();
    path.push(".live-view.json");
    PathBuf::from(path)
}
