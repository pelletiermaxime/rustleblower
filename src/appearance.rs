//! Bounded disk preferences for native color mode. Reads/writes belong on an
//! I/O executor; no appearance state participates in permission admission/reply.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    System,
    Light,
    Dark,
}
impl ColorMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    color_mode: ColorMode,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub mode: ColorMode,
    pub warning: Option<String>,
    hash: Option<[u8; 32]>,
}
#[derive(Clone)]
pub struct Store {
    pub path: PathBuf,
}
impl Store {
    pub fn new(database: &Path) -> Self {
        let mut path = database.as_os_str().to_owned();
        path.push(".appearance.json");
        Self {
            path: PathBuf::from(path),
        }
    }
    fn bytes(&self) -> Result<Option<Vec<u8>>> {
        let meta = match std::fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "Appearance preferences are not a regular file; the entry was preserved"
        );
        let mut bytes = Vec::new();
        File::open(&self.path)?.take(4097).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 4096,
            "Appearance preferences exceed 4 KiB; the file was preserved"
        );
        Ok(Some(bytes))
    }
    pub fn load(&self) -> Result<Snapshot> {
        let Some(bytes) = self.bytes()? else {
            return Ok(Snapshot {
                mode: ColorMode::System,
                warning: None,
                hash: None,
            });
        };
        let parsed = serde_json::from_slice::<Document>(&bytes)
            .ok()
            .filter(|d| d.version == 1);
        Ok(Snapshot{mode:parsed.as_ref().map_or(ColorMode::System,|d|d.color_mode),warning:parsed.is_none().then(||"Appearance preferences are invalid or use an unsupported version. Using System; choosing a color mode explicitly replaces this file.".into()),hash:Some(Sha256::digest(bytes).into())})
    }
    pub fn save(&self, expected: &Snapshot, mode: ColorMode) -> Result<Snapshot> {
        let hash = |bytes: Option<Vec<u8>>| bytes.map(|b| Sha256::digest(b).into());
        ensure!(
            hash(self.bytes()?) == expected.hash,
            "Appearance preferences changed; reload before saving"
        );
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(
            &mut file,
            &Document {
                version: 1,
                color_mode: mode,
            },
        )?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        ensure!(
            hash(self.bytes()?) == expected.hash,
            "Appearance preferences changed while saving; reload before retrying"
        );
        if expected.hash.is_some() {
            file.persist(&self.path).map_err(|e| e.error)?;
        } else {
            file.persist_noclobber(&self.path).map_err(|e| e.error)?;
        }
        self.load()
    }
}
