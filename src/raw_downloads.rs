//! Optional raw URL-to-file action. Independent from normalized subscriptions
//! and daemon live download tasks; disabled until explicitly configured.
mod store;
mod transfer;
mod worker;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};
pub use transfer::{export_source, import_source};
pub use worker::{Handle, Worker};
pub const MAX_CONFIG: usize = 64 * 1024;
pub const MAX_FILE: u64 = 16 * 1024 * 1024;
pub const MAX_BATCH: u64 = 128 * 1024 * 1024;
pub const MAX_RESOURCES: usize = 64;
type Extra = BTreeMap<String, serde_json::Value>;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Seconds,
    Minutes,
    #[default]
    Hours,
    Days,
}
impl Unit {
    pub fn label(self) -> &'static str {
        match self {
            Self::Seconds => "seconds",
            Self::Minutes => "minutes",
            Self::Hours => "hours",
            Self::Days => "days",
        }
    }
    fn multiplier(self) -> f64 {
        match self {
            Self::Seconds => 1.,
            Self::Minutes => 60.,
            Self::Hours => 3600.,
            Self::Days => 86400.,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub name: String,
    pub enabled: bool,
    pub remote: String,
    pub localfile: PathBuf,
    #[serde(default)]
    pub extra: Extra,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    pub interval: String,
    pub units: Unit,
    pub urls: Vec<Resource>,
    #[serde(default)]
    pub notify: serde_json::Value,
    #[serde(default)]
    pub extra: Extra,
}
impl Group {
    pub fn duration(&self) -> Result<Duration> {
        ensure!(
            !self.interval.is_empty() && self.interval.len() <= 16,
            "Interval is too long"
        );
        let seconds = self.interval.parse::<f64>()? * self.units.multiplier();
        ensure!(
            seconds.is_finite() && (1.0..=31_536_000.).contains(&seconds),
            "Interval must be one second through 365 days"
        );
        Ok(Duration::from_secs_f64(seconds))
    }
    pub fn message(&self, success: bool) -> Option<String> {
        self.notify
            .get(if success { "success" } else { "error" })?
            .get("desktop")?
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMetadata {
    #[serde(default)]
    pub outer: Extra,
    #[serde(default)]
    pub actions: Extra,
    #[serde(default)]
    pub downloader: Extra,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub groups: Vec<Group>,
    #[serde(default)]
    pub source: SourceMetadata,
}
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawDownloadSettings")
            .field("enabled", &self.enabled)
            .field("groups", &self.groups.len())
            .finish_non_exhaustive()
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.groups.len() <= 16, "Use at most 16 download groups");
        let mut names = BTreeSet::new();
        let mut paths = BTreeSet::new();
        let mut count = 0;
        for group in &self.groups {
            ensure!(
                name(&group.name) && names.insert(group.name.clone()),
                "Group names must be unique, nonempty and at most 128 bytes"
            );
            group.duration()?;
            count += group.urls.len();
            ensure!(count <= MAX_RESOURCES, "Use at most 64 resources in total");
            for resource in &group.urls {
                ensure!(name(&resource.name), "Resource names must be 1–128 bytes");
                let _ = url(&resource.remote)?;
                path(&resource.localfile)?;
                ensure!(
                    paths.insert(resource.localfile.clone()),
                    "Each resource must have a distinct destination"
                );
            }
            for kind in ["success", "error"] {
                if let Some(value) = group.notify.get(kind).and_then(|v| v.get("desktop")) {
                    ensure!(
                        value.as_str().is_some_and(|text| text.len() <= 1024
                            && !text.chars().any(|c| c.is_control() && c != '\n')),
                        "Desktop messages must be at most 1024 bytes"
                    );
                }
            }
        }
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_CONFIG,
            "Configuration exceeds 64 KiB"
        );
        Ok(())
    }
}
fn name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
pub(super) fn path(value: &Path) -> Result<()> {
    let text = value
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Use a text file path"))?;
    ensure!(
        value.is_absolute()
            && text.len() <= 4096
            && !text.chars().any(char::is_control)
            && value.file_name().is_some(),
        "Choose an absolute destination file path"
    );
    ensure!(
        value.components().all(|c| !matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )),
        "Destination must not contain dot path components"
    );
    Ok(())
}
pub(super) fn url(value: &str) -> Result<reqwest::Url> {
    ensure!(
        value.len() <= 2048 && !value.chars().any(char::is_control),
        "URL is too long or contains controls"
    );
    let url = reqwest::Url::parse(value)?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "Use an HTTP(S) URL without credentials or fragment"
    );
    Ok(url)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Running,
    Published,
    Failed,
    Cancelled,
    Stale,
    TooLarge,
    Protected,
    Changed,
    NoEnabledResources,
    RecoveryNeeded,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Running => "Downloading…",
            Self::Published => "Published",
            Self::Failed => "Download failed",
            Self::Cancelled => "Cancelled",
            Self::Stale => "Settings changed; previous file kept",
            Self::TooLarge => "Download exceeds the configured limits",
            Self::Protected => "Destination is protected or unsupported",
            Self::Changed => "Destination changed; reload before downloading",
            Self::NoEnabledResources => "No enabled resources",
            Self::RecoveryNeeded => "Recovery state needs attention",
        }
    }
}
#[derive(Clone, Debug)]
pub struct ResourceReport {
    pub name: String,
    pub localfile: PathBuf,
    pub status: Status,
    pub bytes: u64,
}
#[derive(Clone, Debug)]
pub struct GroupReport {
    pub name: String,
    pub status: Status,
    pub resources: Vec<ResourceReport>,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub completed_runs: u64,
    pub ready: bool,
    pub settings: Settings,
    pub revision: Option<String>,
    pub warning: Option<String>,
    pub busy: bool,
    pub active_group: Option<String>,
    pub reports: Vec<GroupReport>,
}
#[derive(Clone, Debug)]
pub struct Notice {
    pub group: String,
    pub success: bool,
    pub message: String,
    pub failed_names: Vec<String>,
    pub failed_count: usize,
}
pub enum Command {
    Reload,
    Save {
        expected_revision: Option<String>,
        settings: Settings,
    },
    Import {
        expected_revision: Option<String>,
        json: String,
    },
    Export,
    ReadProfile {
        path: PathBuf,
    },
    WriteProfile {
        path: PathBuf,
        expected_revision: Option<String>,
    },
    RunNow {
        group: String,
        expected_revision: Option<String>,
    },
    Cancel,
}
pub enum Reply {
    State(Snapshot),
    Export(String),
    Profile(Settings),
    Written(PathBuf),
}
pub fn path_for(database: &Path) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(".raw-downloads.json");
    PathBuf::from(path)
}
pub fn journal_path(database: &Path) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(".raw-downloads-journal.json");
    PathBuf::from(path)
}
#[cfg(test)]
mod tests;
