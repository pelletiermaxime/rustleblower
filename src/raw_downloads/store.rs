use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    settings: Settings,
}
fn bytes(path: &Path, max: usize) -> Result<Option<Vec<u8>>> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(file.metadata()?.is_file(), "Expected regular file");
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= max, "File exceeds size limit");
    Ok(Some(bytes))
}
pub(super) fn profile(path: &Path) -> Result<Settings> {
    super::path(path)?;
    let bytes = bytes(path, MAX_CONFIG)?.ok_or_else(|| anyhow::anyhow!("Profile is missing"))?;
    import_source(std::str::from_utf8(&bytes)?)
}
pub(super) fn write_profile(guard: &Guard, path: &Path, settings: &Settings) -> Result<PathBuf> {
    let target = guard.target(path, true)?;
    ensure!(stamp(&target)?.is_none(), "Choose a new profile filename");
    let profile = export_source(settings)?;
    let parent = target.parent().unwrap();
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(profile.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(&target).map_err(|e| e.error)?;
    File::open(parent)?.sync_all()?;
    Ok(target)
}
pub(super) fn read(path: &Path) -> Result<(Settings, Option<String>)> {
    let Some(bytes) = bytes(path, MAX_CONFIG)? else {
        return Ok((Settings::default(), None));
    };
    let document: Document = serde_json::from_slice(&bytes)?;
    ensure!(document.version == 1, "Unsupported configuration version");
    document.settings.validate()?;
    Ok((document.settings, Some(hex::encode(Sha256::digest(bytes)))))
}
pub(super) fn load(path: &Path) -> Snapshot {
    match read(path) {
        Ok((settings, revision)) => Snapshot {
            completed_runs: 0,
            ready: true,
            settings,
            revision,
            warning: None,
            busy: false,
            active_group: None,
            reports: Vec::new(),
        },
        Err(_) => Snapshot {
            completed_runs: 0,
            ready: true,
            settings: Settings::default(),
            revision: None,
            warning: Some(
                "Raw download configuration is invalid or inaccessible; existing file preserved"
                    .into(),
            ),
            busy: false,
            active_group: None,
            reports: Vec::new(),
        },
    }
}
pub(super) fn save(
    path: &Path,
    expected: &Option<String>,
    settings: &Settings,
) -> Result<Snapshot> {
    settings.validate()?;
    ensure!(
        &read(path)?.1 == expected,
        "Configuration changed; reload before saving"
    );
    let bytes = serde_json::to_vec(&Document {
        version: 1,
        settings: settings.clone(),
    })?;
    ensure!(bytes.len() <= MAX_CONFIG, "Configuration exceeds 64 KiB");
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    ensure!(
        &read(path)?.1 == expected,
        "Configuration changed; reload before saving"
    );
    if expected.is_none() {
        temp.persist_noclobber(path).map_err(|e| e.error)?;
    } else {
        temp.persist(path).map_err(|e| e.error)?;
    }
    File::open(parent)?.sync_all()?;
    Ok(load(path))
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Stamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
    mode: u32,
}
pub(super) fn stamp(path: &Path) -> Result<Option<Stamp>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "Destination is not a regular file"
            );
            ensure!(
                meta.len() <= MAX_FILE,
                "Existing destination exceeds 16 MiB"
            );
            Ok(Some(Stamp {
                dev: meta.dev(),
                ino: meta.ino(),
                len: meta.len(),
                mtime: meta.mtime(),
                mtime_ns: meta.mtime_nsec(),
                ctime: meta.ctime(),
                ctime_ns: meta.ctime_nsec(),
                mode: meta.mode(),
            }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn normalized(path: &Path) -> Result<PathBuf> {
    let mut parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Missing parent"))?;
    let mut suffix = Vec::new();
    while !parent.exists() {
        ensure!(suffix.len() < 64, "Too many missing parent directories");
        suffix.push(
            parent
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Missing parent component"))?
                .to_os_string(),
        );
        parent = parent
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Missing ancestor"))?;
    }
    let mut base = parent.canonicalize()?;
    for component in suffix.into_iter().rev() {
        base.push(component);
    }
    base.push(
        path.file_name()
            .ok_or_else(|| anyhow::anyhow!("Missing filename"))?,
    );
    Ok(base)
}
#[derive(Clone)]
pub(super) struct Guard {
    database: PathBuf,
    socket: PathBuf,
    journal: PathBuf,
    prefix: String,
}
impl Guard {
    pub fn new(database: &Path, socket: &Path) -> Self {
        let database = if database.is_absolute() {
            database.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(database)
        };
        let digest = hex::encode(Sha256::digest(database.as_os_str().as_encoded_bytes()));
        Self {
            database: database.clone(),
            socket: socket.into(),
            journal: journal_path(&database),
            prefix: format!(".rustleblower-raw-{}-", &digest[..16]),
        }
    }
    pub fn target(&self, path: &Path, create: bool) -> Result<PathBuf> {
        super::path(path)?;
        let target = normalized(path)?;
        ensure!(
            !target
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .starts_with(".rustleblower-raw-"),
            "Reserved stage filename"
        );
        let mut protected = vec![PathBuf::from("/tmp/osui.sock")];
        if self.socket.is_absolute() {
            protected.push(self.socket.clone());
        }
        for suffix in [
            "",
            "-wal",
            "-shm",
            ".history.json",
            ".preferences.json",
            ".appearance.json",
            ".window.json",
            ".updates.json",
            ".reputation.json",
            ".highlighting.json",
            ".catalogue-policy.json",
            ".live-view.json",
            ".views.json",
            ".raw-downloads.json",
            ".raw-downloads-journal.json",
        ] {
            let mut p = self.database.as_os_str().to_os_string();
            p.push(suffix);
            protected.push(PathBuf::from(p));
        }
        let meta = std::fs::symlink_metadata(&target).ok();
        for path in protected {
            let protected = normalized(&path)?;
            ensure!(target != protected, "Protected Rustleblower path");
            if let (Ok(a), Some(b)) = (std::fs::metadata(&protected), &meta) {
                ensure!(
                    a.dev() != b.dev() || a.ino() != b.ino(),
                    "Protected Rustleblower file identity"
                );
            }
        }
        let subscriptions = self
            .database
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("subscriptions");
        ensure!(
            !target.starts_with(
                normalized(&subscriptions.join("placeholder"))?
                    .parent()
                    .unwrap()
            ),
            "Protected normalized subscription directory"
        );
        if create {
            let parent = target.parent().unwrap();
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700).create(parent)?;
            ensure!(
                parent.canonicalize()? == parent,
                "Destination directory changed"
            );
        }
        let _ = stamp(&target)?;
        Ok(target)
    }
    pub fn settings(&self, settings: &Settings) -> Result<()> {
        let mut paths = BTreeSet::new();
        let mut files = BTreeSet::new();
        for group in &settings.groups {
            for resource in &group.urls {
                let path = self.target(&resource.localfile, false)?;
                ensure!(paths.insert(path.clone()), "Aliased destination paths");
                if let Ok(meta) = std::fs::metadata(&path) {
                    ensure!(
                        files.insert((meta.dev(), meta.ino())),
                        "Aliased destination files"
                    );
                }
            }
        }
        Ok(())
    }
    pub fn recover(&self) -> Result<()> {
        let Some(bytes) = bytes(&self.journal, 16 * 1024)? else {
            return Ok(());
        };
        let record: Record = serde_json::from_slice(&bytes)?;
        ensure!(
            record.version == 1
                && record
                    .path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with(&self.prefix)),
            "Unknown raw-download journal"
        );
        match std::fs::symlink_metadata(&record.path) {
            Ok(meta) => {
                ensure!(
                    meta.is_file()
                        && !meta.file_type().is_symlink()
                        && meta.dev() == record.dev
                        && meta.ino() == record.ino
                        && meta.uid() == record.uid,
                    "Stage identity changed"
                );
                std::fs::remove_file(&record.path)?;
                File::open(record.path.parent().unwrap())?.sync_all()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        clear_journal(&self.journal, &bytes)
    }
    pub fn stage(&self, target: &Path) -> Result<Stage> {
        ensure!(!self.journal.exists(), "Pending recovery journal");
        let mut temp = tempfile::Builder::new()
            .prefix(&self.prefix)
            .tempfile_in(target.parent().unwrap())?;
        let meta = temp.as_file().metadata()?;
        let record = Record {
            version: 1,
            path: temp.path().into(),
            dev: meta.dev(),
            ino: meta.ino(),
            uid: meta.uid(),
        };
        let bytes = serde_json::to_vec(&record)?;
        temp.disable_cleanup(true);
        let mut stage = Stage {
            temp: Some(temp),
            journal: self.journal.clone(),
            keep_journal: false,
            record,
            journal_bytes: None,
        };
        let parent = self.journal.parent().unwrap();
        std::fs::create_dir_all(parent)?;
        let mut journal = tempfile::NamedTempFile::new_in(parent)?;
        journal.write_all(&bytes)?;
        journal.as_file().sync_all()?;
        journal
            .persist_noclobber(&self.journal)
            .map_err(|e| e.error)?;
        stage.journal_bytes = Some(bytes);
        File::open(parent)?.sync_all()?;
        Ok(stage)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    path: PathBuf,
    dev: u64,
    ino: u64,
    uid: u32,
}
impl Record {
    fn matches(&self) -> bool {
        std::fs::symlink_metadata(&self.path).is_ok_and(|meta| {
            meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.dev() == self.dev
                && meta.ino() == self.ino
                && meta.uid() == self.uid
        })
    }
}
fn clear_journal(path: &Path, expected: &[u8]) -> Result<()> {
    match bytes(path, 16 * 1024)? {
        None => return Ok(()),
        Some(current) => ensure!(
            current == expected,
            "Recovery journal changed; preserve external file"
        ),
    }
    std::fs::remove_file(path)?;
    File::open(path.parent().unwrap())?.sync_all()?;
    Ok(())
}
pub(super) struct Stage {
    temp: Option<tempfile::NamedTempFile>,
    journal: PathBuf,
    keep_journal: bool,
    record: Record,
    journal_bytes: Option<Vec<u8>>,
}
impl Stage {
    #[cfg(test)]
    pub(super) fn leave_for_recovery(mut self) -> Result<PathBuf> {
        let (file, path) = self.temp.take().unwrap().keep().map_err(|e| e.error)?;
        drop(file);
        self.keep_journal = true;
        Ok(path)
    }
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.temp.as_mut().unwrap().write_all(bytes)?;
        Ok(())
    }
    pub fn publish(mut self, target: &Path, expected: &Option<Stamp>) -> Result<Status> {
        ensure!(self.record.matches(), "Owned stage identity changed");
        if let Some(expected) = &self.journal_bytes {
            ensure!(
                bytes(&self.journal, 16 * 1024)?.as_deref() == Some(expected.as_slice()),
                "Recovery journal changed"
            );
        }
        let temp = self.temp.take().unwrap();
        ensure!(&stamp(target)? == expected, "Destination changed");
        if let Some(stamp) = expected {
            temp.as_file()
                .set_permissions(std::fs::Permissions::from_mode(stamp.mode & 0o777))?;
            temp.as_file().sync_all()?;
            temp.persist(target).map_err(|e| e.error)?;
        } else {
            temp.as_file().sync_all()?;
            temp.persist_noclobber(target).map_err(|e| e.error)?;
        }
        self.keep_journal = true;
        if File::open(target.parent().unwrap())
            .and_then(|file| file.sync_all())
            .is_err()
        {
            return Ok(Status::RecoveryNeeded);
        }
        if self
            .journal_bytes
            .as_ref()
            .is_none_or(|expected| clear_journal(&self.journal, expected).is_err())
        {
            return Ok(Status::RecoveryNeeded);
        }
        self.keep_journal = false;
        Ok(Status::Published)
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        self.temp.take();
        if self.keep_journal {
            return;
        }
        if self.record.matches() {
            if std::fs::remove_file(&self.record.path).is_err() {
                return;
            }
            if File::open(self.record.path.parent().unwrap())
                .and_then(|file| file.sync_all())
                .is_err()
            {
                return;
            }
        } else if self.record.path.exists() {
            return;
        }
        if let Some(expected) = &self.journal_bytes {
            let _ = clear_journal(&self.journal, expected);
        }
    }
}
