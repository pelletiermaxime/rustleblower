//! Bounded disk preferences for native color mode and density. Reads/writes belong on an
//! I/O executor; no appearance state participates in permission admission/reply.
use crate::i18n::LanguageChoice;
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
/// Native counterpart of OpenSnitch's -20..20 density choice. Each step changes
/// the native rem basis by 2%; fixed-pixel content/window bounds remain stable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "i8", into = "i8")]
pub struct DensityScale(i8);
impl DensityScale {
    pub const MIN: i8 = -20;
    pub const MAX: i8 = 20;
    pub fn new(value: i8) -> Option<Self> {
        (Self::MIN..=Self::MAX)
            .contains(&value)
            .then_some(Self(value))
    }
    pub fn value(self) -> i8 {
        self.0
    }
    pub fn percent(self) -> u8 {
        (100 + i16::from(self.0) * 2) as u8
    }
    pub fn factor(self) -> f32 {
        f32::from(self.percent()) / 100.
    }
}
impl TryFrom<i8> for DensityScale {
    type Error = &'static str;
    fn try_from(value: i8) -> std::result::Result<Self, Self::Error> {
        Self::new(value).ok_or("density scale must be between -20 and 20")
    }
}
impl From<DensityScale> for i8 {
    fn from(value: DensityScale) -> Self {
        value.0
    }
}
/// Stable catalogue filename without an extension; never an arbitrary path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PaletteId(String);
impl PaletteId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for PaletteId {
    type Error = &'static str;
    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        if !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            Ok(Self(value))
        } else {
            Err("palette ID must contain 1–64 ASCII letters, digits, hyphens or underscores")
        }
    }
}
impl From<PaletteId> for String {
    fn from(value: PaletteId) -> Self {
        value.0
    }
}
/// Opaque, validated RGB colors. Native palettes contain data only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PaletteColor(u32);
impl PaletteColor {
    pub fn rgb(self) -> u32 {
        self.0
    }
}
impl TryFrom<String> for PaletteColor {
    type Error = &'static str;
    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        if value.len() != 7
            || !value.starts_with('#')
            || !value.bytes().skip(1).all(|c| c.is_ascii_hexdigit())
        {
            return Err("palette colors must be #RRGGBB");
        }
        u32::from_str_radix(&value[1..], 16)
            .map(Self)
            .map_err(|_| "palette colors must be #RRGGBB")
    }
}
impl From<PaletteColor> for String {
    fn from(value: PaletteColor) -> Self {
        format!("#{:06x}", value.0)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteColors {
    pub background: PaletteColor,
    pub foreground: PaletteColor,
    pub primary: PaletteColor,
    pub primary_foreground: PaletteColor,
    pub secondary: PaletteColor,
    pub secondary_foreground: PaletteColor,
    pub muted: PaletteColor,
    pub muted_foreground: PaletteColor,
    pub accent: PaletteColor,
    pub accent_foreground: PaletteColor,
    pub border: PaletteColor,
    pub danger: PaletteColor,
    pub danger_foreground: PaletteColor,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteDocument {
    pub version: u8,
    pub name: String,
    pub mode: ColorMode,
    pub colors: PaletteColors,
}
#[derive(Clone, Debug)]
pub struct Palette {
    pub id: PaletteId,
    pub document: PaletteDocument,
}
#[derive(Clone, Debug, Default)]
pub struct Catalogue {
    pub palettes: Vec<Palette>,
    pub warning: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    color_mode: ColorMode,
    #[serde(default)]
    density_scale: DensityScale,
    #[serde(default)]
    palette_id: Option<PaletteId>,
    #[serde(default)]
    language: SavedLanguage,
}
#[derive(Default)]
struct SavedLanguage {
    choice: LanguageChoice,
    invalid: bool,
}
impl Serialize for SavedLanguage {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.choice.serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for SavedLanguage {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match serde_json::from_value(value) {
            Ok(choice) => Ok(Self {
                choice,
                invalid: false,
            }),
            Err(_) => Ok(Self {
                choice: LanguageChoice::System,
                invalid: true,
            }),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub mode: ColorMode,
    pub density: DensityScale,
    pub palette: Option<PaletteId>,
    pub language: LanguageChoice,
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
    pub fn themes_path(&self) -> PathBuf {
        self.path.with_extension("").with_extension("themes")
    }
    pub fn languages_path(&self) -> PathBuf {
        self.path.with_extension("").with_extension("languages")
    }
    /// One bounded scan performed only by the background appearance worker.
    /// No recursive traversal, symlink following, watcher or automatic retries.
    pub fn catalogue(&self) -> Result<Catalogue> {
        let directory = self.themes_path();
        let meta = match std::fs::symlink_metadata(&directory) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Catalogue::default()),
            Err(e) => return Err(e.into()),
        };
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "Theme catalogue must be a regular directory"
        );
        let mut paths = Vec::new();
        for (index, entry) in std::fs::read_dir(&directory)?.enumerate() {
            ensure!(
                index < 64,
                "Theme directory exceeds 64 entries; reduce it before reloading"
            );
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                paths.push(path);
            }
        }
        paths.sort();
        ensure!(
            paths.len() <= 32,
            "Theme catalogue exceeds 32 JSON files; reduce it before reloading"
        );
        let mut catalogue = Catalogue::default();
        let mut invalid = 0;
        for path in paths {
            let parsed = (|| -> Result<Palette> {
                let id = PaletteId::try_from(
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_owned(),
                )
                .map_err(anyhow::Error::msg)?;
                let meta = std::fs::symlink_metadata(&path)?;
                ensure!(
                    meta.is_file() && !meta.file_type().is_symlink(),
                    "Theme is not a regular file"
                );
                let mut bytes = Vec::new();
                File::open(&path)?
                    .take(16 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 16 * 1024, "Theme exceeds 16 KiB");
                let document: PaletteDocument = serde_json::from_slice(&bytes)?;
                ensure!(
                    document.version == 1 && document.mode != ColorMode::System,
                    "Theme must use version 1 and light or dark mode"
                );
                ensure!(
                    !document.name.trim().is_empty()
                        && document.name.len() <= 80
                        && !document.name.chars().any(char::is_control),
                    "Theme name must contain 1–80 bytes without control characters"
                );
                Ok(Palette { id, document })
            })();
            match parsed {
                Ok(palette) => catalogue.palettes.push(palette),
                Err(_) => invalid += 1,
            }
        }
        catalogue.warning = (invalid > 0)
            .then(|| format!("Ignored {invalid} invalid or unreadable native palette file(s)."));
        Ok(catalogue)
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
                density: DensityScale::default(),
                palette: None,
                language: LanguageChoice::System,
                warning: None,
                hash: None,
            });
        };
        let parsed = serde_json::from_slice::<Document>(&bytes)
            .ok()
            .filter(|d| d.version == 1);
        Ok(Snapshot {
            mode: parsed.as_ref().map_or(ColorMode::System, |d| d.color_mode),
            density: parsed.as_ref().map_or(DensityScale::default(), |d| d.density_scale),
            palette: parsed.as_ref().and_then(|d| d.palette_id.clone()),
            language: parsed.as_ref().map_or(LanguageChoice::System, |d| d.language.choice.clone()),
            warning: parsed.as_ref().is_some_and(|d| d.language.invalid).then(|| "Saved language is invalid. Using System language; other appearance preferences were retained.".into()).or_else(|| parsed.is_none().then(|| "Appearance preferences are invalid or use an unsupported version. Using System and default density; choosing an appearance preference explicitly replaces this file.".into())),
            hash: Some(Sha256::digest(bytes).into()),
        })
    }
    pub fn save(&self, expected: &Snapshot, mode: ColorMode) -> Result<Snapshot> {
        self.save_preferences(expected, mode, expected.density)
    }
    pub fn save_preferences(
        &self,
        expected: &Snapshot,
        mode: ColorMode,
        density: DensityScale,
    ) -> Result<Snapshot> {
        self.save_appearance(expected, mode, density, expected.palette.clone())
    }
    pub fn save_appearance(
        &self,
        expected: &Snapshot,
        mode: ColorMode,
        density: DensityScale,
        palette: Option<PaletteId>,
    ) -> Result<Snapshot> {
        self.save_all(expected, mode, density, palette, expected.language.clone())
    }
    pub fn save_language(&self, expected: &Snapshot, language: LanguageChoice) -> Result<Snapshot> {
        self.save_all(
            expected,
            expected.mode,
            expected.density,
            expected.palette.clone(),
            language,
        )
    }
    pub fn save_all(
        &self,
        expected: &Snapshot,
        mode: ColorMode,
        density: DensityScale,
        palette: Option<PaletteId>,
        language: LanguageChoice,
    ) -> Result<Snapshot> {
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
                density_scale: density,
                palette_id: palette,
                language: SavedLanguage {
                    choice: language,
                    invalid: false,
                },
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
