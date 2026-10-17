//! One durable publication intent bridges the owned filesystem and SQLite.
//! The worker publishes only one download at a time. Hard links retain the
//! previous inode without copying an entire list into memory or onto disk.
use super::{Row, Store, params};
use rusqlite::OptionalExtension;
use std::{fs, path::Path};

const BACKUP: &str = "publication-backup";
const RESTORE: &str = "publication-restore";
#[cfg(test)]
#[path = "publication_tests.rs"]
mod tests;
fn sync(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())
}
fn regular(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => Ok(true),
        Ok(_) => Err("Publication recovery encountered a nonregular owned file".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}
fn remove(path: &Path) -> Result<(), String> {
    if regular(path)? {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl Store {
    fn publication_parent(&self, row: &Row) -> Result<(), String> {
        // Refuse symlinks/special files within the owned source tree before
        // creating or replacing a destination. Root is the configured manager.
        let mut path = self.root.clone();
        for part in [
            "sources".to_string(),
            row.id.to_string(),
            row.spec.format.directory().into(),
        ] {
            path.push(part);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_dir() => {}
                Ok(_) => return Err("Publication directory is not an owned directory".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(&path).map_err(|e| e.to_string())?;
                    sync(path.parent().unwrap())?;
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }

    fn begin_publication(&self, row: &Row) -> Result<(), String> {
        let pending: bool = self
            .db
            .query_row("SELECT EXISTS(SELECT 1 FROM publication_intent)", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        if pending {
            return Err(
                "An interrupted publication requires recovery before another update".into(),
            );
        }
        self.publication_parent(row)?;
        let destination = self.data_path(row);
        let mut check = row.clone();
        check.file = Some(destination.to_string_lossy().into_owned());
        self.validate_file(&check)?;
        let backup = self.root.join("tmp").join(BACKUP);
        if regular(&backup)? {
            return Err("A previous publication backup requires recovery".into());
        }
        let existed = regular(&destination)?;
        if existed {
            fs::hard_link(&destination, &backup)
                .map_err(|e| format!("Cannot retain the previous list: {e}"))?;
            sync(&backup)?;
            sync(backup.parent().unwrap())?;
        }
        if let Err(error) = self.db.execute(
            "INSERT INTO publication_intent VALUES(1,?1,?2,?3,?4)",
            params![row.id, row.revision, destination.to_string_lossy(), existed],
        ) {
            remove(&backup)?;
            sync(backup.parent().unwrap())?;
            return Err(error.to_string());
        }
        Ok(())
    }

    pub(super) fn recover_publication(&self) -> Result<(), String> {
        let intent: Option<(u64,u64,String,bool)> = self.db.query_row("SELECT subscription,revision,substr(destination,1,4097),existed FROM publication_intent WHERE id=1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e| e.to_string())?;
        let backup = self.root.join("tmp").join(BACKUP);
        let restore = self.root.join("tmp").join(RESTORE);
        if let Some((id, revision, destination, existed)) = intent {
            let row = self.get(id)?;
            if row.revision != revision
                || destination.len() > 4096
                || Path::new(&destination) != self.data_path(&row)
            {
                return Err(
                    "Publication recovery metadata changed; preserving owned files for inspection"
                        .into(),
                );
            }
            self.publication_parent(&row)?;
            let destination = Path::new(&destination);
            regular(destination)?;
            if existed {
                if !regular(&backup)? {
                    return Err(
                        "Previous publication backup is missing; preserving recovery metadata"
                            .into(),
                    );
                }
                remove(&restore)?;
                fs::hard_link(&backup, &restore).map_err(|e| e.to_string())?;
                // Retain backup until the rollback is durably recorded, making
                // repeated crashes during recovery idempotent.
                fs::rename(&restore, destination).map_err(|e| e.to_string())?;
            } else {
                remove(destination)?;
            }
            sync(destination.parent().unwrap())?;
            self.db
                .execute("DELETE FROM publication_intent WHERE id=1", [])
                .map_err(|e| e.to_string())?;
        }
        remove(&restore)?;
        remove(&backup)?;
        sync(backup.parent().unwrap())?;
        Ok(())
    }

    /// Run only after the last committed collection generation is synchronized.
    /// The durable queue makes a stop between pointer switch and unlink safe to
    /// retry without retaining an unaccounted old filename indefinitely.
    pub fn cleanup_publications(&self) -> Result<(), String> {
        let mut query = self.db.prepare("SELECT subscription,substr(file,1,4097) FROM publication_cleanup ORDER BY file LIMIT 5").map_err(|e|e.to_string())?;
        let entries = query
            .query_map([], |r| Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if entries.len() > 4 {
            return Err("Publication cleanup exceeds its recovery limit".into());
        }
        for (id, file) in entries {
            let mut row = self.get(id)?;
            row.file = Some(file.clone());
            self.validate_file(&row)?;
            let path = Path::new(&file);
            let referenced:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM subscription_publications WHERE file=?1) OR EXISTS(SELECT 1 FROM subscriptions WHERE file=?1)",[&file],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !referenced && regular(path)? {
                let relative = path
                    .parent()
                    .unwrap()
                    .strip_prefix(&self.root)
                    .map_err(|_| "Publication cleanup is outside its owned root")?;
                let mut parent = self.root.clone();
                for part in relative.components() {
                    if !matches!(part, std::path::Component::Normal(_)) {
                        return Err("Invalid publication cleanup directory".into());
                    }
                    parent.push(part);
                    if !fs::symlink_metadata(&parent)
                        .map_err(|e| e.to_string())?
                        .file_type()
                        .is_dir()
                    {
                        return Err(
                            "Publication cleanup directory is not an owned directory".into()
                        );
                    }
                }
                fs::remove_file(path).map_err(|e| e.to_string())?;
                sync(path.parent().unwrap())?;
            }
            self.db
                .execute("DELETE FROM publication_cleanup WHERE file=?", [file])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// A recoverable filesystem/SQL failure restores the old file and records
    /// an ordinary failed refresh. Unrecoverable rollback errors stop the
    /// worker, retaining the durable intent for the next startup.
    pub fn publish(
        &mut self,
        row: &Row,
        file: tempfile::NamedTempFile,
        bytes: u64,
        validators: (&str, &str),
        interval: u32,
    ) -> Result<bool, String> {
        if let Err(error) = self.begin_publication(row) {
            self.recover_publication()?;
            self.failed(row, &error, false)?;
            return Ok(false);
        }
        let destination = self.data_path(row);
        let result = (|| {
            file.persist(&destination)
                .map_err(|e| format!("Cannot publish validated list: {}", e.error))?;
            sync(destination.parent().unwrap())?;
            self.success(row, Some(&destination), bytes, validators, interval, false)
        })();
        match result {
            Ok(()) => {
                self.recover_publication()?;
                Ok(true)
            }
            Err(error) => {
                self.recover_publication().map_err(|recovery| {
                    format!("Publication failed: {error}; recovery required: {recovery}")
                })?;
                self.failed(row, &error, false)?;
                Ok(false)
            }
        }
    }
}
