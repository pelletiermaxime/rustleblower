//! Data-only, bounded native localization. Catalogue I/O is explicit and belongs
//! on a background executor. Translation and formatting never access the disk.
use anyhow::{Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, RwLock},
};

pub const MAX_FILES: usize = 32;
pub const MAX_BYTES: usize = 512 * 1024;
pub const MAX_MESSAGES: usize = 4096;
pub const MAX_VALUE_BYTES: usize = 2048;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LocaleId(String);
impl LocaleId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for LocaleId {
    type Error = &'static str;
    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        if value.is_empty() || value.len() > 32 || value.eq_ignore_ascii_case("system") {
            return Err("locale must contain 1–32 ASCII bytes");
        }
        let normalized = value.replace('_', "-");
        let mut parts = Vec::new();
        for (i, part) in normalized.split('-').enumerate() {
            if part.is_empty()
                || part.len() > 8
                || !part.bytes().all(|c| c.is_ascii_alphanumeric())
                || (i == 0 && (part.len() < 2 || !part.bytes().all(|c| c.is_ascii_alphabetic())))
            {
                return Err("locale must use bounded language, script and region subtags");
            }
            parts.push(if i == 0 {
                part.to_ascii_lowercase()
            } else if part.len() == 4 && part.bytes().all(|c| c.is_ascii_alphabetic()) {
                let mut p = part.to_ascii_lowercase();
                p[0..1].make_ascii_uppercase();
                p
            } else if part.len() == 2 {
                part.to_ascii_uppercase()
            } else {
                part.to_ascii_lowercase()
            });
        }
        Ok(Self(parts.join("-")))
    }
}
impl From<LocaleId> for String {
    fn from(value: LocaleId) -> Self {
        value.0
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum LanguageChoice {
    #[default]
    System,
    Locale(LocaleId),
}
impl TryFrom<String> for LanguageChoice {
    type Error = &'static str;
    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        if value == "system" {
            Ok(Self::System)
        } else {
            LocaleId::try_from(value).map(Self::Locale)
        }
    }
}
impl From<LanguageChoice> for String {
    fn from(value: LanguageChoice) -> Self {
        match value {
            LanguageChoice::System => "system".into(),
            LanguageChoice::Locale(id) => id.0,
        }
    }
}

/// Resolve the first nonempty POSIX variable, without changing process state.
/// C/POSIX and invalid values resolve to English. Encoding/modifier are ignored.
pub fn system_locale(
    lc_all: Option<&str>,
    lc_messages: Option<&str>,
    lang: Option<&str>,
) -> LocaleId {
    let raw = [lc_all, lc_messages, lang]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
        .unwrap_or("en");
    let raw = raw.split(['.', '@']).next().unwrap_or("en");
    if raw.eq_ignore_ascii_case("C") || raw.eq_ignore_ascii_case("POSIX") {
        return english_id();
    }
    LocaleId::try_from(raw.to_owned()).unwrap_or_else(|_| english_id())
}
pub fn environment_locale() -> LocaleId {
    system_locale(
        std::env::var("LC_ALL").ok().as_deref(),
        std::env::var("LC_MESSAGES").ok().as_deref(),
        std::env::var("LANG").ok().as_deref(),
    )
}
fn english_id() -> LocaleId {
    LocaleId("en".into())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    locale: LocaleId,
    name: String,
    #[serde(deserialize_with = "unique_messages")]
    messages: BTreeMap<String, String>,
}
fn unique_messages<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<BTreeMap<String, String>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a bounded map with unique message keys")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some((k, v)) = map.next_entry::<String, String>()? {
                if out.len() >= MAX_MESSAGES || out.insert(k, v).is_some() {
                    return Err(serde::de::Error::custom(
                        "too many or duplicate message keys",
                    ));
                }
            }
            Ok(out)
        }
    }
    d.deserialize_map(Visitor)
}
fn placeholders(text: &str) -> Result<BTreeSet<&str>> {
    let mut rest = text;
    let mut names = BTreeSet::new();
    while let Some((_, tail)) = rest.split_once("%{") {
        let (name, next) = tail
            .split_once('}')
            .ok_or_else(|| anyhow!("unterminated named placeholder"))?;
        ensure!(
            !name.is_empty()
                && name.len() <= 48
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "invalid named placeholder"
        );
        names.insert(name);
        ensure!(names.len() <= 16, "too many named placeholders");
        rest = next;
    }
    Ok(names)
}
fn parse(bytes: &[u8], schema: Option<&BTreeMap<String, String>>) -> Result<Document> {
    ensure!(
        bytes.len() <= MAX_BYTES,
        "language catalogue exceeds 512 KiB"
    );
    let doc: Document = serde_json::from_slice(bytes)?;
    ensure!(doc.version == 1, "unsupported language catalogue version");
    ensure!(
        !doc.name.trim().is_empty()
            && doc.name.len() <= 80
            && !doc.name.chars().any(char::is_control),
        "invalid native language name"
    );
    for (key, value) in &doc.messages {
        ensure!(
            !key.is_empty()
                && key.len() <= 96
                && key.is_ascii()
                && !key.chars().any(char::is_control),
            "invalid message key"
        );
        ensure!(
            value.len() <= MAX_VALUE_BYTES
                && !value.is_empty()
                && !value
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t'),
            "invalid message value"
        );
        let names = placeholders(value)?;
        if let Some(schema) = schema {
            let original = schema
                .get(key)
                .ok_or_else(|| anyhow!("unknown message key: {key}"))?;
            ensure!(
                names == placeholders(original)?,
                "named placeholders differ for {key}"
            );
        }
    }
    Ok(doc)
}
fn english() -> &'static Arc<BTreeMap<String, String>> {
    static EN: OnceLock<Arc<BTreeMap<String, String>>> = OnceLock::new();
    EN.get_or_init(|| {
        Arc::new(
            parse(include_bytes!("../locales/en.json"), None)
                .expect("bundled English catalogue must validate")
                .messages,
        )
    })
}
/// Original bundled native catalogues. External partial catalogues still use
/// per-message English fallback and report their actual coverage.
const BUNDLED: &[(&str, &[u8])] = &[
    ("fr", include_bytes!("../locales/fr.json")),
    ("de", include_bytes!("../locales/de.json")),
    ("es", include_bytes!("../locales/es.json")),
    ("ja", include_bytes!("../locales/ja.json")),
];
fn read(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "language catalogue must be a regular file"
    );
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "language catalogue exceeds 512 KiB"
    );
    Ok(bytes)
}
#[derive(Clone, Debug)]
struct Source {
    path: PathBuf,
    hash: [u8; 32],
}
#[derive(Clone, Debug)]
pub struct Language {
    pub id: LocaleId,
    pub name: String,
    pub translated: usize,
    pub total: usize,
    source: Option<Source>,
}
#[derive(Clone, Debug)]
pub struct Catalogue {
    pub languages: Vec<Language>,
    pub warning: Option<String>,
}
impl Default for Catalogue {
    fn default() -> Self {
        Self::bundled()
    }
}
impl Catalogue {
    pub fn bundled() -> Self {
        let en = english();
        let mut languages = vec![Language {
            id: english_id(),
            name: "English".into(),
            translated: en.len(),
            total: en.len(),
            source: None,
        }];
        for (id, bytes) in BUNDLED {
            let doc = parse(bytes, Some(en)).expect("bundled catalogue must validate");
            assert_eq!(doc.locale.as_str(), *id, "bundled locale ID must match");
            if *id == "fr" {
                assert_eq!(doc.messages.len(), en.len(), "French must remain complete");
            }
            languages.push(Language {
                id: doc.locale,
                name: doc.name,
                translated: doc.messages.len(),
                total: en.len(),
                source: None,
            });
        }
        Self {
            languages,
            warning: None,
        }
    }
    /// Scan at most 64 directory entries and 32 JSON files. Reject symlinks,
    /// oversized/invalid/duplicate locales; retain no installed message bodies.
    pub fn discover(directory: &Path) -> Result<Self> {
        let mut catalogue = Self::bundled();
        let meta = match std::fs::symlink_metadata(directory) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(catalogue),
            Err(e) => return Err(e.into()),
        };
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "language catalogue must be a regular directory"
        );
        let mut paths = Vec::new();
        for (i, entry) in std::fs::read_dir(directory)?.enumerate() {
            ensure!(i < 64, "language directory exceeds 64 entries");
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "json") {
                paths.push(path);
            }
        }
        ensure!(
            paths.len() <= MAX_FILES,
            "language directory exceeds 32 JSON files"
        );
        paths.sort();
        let mut invalid = 0;
        for path in paths {
            let result = (|| -> Result<Language> {
                let bytes = read(&path)?;
                let doc = parse(&bytes, Some(english()))?;
                ensure!(
                    !catalogue.languages.iter().any(|l| l.id == doc.locale),
                    "duplicate language"
                );
                ensure!(
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .and_then(|s| LocaleId::try_from(s.to_owned()).ok())
                        .as_ref()
                        == Some(&doc.locale),
                    "locale and filename differ"
                );
                Ok(Language {
                    id: doc.locale,
                    name: doc.name,
                    translated: doc.messages.len(),
                    total: english().len(),
                    source: Some(Source {
                        path,
                        hash: Sha256::digest(bytes).into(),
                    }),
                })
            })();
            match result {
                Ok(language) => catalogue.languages.push(language),
                Err(_) => invalid += 1,
            }
        }
        if invalid != 0 {
            catalogue.warning = Some(format!("Skipped {invalid} invalid language catalogue(s)."));
        }
        Ok(catalogue)
    }
    /// Prefer exact locale then progressively shorter subtags, finally English.
    /// Recheck the selected file's hash: discovery and activation form one snapshot.
    pub fn activate(&self, choice: &LanguageChoice, system: &LocaleId) -> Localizer {
        let requested = match choice {
            LanguageChoice::System => system,
            LanguageChoice::Locale(id) => id,
        };
        let mut candidate = requested.as_str();
        let language = loop {
            if let Some(found) = self.languages.iter().find(|l| l.id.as_str() == candidate) {
                break Some(found);
            }
            match candidate.rsplit_once('-') {
                Some((parent, _)) => candidate = parent,
                None => break None,
            }
        };
        let mut localizer = Localizer {
            warning: self.warning.clone(),
            ..Localizer::default()
        };
        let Some(language) = language else {
            localizer.warning = Some("The selected language is unavailable; using English.".into());
            return localizer;
        };
        let result = (|| -> Result<BTreeMap<String, String>> {
            if language.id.as_str() == "en" {
                return Ok(BTreeMap::new());
            }
            if let Some(source) = &language.source {
                let bytes = read(&source.path)?;
                ensure!(
                    <[u8; 32]>::from(Sha256::digest(&bytes)) == source.hash,
                    "language catalogue changed; reload before using it"
                );
                let doc = parse(&bytes, Some(english()))?;
                ensure!(
                    doc.locale == language.id,
                    "language catalogue locale changed"
                );
                Ok(doc.messages)
            } else {
                let (_, bytes) = BUNDLED
                    .iter()
                    .find(|(id, _)| *id == language.id.as_str())
                    .ok_or_else(|| anyhow!("bundled language is unavailable"))?;
                Ok(parse(bytes, Some(english()))?.messages)
            }
        })();
        match result {
            Ok(messages) => {
                localizer.locale = language.id.clone();
                localizer.messages = Arc::new(messages);
            }
            Err(e) => {
                localizer.warning = Some(format!(
                    "Cannot load selected language; using English: {e:#}"
                ));
            }
        }
        localizer
    }
}
#[derive(Clone, Debug)]
pub struct Localizer {
    pub locale: LocaleId,
    pub warning: Option<String>,
    messages: Arc<BTreeMap<String, String>>,
}
impl Default for Localizer {
    fn default() -> Self {
        Self {
            locale: english_id(),
            warning: None,
            messages: Arc::new(BTreeMap::new()),
        }
    }
}
impl Localizer {
    /// Actual activated catalogue coverage, independent of a saved regional alias.
    /// Invalid/missing selected files activate English and report English coverage.
    pub fn coverage(&self) -> (usize, usize) {
        let total = english().len();
        (
            if self.locale.as_str() == "en" {
                total
            } else {
                self.messages.len()
            },
            total,
        )
    }
    pub fn fallback(warning: String) -> Self {
        Self {
            warning: Some(warning),
            ..Self::default()
        }
    }
    pub fn text(&self, key: &str) -> String {
        self.messages
            .get(key)
            .or_else(|| english().get(key))
            .cloned()
            .unwrap_or_else(|| {
                if key.len() <= 96 {
                    key.to_owned()
                } else {
                    "Unknown message".into()
                }
            })
    }
    /// Values are plain text, never reparsed as placeholders or markup.
    pub fn format(&self, key: &str, args: &[(&str, &str)]) -> Result<String> {
        format_message(&self.text(key), args)
    }
    /// Format a fixed English presentation source. Missing catalogue entries use
    /// validated English formatting; known-source argument mistakes stay visible.
    /// Neither source nor inserted arguments are reparsed as markup or templates.
    pub fn format_source(&self, source: &str, args: &[(&str, &str)]) -> String {
        match source_index().get(source) {
            Some(key) => self.format(key, args),
            None => format_message(source, args),
        }
        .unwrap_or_else(|_| source.to_owned())
    }
}
fn format_message(text: &str, args: &[(&str, &str)]) -> Result<String> {
    ensure!(text.len() <= 16384, "localized source exceeds 16 KiB");
    ensure!(
        args.len() <= 16 && args.iter().all(|(_, v)| v.len() <= 4096),
        "localized arguments exceed bounds"
    );
    let names = placeholders(text)?;
    let provided: BTreeMap<_, _> = args.iter().copied().collect();
    ensure!(
        provided.len() == args.len() && names.iter().copied().eq(provided.keys().copied()),
        "localized arguments do not match named placeholders"
    );
    let mut out = String::new();
    let mut rest = text;
    while let Some((prefix, tail)) = rest.split_once("%{") {
        let (name, next) = tail
            .split_once('}')
            .ok_or_else(|| anyhow!("invalid placeholder"))?;
        out.push_str(prefix);
        out.push_str(provided[name]);
        rest = next;
        ensure!(out.len() <= 16384, "localized message exceeds 16 KiB");
    }
    out.push_str(rest);
    ensure!(out.len() <= 16384, "localized message exceeds 16 KiB");
    Ok(out)
}

fn runtime() -> &'static RwLock<Localizer> {
    static ACTIVE: OnceLock<RwLock<Localizer>> = OnceLock::new();
    ACTIVE.get_or_init(|| RwLock::new(Localizer::default()))
}
pub fn current() -> Localizer {
    runtime().read().unwrap_or_else(|e| e.into_inner()).clone()
}
pub fn text(key: &str) -> String {
    current().text(key)
}
pub fn format(key: &str, args: &[(&str, &str)]) -> Result<String> {
    current().format(key, args)
}
/// Install a fully loaded immutable language; never perform I/O here.
pub fn install(localizer: Localizer) {
    #[cfg(feature = "desktop")]
    {
        register_kit();
        gpui_kit::component::set_locale(localizer.locale.as_str());
    }
    *runtime().write().unwrap_or_else(|e| e.into_inner()) = localizer;
}
/// The pinned Kit exposes its rust-i18n extension; no fork or vendor patch.
#[cfg(feature = "desktop")]
pub fn register_kit() {
    use std::{borrow::Cow, sync::Once};
    struct Backend;
    impl rust_i18n::Backend for Backend {
        fn available_locales(&self) -> Vec<Cow<'_, str>> {
            vec![Cow::Borrowed("en"), Cow::Owned(current().locale.0)]
        }
        fn translate(&self, locale: &str, key: &str) -> Option<Cow<'_, str>> {
            let active = current();
            if locale != active.locale.as_str() && locale != "en" {
                return None;
            }
            let value = if locale == "en" {
                english().get(key).cloned()
            } else {
                active
                    .messages
                    .get(key)
                    .or_else(|| english().get(key))
                    .cloned()
            };
            value.map(Cow::Owned)
        }
        fn messages_for_locale(&self, locale: &str) -> Option<Vec<(Cow<'_, str>, Cow<'_, str>)>> {
            let active = current();
            if locale != active.locale.as_str() && locale != "en" {
                return None;
            }
            Some(
                english()
                    .keys()
                    .map(|key| {
                        (
                            Cow::Owned(key.clone()),
                            Cow::Owned(if locale == "en" {
                                english()[key].clone()
                            } else {
                                active.text(key)
                            }),
                        )
                    })
                    .collect(),
            )
        }
    }
    static BACKEND: Backend = Backend;
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| gpui_kit::component::_rust_i18n_extend(&BACKEND, "gpui_component"));
}

/// Translate a fixed English presentation string. The schema remains stable IDs;
/// the reverse index is built once from the compiled English catalogue.
fn source_index() -> &'static BTreeMap<String, String> {
    static INDEX: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    INDEX.get_or_init(|| {
        english()
            .iter()
            .map(|(key, value)| (value.clone(), key.clone()))
            .collect()
    })
}
pub fn tr(source: &str) -> String {
    source_index()
        .get(source)
        .map(|key| text(key))
        .unwrap_or_else(|| source.to_owned())
}
/// A translated title with a typed, unchanged machine value. The title is read
/// dynamically, so startup's asynchronous install does not reset a selection.
#[cfg(feature = "desktop")]
#[derive(Clone)]
pub struct LocalizedOption<T: Clone + PartialEq> {
    value: T,
    title: OptionTitle,
}
#[cfg(feature = "desktop")]
#[derive(Clone)]
enum OptionTitle {
    Source(String),
    Raw(String),
    Computed(Arc<dyn Fn() -> String + Send + Sync>),
}
#[cfg(feature = "desktop")]
impl<T: Clone + PartialEq> LocalizedOption<T> {
    pub fn new(value: T, source: impl Into<String>) -> Self {
        Self {
            value,
            title: OptionTitle::Source(source.into()),
        }
    }
    pub fn raw(value: T, title: impl Into<String>) -> Self {
        Self {
            value,
            title: OptionTitle::Raw(title.into()),
        }
    }
    pub(crate) fn with_title(value: T, title: impl Fn() -> String + Send + Sync + 'static) -> Self {
        Self {
            value,
            title: OptionTitle::Computed(Arc::new(title)),
        }
    }
}
#[cfg(feature = "desktop")]
impl<T: Clone + PartialEq> gpui_kit::component::searchable_list::SearchableListItem
    for LocalizedOption<T>
{
    type Value = T;
    fn title(&self) -> gpui::SharedString {
        match &self.title {
            OptionTitle::Source(source) => tr(source),
            OptionTitle::Raw(raw) => raw.clone(),
            OptionTitle::Computed(title) => title(),
        }
        .into()
    }
    fn value(&self) -> &Self::Value {
        &self.value
    }
}
#[cfg(feature = "desktop")]
mod inputs {
    use super::*;
    use gpui::{AnyWindowHandle, App, Context, Global, WeakEntity, Window};
    use gpui_kit::component::input::InputState;
    #[derive(Clone)]
    struct Entry {
        input: WeakEntity<InputState>,
        window: AnyWindowHandle,
        source: &'static str,
    }
    #[derive(Default)]
    struct Registry(Vec<Entry>);
    impl Global for Registry {}
    pub fn placeholder(
        source: &'static str,
        window: &Window,
        cx: &mut Context<InputState>,
    ) -> String {
        if !cx.has_global::<Registry>() {
            cx.set_global(Registry::default());
        }
        let entry = Entry {
            input: cx.weak_entity(),
            window: window.window_handle(),
            source,
        };
        let registry = cx.global_mut::<Registry>();
        registry.0.retain(|e| e.input.upgrade().is_some());
        // Ordinary UI entity construction remains usable even at the bound.
        // Extra windows use their creation-time translation until restarted.
        if registry.0.len() < 512 {
            registry.0.push(entry);
        }
        tr(source)
    }
    pub fn refresh(cx: &mut App) {
        if !cx.has_global::<Registry>() {
            return;
        }
        let entries = cx.global::<Registry>().0.clone();
        for entry in entries {
            let _ = entry.window.update(cx, |_, window, cx| {
                let _ = entry.input.update(cx, |state, cx| {
                    state.set_placeholder(tr(entry.source), window, cx)
                });
            });
        }
        cx.global_mut::<Registry>()
            .0
            .retain(|e| e.input.upgrade().is_some());
    }
}
#[cfg(feature = "desktop")]
pub use inputs::{placeholder, refresh as refresh_inputs};

/// Render an explicitly named presentation template. Schema validation ensures
/// translations keep the same placeholders; a program mismatch stays visible.
pub fn tr_args(source: &str, args: &[(&str, &str)]) -> String {
    current().format_source(source, args)
}

/// Translate explicitly user-facing messages at presentation. Core/control
/// strings remain unchanged; messages outside the schema remain visible as-is.
pub fn presentation(source: &str) -> String {
    current().presentation(source)
}
// Producer contracts reviewed explicitly. These templates are never inferred
// from installed catalogues, so a catalogue cannot broaden what is matched.
const PRESENTATION_TEMPLATES: &[&str] = &[
    "Ignored %{count} invalid or unreadable native palette file(s).",
    "Skipped %{count} invalid language catalogue(s).",
    "Cannot load selected language; using English: %{reason}",
    "Unsupported helper option: %{value}",
    "Invalid firewall JSON: %{reason}",
    "HTTP request failed: %{reason}",
    "Download stream failed: %{reason}",
    "Line %{count} is not valid IP/CIDR data",
    "Cannot retain the previous list: %{reason}",
    "Cannot publish validated list: %{reason}",
    "Collection switched; previous owned generation cleanup failed: %{reason}",
    "Cannot start URL test: %{reason}",
    "URL test failed: %{reason}",
    "URL sample failed: %{reason}",
    "Choose a new export filename: %{reason}",
    "Export saved; directory sync failed: %{reason}",
    "History view could not be restored: %{reason}. Using defaults; the settings file was preserved. Restart after repairing it to resume saving.",
    "Daemon reported a rule or node command error: %{reason}. Its live state may have changed; reconnect to verify.",
    "Waiting to queue the task stop; retrying: %{reason}",
];
impl Localizer {
    /// Localize an explicitly user-facing core message without changing stored
    /// or wire text. Unknown messages, oversized arguments and invalid counts
    /// stay verbatim. Captured values are plain text, never parsed recursively.
    pub fn presentation(&self, source: &str) -> String {
        if let Some(key) = source_index().get(source) {
            return self.text(key);
        }
        if source.len() > 16384 {
            return source.to_owned();
        }
        for template in PRESENTATION_TEMPLATES {
            let Some((prefix, rest)) = template.split_once("%{") else {
                continue;
            };
            let Some((name, suffix)) = rest.split_once('}') else {
                continue;
            };
            let Some(value) = source
                .strip_prefix(prefix)
                .and_then(|s| s.strip_suffix(suffix))
            else {
                continue;
            };
            if value.is_empty()
                || value.len() > 4096
                || value.contains('\0')
                || (name == "count"
                    && (value.len() > 20 || !value.bytes().all(|c| c.is_ascii_digit())))
            {
                continue;
            }
            if let Some(key) = source_index().get(*template)
                && let Ok(translated) = self.format(key, &[(name, value)])
            {
                return translated;
            }
        }
        source.to_owned()
    }
}

#[cfg(feature = "desktop")]
mod titles {
    use super::*;
    use gpui::{AnyWindowHandle, App, Global};
    #[derive(Default)]
    struct Registry(Vec<(AnyWindowHandle, &'static str)>);
    impl Global for Registry {}
    pub fn register(handle: AnyWindowHandle, source: &'static str, cx: &mut App) {
        if !cx.has_global::<Registry>() {
            cx.set_global(Registry::default());
        }
        // Bounded by native windows; prune closed handles before admitting more.
        refresh(cx);
        let registry = cx.global_mut::<Registry>();
        registry
            .0
            .retain(|(window, _)| window.window_id() != handle.window_id());
        if registry.0.len() < 64 {
            registry.0.push((handle, source));
        }
    }
    pub fn refresh(cx: &mut App) {
        if !cx.has_global::<Registry>() {
            return;
        }
        let entries = cx.global::<Registry>().0.clone();
        let mut alive = Vec::with_capacity(entries.len());
        for (handle, source) in entries {
            if handle
                .update(cx, |_, window, _| window.set_window_title(&tr(source)))
                .is_ok()
            {
                alive.push((handle, source));
            }
        }
        cx.global_mut::<Registry>().0 = alive;
    }
}
#[cfg(feature = "desktop")]
pub use titles::{refresh as refresh_titles, register as register_title};
