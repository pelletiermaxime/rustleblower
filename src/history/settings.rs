//! Small column preferences, independent of the permission preference document.
use super::query::Field;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, RwLock},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub version: u8,
    pub table: bool,
    pub columns: Vec<Field>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            version: 1,
            table: false,
            columns: vec![
                Field::Time,
                Field::Action,
                Field::Process,
                Field::Host,
                Field::Port,
                Field::Protocol,
            ],
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported history settings version");
        ensure!(
            !self.columns.is_empty() && self.columns.len() <= super::query::FIELDS.len(),
            "Choose at least one history column"
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
    writer: Mutex<()>,
}
impl Store {
    pub fn open(database: &Path) -> Result<Self> {
        let path = PathBuf::from(format!("{}.history.json", database.display()));
        let value = match std::fs::File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(4097).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 4096, "History settings exceed 4 KiB");
                let options: Options = serde_json::from_slice(&bytes)?;
                options.validate()?;
                options
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Options::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path,
            value: RwLock::new(value),
            writer: Mutex::new(()),
        })
    }
    pub fn snapshot(&self) -> Options {
        self.value.read().unwrap().clone()
    }
    pub fn save(&self, options: Options) -> Result<()> {
        options.validate()?;
        let _writer = self.writer.lock().unwrap();
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        let bytes = serde_json::to_vec_pretty(&options)?;
        ensure!(bytes.len() <= 4096, "History settings exceed 4 KiB");
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(&self.path).map_err(|error| error.error)?;
        *self.value.write().unwrap() = options;
        Ok(())
    }
}
