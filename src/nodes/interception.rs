//! UI-owned, opt-in pause restoration. Only acknowledged choices update memory.
//! Runtime snapshots never perform I/O; the existing SQLite worker owns writes.
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};
use std::{
    collections::BTreeMap,
    sync::{Mutex, RwLock},
};

#[derive(Default)]
pub struct State {
    values: RwLock<BTreeMap<String, (bool, u64)>>,
    pub error: Mutex<Option<String>>,
}
impl State {
    /// None means opt-out; Some(false) requests pause on a future running report.
    pub fn get(&self, node: &str) -> Option<bool> {
        self.values
            .read()
            .unwrap()
            .get(node)
            .map(|(running, _)| *running)
    }
    pub(crate) fn may_restore(&self, node: &str, session: u64) -> bool {
        self.values.read().unwrap().get(node) == Some(&(false, session))
    }
    pub(crate) fn synchronize(&self, db: &Connection) {
        let result = read(db);
        match result {
            Ok(values) => {
                *self.values.write().unwrap() = values;
                *self.error.lock().unwrap() = None;
            }
            Err(error) => {
                self.values.write().unwrap().clear();
                *self.error.lock().unwrap() = Some(format!(
                    "Saved pause preferences are unavailable: {error:#}. Automatic pause restoration is suspended."
                ));
            }
        }
    }
}
pub(crate) fn schema(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS node_pause(name TEXT PRIMARY KEY CHECK(length(CAST(name AS BLOB)) BETWEEN 1 AND 128),running INTEGER NOT NULL CHECK(running IN (0,1)));")
}
fn read(db: &Connection) -> Result<BTreeMap<String, (bool, u64)>> {
    let (count, bytes): (usize, usize) = db.query_row(
        "SELECT count(*),coalesce(sum(length(CAST(name AS BLOB))),0) FROM node_pause",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(
        count <= super::MAX_NODES && bytes <= super::MAX_NODES * 128,
        "Pause preferences exceed the 32-node bound"
    );
    let mut query=db.prepare("SELECT p.name,p.running,n.session FROM node_pause p JOIN node_info n ON n.name=p.name ORDER BY p.name LIMIT 32")?;
    Ok(query
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (r.get::<_, bool>(1)?, r.get::<_, u64>(2)?),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?)
}
pub(crate) fn set(
    db: &mut Connection,
    session: u64,
    node: &str,
    remember: bool,
    expected_running: bool,
    expected: Option<bool>,
) -> Result<()> {
    let tx = db.transaction()?;
    let (saved_session, running): (u64, bool) = tx.query_row(
        "SELECT session,running FROM node_info WHERE name=?",
        [node],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(
        saved_session == session && running == expected_running,
        "Node changed while choosing pause persistence; reload its state"
    );
    let values = read(&tx)?;
    ensure!(
        values.get(node).map(|(running, _)| *running) == expected,
        "Pause preference changed; reload before saving"
    );
    if remember {
        ensure!(
            values.contains_key(node) || values.len() < super::MAX_NODES,
            "At most 32 nodes can remember interception state"
        );
        tx.execute("INSERT INTO node_pause VALUES(?,?) ON CONFLICT(name) DO UPDATE SET running=excluded.running",params![node,running])?;
    } else {
        tx.execute("DELETE FROM node_pause WHERE name=?", [node])?;
    }
    let _ = read(&tx)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_node_eviction_bounds_saved_pauses_and_corruption_suspends_restore() -> Result<()> {
        let mut db = Connection::open_in_memory()?;
        crate::rules::schema(&db)?;
        crate::nodes::schema(&db)?;
        let state = State::default();
        for id in 0..33 {
            let name = format!("owned-{id:02}");
            let config = crate::protocol::ClientConfig {
                is_firewall_running: true,
                ..Default::default()
            };
            crate::nodes::save(
                &mut db,
                id,
                &name,
                crate::nodes::Saved::prepare(&config).map_err(anyhow::Error::msg)?,
                &[],
            )?;
            set(&mut db, id, &name, true, true, None)?;
        }
        assert_eq!(
            db.query_row("SELECT count(*) FROM node_pause", [], |r| r
                .get::<_, u64>(0))?,
            32
        );
        state.synchronize(&db);
        assert_eq!(state.get("owned-00"), None);
        assert_eq!(state.get("owned-32"), Some(true));
        db.execute("INSERT INTO node_pause VALUES('owned-corrupt-extra',0)", [])?;
        state.synchronize(&db);
        assert!(
            state
                .error
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("32-node")
        );
        assert_eq!(
            state.get("owned-32"),
            None,
            "no restoration from a partial oversized preference snapshot"
        );
        Ok(())
    }
}
