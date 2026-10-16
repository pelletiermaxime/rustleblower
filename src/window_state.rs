//! Small versioned main-window state, independent of daemon policy/history I/O.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Page {
    #[default]
    History,
    Rules,
    Statistics,
    Nodes,
    Alerts,
    Live,
    Subscriptions,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.x.abs() <= 1_000_000.
            && self.y.abs() <= 1_000_000.
            && (1. ..=32768.).contains(&self.width)
            && (1. ..=32768.).contains(&self.height)
    }
    fn overlap(self, other: Self) -> f32 {
        let w = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let h = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        w.max(0.) * h.max(0.)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub version: u8,
    pub bounds: Rect,
    pub mode: Mode,
    pub page: Page,
}
impl State {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.version == 1 && self.bounds.valid(),
            "Invalid or unsupported window state"
        );
        Ok(())
    }
    /// Pick the most-overlapped current display, or the first display after a
    /// monitor disconnect. Keep the titlebar and bounded restore size reachable.
    pub fn fit(mut self, displays: &[Rect]) -> Result<Self> {
        self.validate()?;
        ensure!(
            !displays.is_empty() && displays.len() <= 16 && displays.iter().all(|r| r.valid()),
            "Invalid current display bounds"
        );
        let mut display = displays[0];
        let mut area = 0.;
        for d in displays {
            let overlap = self.bounds.overlap(*d);
            if overlap > area {
                area = overlap;
                display = *d;
            }
        }
        let width = self.bounds.width.max(780.).min(display.width);
        let height = self.bounds.height.max(560.).min(display.height);
        self.bounds = Rect {
            x: self
                .bounds
                .x
                .clamp(display.x, display.x + display.width - width),
            y: self
                .bounds
                .y
                .clamp(display.y, display.y + display.height - height),
            width,
            height,
        };
        Ok(self)
    }
}
#[derive(Clone)]
pub struct Store {
    pub path: PathBuf,
}
#[derive(Clone)]
pub struct Snapshot {
    pub state: Option<State>,
    hash: Option<[u8; 32]>,
}
impl Store {
    pub fn new(database: &Path) -> Self {
        let mut p = database.as_os_str().to_owned();
        p.push(".window.json");
        Self {
            path: PathBuf::from(p),
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
            "Window state is not a regular file; it was preserved"
        );
        let mut b = Vec::new();
        File::open(&self.path)?.take(4097).read_to_end(&mut b)?;
        ensure!(
            b.len() <= 4096,
            "Window state exceeds 4 KiB; it was preserved"
        );
        Ok(Some(b))
    }
    pub fn load(&self) -> Result<Snapshot> {
        let Some(b) = self.bytes()? else {
            return Ok(Snapshot {
                state: None,
                hash: None,
            });
        };
        let state: State = serde_json::from_slice(&b)?;
        state.validate()?;
        Ok(Snapshot {
            state: Some(state),
            hash: Some(Sha256::digest(b).into()),
        })
    }
    pub fn save(&self, expected: &Snapshot, state: State) -> Result<Snapshot> {
        state.validate()?;
        let hash = |v: Option<Vec<u8>>| v.map(|b| Sha256::digest(b).into());
        ensure!(
            hash(self.bytes()?) == expected.hash,
            "Window state changed externally; automatic saves stopped"
        );
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut f = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut f, &state)?;
        f.write_all(b"\n")?;
        f.as_file().sync_all()?;
        ensure!(
            hash(self.bytes()?) == expected.hash,
            "Window state changed while saving; automatic saves stopped"
        );
        if expected.hash.is_some() {
            f.persist(&self.path).map_err(|e| e.error)?;
        } else {
            f.persist_noclobber(&self.path).map_err(|e| e.error)?;
        }
        File::open(parent)?.sync_all()?;
        self.load()
    }
}
