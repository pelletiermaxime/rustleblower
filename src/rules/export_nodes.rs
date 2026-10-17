//! Immutable disk-backed multi-node rule exports. Only one rule is decoded at a
//! time; no SQLite snapshot spans user review or destination filesystem work.
use super::{Query, document};
use anyhow::{Result, ensure};
use prost::Message;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
#[derive(Clone, Serialize)]
pub struct Node {
    pub name: String,
    pub directory: String,
    pub rules: usize,
    pub awaiting: usize,
    pub bytes: usize,
}
pub struct Review {
    source: tempfile::TempDir,
    pub nodes: Vec<Node>,
    pub rules: usize,
    pub bytes: usize,
}
fn open(database: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(Duration::from_millis(250))?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-512; PRAGMA mmap_size=0;")?;
    Ok(db)
}
pub fn catalogue(database: &Path) -> Result<Vec<String>> {
    let db = open(database)?;
    let mut stmt = db.prepare("SELECT node FROM application_rules UNION SELECT node FROM prompt_rules UNION SELECT name FROM node_info ORDER BY 1 LIMIT 33")?;
    let mut names = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name = row.get_ref(0)?.as_str()?;
        ensure!(
            !name.is_empty() && name.len() <= 128,
            "Invalid cached node identity"
        );
        names.push(name.to_owned());
    }
    ensure!(
        names.len() <= crate::nodes::MAX_NODES,
        "More than 32 cached nodes; narrow the cache before exporting"
    );
    ensure!(
        names.iter().all(|n| !n.is_empty() && n.len() <= 128),
        "Invalid cached node identity"
    );
    Ok(names)
}
fn running(stop: &AtomicBool) -> Result<()> {
    ensure!(
        !stop.load(Ordering::Acquire),
        "Export cancelled; no destination was published"
    );
    Ok(())
}
fn component(name: &str) -> String {
    let prefix: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(32)
        .collect();
    format!("{prefix}-{}", hex::encode(Sha256::digest(name.as_bytes())))
}
/// Copy only selected rule blobs into bounded sequential disk spools, then
/// release the live SQLite reader before JSON encoding or individual file writes.
pub fn capture(
    database: &Path,
    names: &[String],
    filter: &Query,
    stop: &AtomicBool,
) -> Result<Review> {
    ensure!(
        !names.is_empty() && names.len() <= crate::nodes::MAX_NODES,
        "Choose 1–32 cached nodes"
    );
    let mut unique = BTreeSet::new();
    ensure!(
        names
            .iter()
            .all(|n| !n.is_empty() && n.len() <= 128 && unique.insert(n)),
        "Duplicate or invalid node identity"
    );
    let predicate = super::query::Predicate::new(filter)?;
    running(stop)?;
    let source = tempfile::Builder::new()
        .prefix("rustleblower-rule-snapshot-")
        .tempdir()?;
    let mut spools = Vec::new();
    {
        let mut db = open(database)?;
        let tx = db.transaction()?;
        for name in names {
            running(stop)?;
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM node_info WHERE name=?1 UNION ALL SELECT 1 FROM application_rules WHERE node=?1 UNION ALL SELECT 1 FROM prompt_rules WHERE node=?1)",[name],|r|r.get(0))?;
            ensure!(
                exists,
                "A selected node disappeared; refresh the node choices"
            );
            let (count, bytes, max):(usize,usize,usize)=tx.query_row("SELECT count(*),coalesce(sum(length(wire)),0),coalesce(max(length(wire)),0) FROM (SELECT wire FROM application_rules WHERE node=?1 UNION ALL SELECT wire FROM prompt_rules WHERE node=?1)",[name],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            ensure!(
                count <= super::MAX_RULES
                    && bytes <= super::MAX_CATALOG_BYTES
                    && max <= super::MAX_RULE_BYTES,
                "Cached node exceeds the bounded rule catalogue"
            );
            let mut node = Node {
                name: name.clone(),
                directory: component(name),
                rules: 0,
                awaiting: 0,
                bytes: 0,
            };
            let mut spool = tempfile::NamedTempFile::new_in(source.path())?;
            let mut query=tx.prepare("WITH catalogue AS (SELECT *,0 AS awaiting FROM application_rules UNION ALL SELECT *,1 FROM prompt_rules) SELECT c.name,c.wire,c.awaiting,o.received FROM catalogue c LEFT JOIN rule_observations o ON o.node=c.node AND o.name=c.name WHERE c.node=?1 ORDER BY c.name")?;
            let mut rows = query.query([name])?;
            let mut allocated = 0;
            while let Some(row) = rows.next()? {
                running(stop)?;
                let stored_name = row.get_ref(0)?.as_str()?;
                ensure!(
                    stored_name.len() <= 1024,
                    "Cached rule name exceeds its limit"
                );
                let wire: Vec<u8> = row.get(1)?;
                let rule = crate::protocol::Rule::decode(wire.as_slice())?;
                super::valid(&rule).map_err(anyhow::Error::msg)?;
                ensure!(
                    stored_name == rule.name,
                    "Cached rule identity does not match its document"
                );
                if !predicate.matches_received(name, &rule, row.get(3)?) {
                    continue;
                }
                allocated += super::memory_bytes(&rule);
                ensure!(
                    allocated <= 8 * 1024 * 1024,
                    "Node rule allocations exceed 8 MiB"
                );
                let awaiting = row.get::<_, bool>(2)?;
                spool.write_all(&(wire.len() as u32).to_le_bytes())?;
                spool.write_all(&[u8::from(awaiting)])?;
                spool.write_all(&wire)?;
                node.rules += 1;
                node.awaiting += usize::from(awaiting);
            }
            spools.push((node, spool));
        }
        tx.commit()?;
    } // No live SQLite connection/read snapshot survives JSON/filesystem work.
    let mut review = Review {
        source,
        nodes: Vec::new(),
        rules: 0,
        bytes: 0,
    };
    for (mut node, mut spool) in spools {
        running(stop)?;
        let dir = review.source.path().join(&node.directory);
        std::fs::create_dir(&dir)?;
        spool.seek(SeekFrom::Start(0))?;
        for _ in 0..node.rules {
            running(stop)?;
            let mut header = [0; 5];
            spool.read_exact(&mut header)?;
            let length = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
            ensure!(
                length <= super::MAX_RULE_BYTES,
                "Captured rule exceeds its limit"
            );
            let mut wire = vec![0; length];
            spool.read_exact(&mut wire)?;
            let rule = crate::protocol::Rule::decode(wire.as_slice())?;
            let text = serde_json::to_vec_pretty(&document::rule_json(&rule))?;
            ensure!(text.len() <= 512 * 1024, "Rule JSON exceeds 512 KiB");
            node.bytes += text.len() + 1;
            ensure!(
                node.bytes <= document::MAX_DOCUMENT_BYTES,
                "Node export exceeds 8 MiB JSON"
            );
            let mut file = File::options()
                .write(true)
                .create_new(true)
                .open(dir.join(format!("{}.json", component(&rule.name))))?;
            file.write_all(&text)?;
            file.write_all(b"\n")?;
        }
        review.rules += node.rules;
        review.bytes += node.bytes;
        review.nodes.push(node);
    }
    running(stop)?;
    Ok(review)
}
#[derive(Serialize)]
struct Manifest<'a> {
    format: &'static str,
    nodes: &'a [Node],
    rules: usize,
    bytes: usize,
}
/// Publish a new complete tree; never merge into/overwrite an existing export.
pub fn publish(review: &Review, parent: &Path, stop: &AtomicBool) -> Result<PathBuf> {
    running(stop)?;
    let stage = tempfile::Builder::new()
        .prefix(".rustleblower-nodes-")
        .tempdir_in(parent)?;
    for node in &review.nodes {
        running(stop)?;
        let dest = stage.path().join(&node.directory);
        std::fs::create_dir(&dest)?;
        let mut count = 0;
        let mut bytes = 0;
        for entry in std::fs::read_dir(review.source.path().join(&node.directory))? {
            running(stop)?;
            let entry = entry?;
            count += 1;
            ensure!(
                count <= node.rules && entry.file_type()?.is_file(),
                "Captured export changed unexpectedly"
            );
            let size = entry.metadata()?.len() as usize;
            ensure!(size <= 512 * 1024 + 1, "Captured file exceeds its limit");
            bytes += size;
            ensure!(
                bytes <= node.bytes,
                "Captured export exceeds its reviewed size"
            );
            let input = File::open(entry.path())?;
            let mut input = input.take(size as u64 + 1);
            let mut output = File::options()
                .write(true)
                .create_new(true)
                .open(dest.join(entry.file_name()))?;
            ensure!(
                std::io::copy(&mut input, &mut output)? == size as u64,
                "Captured file changed while exporting"
            );
            output.sync_all()?;
        }
        ensure!(
            count == node.rules && bytes == node.bytes,
            "Captured export is incomplete"
        );
        File::open(&dest)?.sync_all()?;
    }
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(stage.path().join("manifest.json"))?;
    serde_json::to_writer_pretty(
        &mut file,
        &Manifest {
            format: "rustleblower-rules-v1",
            nodes: &review.nodes,
            rules: review.rules,
            bytes: review.bytes,
        },
    )?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    File::open(stage.path())?.sync_all()?;
    running(stop)?;
    let basename = stage.path().file_name().unwrap().to_string_lossy();
    let destination = parent.join(basename.trim_start_matches('.'));
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        stage.path(),
        rustix::fs::CWD,
        &destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )?;
    File::open(parent)?.sync_all()?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_snapshot_cleans_staged_output_without_publishing_any_node() {
        let parent = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let mut nodes = Vec::new();
        for name in ["first", "second"] {
            let dir = component(name);
            std::fs::create_dir(source.path().join(&dir)).unwrap();
            std::fs::write(source.path().join(&dir).join("rule.json"), b"{}\n").unwrap();
            nodes.push(Node {
                name: name.into(),
                directory: dir,
                rules: 1,
                awaiting: 0,
                bytes: 3,
            });
        }
        // A lost second captured file must also discard the first staged node.
        std::fs::remove_file(source.path().join(&nodes[1].directory).join("rule.json")).unwrap();
        let review = Review {
            source,
            nodes,
            rules: 2,
            bytes: 6,
        };
        assert!(publish(&review, parent.path(), &AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
    }
}
