use super::model::*;
mod transfer;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
pub(super) struct Store {
    pub db: Connection,
    pub root: PathBuf,
    _lock: File,
}
pub(super) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
const COLUMNS: &str = "id,revision,substr(config,1,8193),removed,checked,updated,next_due,failures,bytes,substr(status,1,64),substr(error,1,1024),substr(etag,1,1024),substr(modified,1,1024),substr(file,1,4097)";
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let data: String = r.get(2)?;
    if data.len() > 8192 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let spec: Spec = serde_json::from_str(&data).map_err(|_| rusqlite::Error::InvalidQuery)?;
    spec.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(Row {
        id: r.get(0)?,
        revision: r.get(1)?,
        spec,
        removed: r.get(3)?,
        checked: r.get(4)?,
        updated: r.get(5)?,
        next_due: r.get(6)?,
        failures: r.get(7)?,
        bytes: r.get(8)?,
        status: r.get(9)?,
        error: r.get(10)?,
        etag: r.get(11)?,
        modified: r.get(12)?,
        file: r.get(13)?,
        publication: None,
    })
}
impl Store {
    pub fn open(root: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(root.join("manager.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock()
            .map_err(|_| "Another subscription manager owns this directory")?;
        std::fs::create_dir_all(root.join("tmp")).map_err(|e| e.to_string())?;
        let db = Connection::open(root.join("subscriptions.sqlite3")).map_err(|e| e.to_string())?;
        db.busy_timeout(Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA cache_size=-512; PRAGMA mmap_size=0;
        CREATE TABLE IF NOT EXISTS settings(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL,config TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS subscriptions(id INTEGER PRIMARY KEY AUTOINCREMENT,revision INTEGER NOT NULL,config TEXT NOT NULL,enabled INTEGER NOT NULL,removed INTEGER NOT NULL DEFAULT 0,checked INTEGER NOT NULL DEFAULT 0,updated INTEGER NOT NULL DEFAULT 0,next_due INTEGER NOT NULL DEFAULT 0,failures INTEGER NOT NULL DEFAULT 0,bytes INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL DEFAULT 'never',error TEXT NOT NULL DEFAULT '',etag TEXT NOT NULL DEFAULT '',modified TEXT NOT NULL DEFAULT '',file TEXT);
        CREATE TABLE IF NOT EXISTS logs(id INTEGER PRIMARY KEY AUTOINCREMENT,subscription INTEGER NOT NULL,time INTEGER NOT NULL,message TEXT NOT NULL);").map_err(|e|e.to_string())?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS subscription_publications(subscription INTEGER NOT NULL,kind TEXT NOT NULL,format TEXT NOT NULL,url TEXT NOT NULL,file TEXT NOT NULL,bytes INTEGER NOT NULL,PRIMARY KEY(subscription,kind));").map_err(|e|e.to_string())?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS collection_epoch(id INTEGER PRIMARY KEY CHECK(id=1),value INTEGER NOT NULL); INSERT OR IGNORE INTO collection_epoch VALUES(1,0);").map_err(|e|e.to_string())?;
        db.execute(
            "INSERT OR IGNORE INTO settings VALUES(1,1,?)",
            [serde_json::to_string(&Settings::default()).unwrap()],
        )
        .map_err(|e| e.to_string())?;
        transfer::initialize(&db)?;
        let mut store = Self {
            db,
            root: root.to_owned(),
            _lock: lock,
        };
        store.recover()?;
        Ok(store)
    }
    fn recover(&mut self) -> Result<(), String> {
        for (index, entry) in std::fs::read_dir(self.root.join("tmp"))
            .map_err(|e| e.to_string())?
            .enumerate()
        {
            if index >= 256 {
                return Err("Too many temporary subscription files".into());
            }
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_file()
                && entry.file_name().to_string_lossy().starts_with("download-")
            {
                std::fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
            }
        }
        self.db.execute("UPDATE subscriptions SET status='interrupted',error='Previous update was interrupted; refresh to synchronize',etag='',modified='',next_due=0 WHERE status='downloading'",[]).map_err(|e|e.to_string())?;
        Ok(())
    }
    pub fn settings(&self) -> Result<(u64, Settings), String> {
        let (revision, text): (u64, String) = self
            .db
            .query_row(
                "SELECT revision,substr(config,1,8193) FROM settings WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if text.len() > 8192 {
            return Err("Subscription settings exceed their limit".into());
        }
        let config: Settings = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        config.validate()?;
        Ok((revision, config))
    }
    pub fn save_settings(&mut self, revision: u64, config: &Settings) -> Result<(), String> {
        config.validate()?;
        let tx = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        let schedule = validate_limits(&self.db, config)?;
        let count = self
            .db
            .execute(
                "UPDATE settings SET revision=revision+1,config=? WHERE id=1 AND revision=?",
                params![serde_json::to_string(config).unwrap(), revision],
            )
            .map_err(|e| e.to_string())?;
        if count != 1 {
            return Err("Settings changed; reopen them before saving".into());
        }
        apply_schedule(&self.db, &schedule)?;
        tx.commit().map_err(|e| e.to_string())
    }
    pub fn get(&self, id: u64) -> Result<Row, String> {
        let mut row = self
            .db
            .query_row(
                &format!("SELECT {COLUMNS} FROM subscriptions WHERE id=?"),
                [id],
                row,
            )
            .map_err(|e| e.to_string())?;
        self.validate_file(&row)?;
        row.publication = self.publication(&row)?;
        Ok(row)
    }
    fn publication(&self, row: &Row) -> Result<Option<Publication>, String> {
        let value:Option<(String,String,String,u64)>=self.db.query_row("SELECT substr(format,1,32),substr(url,1,2049),substr(file,1,4097),bytes FROM subscription_publications WHERE subscription=? AND kind=?",params![row.id,row.spec.format.directory()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
        let Some((format, url, file, bytes)) = value else {
            return Ok(None);
        };
        if url != row.spec.url || format != row.spec.format.key() {
            return Ok(None);
        }
        let mut check = row.clone();
        check.file = Some(file.clone());
        self.validate_file(&check)?;
        if Path::new(&file).parent() != Some(self.data_path(row).parent().unwrap())
            || !std::fs::metadata(&file).is_ok_and(|m| m.is_file())
        {
            return Ok(None);
        }
        Ok(Some(Publication {
            file,
            bytes,
            format: row.spec.format.clone(),
            url,
        }))
    }
    pub fn collection_sources(&self) -> Result<Vec<super::collections::Source>, String> {
        let mut query = self
            .db
            .prepare(
                "SELECT id FROM subscriptions WHERE removed=0 AND enabled=1 ORDER BY id LIMIT 129",
            )
            .map_err(|e| e.to_string())?;
        let ids = query
            .query_map([], |r| r.get::<_, u64>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        if ids.len() > MAX_SUBSCRIPTIONS {
            return Err("Active collection sources exceed 128 subscriptions".into());
        }
        let mut sources = Vec::new();
        for id in ids {
            let row = self.get(id)?;
            let value:Option<(String,String,String,u64)>=self.db.query_row("SELECT substr(format,1,32),substr(url,1,2049),substr(file,1,4097),bytes FROM subscription_publications WHERE subscription=? AND kind=?",params![id,row.spec.format.directory()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
            if let Some((format, url, file, bytes)) = value {
                let format = Format::parse(&format)?;
                if format.directory() != row.spec.format.directory() {
                    return Err("Stored collection source has a mismatched kind".into());
                }
                let mut check = row.clone();
                check.file = Some(file.clone());
                self.validate_file(&check)?;
                if !std::fs::metadata(&file).is_ok_and(|m| m.is_file()) {
                    continue;
                }
                sources.push(super::collections::Source {
                    id,
                    groups: row.spec.groups,
                    publication: Publication {
                        file,
                        bytes,
                        format,
                        url,
                    },
                });
            }
        }
        Ok(sources)
    }
    pub fn collection_epoch(&self) -> Result<u64, String> {
        self.db
            .query_row("SELECT value FROM collection_epoch WHERE id=1", [], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())
    }
    pub fn replacing_bytes(&self, row: &Row) -> Result<u64, String> {
        let value: Option<u64> = self
            .db
            .query_row(
                "SELECT bytes FROM subscription_publications WHERE subscription=? AND kind=?",
                params![row.id, row.spec.format.directory()],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(bytes) = value {
            return Ok(bytes);
        }
        let exists: bool = self
            .db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM subscription_publications WHERE subscription=?)",
                [row.id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(if exists { 0 } else { row.bytes })
    }
    pub fn unreferenced_files(&self, row: &Row) -> Result<Vec<String>, String> {
        let mut candidates = vec![];
        if let Some(file) = &row.file {
            candidates.push(file.clone());
        }
        if let Some(file) = self
            .db
            .query_row(
                "SELECT substr(file,1,4097) FROM subscription_publications WHERE subscription=? AND kind=?",
                params![row.id, row.spec.format.directory()],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
        {
            let mut check=row.clone();check.file=Some(file.clone());self.validate_file(&check)?;
            candidates.push(file);
        }
        candidates.sort();
        candidates.dedup();
        Ok(candidates)
    }
    pub fn referenced(&self, file: &str) -> Result<bool, String> {
        self.db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM subscription_publications WHERE file=?)",
                [file],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }
    fn validate_file(&self, row: &Row) -> Result<(), String> {
        if let Some(file) = &row.file {
            let path = Path::new(file);
            let parent = self.root.join("sources").join(row.id.to_string());
            let owned_parent = path.parent().is_some_and(|p| {
                p == parent
                    || ["domains", "ips", "nets", "regexp", "md5"]
                        .iter()
                        .any(|kind| p == parent.join(kind))
            });
            if file.len() > 4096
                || !owned_parent
                || !path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("download-"))
            {
                return Err("Stored subscription file is outside its owned directory".into());
            }
            match std::fs::symlink_metadata(path) {
                Ok(metadata) if !metadata.file_type().is_file() => {
                    return Err("Stored subscription file is not a regular owned file".into());
                }
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.to_string());
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub fn bulk_review(
        &self,
        scope: &super::bulk::Scope,
        patch: &super::bulk::Patch,
    ) -> Result<super::bulk::Review, String> {
        use super::bulk::{Entry, Review, Scope};
        scope.validate()?;
        patch.validate()?;
        let ids = match scope {
            Scope::Selected(ids) => ids.clone(),
            Scope::Matching(search) => {
                let mut ids = Vec::new();
                let mut query = Query {
                    search: search.clone(),
                    ..Default::default()
                };
                loop {
                    let page = self.page(&query)?;
                    ids.extend(page.rows.iter().map(|r| r.id));
                    if ids.len() > MAX_SUBSCRIPTIONS {
                        return Err("Bulk scope exceeds 128 subscriptions".into());
                    }
                    if page.offset + PAGE >= page.total {
                        break;
                    }
                    query.offset += PAGE;
                }
                ids
            }
        };
        let mut review = Review {
            patch: patch.clone(),
            entries: Vec::new(),
            scanned: ids.len(),
        };
        for id in ids {
            let row = self.get(id)?;
            if row.removed {
                return Err("A selected subscription was removed; refresh the selection".into());
            }
            let after = patch.apply(&row.spec)?;
            effective(&after, &self.settings()?.1)?;
            if after != row.spec {
                review.entries.push(Entry {
                    id,
                    revision: row.revision,
                    before: row.spec,
                    after,
                });
            }
        }
        review.entries.sort_by(|a, b| {
            a.before
                .name
                .to_lowercase()
                .cmp(&b.before.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        review.validate()?;
        Ok(review)
    }
    pub fn apply_bulk(&mut self, review: &super::bulk::Review) -> Result<usize, String> {
        review.validate()?;
        let settings = self.settings()?.1;
        let tx = self.db.transaction().map_err(|e| e.to_string())?;
        // Revalidate the entire immutable plan before writing its first row.
        for entry in &review.entries {
            effective(&entry.after, &settings)?;
            let current: Option<(u64, String, bool)> = tx
                .query_row(
                    "SELECT revision,substr(config,1,8193),removed FROM subscriptions WHERE id=?",
                    [entry.id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let Some((revision, config, removed)) = current else {
                return Err("A reviewed subscription disappeared; rebuild the review".into());
            };
            if removed
                || revision != entry.revision
                || serde_json::from_str::<Spec>(&config).map_err(|e| e.to_string())? != entry.before
            {
                return Err(format!(
                    "{} changed since review; rebuild before applying",
                    entry.before.name
                ));
            }
        }
        for entry in &review.entries {
            let format_changed = entry.before.format != entry.after.format;
            tx.execute("UPDATE subscriptions SET revision=revision+1,config=?1,enabled=?2,next_due=0,status='changed',etag=CASE WHEN ?3 THEN '' ELSE etag END,modified=CASE WHEN ?3 THEN '' ELSE modified END,error='' WHERE id=?4 AND revision=?5",params![serde_json::to_string(&entry.after).map_err(|e|e.to_string())?,entry.after.enabled,format_changed,entry.id,entry.revision]).map_err(|e|e.to_string())?;
            tx.execute(
                "INSERT INTO logs(subscription,time,message) VALUES(?,?,?)",
                params![
                    entry.id,
                    now(),
                    "Updated by a reviewed bulk subscription change"
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "DELETE FROM logs WHERE id NOT IN(SELECT id FROM logs ORDER BY id DESC LIMIT 1024)",
            [],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(review.entries.len())
    }
    pub fn save(&mut self, id: Option<(u64, u64)>, spec: &Spec) -> Result<u64, String> {
        spec.validate()?;
        let tx = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        effective(spec, &self.settings()?.1)?;
        let data = serde_json::to_string(spec).map_err(|e| e.to_string())?;
        if data.len() > 8192 {
            return Err("Subscription config exceeds 8 KiB".into());
        }
        let active: usize = self
            .db
            .query_row(
                "SELECT count(*) FROM subscriptions WHERE removed=0",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let result = if let Some((id, revision)) = id {
            let old = self.get(id)?;
            if old.revision != revision {
                return Err("Subscription changed; reopen before saving".into());
            }
            if old.removed && active >= MAX_SUBSCRIPTIONS {
                return Err("At most 128 active subscriptions are supported".into());
            }
            let changed = old.spec.url != spec.url || old.spec.format != spec.format;
            self.db.execute("UPDATE subscriptions SET revision=revision+1,config=?1,enabled=?2,removed=0,next_due=0,status='changed',etag=CASE WHEN ?3 THEN '' ELSE etag END,modified=CASE WHEN ?3 THEN '' ELSE modified END,error='' WHERE id=?4 AND revision=?5",params![data,spec.enabled,changed,id,revision]).map_err(|e|e.to_string())?;
            id
        } else {
            let total: usize = self
                .db
                .query_row("SELECT count(*) FROM subscriptions", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if active >= MAX_SUBSCRIPTIONS || total >= MAX_RECORDS {
                return Err(
                    "Subscription catalogue limit reached (128 active / 256 retained)".into(),
                );
            }
            self.db
                .execute(
                    "INSERT INTO subscriptions(revision,config,enabled) VALUES(1,?,?)",
                    params![data, spec.enabled],
                )
                .map_err(|e| e.to_string())?;
            self.db.last_insert_rowid() as u64
        };
        self.log(result, "Subscription saved")?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }
    pub fn remove(&mut self, id: u64, revision: u64) -> Result<(), String> {
        let count=self.db.execute("UPDATE subscriptions SET removed=1,enabled=0,revision=revision+1,status='removed' WHERE id=? AND revision=?",params![id,revision]).map_err(|e|e.to_string())?;
        if count != 1 {
            return Err("Subscription changed before removal".into());
        }
        self.log(
            id,
            "Removed from scheduling; downloaded data retained for existing rules",
        )
    }
    pub fn page(&self, q: &Query) -> Result<Page, String> {
        if q.search.len() > 256 || q.offset > MAX_RECORDS {
            return Err("Invalid subscription query".into());
        }
        let column = match q.sort {
            Sort::Name => "lower(json_extract(config,'$.name'))",
            Sort::Updated => "updated",
            Sort::Size => "bytes",
            Sort::Status => "status",
        };
        let direction = if q.descending { "DESC" } else { "ASC" };
        let filter = "removed=?1 AND instr(lower(json_extract(config,'$.name')||' '||json_extract(config,'$.url')),lower(?2))>0";
        let total = self
            .db
            .query_row(
                &format!("SELECT count(*) FROM subscriptions WHERE {filter}"),
                params![q.removed, q.search],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let mut stmt=self.db.prepare(&format!("SELECT {COLUMNS} FROM subscriptions WHERE {filter} ORDER BY {column} {direction},id LIMIT ?3 OFFSET ?4")).map_err(|e|e.to_string())?;
        let mut rows = stmt
            .query_map(params![q.removed, q.search, PAGE, q.offset], row)
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        for row in &mut rows {
            self.validate_file(row)?;
            row.publication = self.publication(row)?;
        }
        Ok(Page {
            settings: self.settings()?,
            rows,
            total,
            offset: q.offset,
        })
    }
    pub fn due(&self, time: i64) -> Result<Option<u64>, String> {
        self.db.query_row("SELECT id FROM subscriptions WHERE removed=0 AND enabled=1 AND next_due<=? ORDER BY next_due,id LIMIT 1",[time],|r|r.get(0)).optional().map_err(|e|e.to_string())
    }
    pub fn mark_started(&self, id: u64) -> Result<(), String> {
        self.db
            .execute(
                "UPDATE subscriptions SET checked=?,status='downloading',error='' WHERE id=?",
                params![now(), id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn failed(&self, row: &Row, error: &str, cancelled: bool) -> Result<(), String> {
        let delay = if cancelled {
            60
        } else {
            (60u64.saturating_mul(2u64.saturating_pow(row.failures.saturating_add(1).min(10))))
                .min(6 * 3600)
        };
        self.db.execute("UPDATE subscriptions SET status=?1,error=?2,failures=min(1000,failures+?3),next_due=?4 WHERE id=?5 AND revision=?6",params![if cancelled{"cancelled"}else{"error"},crate::history::bounded(error,1024),!cancelled,now()+delay as i64,row.id,row.revision]).map_err(|e|e.to_string())?;
        self.log(row.id, error)
    }
    pub fn success(
        &mut self,
        row: &Row,
        path: Option<&Path>,
        bytes: u64,
        validators: (&str, &str),
        interval: u32,
        unchanged: bool,
    ) -> Result<(), String> {
        let (etag, modified) = validators;
        let tx = self.db.transaction().map_err(|e| e.to_string())?;
        if let Some(path) = path {
            if row.spec.enabled {
                tx.execute("UPDATE collection_epoch SET value=value+1 WHERE id=1", [])
                    .map_err(|e| e.to_string())?;
            }
            tx.execute("INSERT INTO subscription_publications VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(subscription,kind) DO UPDATE SET format=excluded.format,url=excluded.url,file=excluded.file,bytes=excluded.bytes",params![row.id,row.spec.format.directory(),row.spec.format.key(),row.spec.url,path.to_string_lossy(),bytes]).map_err(|e|e.to_string())?;
        }
        tx.execute("UPDATE subscriptions SET status=?1,error='',failures=0,bytes=?2,file=COALESCE(?3,file),etag=?4,modified=?5,next_due=?6,updated=CASE WHEN ?7 THEN updated ELSE ?8 END WHERE id=?9 AND revision=?10",params![if unchanged{"unchanged"}else{"updated"},bytes,path.map(|p|p.to_string_lossy().to_string()),etag,modified,now()+i64::from(interval),unchanged,now(),row.id,row.revision]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        self.log(
            row.id,
            if unchanged {
                "HTTP 304; retained downloaded data"
            } else {
                "Download validated and published"
            },
        )
    }
    pub fn used(&self) -> Result<u64, String> {
        self.db
            .query_row(
                "SELECT COALESCE(sum(bytes),0) FROM (SELECT bytes FROM subscription_publications UNION ALL SELECT bytes FROM subscriptions WHERE NOT EXISTS(SELECT 1 FROM subscription_publications WHERE subscription=subscriptions.id))",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }
    pub fn data_path(&self, row: &Row) -> PathBuf {
        self.root
            .join("sources")
            .join(row.id.to_string())
            .join(row.spec.format.directory())
            .join(format!(
                "download-{}{}",
                row.spec.filename,
                if row.spec.filename.contains('.') {
                    ""
                } else {
                    ".txt"
                }
            ))
    }
    pub fn log(&self, id: u64, message: &str) -> Result<(), String> {
        self.db
            .execute(
                "INSERT INTO logs(subscription,time,message) VALUES(?,?,?)",
                params![id, now(), crate::history::bounded(message, 1024)],
            )
            .map_err(|e| e.to_string())?;
        self.db
            .execute(
                "DELETE FROM logs WHERE id NOT IN(SELECT id FROM logs ORDER BY id DESC LIMIT 1024)",
                [],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn logs(&self, id: u64) -> Result<Vec<String>, String> {
        let mut s = self
            .db
            .prepare(
                "SELECT time,message FROM logs WHERE subscription=? ORDER BY id DESC LIMIT 100",
            )
            .map_err(|e| e.to_string())?;
        s.query_map([id], |r| {
            Ok(format!(
                "{} · {}",
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())
    }
}

/// Stream at most one bounded spec at a time; execute within the caller's transaction.
fn validate_limits(db: &Connection, settings: &Settings) -> Result<Vec<(u64, i64)>, String> {
    let mut statement = db
        .prepare("SELECT substr(config,1,8193),id,checked,failures,removed,status FROM subscriptions ORDER BY id LIMIT 257")
        .map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut count = 0;
    let mut schedule = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        count += 1;
        if count > MAX_RECORDS {
            return Err("Catalogue exceeds 256 retained records".into());
        }
        let text: String = row.get(0).map_err(|e| e.to_string())?;
        if text.len() > 8192 {
            return Err("Subscription config exceeds 8 KiB".into());
        }
        let spec: Spec = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let (interval, _, _) = effective(&spec, settings)
            .map_err(|e| format!("{}: {e}; defaults were not saved", spec.name))?;
        let failures: u32 = row.get(3).map_err(|e| e.to_string())?;
        let removed: bool = row.get(4).map_err(|e| e.to_string())?;
        let status: String = row.get(5).map_err(|e| e.to_string())?;
        if failures == 0 && !removed && status != "downloading" {
            let id: u64 = row.get(1).map_err(|e| e.to_string())?;
            let checked: i64 = row.get(2).map_err(|e| e.to_string())?;
            schedule.push((
                id,
                if checked == 0 {
                    0
                } else {
                    checked.saturating_add(i64::from(interval))
                },
            ));
        }
    }
    Ok(schedule)
}
fn apply_schedule(db: &Connection, rows: &[(u64, i64)]) -> Result<(), String> {
    for (id, due) in rows {
        db.execute(
            "UPDATE subscriptions SET next_due=? WHERE id=?",
            params![due, id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
