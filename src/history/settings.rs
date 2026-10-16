//! Small column preferences, independent of the permission preference document.
use super::query::{FIELDS, Field, Filter};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
};

/// Persist user choices, never row identities, keyset cursors or old ceilings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presentation {
    pub search: String,
    pub action: String,
    pub filters: Vec<Filter>,
    pub sort: Option<(Field, bool)>,
    pub widths: Vec<(Field, f32)>,
}
impl Presentation {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.search.len() <= 256, "History search exceeds 256 bytes");
        ensure!(
            matches!(self.action.as_str(), "" | "allow" | "deny" | "reject"),
            "Invalid history action filter"
        );
        ensure!(
            self.filters.len() <= 8,
            "Use at most eight history conditions"
        );
        for filter in &self.filters {
            filter.validate()?;
        }
        ensure!(self.widths.len() <= FIELDS.len(), "Too many history widths");
        for (index, (field, width)) in self.widths.iter().enumerate() {
            ensure!(
                width.is_finite() && (40. ..=2000.).contains(width),
                "Invalid history column width"
            );
            ensure!(
                !self.widths[..index].iter().any(|(old, _)| old == field),
                "Repeated history column width"
            );
        }
        Ok(())
    }
    pub fn query(&self) -> super::Query {
        super::Query {
            search: self.search.clone(),
            action: self.action.clone(),
            filters: self.filters.clone(),
            sort: self.sort,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Options {
    pub version: u8,
    pub table: bool,
    pub columns: Vec<Field>,
    #[serde(default = "default_refresh")]
    pub refresh_seconds: u16,
    #[serde(default)]
    pub presentation: Presentation,
}
fn default_refresh() -> u16 {
    1
}
impl Default for Options {
    fn default() -> Self {
        Self {
            version: 3,
            table: false,
            refresh_seconds: 1,
            columns: vec![
                Field::Time,
                Field::Action,
                Field::Process,
                Field::Host,
                Field::Port,
                Field::Protocol,
            ],
            presentation: Presentation::default(),
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 3, "Unsupported history settings version");
        self.presentation.validate()?;
        ensure!(
            !self.columns.is_empty() && self.columns.len() <= super::query::FIELDS.len(),
            "Choose at least one history column"
        );
        ensure!(
            self.refresh_seconds <= 60,
            "Refresh interval must be 0 (manual) or 1–60 seconds"
        );
        for (index, field) in self.columns.iter().enumerate() {
            ensure!(
                !self.columns[..index].contains(field),
                "Repeated history column"
            );
        }
        Ok(())
    }
}
pub struct Store {
    path: PathBuf,
    value: RwLock<Options>,
    writer: Mutex<Option<[u8; 32]>>,
    load_error: Option<String>,
}
impl Store {
    /// Presentation settings must never prevent the UI-side RPC service starting.
    /// Keep a damaged file untouched and refuse every write until a fresh open.
    pub fn open_for_app(database: &Path) -> Self {
        Self::open(database).unwrap_or_else(|error| {
            let mut path = database.as_os_str().to_owned();
            path.push(".history.json");
            Self {
                path: PathBuf::from(path), value: RwLock::new(Options::default()),
                writer: Mutex::new(None),
                load_error: Some(crate::history::bounded(&format!("History view could not be restored: {error:#}. Using defaults; the settings file was preserved. Restart after repairing it to resume saving."),1024)),
            }
        })
    }
    pub fn load_error(&self) -> Option<String> {
        self.load_error.clone()
    }
    pub fn open(database: &Path) -> Result<Self> {
        let mut path = database.as_os_str().to_owned();
        path.push(".history.json");
        let path = PathBuf::from(path);
        let bytes = Self::bytes(&path)?;
        let value = match bytes.as_deref() {
            Some(bytes) => {
                let mut options: Options = serde_json::from_slice(bytes)?;
                ensure!(
                    matches!(options.version, 1..=3),
                    "Unsupported history settings version"
                );
                options.version = 3;
                options.validate()?;
                options
            }
            None => Options::default(),
        };
        Ok(Self {
            path,
            value: RwLock::new(value),
            writer: Mutex::new(bytes.map(|bytes| Sha256::digest(bytes).into())),
            load_error: None,
        })
    }
    pub fn snapshot(&self) -> Options {
        self.value.read().unwrap().clone()
    }
    pub fn save(&self, options: Options) -> Result<()> {
        options.validate()?;
        let mut writer = self.writer.lock().unwrap();
        self.write_locked(options, &mut writer)
    }
    pub fn save_refresh(&self, seconds: u16) -> Result<()> {
        let mut writer = self.writer.lock().unwrap();
        let mut options = self.snapshot();
        options.refresh_seconds = seconds;
        options.validate()?;
        self.write_locked(options, &mut writer)
    }
    pub fn save_columns(&self, columns: Options) -> Result<()> {
        let mut writer = self.writer.lock().unwrap();
        let mut options = self.snapshot();
        options.table = columns.table;
        options.columns = columns.columns;
        options.validate()?;
        self.write_locked(options, &mut writer)
    }
    pub fn save_presentation(&self, presentation: Presentation) -> Result<()> {
        presentation.validate()?;
        let mut writer = self.writer.lock().unwrap();
        let mut options = self.snapshot();
        options.presentation = presentation;
        self.write_locked(options, &mut writer)
    }
    fn bytes(path: &Path) -> Result<Option<Vec<u8>>> {
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "History settings are not a regular file; preserved"
        );
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(16385)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 16384,
            "History settings exceed 16 KiB; preserved"
        );
        Ok(Some(bytes))
    }
    fn write_locked(&self, options: Options, expected: &mut Option<[u8; 32]>) -> Result<()> {
        ensure!(
            self.load_error.is_none(),
            "History view was not restored; repair the settings file and restart before saving"
        );
        options.validate()?;
        let hash = |bytes: Option<Vec<u8>>| bytes.map(|bytes| Sha256::digest(bytes).into());
        ensure!(
            hash(Self::bytes(&self.path)?) == *expected,
            "History settings changed externally; save stopped"
        );
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        let bytes = serde_json::to_vec_pretty(&options)?;
        ensure!(bytes.len() <= 16384, "History settings exceed 16 KiB");
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        ensure!(
            hash(Self::bytes(&self.path)?) == *expected,
            "History settings changed while saving; save stopped"
        );
        if expected.is_some() {
            file.persist(&self.path).map_err(|error| error.error)?;
        } else {
            file.persist_noclobber(&self.path)
                .map_err(|error| error.error)?;
        }
        *expected = Some(Sha256::digest(&bytes).into());
        std::fs::File::open(parent)?.sync_all()?;
        *self.value.write().unwrap() = options;
        Ok(())
    }
}
