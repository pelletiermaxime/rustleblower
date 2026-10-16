//! Bounded JSON interchange, run on a background executor. Export reads its own
//! read-only SQLite snapshot so history ingestion can continue independently.
use super::{MAX_CATALOG_BYTES, MAX_RULES, Query, document, memory_bytes};
use crate::protocol::Rule;
use anyhow::{Context, Result, bail, ensure};
use prost::Message;
use rusqlite::{Connection, OpenFlags};
use serde::de::{DeserializeSeed, SeqAccess, Visitor};
use serde_json::value::RawValue;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
pub struct ExportScope {
    pub query: Query,
    /// None exports the complete filter, including pages not currently visible.
    pub names: Option<BTreeSet<String>>,
}

#[derive(Default)]
pub struct Import {
    pub rules: Vec<Rule>,
    pub bytes: usize,
    encoded: usize,
    allocated: usize,
    names: BTreeSet<String>,
}
// Borrow each object directly from the input. A large array is never expanded
// into a Value tree or Vec<Value> before its count and memory limits are checked.
struct RuleArray<'a>(&'a mut Import);
impl<'de> DeserializeSeed<'de> for RuleArray<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for RuleArray<'_> {
    type Value = ();
    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an array of rule objects")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> std::result::Result<(), A::Error> {
        while let Some(raw) = sequence.next_element::<&RawValue>()? {
            self.0.push_raw(raw).map_err(serde::de::Error::custom)?;
        }
        Ok(())
    }
}
impl Import {
    fn push_raw(&mut self, raw: &RawValue) -> Result<()> {
        ensure!(self.rules.len() < MAX_RULES, "Import exceeds 10,000 rules");
        ensure!(
            raw.get().len() <= super::MAX_RULE_BYTES * 8,
            "A rule JSON object exceeds 512 KiB"
        );
        let value = serde_json::from_str(raw.get())?;
        let rule = document::rule_from_json(&value).map_err(anyhow::Error::msg)?;
        self.encoded += rule.encoded_len();
        self.allocated += memory_bytes(&rule);
        ensure!(
            self.encoded <= MAX_CATALOG_BYTES && self.allocated <= 8 * 1024 * 1024,
            "Imported rule data exceeds its memory limit"
        );
        ensure!(
            self.names.insert(rule.name.clone()),
            "Duplicate rule name in import: {}",
            rule.name
        );
        self.rules.push(rule);
        Ok(())
    }
    fn append(&mut self, text: &str) -> Result<()> {
        self.bytes += text.len();
        ensure!(
            self.bytes <= document::MAX_DOCUMENT_BYTES,
            "Import exceeds 8 MiB of JSON"
        );
        // OpenSnitch copies adjacent JSON objects to the clipboard. Accept that
        // format as well as single objects and JSON arrays.
        for raw in serde_json::Deserializer::from_str(text).into_iter::<&RawValue>() {
            let raw = raw.context("Invalid rule JSON")?;
            if raw.get().starts_with('[') {
                RuleArray(self).deserialize(&mut serde_json::Deserializer::from_str(raw.get()))?;
            } else {
                self.push_raw(raw)?;
            }
        }
        Ok(())
    }
    pub fn from_text(text: &str) -> Result<Self> {
        let mut import = Self::default();
        import.append(text)?;
        ensure!(!import.rules.is_empty(), "No rules found in the import");
        Ok(import)
    }
    pub fn from_path(path: &Path) -> Result<Self> {
        let paths = if path.is_dir() {
            let mut paths = Vec::new();
            for (index, entry) in std::fs::read_dir(path)?.enumerate() {
                ensure!(index < 20_000, "Import directory has too many entries");
                let entry = entry?;
                if entry.path().extension().is_some_and(|v| v == "json")
                    && entry.file_type()?.is_file()
                {
                    ensure!(
                        paths.len() < MAX_RULES,
                        "Import directory exceeds 10,000 JSON files"
                    );
                    paths.push(entry.path());
                }
            }
            paths.sort();
            paths
        } else {
            vec![path.to_owned()]
        };
        let mut import = Self::default();
        for path in paths {
            let mut text = String::new();
            File::open(&path)?
                .take((document::MAX_DOCUMENT_BYTES - import.bytes + 1) as u64)
                .read_to_string(&mut text)
                .with_context(|| format!("Cannot read {}", path.display()))?;
            import
                .append(&text)
                .with_context(|| format!("Cannot import {}", path.display()))?;
        }
        ensure!(
            !import.rules.is_empty(),
            "No JSON rules found in the selected path"
        );
        Ok(import)
    }
}

pub(crate) fn visit(
    database: &Path,
    scope: &ExportScope,
    output: impl FnMut(&Rule, bool) -> Result<()>,
) -> Result<usize> {
    let mut db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(Duration::from_millis(250))?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-2048; PRAGMA mmap_size=0;")?;
    let snapshot = db.transaction()?;
    let count = visit_snapshot(&snapshot, scope, output)?;
    snapshot.commit()?;
    Ok(count)
}

pub(crate) fn visit_snapshot(
    snapshot: &Connection,
    scope: &ExportScope,
    mut output: impl FnMut(&Rule, bool) -> Result<()>,
) -> Result<usize> {
    let mut query = scope.query.clone();
    query.after.clear();
    let mut count = 0;
    let mut bytes = 0;
    loop {
        let page = super::query(snapshot, &query)?;
        for rule in &page.rules {
            if scope
                .names
                .as_ref()
                .is_none_or(|names| names.contains(&rule.name))
            {
                count += 1;
                bytes += rule.encoded_len();
                ensure!(
                    count <= MAX_RULES && bytes <= MAX_CATALOG_BYTES,
                    "Export exceeds catalogue limits"
                );
                output(rule, page.awaiting.contains(&rule.name))?;
            }
        }
        if !page.has_more {
            break;
        }
        query.after = page
            .rules
            .last()
            .context("Empty intermediate export page")?
            .name
            .clone();
    }
    if let Some(names) = &scope.names {
        ensure!(
            count == names.len(),
            "Some selected rules changed or disappeared; refresh the catalogue and retry"
        );
    }
    Ok(count)
}

pub fn export_text(database: &Path, scope: &ExportScope) -> Result<(String, usize)> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"[\n");
    let count = visit(database, scope, |rule, _| {
        if bytes.len() > 2 {
            bytes.extend_from_slice(b",\n");
        }
        serde_json::to_writer(&mut bytes, &document::rule_json(rule))?;
        ensure!(
            bytes.len() <= document::MAX_DOCUMENT_BYTES - 3,
            "Export exceeds 8 MiB of JSON"
        );
        Ok(())
    })?;
    bytes.extend_from_slice(b"\n]\n");
    Ok((String::from_utf8(bytes)?, count))
}

pub fn load_rules(database: &Path, scope: &ExportScope) -> Result<Vec<Rule>> {
    let mut rules = Vec::new();
    let mut allocated = 0;
    visit(database, scope, |rule, awaiting| {
        ensure!(
            !awaiting,
            "A selected rule is awaiting daemon observation; reconnect to refresh its inventory before editing"
        );
        allocated += memory_bytes(rule);
        ensure!(
            allocated <= 8 * 1024 * 1024,
            "Rule review exceeds its 8 MiB memory limit"
        );
        rules.push(rule.clone());
        Ok(())
    })?;
    Ok(rules)
}

pub fn export_file(database: &Path, scope: &ExportScope, path: &Path) -> Result<usize> {
    use std::os::unix::fs::MetadataExt;
    if let (Ok(source), Ok(target)) = (std::fs::metadata(database), std::fs::metadata(path)) {
        ensure!(
            source.dev() != target.dev() || source.ino() != target.ino(),
            "Choose a JSON destination other than the active history database"
        );
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(b"[\n")?;
    let mut bytes = 2;
    let mut first = true;
    let count = visit(database, scope, |rule, _| {
        if !first {
            file.write_all(b",\n")?;
            bytes += 2;
        }
        first = false;
        let text = serde_json::to_vec(&document::rule_json(rule))?;
        bytes += text.len();
        ensure!(
            bytes <= document::MAX_DOCUMENT_BYTES - 3,
            "Export exceeds 8 MiB of JSON"
        );
        file.write_all(&text)?;
        Ok(())
    })?;
    file.write_all(b"\n]\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(count)
}

/// A new child directory avoids partially overwriting an existing export. Each
/// file is OpenSnitch-compatible; the directory is published only when complete.
pub fn export_directory(
    database: &Path,
    scope: &ExportScope,
    parent: &Path,
) -> Result<(PathBuf, usize)> {
    let staging = tempfile::Builder::new()
        .prefix(".rustleblower-export-")
        .tempdir_in(parent)?;
    let mut bytes = 0;
    let count = visit(database, scope, |rule, _| {
        super::valid(rule).map_err(anyhow::Error::msg)?;
        let filename = if rule.name.len() <= 200 {
            format!("{}.json", rule.name)
        } else {
            use sha2::{Digest, Sha256};
            format!("rule-{:x}.json", Sha256::digest(rule.name.as_bytes()))
        };
        let mut file = File::options()
            .create_new(true)
            .write(true)
            .open(staging.path().join(filename))?;
        let text = serde_json::to_vec_pretty(&document::rule_json(rule))?;
        bytes += text.len() + 1;
        ensure!(
            bytes <= document::MAX_DOCUMENT_BYTES,
            "Export exceeds 8 MiB of JSON"
        );
        file.write_all(&text)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    })?;
    let destination = parent.join(format!(
        "rustleblower-rules-{}",
        crate::history::now_nanos()
    ));
    if destination.exists() {
        bail!("Export destination already exists");
    }
    std::fs::rename(staging.path(), &destination)?;
    Ok((destination, count))
}

pub fn existing_names(database: &Path, node: &str) -> Result<BTreeSet<String>> {
    let db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(Duration::from_millis(250))?;
    let mut statement =
        db.prepare("SELECT name FROM (SELECT node,name FROM application_rules UNION ALL SELECT node,name FROM prompt_rules) WHERE node=? ORDER BY name LIMIT 10001")?;
    let names = statement
        .query_map([node], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    ensure!(
        names.len() <= MAX_RULES,
        "Catalogue exceeds the supported rule count"
    );
    Ok(names)
}

pub fn find_rule(database: &Path, node: &str, name: &str) -> Result<Option<Rule>> {
    use rusqlite::OptionalExtension;
    let db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(Duration::from_millis(250))?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-2048; PRAGMA mmap_size=0;")?;
    let wire: Option<Vec<u8>> = db
        .query_row(
            "SELECT wire FROM application_rules WHERE node=?1 AND name=?2 UNION ALL SELECT wire FROM prompt_rules WHERE node=?1 AND name=?2 LIMIT 1",
            rusqlite::params![node, name],
            |row| row.get(0),
        )
        .optional()?;
    wire.map(|wire| Ok(Rule::decode(wire.as_slice())?))
        .transpose()
}

pub fn find_confirmed_rule(database: &Path, node: &str, name: &str) -> Result<Option<Rule>> {
    use rusqlite::OptionalExtension;
    let db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(Duration::from_millis(250))?;
    let wire: Option<Vec<u8>> = db
        .query_row(
            "SELECT wire FROM application_rules WHERE node=? AND name=?",
            rusqlite::params![node, name],
            |row| row.get(0),
        )
        .optional()?;
    wire.map(|wire| Ok(Rule::decode(wire.as_slice())?))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Operator;
    #[test]
    fn export_uses_one_snapshot_while_the_writer_updates_later_pages() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("rules.sqlite3");
        let mut writer = Connection::open(&database).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        crate::rules::schema(&writer).unwrap();
        let original = (0..260)
            .map(|i| Rule {
                name: format!("app.{i:03}"),
                enabled: true,
                action: "allow".into(),
                duration: "always".into(),
                operator: Some(Operator {
                    r#type: "simple".into(),
                    operand: "process.path".into(),
                    data: "/bin/example".into(),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        crate::rules::replace(&mut writer, "node", &original).unwrap();
        let scope = ExportScope {
            query: Query {
                node: "node".into(),
                ..Default::default()
            },
            names: None,
        };
        let mut seen = Vec::new();
        visit(&database, &scope, |rule, _| {
            if seen.is_empty() {
                let mut changed = original[205].clone();
                changed.action = "deny".into();
                crate::rules::apply(
                    &mut writer,
                    "node",
                    crate::protocol::Action::ChangeRule,
                    &changed,
                )?;
            }
            seen.push(rule.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, original);
        assert_eq!(
            crate::rules::query(
                &writer,
                &Query {
                    node: "node".into(),
                    search: "app.205".into(),
                    ..Default::default()
                }
            )
            .unwrap()
            .rules[0]
                .action,
            "deny"
        );
    }
}
