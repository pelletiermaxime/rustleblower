//! Bounded configuration interchange. No source directories or cached downloads
//! are adopted, and preserved action metadata is never executed.
use super::{
    bulk::Scope,
    limits::{Override, Unit, Units},
    model::*,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::Path,
};
pub const MAX_DOCUMENT: usize = 2 * 1024 * 1024;
pub const MAX_REVIEW_BYTES: usize = 6 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Native,
    OpenSnitch,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u32,
    pub settings: Settings,
    pub subscriptions: Vec<Spec>,
    #[serde(default = "empty_object")]
    pub opensnitch: Value,
}
fn empty_object() -> Value {
    json!({})
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Existing {
    pub id: u64,
    pub revision: u64,
    pub spec: Spec,
    pub removed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub before: Option<Existing>,
    pub after: Spec,
    pub conflict: bool,
    pub metadata: Option<Value>,
    pub metadata_changed: bool,
    pub resolution: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub document: Document,
    pub replace: bool,
    pub defaults: bool,
    pub replace_catalogue: bool,
    pub removals: Vec<Existing>,
    pub archived_remove: Vec<u64>,
    pub entries: Vec<Entry>,
    pub catalogue: Vec<(u64, u64)>,
    pub settings_before: (u64, Settings),
    pub template_revision: u64,
    pub template_after: Value,
}
impl Review {
    pub fn changed(&self) -> usize {
        self.removals.len()
            + self
                .entries
                .iter()
                .filter(|e| {
                    !e.conflict
                        && (e.metadata_changed
                            || e.before
                                .as_ref()
                                .is_none_or(|b| b.removed || b.spec != e.after))
                })
                .count()
    }
    pub fn review_len(&self) -> usize {
        self.entries.len() + self.removals.len()
    }
    pub fn conflicts(&self) -> usize {
        self.entries.iter().filter(|e| e.conflict).count()
    }
    pub fn validate(&self) -> Result<(), String> {
        self.document.validate()?;
        if self.entries.len() > MAX_SUBSCRIPTIONS
            || self.catalogue.len() > MAX_RECORDS
            || self.removals.len() > MAX_SUBSCRIPTIONS
            || self.archived_remove.len() > MAX_RECORDS
        {
            return Err("Import review exceeds catalogue bounds".into());
        }
        if serde_json::to_vec(&self.template_after)
            .map_err(|e| e.to_string())?
            .len()
            > MAX_DOCUMENT
        {
            return Err("Preserved action metadata exceeds 2 MiB".into());
        }
        let mut payload = serde_json::to_vec(&self.document)
            .map_err(|e| e.to_string())?
            .len()
            + serde_json::to_vec(&self.template_after)
                .map_err(|e| e.to_string())?
                .len()
            + 8192;
        for entry in &self.entries {
            entry.after.validate()?;
            payload += serde_json::to_vec(&entry.after)
                .map_err(|e| e.to_string())?
                .len()
                + entry.resolution.len()
                + 64;
            if let Some(before) = &entry.before {
                payload += serde_json::to_vec(&before.spec)
                    .map_err(|e| e.to_string())?
                    .len()
                    + 32;
            }
            if let Some(extra) = &entry.metadata {
                payload += serde_json::to_vec(extra).map_err(|e| e.to_string())?.len();
            }
            if payload > MAX_REVIEW_BYTES {
                return Err("Import review exceeds 6 MiB".into());
            }
            if entry.resolution.len() > 256 {
                return Err("Import identity explanation exceeds 256 bytes".into());
            }
            if let Some(metadata) = &entry.metadata {
                validate_opaque(metadata)?;
            }
            if let Some(before) = &entry.before {
                before.spec.validate()?;
            }
        }
        for row in &self.removals {
            row.spec.validate()?;
            payload += serde_json::to_vec(&row.spec)
                .map_err(|e| e.to_string())?
                .len()
                + 32;
        }
        if payload > MAX_REVIEW_BYTES {
            return Err("Import review exceeds 6 MiB".into());
        }
        guard(&serde_json::to_string(&self.template_after).map_err(|e| e.to_string())?)?;
        self.settings_before.1.validate()
    }
}
impl Document {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.version, 1 | 2)
            || self.subscriptions.len() > MAX_SUBSCRIPTIONS
            || !self.opensnitch.is_object()
        {
            return Err("Use configuration version 1 or 2 with at most 128 subscriptions".into());
        }
        self.settings.validate()?;
        validate_template(&self.opensnitch)?;
        for spec in &self.subscriptions {
            spec.validate()?;
            effective(spec, &self.settings)?;
        }
        metadata_records(self)?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        guard(&String::from_utf8(bytes).map_err(|e| e.to_string())?)
    }
}
/// Preflight before Value/typed allocation: reject deep, wide, duplicate-key or
/// excessive-node documents while visiting borrowed scalars.
pub fn guard(text: &str) -> Result<(), String> {
    use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
    if text.len() > MAX_DOCUMENT {
        return Err("Configuration exceeds 2 MiB".into());
    }
    struct Seed<'a> {
        depth: usize,
        nodes: &'a mut usize,
    }
    impl<'de> DeserializeSeed<'de> for Seed<'_> {
        type Value = ();
        fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
            *self.nodes += 1;
            if self.depth > 16 || *self.nodes > 16384 {
                return Err(de::Error::custom("JSON exceeds depth 16 or 16384 nodes"));
            }
            d.deserialize_any(self)
        }
    }
    impl<'de> Visitor<'de> for Seed<'_> {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("bounded configuration JSON")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<(), E> {
            if v.len() > 8192 {
                Err(E::custom("JSON string exceeds 8 KiB"))
            } else {
                Ok(())
            }
        }
        fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
            Ok(())
        }
        fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
            Ok(())
        }
        fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
            Ok(())
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
            Ok(())
        }
        fn visit_unit<E: de::Error>(self) -> Result<(), E> {
            Ok(())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
            for i in 0..=256 {
                if a.next_element_seed(Seed {
                    depth: self.depth + 1,
                    nodes: self.nodes,
                })?
                .is_none()
                {
                    return Ok(());
                }
                if i == 256 {
                    return Err(de::Error::custom("JSON array exceeds 256 entries"));
                }
            }
            unreachable!()
        }
        fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<(), A::Error> {
            let mut keys = BTreeSet::new();
            while let Some(key) = a.next_key::<String>()? {
                if key.len() > 256 || keys.len() >= 128 || !keys.insert(key) {
                    return Err(de::Error::custom(
                        "JSON object has duplicate, oversized or too many keys",
                    ));
                }
                a.next_value_seed(Seed {
                    depth: self.depth + 1,
                    nodes: self.nodes,
                })?;
            }
            Ok(())
        }
    }
    let mut nodes = 0;
    let mut d = serde_json::Deserializer::from_str(text);
    Seed {
        depth: 0,
        nodes: &mut nodes,
    }
    .deserialize(&mut d)
    .map_err(|e| e.to_string())?;
    d.end().map_err(|e| e.to_string())
}
pub fn read(path: &Path, base: &Settings) -> Result<Document, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Choose a regular JSON file, not a symlink or directory".into());
    }
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)
        .map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular configuration file".into());
    }
    let mut text = String::new();
    file.take(MAX_DOCUMENT as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    decode(&text, base)
}
fn object<'a>(v: &'a Value, label: &str) -> Result<&'a serde_json::Map<String, Value>, String> {
    v.as_object()
        .ok_or_else(|| format!("{label} must be an object"))
}
fn string(v: &Value, key: &str, default: &str) -> Result<String, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default.into()),
        Some(Value::String(s)) => Ok(s.trim().to_owned()),
        _ => Err(format!("{key} must be text")),
    }
}
fn boolean(v: &Value, key: &str, default: bool) -> Result<bool, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(format!("{key} must be true or false")),
    }
}
fn integer(v: &Value) -> Result<u64, String> {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .ok_or_else(|| "Limits must be whole nonnegative numbers".into())
}
fn multiplier(unit: &str, size: bool) -> Result<u64, String> {
    match (size, unit.to_ascii_lowercase().as_str()) {
        (false, "seconds" | "s" | "composite") | (true, "bytes") => Ok(1),
        (false, "minutes" | "m") => Ok(60),
        (false, "hours" | "h") => Ok(3600),
        (false, "days" | "d") => Ok(86400),
        (false, "weeks" | "w") => Ok(604800),
        (true, "kb") => Ok(1024),
        (true, "mb") => Ok(1024 * 1024),
        (true, "gb") => Ok(1024 * 1024 * 1024),
        _ => Err(format!("Unsupported unit: {unit}")),
    }
}
fn unquote_path(value: &str) -> String {
    let mut out = Vec::new();
    let input = value.as_bytes();
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%'
            && i + 2 < input.len()
            && let (Some(a), Some(b)) = (
                (input[i + 1] as char).to_digit(16),
                (input[i + 2] as char).to_digit(16),
            )
        {
            out.push((a * 16 + b) as u8);
            i += 3;
        } else {
            out.push(input[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn compact(text: &str) -> Option<u64> {
    let text = text.trim().to_ascii_lowercase().replace(' ', "");
    let mut total = 0u64;
    let mut number = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            number.push(c);
        } else {
            let n = number.parse::<u64>().ok()?;
            number.clear();
            let m = multiplier(&c.to_string(), false).ok()?;
            total = total.checked_add(n.checked_mul(m)?)?;
        }
    }
    if number.is_empty() && total > 0 {
        Some(total)
    } else {
        None
    }
}
fn limit(
    v: &Value,
    key: &str,
    units: &str,
    default: u64,
    default_unit: &str,
    size: bool,
) -> Result<u64, String> {
    let raw = v
        .get(key)
        .filter(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty()));
    let unit = string(v, units, default_unit)?;
    let unit = if unit.is_empty() {
        default_unit.to_owned()
    } else {
        unit
    };
    if !size
        && v.get(units)
            .is_none_or(|v| v.is_null() || v.as_str().is_some_and(|s| s.trim().is_empty()))
        && let Some(value) = raw.and_then(Value::as_str).and_then(compact)
    {
        return Ok(value);
    }
    let n = raw.map(integer).transpose()?.unwrap_or(default);
    n.checked_mul(multiplier(&unit, size)?)
        .ok_or_else(|| "Limit conversion overflow".into())
}
fn optional_limit(
    v: &Value,
    key: &str,
    units: &str,
    size: bool,
) -> Result<Option<Override>, String> {
    let raw = v
        .get(key)
        .filter(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty()));
    let unit = string(v, units, "")?;
    if !size
        && unit.is_empty()
        && let Some(compact) = raw.and_then(Value::as_str).and_then(compact)
    {
        return Ok(Some(Override::Absolute(compact)));
    }
    Override::parts(
        raw.map(integer).transpose()?,
        if unit.is_empty() {
            None
        } else {
            Some(Unit::parse(&unit, size)?)
        },
    )
    .map(|v| {
        v.validate(size)?;
        Ok(v)
    })
    .transpose()
}
pub fn decode(text: &str, base: &Settings) -> Result<Document, String> {
    guard(text)?;
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    object(&value, "Configuration")?;
    if value.get("actions").is_none()
        && value.get("settings").is_some()
        && value.get("version").is_some()
    {
        let d: Document = serde_json::from_value(value).map_err(|e| e.to_string())?;
        d.validate()?;
        return Ok(d);
    }
    let (config, automatic, mut template) = if let Some(actions) = value.get("actions") {
        let action = object(actions, "actions")?
            .get("list_subscriptions")
            .ok_or("This action file has no list_subscriptions entry")?;
        let config = action
            .get("config")
            .ok_or("Missing subscription action config")?;
        (config, boolean(action, "enabled", false)?, value.clone())
    } else {
        (
            &value,
            false,
            json!({"actions":{"list_subscriptions":{"enabled":false,"config":value.clone()}}}),
        )
    };
    object(config, "Subscription config")?;
    let unit = |key, default: &str, size| -> Result<Unit, String> {
        let text = string(config, key, default)?;
        Unit::parse(if text.is_empty() { default } else { &text }, size)
    };
    let units = Units {
        interval: unit("interval_units", "hours", false)?,
        timeout: unit("timeout_units", "seconds", false)?,
        size: unit("max_size_units", "MB", true)?,
    };
    let settings = Settings {
        units: Some(units),
        automatic,
        interval: u32::try_from(limit(
            config,
            "interval",
            "interval_units",
            24,
            "hours",
            false,
        )?)
        .map_err(|_| "Interval is too large")?,
        timeout: u32::try_from(limit(
            config,
            "timeout",
            "timeout_units",
            60,
            "seconds",
            false,
        )?)
        .map_err(|_| "Timeout is too large")?,
        max_bytes: limit(config, "max_size", "max_size_units", 20, "MB", true)?,
        disk_bytes: base.disk_bytes,
        user_agent: string(
            config,
            "user_agent",
            "Mozilla/5.0 (X11; Linux x86_64; rv:148.0) Gecko/20100101 Firefox/148.0",
        )?,
    };
    let subs = config
        .get("subscriptions")
        .and_then(Value::as_array)
        .ok_or("Expected a subscriptions array")?;
    if subs.len() > MAX_SUBSCRIPTIONS {
        return Err("At most 128 imported subscriptions are supported".into());
    }
    let mut subscriptions = Vec::new();
    for (i, raw) in subs.iter().enumerate() {
        object(raw, &format!("Subscription {}", i + 1))?;
        let format = Format::parse(&string(raw, "format", "hosts")?.to_ascii_lowercase())?;
        let url = string(raw, "url", "")?;
        let mut name = string(raw, "name", "")?;
        let explicit = string(raw, "filename", "")?;
        let fallback = reqwest::Url::parse(&url)
            .ok()
            .map(|v| unquote_path(v.path()))
            .unwrap_or_default();
        fn base(value: &str) -> &str {
            if value.ends_with('/') {
                ""
            } else {
                value.rsplit('/').next().unwrap_or("")
            }
        }
        let explicit_base = base(&explicit);
        let fallback_base = base(&fallback);
        let mut filename = if !explicit_base.is_empty() {
            explicit_base.to_owned()
        } else {
            fallback_base.to_owned()
        };
        if filename.is_empty() {
            let mut slug = String::new();
            let mut separator = false;
            for c in name.trim().to_lowercase().chars() {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    if separator {
                        slug.push('-');
                        separator = false;
                    }
                    slug.push(c);
                } else {
                    separator = true;
                }
            }
            filename = slug.trim_matches(['-', '.', '_']).to_owned();
            if filename.is_empty() {
                filename = "subscription".into();
            }
            if !filename.contains('.') {
                filename.push_str(".list");
            }
        }
        let dot = filename
            .rfind('.')
            .filter(|i| filename[..*i].chars().any(|c| c != '.'));
        let (stem, ext) = dot
            .map(|i| (filename[..i].to_owned(), filename[i..].to_owned()))
            .unwrap_or((filename.clone(), ".txt".into()));
        let suffix = format!("-{}", format.key());
        filename = format!(
            "{}{}{}",
            stem,
            if stem.to_ascii_lowercase().ends_with(&suffix) {
                ""
            } else {
                &suffix
            },
            ext
        );
        if name.is_empty() {
            name = filename.clone();
        }
        let groups_value = raw.get("groups").filter(|v| !v.is_null());
        let legacy = raw.get("group").filter(|v| !v.is_null());
        let mut labels = match groups_value {
            Some(Value::Array(a)) => a
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_owned)
                        .ok_or("Groups must contain text")
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(Value::String(v)) if legacy.is_none() => v.split(',').map(str::to_owned).collect(),
            Some(Value::String(v)) => vec![v.clone()],
            None => Vec::new(),
            _ => return Err("Groups must be text or an array of labels".into()),
        };
        if let Some(legacy) = legacy {
            labels.push(
                legacy
                    .as_str()
                    .ok_or("Legacy group must contain text")?
                    .to_owned(),
            );
        }
        let mut groups = Vec::new();
        for label in labels {
            if label.len() > 64 {
                return Err("Imported group label exceeds 64 bytes".into());
            }
            if let Some(group) = super::collections::normalize_group(&label)
                && !groups.contains(&group)
            {
                groups.push(group);
            }
        }
        let spec = Spec {
            name,
            url,
            filename,
            format,
            enabled: boolean(raw, "enabled", true)?,
            groups,
            interval: optional_limit(raw, "interval", "interval_units", false)?,
            timeout: optional_limit(raw, "timeout", "timeout_units", false)?,
            max_bytes: optional_limit(raw, "max_size", "max_size_units", true)?,
        };
        spec.validate()
            .map_err(|e| format!("Subscription {}: {e}", i + 1))?;
        effective(&spec, &settings).map_err(|e| format!("Subscription {}: {e}", i + 1))?;
        let mut spec = spec;
        if subscriptions
            .iter()
            .any(|other: &Spec| other.filename == spec.filename && other.url != spec.url)
        {
            let dot = spec.filename.rfind('.').unwrap();
            let ext = spec.filename[dot..].to_owned();
            let stem = spec.filename[..dot]
                .trim_end_matches(&format!("-{}", spec.format.key()))
                .trim_end_matches('-')
                .to_owned();
            let mut index = 2;
            let mut candidate = spec.filename.clone();
            while subscriptions.iter().any(|s| s.filename == candidate) {
                candidate = format!("{stem}-{index}-{}{ext}", spec.format.key());
                index += 1;
            }
            spec.filename = candidate;
            spec.name = format!("{} ({})", spec.name, index - 1);
            spec.validate()?;
        }
        subscriptions.push(spec);
    }
    if let Some(records) = template
        .pointer_mut("/actions/list_subscriptions/config/subscriptions")
        .and_then(Value::as_array_mut)
    {
        for (record, spec) in records.iter_mut().zip(&subscriptions) {
            set_record_identity(record, spec);
        }
    }
    let document = Document {
        version: 2,
        settings,
        subscriptions,
        opensnitch: template,
    };
    document.validate()?;
    Ok(document)
}
fn validate_template(value: &Value) -> Result<(), String> {
    object(value, "Action metadata")?;
    if let Some(actions) = value.get("actions") {
        object(actions, "Action metadata actions")?;
        if let Some(action) = actions.get("list_subscriptions") {
            object(action, "Subscription action metadata")?;
            if let Some(config) = action.get("config") {
                object(config, "Subscription config metadata")?;
                if let Some(rows) = config.get("subscriptions") {
                    for row in rows
                        .as_array()
                        .ok_or("Metadata subscriptions must be an array")?
                    {
                        object(row, "Subscription metadata")?;
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn global_template(value: &Value) -> Result<Value, String> {
    validate_template(value)?;
    let mut root = value.clone();
    if root.get("actions").is_none() {
        root["actions"] = json!({});
    }
    if root["actions"].get("list_subscriptions").is_none() {
        root["actions"]["list_subscriptions"] = json!({});
    }
    if root["actions"]["list_subscriptions"]
        .get("config")
        .is_none()
    {
        root["actions"]["list_subscriptions"]["config"] = json!({});
    }
    root["actions"]["list_subscriptions"]["config"]["subscriptions"] = json!([]);
    Ok(root)
}
pub fn merge_template(existing: &Value, incoming: &Value) -> Result<Value, String> {
    let root = if incoming.as_object().is_some_and(|v| !v.is_empty()) {
        incoming
    } else {
        existing
    };
    let value = global_template(root)?;
    guard(&serde_json::to_string(&value).map_err(|e| e.to_string())?)?;
    Ok(value)
}
const SOURCE_KEYS: &[&str] = &[
    "name",
    "url",
    "filename",
    "format",
    "enabled",
    "groups",
    "group",
    "interval",
    "interval_units",
    "timeout",
    "timeout_units",
    "max_size",
    "max_size_units",
];
pub fn opaque_record(record: &Value) -> Result<Value, String> {
    let mut value = object(record, "Source metadata")?.clone();
    for key in SOURCE_KEYS {
        value.remove(*key);
    }
    Ok(Value::Object(value))
}
pub fn validate_opaque(value: &Value) -> Result<(), String> {
    object(value, "Retained source fields")?;
    guard(&serde_json::to_string(value).map_err(|e| e.to_string())?)
}
fn set_record_identity(value: &mut Value, spec: &Spec) {
    for (key, value_new) in [
        ("name", json!(spec.name)),
        ("url", json!(spec.url)),
        ("filename", json!(spec.filename)),
        ("format", json!(spec.format.key())),
    ] {
        value[key] = value_new;
    }
}
pub fn metadata_records(document: &Document) -> Result<Vec<Option<Value>>, String> {
    let Some(rows) = document
        .opensnitch
        .pointer("/actions/list_subscriptions/config/subscriptions")
        .and_then(Value::as_array)
        .filter(|rows| !rows.is_empty())
    else {
        return Ok(vec![None; document.subscriptions.len()]);
    };
    if rows.len() != document.subscriptions.len() {
        return Err("Embedded source metadata must match the subscription array length".into());
    }
    rows.iter().zip(&document.subscriptions).map(|(record,spec)|{
        for (key,expected) in [("name",spec.name.as_str()),("url",spec.url.as_str()),("filename",spec.filename.as_str()),("format",spec.format.key())] {
            if record.get(key).and_then(Value::as_str)!=Some(expected){return Err("Embedded source identities do not match the ordered subscriptions; export a fresh native configuration".into());}
        }
        opaque_record(record).map(Some)
    }).collect()
}
pub fn bind_metadata(
    globals: &Value,
    specs: &[Spec],
    metadata: &[Option<Value>],
) -> Result<Value, String> {
    if specs.len() != metadata.len() {
        return Err("Metadata binding count differs from subscriptions".into());
    }
    let mut root = global_template(globals)?;
    let mut rows = Vec::new();
    for (spec, opaque) in specs.iter().zip(metadata) {
        let mut record = opaque.clone().unwrap_or_else(empty_object);
        validate_opaque(&record)?;
        set_record_identity(&mut record, spec);
        rows.push(record);
    }
    root["actions"]["list_subscriptions"]["config"]["subscriptions"] = Value::Array(rows);
    Ok(root)
}
pub fn typed_filename(filename: &str, format: &Format) -> String {
    let dot = filename
        .rfind('.')
        .filter(|i| filename[..*i].chars().any(|c| c != '.'));
    let (stem, ext) = dot
        .map(|i| (&filename[..i], &filename[i..]))
        .unwrap_or((filename, ".txt"));
    let suffix = format!("-{}", format.key());
    format!(
        "{stem}{}{ext}",
        if stem.to_ascii_lowercase().ends_with(&suffix) {
            ""
        } else {
            &suffix
        }
    )
}
fn portable_document(document: &Document) -> Result<Document, String> {
    let mut portable = document.clone();
    let mut filenames = BTreeSet::new();
    for spec in &mut portable.subscriptions {
        let first = typed_filename(&spec.filename, &spec.format);
        let dot = first.rfind('.').unwrap();
        let ext = &first[dot..];
        let root = first[..dot]
            .trim_end_matches(format!("-{}", spec.format.key()).as_str())
            .trim_end_matches('-');
        let mut filename = first.clone();
        let mut index = 2;
        while filenames.contains(&filename) {
            filename = format!("{root}-{index}-{}{ext}", spec.format.key());
            index += 1;
        }
        spec.filename = filename;
        spec.validate().map_err(|e|format!("OpenSnitch export filename normalization: {e}. Shorten the filename or use native configuration."))?;
        filenames.insert(spec.filename.clone());
    }
    portable.opensnitch = bind_metadata(
        &document.opensnitch,
        &portable.subscriptions,
        &metadata_records(document)?,
    )?;
    portable.validate()?;
    Ok(portable)
}
pub fn action(document: &Document) -> Result<Value, String> {
    let mut root = merge_template(&empty_object(), &document.opensnitch)?;
    root.as_object_mut()
        .unwrap()
        .entry("name")
        .or_insert(json!("listSubscriptionsActions"));
    root.as_object_mut()
        .unwrap()
        .entry("description")
        .or_insert(json!("Manage and auto-update blocklist subscriptions"));
    root.as_object_mut()
        .unwrap()
        .entry("type")
        .or_insert(json!(["global", "main-dialog"]));
    let action = &mut root["actions"]["list_subscriptions"];
    action["enabled"] = json!(document.settings.automatic);
    let config = &mut action["config"];
    let previous = metadata_records(document)?;
    config
        .as_object_mut()
        .ok_or("Action config must be an object")?
        .entry("lists_dir")
        .or_insert(json!(""));
    let units = document.settings.units();
    for (key, value) in [
        (
            "interval",
            json!(u64::from(document.settings.interval) / units.interval.factor()),
        ),
        ("interval_units", json!(units.interval.label())),
        (
            "timeout",
            json!(u64::from(document.settings.timeout) / units.timeout.factor()),
        ),
        ("timeout_units", json!(units.timeout.label())),
        (
            "max_size",
            json!(document.settings.max_bytes / units.size.factor()),
        ),
        ("max_size_units", json!(units.size.label())),
        ("user_agent", json!(document.settings.user_agent)),
    ] {
        config[key] = value;
    }
    let mut records = Vec::new();
    for (spec, metadata) in document.subscriptions.iter().zip(previous) {
        let mut record = metadata.unwrap_or_else(empty_object);
        let map = record
            .as_object_mut()
            .ok_or("Subscription metadata must be an object")?;
        map.remove("group");
        for (key, value) in [
            ("name", json!(spec.name)),
            ("url", json!(spec.url)),
            ("filename", json!(spec.filename)),
            ("format", json!(spec.format.key())),
            ("enabled", json!(spec.enabled)),
            ("groups", json!(spec.groups)),
        ] {
            map.insert(key.into(), value);
        }
        for (key, units, value, size) in [
            ("interval", "interval_units", spec.interval, false),
            ("timeout", "timeout_units", spec.timeout, false),
            ("max_size", "max_size_units", spec.max_bytes, true),
        ] {
            let (count, unit) = match value {
                None => (None, None),
                Some(Override::Absolute(n)) => (
                    Some(n),
                    Some(if size { Unit::Bytes } else { Unit::Seconds }),
                ),
                Some(Override::Parts(p)) => (p.count, p.unit),
            };
            map.insert(key.into(), json!(count));
            map.insert(units.into(), json!(unit.map(Unit::label)));
        }
        records.push(record);
    }
    config["subscriptions"] = Value::Array(records);
    Ok(root)
}
pub fn write(path: &Path, document: &Document, encoding: Encoding) -> Result<usize, String> {
    document.validate()?;
    let value = match encoding {
        Encoding::Native => action(document)?,
        Encoding::OpenSnitch => action(&portable_document(document)?)?,
    };
    let bytes = match encoding {
        Encoding::Native => serde_json::to_vec_pretty(&Document {
            opensnitch: value,
            ..document.clone()
        }),
        Encoding::OpenSnitch => serde_json::to_vec_pretty(&value),
    }
    .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_DOCUMENT {
        return Err("Export exceeds 2 MiB".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    file.persist_noclobber(path)
        .map_err(|e| format!("Choose a new export filename: {}", e.error))?;
    std::fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("Export saved; directory sync failed: {e}"))?;
    Ok(document.subscriptions.len())
}
pub fn validate_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().len() > 4096 || !path.is_absolute() {
        Err("Choose an absolute path of at most 4096 bytes".into())
    } else {
        Ok(())
    }
}
pub fn validate_export(scope: &Scope, path: &Path) -> Result<(), String> {
    scope.validate()?;
    validate_path(path)
}
