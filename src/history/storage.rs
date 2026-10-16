//! Transactional local retention preferences; storage work stays on the writer.
use super::{now_nanos, now_seconds};
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::RwLock;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub days: u32,
    pub mib: u32,
    pub cleanup_minutes: u32,
}
impl Config {
    pub fn validate(self) -> Result<()> {
        ensure!(
            (1..=365).contains(&self.days),
            "Retention must be 1–365 days"
        );
        ensure!(
            (16..=4096).contains(&self.mib),
            "Database target must be 16–4096 MiB"
        );
        ensure!(
            (1..=60).contains(&self.cleanup_minutes),
            "Idle cleanup interval must be 1–60 minutes"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub config: Config,
    pub revision: u64,
}
pub struct State {
    value: RwLock<Snapshot>,
}
impl State {
    pub fn new(value: Snapshot) -> Self {
        Self {
            value: RwLock::new(value),
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        *self.value.read().unwrap()
    }
    pub(super) fn publish(&self, value: Snapshot) {
        *self.value.write().unwrap() = value;
    }
}
#[derive(Default, Clone, Copy)]
pub struct Overrides {
    pub days: Option<u32>,
    pub mib: Option<u32>,
}
#[derive(Clone, Debug)]
pub struct Disk {
    pub allocated: u64,
    pub reusable: u64,
    pub page_limit: u64,
    pub target_limit: u64,
}
impl Disk {
    pub fn pending(&self) -> bool {
        self.allocated > self.target_limit || self.page_limit > self.target_limit
    }
}
#[derive(Clone, Debug)]
pub struct Preview {
    pub previous: Snapshot,
    pub candidate: Config,
    pub expired_events: u64,
    pub expired_alerts: u64,
    pub expired_counters: u64,
    pub disk: Disk,
}
#[derive(Clone, Debug)]
pub struct Applied {
    pub snapshot: Snapshot,
    pub removed_events: u64,
    pub removed_alerts: u64,
    pub removed_counters: u64,
    pub disk: Disk,
    pub warning: Option<String>,
    pub disk_verified: bool,
}
pub(super) fn schema(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS history_preferences(id INTEGER PRIMARY KEY CHECK(id=1),days INTEGER NOT NULL,mib INTEGER NOT NULL,cleanup_minutes INTEGER NOT NULL,revision INTEGER NOT NULL,edited INTEGER NOT NULL);")?;
    Ok(())
}
fn read(db: &Connection) -> Result<Option<(Snapshot, bool)>> {
    Ok(db
        .query_row(
            "SELECT days,mib,cleanup_minutes,revision,edited FROM history_preferences WHERE id=1",
            [],
            |r| {
                Ok((
                    Snapshot {
                        config: Config {
                            days: r.get(0)?,
                            mib: r.get(1)?,
                            cleanup_minutes: r.get(2)?,
                        },
                        revision: r.get(3)?,
                    },
                    r.get(4)?,
                ))
            },
        )
        .optional()?)
}
fn put(db: &Connection, value: Snapshot, edited: bool) -> Result<()> {
    db.execute("INSERT INTO history_preferences VALUES(1,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET days=excluded.days,mib=excluded.mib,cleanup_minutes=excluded.cleanup_minutes,revision=excluded.revision,edited=excluded.edited",params![value.config.days,value.config.mib,value.config.cleanup_minutes,value.revision,edited])?;
    Ok(())
}
pub(super) fn initialize(
    db: &Connection,
    days: u32,
    mib: u32,
    overrides: Overrides,
) -> Result<Snapshot> {
    let old = read(db)?;
    let defaults = Config {
        days,
        mib,
        cleanup_minutes: 5,
    };
    let mut candidate = old
        .filter(|(_, edited)| *edited)
        .map_or(defaults, |(s, _)| s.config);
    if let Some(days) = overrides.days {
        candidate.days = days;
    }
    if let Some(mib) = overrides.mib {
        candidate.mib = mib;
    }
    candidate.validate()?;
    let value = Snapshot {
        config: candidate,
        revision: old.map_or(0, |(s, _)| s.revision + u64::from(s.config != candidate)),
    };
    put(
        db,
        value,
        old.is_some_and(|(_, edited)| edited)
            || overrides.days.is_some()
            || overrides.mib.is_some(),
    )?;
    Ok(value)
}
pub fn disk(db: &Connection, mib: u32) -> Result<Disk> {
    let page_size: u64 = db.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    let pages: u64 = db.query_row("PRAGMA page_count", [], |r| r.get(0))?;
    let free: u64 = db.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    let limit: u64 = db.query_row("PRAGMA max_page_count", [], |r| r.get(0))?;
    Ok(Disk {
        allocated: pages * page_size,
        reusable: free * page_size,
        page_limit: limit * page_size,
        target_limit: u64::from(mib) * 1024 * 1024 * 3 / 4,
    })
}
pub(super) fn cap(db: &Connection, mib: u32) -> Result<Disk> {
    let size: u64 = db.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    db.pragma_update(
        None,
        "max_page_count",
        u64::from(mib) * 1024 * 1024 * 3 / 4 / size,
    )?;
    disk(db, mib)
}
pub(super) fn preview(db: &Connection, candidate: Config, expected: u64) -> Result<Preview> {
    candidate.validate()?;
    let previous = read(db)?
        .ok_or_else(|| anyhow::anyhow!("Retention settings are missing"))?
        .0;
    ensure!(
        previous.revision == expected,
        "Retention changed in another editor; reopen preferences before reviewing again"
    );
    let cutoff = now_seconds() - i64::from(candidate.days) * 86400;
    Ok(Preview {
        previous,
        candidate,
        expired_events: db.query_row(
            "SELECT count(*) FROM events WHERE received<?",
            [cutoff],
            |r| r.get(0),
        )?,
        expired_alerts: db.query_row(
            "SELECT count(*) FROM alerts WHERE received<?",
            [cutoff],
            |r| r.get(0),
        )?,
        expired_counters: db.query_row(
            "SELECT count(*) FROM statistics_counts WHERE updated<?",
            [now_nanos() - i64::from(candidate.days) * 86400 * 1_000_000_000],
            |r| r.get(0),
        )?,
        disk: disk(db, candidate.mib)?,
    })
}
fn counts(db: &Connection) -> Result<(u64, u64, u64)> {
    Ok((
        db.query_row("SELECT count(*) FROM events", [], |r| r.get(0))?,
        db.query_row("SELECT count(*) FROM alerts", [], |r| r.get(0))?,
        db.query_row("SELECT count(*) FROM statistics_counts", [], |r| r.get(0))?,
    ))
}
pub(super) fn apply(db: &mut Connection, candidate: Config, expected: u64) -> Result<Applied> {
    let reviewed = preview(db, candidate, expected)?;
    let before = counts(db)?;
    let snapshot = Snapshot {
        config: candidate,
        revision: reviewed.previous.revision + 1,
    };
    let tx = db.transaction()?;
    super::prune_reserved(&tx, candidate.days, candidate.mib, 0)?;
    put(&tx, snapshot, true)?;
    let after = counts(&tx)?;
    tx.commit()?;
    // Data/settings are already committed. A physical maintenance problem must not
    // be reported as a rejected settings save or as lost telemetry.
    let mut warning = super::maintain(db, candidate.days, candidate.mib)
        .err()
        .map(|e| format!("Settings saved; physical cleanup is pending: {e:#}"));
    let (disk, disk_verified) = match cap(db, candidate.mib) {
        Ok(disk) => (disk, true),
        Err(error) => {
            warning = Some(format!(
                "Settings saved; physical database limits could not be confirmed: {error:#}"
            ));
            (reviewed.disk, false)
        }
    };
    Ok(Applied {
        snapshot,
        removed_events: before.0.saturating_sub(after.0),
        removed_alerts: before.1.saturating_sub(after.1),
        removed_counters: before.2.saturating_sub(after.2),
        disk,
        warning,
        disk_verified,
    })
}
