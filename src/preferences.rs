//! Small UI preferences. Filesystem work never holds the prompt-facing state lock.
use crate::{
    config::Decision,
    prompts::{
        Initial,
        choice::{self, Choice},
    },
};
use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
};

pub const TARGETS: &[(&str, &str)] = &[
    ("connection", "Executable + destination"),
    ("process.path", "This executable"),
    ("app-host", "This executable to the exact hostname"),
    ("app-address", "This executable to the exact IP address"),
    ("process.command", "This command line"),
    ("process.id", "Process ID"),
    ("user.id", "User ID"),
    ("dest.host", "Destination host"),
    ("dest.ip", "Destination IP"),
    ("dest.port", "Destination port"),
    ("protocol", "Protocol"),
];
pub const CONSTRAINTS: &[(&str, &str)] = &[
    ("dest.ip", "Destination IP"),
    ("dest.port", "Destination port"),
    ("user.id", "User ID"),
    ("checksum", "Reported checksum"),
];
pub const FALLBACKS: &[(&str, &str)] = &[
    ("inherit", "Use launch setting"),
    ("allow", "Allow once"),
    ("deny", "Deny once"),
    ("reject", "Reject once"),
];
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Popup {
    pub duration: String,
    pub target: String,
    pub popups: bool,
    pub desktop_notifications: bool,
    pub priority: bool,
    pub logging: bool,
    pub show_advanced: bool,
    pub constraints: Vec<String>,
    pub timeout: Option<u64>,
    pub fallback: Option<Decision>,
    pub position: crate::prompts::placement::Position,
}
impl Default for Popup {
    fn default() -> Self {
        Self {
            duration: "once".into(),
            target: "process.path".into(),
            popups: true,
            desktop_notifications: true,
            priority: false,
            logging: true,
            show_advanced: false,
            constraints: Vec::new(),
            timeout: None,
            fallback: None,
            position: Default::default(),
        }
    }
}
impl Popup {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            choice::DURATIONS
                .iter()
                .any(|(key, _)| *key == self.duration),
            "Unsupported default duration"
        );
        ensure!(
            TARGETS.iter().any(|(key, _)| *key == self.target),
            "Unsupported default match target"
        );
        ensure!(
            self.timeout
                .is_none_or(|seconds| (1..=110).contains(&seconds)),
            "Permission timeout must be between 1 and 110 seconds"
        );
        ensure!(
            self.constraints.len() <= CONSTRAINTS.len(),
            "At most four default conditions are supported"
        );
        let mut seen = std::collections::BTreeSet::new();
        for field in &self.constraints {
            ensure!(
                CONSTRAINTS.iter().any(|(key, _)| *key == field) && seen.insert(field),
                "Unsupported or repeated default condition"
            );
        }
        Ok(())
    }
    pub fn choice(&self, connection: &crate::protocol::Connection) -> Choice {
        self.initial(connection, Decision::Deny).choice
    }
    pub fn initial(&self, connection: &crate::protocol::Connection, fallback: Decision) -> Initial {
        let targets = choice::targets(connection);
        let target = if targets.iter().any(|target| target.key == self.target) {
            self.target.clone()
        } else {
            "connection".into()
        };
        let mut constraints = Vec::new();
        let mut unavailable = Vec::new();
        for key in &self.constraints {
            let found = if key == "checksum" {
                targets
                    .iter()
                    .find(|target| target.key == "process.hash.md5")
                    .or_else(|| {
                        targets
                            .iter()
                            .find(|target| target.key == "process.hash.sha1")
                    })
            } else {
                targets.iter().find(|target| target.key == *key)
            };
            if let Some(target) = found {
                constraints.push(target.key.clone());
            } else {
                unavailable.push(
                    CONSTRAINTS
                        .iter()
                        .find(|(field, _)| *field == key)
                        .map_or(key.as_str(), |(_, label)| *label)
                        .to_string(),
                );
            }
        }
        Initial {
            choice: Choice {
                duration: self.duration.clone(),
                target,
                priority: self.priority,
                logging: self.logging,
                constraints,
            },
            expanded: self.show_advanced || !unavailable.is_empty(),
            unavailable,
            fallback: Some(fallback),
            position: self.position,
        }
    }
    fn json(&self) -> serde_json::Value {
        serde_json::json!({"version":4,"duration":self.duration,"target":self.target,"popups":self.popups,"desktop_notifications":self.desktop_notifications,"priority":self.priority,"logging":self.logging,"show_advanced":self.show_advanced,"constraints":self.constraints,"timeout":self.timeout,"fallback":self.fallback.map(Decision::action),"position":self.position.key()})
    }
    fn parse(bytes: &[u8]) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        ensure!(
            matches!(value["version"].as_u64(), Some(1..=4)),
            "Unsupported UI preferences version"
        );
        let text = |key| {
            value[key]
                .as_str()
                .map(str::to_owned)
                .with_context(|| format!("Missing preferences field {key}"))
        };
        let flag = |key| {
            value[key]
                .as_bool()
                .with_context(|| format!("Missing preferences field {key}"))
        };
        let popup = Self {
            duration: text("duration")?,
            target: text("target")?,
            popups: flag("popups")?,
            desktop_notifications: value
                .get("desktop_notifications")
                .map(|value| {
                    value
                        .as_bool()
                        .context("Invalid desktop notifications preference")
                })
                .transpose()?
                .unwrap_or(true),
            priority: flag("priority")?,
            logging: flag("logging")?,
            position: match value.get("position") {
                None => Default::default(),
                Some(value) => value
                    .as_str()
                    .and_then(crate::prompts::placement::Position::parse)
                    .context("Invalid popup position")?,
            },
            show_advanced: value
                .get("show_advanced")
                .map(|value| value.as_bool().context("Invalid show_advanced preference"))
                .transpose()?
                .unwrap_or(false),
            constraints: match value.get("constraints") {
                None => Vec::new(),
                Some(value) => value
                    .as_array()
                    .context("Invalid default conditions")?
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .context("Invalid default condition")
                    })
                    .collect::<Result<Vec<_>>>()?,
            },
            timeout: match value.get("timeout") {
                None | Some(serde_json::Value::Null) => None,
                Some(value) => Some(value.as_u64().context("Invalid permission timeout")?),
            },
            fallback: match value.get("fallback") {
                None | Some(serde_json::Value::Null) => None,
                Some(value) => Some(
                    [Decision::Allow, Decision::Deny, Decision::Reject]
                        .into_iter()
                        .find(|decision| Some(decision.action()) == value.as_str())
                        .context("Invalid permission fallback")?,
                ),
            },
        };
        popup.validate()?;
        Ok(popup)
    }
}
pub struct Store {
    path: PathBuf,
    state: RwLock<Popup>,
    writer: Mutex<()>,
}
impl Store {
    pub fn open(database: &Path) -> Result<Self> {
        let mut name = database.as_os_str().to_owned();
        name.push(".preferences.json");
        let path = PathBuf::from(name);
        let state = match File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(4097).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 4096, "UI preferences exceed 4 KiB");
                Popup::parse(&bytes)
                    .with_context(|| format!("Invalid UI preferences at {}", path.display()))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Popup::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path,
            state: RwLock::new(state),
            writer: Mutex::new(()),
        })
    }
    pub fn snapshot(&self) -> Popup {
        self.state.read().unwrap().clone()
    }
    /// Call on a background executor. Serialize writers without locking snapshots during I/O.
    pub fn save(&self, value: Popup) -> Result<()> {
        value.validate()?;
        let _writer = self.writer.lock().unwrap();
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut file, &value.json())?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(&self.path)?;
        *self.state.write().unwrap() = value;
        Ok(())
    }
}
