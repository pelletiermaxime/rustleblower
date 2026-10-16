//! Typed group/all-list directories. Source data remains separately retained.
use super::{model::*, store::Store};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const MAX_LINKS: usize = MAX_SUBSCRIPTIONS * 9;
const KINDS: &[&str] = &["domains", "ips", "nets", "md5", "regexp"];
const OWNER: &str = "Rustleblower collection generation v1\n";
pub const REFRESH_MARKER: &str = "0-refresh.txt";
#[derive(Clone, Debug)]
pub struct Collection {
    pub group: Option<String>,
    pub format: Format,
    pub directory: PathBuf,
    pub members: usize,
}
#[derive(Clone, Default)]
pub struct Query {
    pub search: String,
    pub offset: usize,
    pub sort: Sort,
    pub descending: bool,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Group,
    Kind,
    Members,
}
pub struct Page {
    pub rows: Vec<Collection>,
    pub total: usize,
    pub offset: usize,
}
pub(super) struct Source {
    pub id: u64,
    pub groups: Vec<String>,
    pub publication: Publication,
}

/// Match the upstream slug rules without admitting path separators or aliases.
pub fn normalize_group(value: &str) -> Option<String> {
    if value.len() > 64 {
        return None;
    }
    let mut name = String::new();
    let mut separator = false;
    for character in value.trim().to_lowercase().chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            if separator {
                name.push('-');
                separator = false;
            }
            name.push(character);
        } else {
            separator = true;
        }
    }
    let name = name.trim_matches(['-', '.', '_']).to_owned();
    if name.is_empty() || name == "all" || name.len() > 64 {
        None
    } else {
        Some(name)
    }
}
pub fn page(catalogue: &[Collection], query: &Query) -> Result<Page, String> {
    if query.search.len() > 256 || query.offset > MAX_LINKS + 5 {
        return Err("Invalid collection query".into());
    }
    let search = query.search.to_lowercase();
    let mut matching = catalogue
        .iter()
        .filter(|collection| {
            format!(
                "{} {}",
                collection.group.as_deref().unwrap_or("all"),
                collection.format.directory()
            )
            .contains(&search)
        })
        .collect::<Vec<_>>();
    matching.sort_by(|a, b| {
        let order = match query.sort {
            Sort::Group => a.group.cmp(&b.group),
            Sort::Kind => a.format.directory().cmp(b.format.directory()),
            Sort::Members => a.members.cmp(&b.members),
        }
        .then_with(|| a.group.cmp(&b.group))
        .then_with(|| a.format.directory().cmp(b.format.directory()));
        if query.descending {
            order.reverse()
        } else {
            order
        }
    });
    let total = matching.len();
    let rows = matching
        .into_iter()
        .skip(query.offset)
        .take(PAGE)
        .cloned()
        .collect();
    Ok(Page {
        rows,
        total,
        offset: query.offset,
    })
}
pub fn draft(collection: &Collection) -> Result<crate::protocol::Rule, String> {
    let mut rule =
        super::attachment::directory_draft(collection.format.operand(), &collection.directory)?;
    let name = collection.group.as_deref().unwrap_or("all");
    rule.name = format!(
        "rustleblower-group-{name}-{}",
        collection.format.directory()
    );
    rule.description = format!(
        "Subscription collection: {name} · {} · {} active sources. Local-user scope.",
        collection.format.directory(),
        collection.members
    );
    crate::rules::document::validate_edit(&rule)?;
    Ok(rule)
}
pub(super) fn synchronize(store: &Store) -> Result<Vec<Collection>, String> {
    let sources = store.collection_sources()?;
    let root = fs::canonicalize(&store.root).map_err(|e| e.to_string())?;
    let base = root.join("collections");
    let generations = base.join("generations");
    owned_directory(&base)?;
    owned_directory(&generations)?;
    let current = base.join("current");
    let previous = match fs::symlink_metadata(&current) {
        Ok(meta) => {
            if !meta.file_type().is_symlink() {
                return Err(
                    "Collection pointer is not an owned symlink; refusing to replace it".into(),
                );
            }
            let target = fs::read_link(&current).map_err(|e| e.to_string())?;
            validate_generation(&target, &generations)?;
            Some(target)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    for (index, entry) in fs::read_dir(&generations)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if index >= 4 {
            return Err(
                "Too many retained collection generations; inspect owned recovery state".into(),
            );
        }
        let path = entry.map_err(|e| e.to_string())?.path();
        if previous.as_ref() != Some(&path) {
            validate_generation(&path, &generations)?;
            fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        }
    }
    let mut links = BTreeMap::<(Option<String>, String), BTreeMap<u64, PathBuf>>::new();
    for kind in KINDS {
        links.insert((None, kind.to_string()), BTreeMap::new());
    }
    let mut count = 0;
    let mut signature = Sha256::new();
    signature.update(OWNER.as_bytes());
    signature.update(store.collection_epoch()?.to_le_bytes());
    for source in sources {
        let target = fs::canonicalize(&source.publication.file).map_err(|e| e.to_string())?;
        let metadata = fs::metadata(&target).map_err(|e| e.to_string())?;
        signature.update(source.id.to_le_bytes());
        signature.update(target.as_os_str().as_encoded_bytes());
        signature.update(metadata.len().to_le_bytes());
        signature.update(
            metadata
                .modified()
                .map_err(|e| e.to_string())?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
                .to_le_bytes(),
        );
        if target.parent()
            != Some(
                root.join("sources")
                    .join(source.id.to_string())
                    .join(source.publication.format.directory())
                    .as_path(),
            )
        {
            return Err("Collection source resolves outside its owned typed directory".into());
        }
        let mut groups = source
            .groups
            .iter()
            .filter_map(|g| normalize_group(g))
            .collect::<Vec<_>>();
        groups.sort();
        groups.dedup();
        for group in &groups {
            signature.update([0]);
            signature.update(group.as_bytes());
        }
        for group in std::iter::once(None).chain(groups.into_iter().map(Some)) {
            count += 1;
            if count > MAX_LINKS {
                return Err("Collection links exceed their bounded catalogue".into());
            }
            links
                .entry((group, source.publication.format.directory().into()))
                .or_default()
                .insert(source.id, target.clone());
        }
    }
    let signature = format!("{:x}", signature.finalize());
    let make_catalogue=|links:&BTreeMap<(Option<String>,String),BTreeMap<u64,PathBuf>>|->Result<Vec<Collection>,String>{links.iter().map(|((group,kind),members)|Ok(Collection{group:group.clone(),format:Format::parse(kind)?,directory:current.join(relative(group.as_deref(),kind)),members:members.len()})).collect()};
    if previous
        .as_ref()
        .is_some_and(|p| signature_matches(&p.join(".signature"), &signature))
    {
        return make_catalogue(&links);
    }
    let previous_time = previous
        .as_ref()
        .and_then(|p| fs::metadata(p.join(".owner")).ok())
        .and_then(|m| m.modified().ok())
        .unwrap_or(std::time::UNIX_EPOCH);
    let epoch = std::time::SystemTime::now().max(previous_time + std::time::Duration::from_secs(1));
    let generation = tempfile::Builder::new()
        .prefix("generation-")
        .tempdir_in(&generations)
        .map_err(|e| e.to_string())?;
    fs::write(generation.path().join(".owner"), OWNER).map_err(|e| e.to_string())?;
    fs::File::open(generation.path().join(".owner"))
        .and_then(|f| f.set_modified(epoch))
        .map_err(|e| e.to_string())?;
    fs::write(generation.path().join(".signature"), &signature).map_err(|e| e.to_string())?;
    let mut catalogue = Vec::new();
    for ((group, kind), members) in links {
        let relative = relative(group.as_deref(), &kind);
        let path = generation.path().join(&relative);
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        // The daemon detects mtime/count changes, but misses replacement names
        // when the file count stays equal. A stable, comment-only marker forces
        // a reload for every changed collection generation, on coarse filesystems too.
        fs::write(
            path.join(REFRESH_MARKER),
            "# Rustleblower collection generation\n",
        )
        .map_err(|e| e.to_string())?;
        fs::File::open(path.join(REFRESH_MARKER))
            .and_then(|f| f.set_modified(epoch).and_then(|_| f.sync_all()))
            .map_err(|e| e.to_string())?;
        for (id, target) in &members {
            std::os::unix::fs::symlink(target, path.join(format!("{id}.txt")))
                .map_err(|e| e.to_string())?;
        }
        fs::File::open(&path)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        catalogue.push(Collection {
            group,
            format: Format::parse(&kind)?,
            directory: current.join(relative),
            members: members.len(),
        });
    }
    fs::File::open(generation.path())
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    let pending = base.join(format!(
        ".next-{}",
        generation.path().file_name().unwrap().to_string_lossy()
    ));
    std::os::unix::fs::symlink(generation.path(), &pending).map_err(|e| e.to_string())?;
    if let Err(error) = fs::rename(&pending, &current) {
        let _ = fs::remove_file(&pending);
        return Err(error.to_string());
    }
    let _ = generation.keep();
    fs::File::open(&base)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    if let Some(previous) = previous {
        fs::remove_dir_all(previous).map_err(|e| {
            format!("Collection switched; previous owned generation cleanup failed: {e}")
        })?;
    }
    Ok(catalogue)
}
fn relative(group: Option<&str>, kind: &str) -> PathBuf {
    match group {
        None => PathBuf::from("all").join(kind),
        Some(group) => PathBuf::from("groups").join(group).join(kind),
    }
}
fn owned_directory(path: &Path) -> Result<(), String> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
                Ok(())
            } else {
                Err("Collection directory is not an owned regular directory".into())
            }
        }
        Err(error) => Err(error.to_string()),
    }
}
fn validate_generation(path: &Path, generations: &Path) -> Result<(), String> {
    if path.parent() != Some(generations)
        || !path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("generation-"))
        || !fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
    {
        return Err("Collection pointer does not identify an owned generation".into());
    }
    use std::io::Read;
    let mut marker = String::new();
    fs::File::open(path.join(".owner"))
        .and_then(|f| f.take(128).read_to_string(&mut marker))
        .map_err(|e| e.to_string())?;
    if marker != OWNER {
        return Err("Collection generation ownership marker does not match".into());
    }
    Ok(())
}

fn signature_matches(path: &Path, signature: &str) -> bool {
    use std::io::Read;
    let mut data = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(65).read_to_end(&mut data))
        .is_ok()
        && data == signature.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collection_directory_symlinks_are_refused_without_touching_target() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        std::os::unix::fs::symlink(external.path(), root.path().join("collections")).unwrap();
        assert!(
            synchronize(&store)
                .unwrap_err()
                .contains("regular directory")
        );
        assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
    }
    #[test]
    fn only_marked_orphan_generations_are_recovered() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        synchronize(&store).unwrap();
        let generations = root.path().join("collections/generations");
        let owned = generations.join("generation-orphan");
        fs::create_dir(&owned).unwrap();
        fs::write(owned.join(".owner"), OWNER).unwrap();
        synchronize(&store).unwrap();
        assert!(!owned.exists());
        let unknown = generations.join("generation-unrelated");
        fs::create_dir(&unknown).unwrap();
        fs::write(unknown.join("keep.txt"), "unrelated").unwrap();
        assert!(synchronize(&store).is_err());
        assert_eq!(
            fs::read_to_string(unknown.join("keep.txt")).unwrap(),
            "unrelated"
        );
    }
    #[test]
    fn equal_size_and_equal_mtime_publications_still_reload_collections() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path()).unwrap();
        let id = store
            .save(
                None,
                &Spec {
                    name: "Coarse filesystem fixture".into(),
                    url: "http://127.0.0.1:9/fixture".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let row = store.get(id).unwrap();
        let path = store.data_path(&row);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(100);
        fs::write(&path, "0.0.0.0 first.example\n").unwrap();
        fs::File::open(&path).unwrap().set_modified(time).unwrap();
        store
            .success(&row, Some(&path), 22, ("", ""), 60, false)
            .unwrap();
        let catalog = synchronize(&store).unwrap();
        let pointer = root.path().join("collections/current");
        let before = fs::read_link(&pointer).unwrap();
        let collection = catalog
            .iter()
            .find(|c| c.group.is_none() && c.format == Format::Domains)
            .unwrap();
        let marker = fs::metadata(collection.directory.join(REFRESH_MARKER))
            .unwrap()
            .modified()
            .unwrap();
        let row = store.get(id).unwrap();
        fs::write(&path, "0.0.0.0 other.example\n").unwrap();
        fs::File::open(&path).unwrap().set_modified(time).unwrap();
        store
            .success(&row, Some(&path), 22, ("", ""), 60, false)
            .unwrap();
        synchronize(&store).unwrap();
        assert_ne!(fs::read_link(&pointer).unwrap(), before);
        assert!(
            fs::metadata(collection.directory.join(REFRESH_MARKER))
                .unwrap()
                .modified()
                .unwrap()
                > marker
        );
    }
}
