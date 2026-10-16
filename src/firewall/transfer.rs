//! Bounded local configuration/profile files. Call from a background executor.
use super::{
    MAX_JSON,
    document::{Chain, Document},
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

pub const DEFAULT_PROFILES: &str = "/etc/opensnitchd/system-fw.d/profiles";
pub const MAX_PROFILES: usize = 256;
const MAX_PATH: usize = 4096;

pub fn parse_profile(text: &str) -> Result<Chain, String> {
    if text.len() > MAX_JSON {
        return Err("Profile exceeds the 1 MiB JSON limit".into());
    }
    #[derive(serde::Deserialize)]
    struct Envelope<'a> {
        #[serde(rename = "Profile", borrow)]
        profile: Option<&'a serde_json::value::RawValue>,
    }
    // OpenSnitch's predefined files use {Name, Profile}; its importer selects
    // Profile and ignores outer display metadata. Also accept a bare FwChain.
    // Borrow the raw chain rather than expanding an unchecked JSON Value tree.
    let envelope: Envelope<'_> =
        serde_json::from_str(text).map_err(|e| format!("Invalid profile JSON: {e}"))?;
    let text = envelope.profile.map_or(text, |value| value.get());
    // Reuse the container preflight and exact wire/heap validation. This also
    // rejects unknown fields before allocating large typed repeated messages.
    let wrapped = format!("{{\"Version\":1,\"SystemRules\":[{{\"Chains\":[{text}]}}]}}");
    let mut doc = Document::parse(&wrapped)?;
    Ok(doc.system_rules.remove(0).chains.remove(0))
}
pub fn profile_json(profile: &Chain) -> Result<String, String> {
    #[derive(serde::Serialize)]
    struct Export<'a> {
        #[serde(rename = "Name")]
        name: &'a str,
        #[serde(rename = "Profile")]
        profile: &'a Chain,
    }
    let text = serde_json::to_string_pretty(&Export {
        name: &profile.name,
        profile,
    })
    .map_err(|e| e.to_string())?;
    parse_profile(&text)?;
    Ok(text)
}
pub fn read_text(path: &Path) -> Result<String, String> {
    check_path(path)?;
    let meta = std::fs::metadata(path).map_err(|e| format!("Cannot inspect file: {e}"))?;
    if !meta.is_file() {
        return Err("Choose a regular JSON or .profile file".into());
    }
    if meta.len() > MAX_JSON as u64 {
        return Err("Firewall file exceeds 1 MiB".into());
    }
    let file = File::open(path).map_err(|e| format!("Cannot open firewall file: {e}"))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular firewall file".into());
    }
    let mut text = String::new();
    file.take((MAX_JSON + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|e| format!("Cannot read UTF-8 firewall JSON: {e}"))?;
    if text.len() > MAX_JSON {
        return Err("Firewall file exceeds 1 MiB".into());
    }
    Ok(text)
}
pub fn write_new(path: &Path, text: &str) -> Result<(), String> {
    check_path(path)?;
    if text.len() > MAX_JSON {
        return Err("Firewall export exceeds 1 MiB".into());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("Cannot create export (existing files are preserved): {e}"))?;
    file.write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("Export failed; a partial file may remain: {e}"))
}
fn check_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.as_os_str().len() > MAX_PATH
        || path.to_str().is_none_or(|p| p.contains('\0'))
    {
        return Err("Choose a UTF-8 path of at most 4096 bytes without NUL".into());
    }
    Ok(())
}
/// Keep paths only: loading every profile's JSON would multiply retained memory.
pub fn profiles(directory: &Path) -> Result<Vec<PathBuf>, String> {
    check_path(directory)?;
    let mut paths = Vec::new();
    let entries =
        std::fs::read_dir(directory).map_err(|e| format!("Cannot list profile directory: {e}"))?;
    for (index, entry) in entries.enumerate() {
        if index >= 4096 {
            return Err("Profile directory exceeds 4096 entries".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().extension().is_some_and(|ext| ext == "profile")
            && entry.file_type().map_err(|e| e.to_string())?.is_file()
        {
            if paths.len() == MAX_PROFILES {
                return Err("Profile directory exceeds 256 profiles".into());
            }
            let path = entry.path();
            check_path(&path)?;
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}
