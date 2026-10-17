//! Retained-event detail grouping on a private SQLite snapshot. Counts belong
//! to retained reports, never daemon counters. All methods doing I/O are called
//! from a background executor; a retained permit bounds the snapshot lifetime.
mod output;
use super::{
    Query, Row, export,
    query::{self, Field},
};
use anyhow::{Context, Result, ensure};
pub use output::{Export, Selection};
use rusqlite::{Connection, OpenFlags, params_from_iter, types::Value};
use std::{
    path::Path,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const PAGE_SIZE: usize = 200;
pub const QUERY_SECONDS: u64 = 15;
const COLUMNS: &str = "id,unixnano,action,protocol,process,destination,port,reason,node,metadata";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Node,
    Rule,
    Host,
    Application,
    Address,
    Port,
    User,
    Protocol,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Node => "Node",
            Self::Rule => "Matched rule",
            Self::Host => "Destination host",
            Self::Application => "Executable",
            Self::Address => "Destination IP",
            Self::Port => "Destination port",
            Self::User => "UID",
            Self::Protocol => "Protocol",
        }
    }
    fn field(self) -> Field {
        match self {
            Self::Node => Field::Node,
            Self::Rule => Field::Rule,
            Self::Host => Field::Host,
            Self::Application => Field::Process,
            Self::Address => Field::DestinationIp,
            Self::Port => Field::Port,
            Self::User => Field::User,
            Self::Protocol => Field::Protocol,
        }
    }
    fn keys(self) -> &'static [Field] {
        use Field::*;
        match self {
            Self::Node => &[
                Process,
                Command,
                User,
                SourceIp,
                DestinationIp,
                Host,
                Port,
                Protocol,
            ],
            Self::Rule => &[Process, Command, User, DestinationIp, Host, Port],
            Self::Host => &[
                Pid,
                Process,
                Command,
                SourceIp,
                DestinationIp,
                Port,
                Protocol,
                Action,
                Node,
            ],
            Self::Application => &[
                SourceIp,
                DestinationIp,
                Host,
                Port,
                User,
                Action,
                Node,
                Pid,
                Command,
            ],
            Self::Address => &[
                Pid, Process, Command, SourceIp, Port, Host, Protocol, Action, User, Node,
            ],
            Self::Port => &[
                Pid,
                Process,
                Command,
                Host,
                SourceIp,
                DestinationIp,
                Protocol,
                Action,
                User,
                Node,
            ],
            Self::User => &[
                Pid,
                Process,
                Command,
                SourceIp,
                DestinationIp,
                Host,
                Port,
                Protocol,
                Action,
                Node,
            ],
            Self::Protocol => &[
                Pid,
                Process,
                Command,
                User,
                SourceIp,
                DestinationIp,
                Host,
                Port,
                Action,
                Node,
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub kind: Kind,
    pub node: String,
    pub value: String,
}
impl Scope {
    fn predicate(&self) -> Result<(String, Vec<Value>)> {
        ensure!(
            self.node.len() <= 128 && self.value.len() <= 4096,
            "Grouped history scope exceeds its size limit"
        );
        ensure!(
            !matches!(self.kind, Kind::Node | Kind::Rule) || !self.node.is_empty(),
            "Choose the exact node for grouped node or rule history"
        );
        ensure!(
            self.kind != Kind::Rule || (!self.value.is_empty() && self.value.len() <= 1024),
            "Choose a valid matched rule name"
        );
        ensure!(
            self.kind != Kind::Node || self.value.is_empty(),
            "Node history uses the captured node name"
        );
        let mut terms = vec![];
        let mut values = vec![];
        if !self.node.is_empty() {
            terms.push("node=?".to_owned());
            values.push(Value::Text(self.node.clone()));
        }
        if self.kind != Kind::Node {
            let field = self.kind.field();
            if self.value.is_empty() {
                terms.push(format!("({0} IS NULL OR {0}='')", field.sql()));
            } else if field.numeric() {
                let value = self
                    .value
                    .parse::<u32>()
                    .context("Choose a numeric history category value")?;
                ensure!(
                    field != Field::Port || value <= 65535,
                    "Destination port must be between 0 and 65535"
                );
                terms.push(format!("{}=?", field.sql()));
                values.push(Value::Integer(i64::from(value)));
            } else {
                terms.push(format!("{}=?", field.sql()));
                values.push(Value::Text(self.value.clone()));
            }
        }
        Ok((terms.join(" AND "), values))
    }
    pub fn validate(&self) -> Result<()> {
        self.predicate().map(|_| ())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Hits,
    Event(Field),
}
impl Column {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hits => "Retained event hits",
            Self::Event(field) => field.label(),
        }
    }
    fn sort_sql(self) -> String {
        match self {
            Self::Hits => "hits".into(),
            Self::Event(field) => format!(
                "coalesce({}, {})",
                field.sql(),
                if field.numeric() { "-1" } else { "''" }
            ),
        }
    }
}
#[derive(Clone)]
pub struct Group {
    pub event: Row,
    pub hits: u64,
}
pub struct Snapshot {
    file: tempfile::NamedTempFile,
    database: std::path::PathBuf,
    _permit: Permit,
    pub scope: Scope,
    pub ceiling: i64,
    pub groups: usize,
    pub events: u64,
}
pub struct Permit {
    _owned: OwnedSemaphorePermit,
}
pub fn permit() -> Option<Permit> {
    static BUDGET: OnceLock<Arc<Semaphore>> = OnceLock::new();
    BUDGET
        .get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
        .try_acquire_owned()
        .ok()
        .map(|owned| Permit { _owned: owned })
}
fn deadline(db: &Connection, stop: Arc<AtomicBool>) -> Result<()> {
    let started = Instant::now();
    db.progress_handler(
        10_000,
        Some(move || {
            stop.load(Ordering::Relaxed) || started.elapsed() >= Duration::from_secs(QUERY_SECONDS)
        }),
    )?;
    Ok(())
}
pub fn capture(
    database: &Path,
    scope: Scope,
    mut query: Query,
    stop: Arc<AtomicBool>,
    permit: Permit,
) -> Result<Snapshot> {
    let (scope_sql, scope_values) = scope.predicate()?;
    ensure!(
        !stop.load(Ordering::Relaxed),
        "Grouped history capture stopped"
    );
    let (file, db) = export::snapshot(database, &stop)?;
    deadline(&db, stop.clone())?;
    let maximum: i64 = db.query_row("SELECT coalesce(max(id),0) FROM events", [], |row| {
        row.get(0)
    })?;
    let ceiling = query.ceiling.unwrap_or(maximum).min(maximum);
    query.cursor = None;
    query.before = None;
    query.ceiling = Some(ceiling);
    let (base, mut values) = query::sql(&query, ceiling, 0)?;
    let base = base
        .rsplit_once(" ORDER BY ")
        .context("Invalid fixed history query")?
        .0;
    values.extend(scope_values);
    let keys = scope
        .kind
        .keys()
        .iter()
        .map(|field| field.sql())
        .collect::<Vec<_>>()
        .join(",");
    db.execute_batch("PRAGMA query_only=OFF; PRAGMA cache_size=-512; PRAGMA mmap_size=0; PRAGMA temp_store=FILE; PRAGMA journal_mode=OFF;")?;
    db.execute(&format!("CREATE TABLE grouped_detail AS WITH matched({COLUMNS},sort_value) AS ({base}), ranked AS (SELECT {COLUMNS}, count(*) OVER(PARTITION BY {keys}) AS hits, row_number() OVER(PARTITION BY {keys} ORDER BY unixnano DESC,id DESC) AS representative FROM matched WHERE {scope_sql}) SELECT {COLUMNS},hits FROM ranked WHERE representative=1"),params_from_iter(values))?;
    db.execute_batch("CREATE INDEX grouped_hits ON grouped_detail(hits DESC,unixnano DESC,id DESC); CREATE INDEX grouped_time ON grouped_detail(unixnano DESC,id DESC);")?;
    let (groups, events): (i64, i64) = db.query_row(
        "SELECT count(*),coalesce(sum(hits),0) FROM grouped_detail",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    ensure!(
        !stop.load(Ordering::Relaxed),
        "Grouped history capture stopped"
    );
    Ok(Snapshot {
        file,
        database: database.to_owned(),
        _permit: permit,
        scope,
        ceiling,
        groups: usize::try_from(groups)?,
        events: u64::try_from(events)?,
    })
}
impl Snapshot {
    fn open(&self, stop: Arc<AtomicBool>) -> Result<Connection> {
        ensure!(
            !stop.load(Ordering::Relaxed),
            "Grouped history read stopped"
        );
        let db = Connection::open_with_flags(
            self.file.path(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-512; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;")?;
        deadline(&db, stop)?;
        Ok(db)
    }
    pub fn detail(&self, id: i64, stop: Arc<AtomicBool>) -> Result<Option<super::details::Detail>> {
        let db = self.open(stop)?;
        let retained: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM grouped_detail WHERE id=?)",
            [id],
            |row| row.get(0),
        )?;
        if !retained {
            return Ok(None);
        }
        super::query_detail(&db, id)
    }
    pub fn page(
        &self,
        offset: usize,
        sort: (Column, bool),
        stop: Arc<AtomicBool>,
    ) -> Result<Vec<Group>> {
        let db = self.open(stop.clone())?;
        let direction = if sort.1 { "ASC" } else { "DESC" };
        let sql = format!(
            "SELECT {COLUMNS},hits FROM grouped_detail ORDER BY {} {direction},id {direction} LIMIT {PAGE_SIZE} OFFSET ?",
            sort.0.sort_sql()
        );
        let mut statement = db.prepare(&sql)?;
        let rows = statement
            .query_map([i64::try_from(offset)?], |record| {
                Ok(Group {
                    event: super::read_row(record)?,
                    hits: record.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            !stop.load(Ordering::Relaxed),
            "Grouped history read stopped"
        );
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        history::{self, Worker},
        protocol::{Connection as Network, Event, Rule},
    };
    const NODE: &str = "owned-node'_%";
    const HOST: &str = "literal_%' OR 1=1 --";
    const PROCESS: &str = "/owned/literal_%' OR 1=1 --";
    const RULE: &str = "owned-rule'_%";
    fn scope(kind: Kind) -> Scope {
        Scope {
            kind,
            node: if matches!(kind, Kind::Node | Kind::Rule) {
                NODE.into()
            } else {
                String::new()
            },
            value: match kind {
                Kind::Node => "",
                Kind::Rule => RULE,
                Kind::Host => HOST,
                Kind::Application => PROCESS,
                Kind::Address => "198.51.100.7",
                Kind::Port => "443",
                Kind::User => "1000",
                Kind::Protocol => "tcp",
            }
            .into(),
        }
    }
    fn event(index: usize, block: usize, time: i64) -> Event {
        Event {
            unixnano: time + index as i64 + if block > 0 { 100 } else { 0 },
            connection: Some(Network {
                process_path: PROCESS.into(),
                process_args: vec![format!("literal argument_%'{index:04}")],
                process_id: (1000 + index) as u32,
                user_id: 1000,
                src_ip: "192.0.2.1".into(),
                src_port: (5000 + block) as u32,
                dst_ip: "198.51.100.7".into(),
                dst_host: HOST.into(),
                dst_port: 443,
                protocol: "tcp".into(),
                ..Default::default()
            }),
            rule: Some(Rule {
                name: RULE.into(),
                duration: "always".into(),
                action: "allow".into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn owned_permit() -> Permit {
        Permit {
            _owned: Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap(),
        }
    }
    fn rows(snapshot: &Snapshot, sort: (Column, bool)) -> Result<Vec<Group>> {
        let mut rows = vec![];
        for offset in (0..snapshot.groups).step_by(PAGE_SIZE) {
            let page = snapshot.page(offset, sort, Arc::new(AtomicBool::new(false)))?;
            assert!(page.len() <= PAGE_SIZE);
            rows.extend(page);
        }
        Ok(rows)
    }

    #[test]
    fn private_backup_byte_cap_uses_the_actual_sqlite_page_size_and_vm_cancel_interrupts()
    -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("large-page.sqlite3");
        let db = Connection::open(&path)?;
        db.execute_batch("PRAGMA page_size=65536; VACUUM;")?;
        drop(db);
        let _worker = Worker::start(&path, 7, 64)?;
        let (_file, db) = export::snapshot(&path, &AtomicBool::new(false))?;
        let page_size: u64 = db.query_row("PRAGMA page_size", [], |row| row.get(0))?;
        let pages: u64 = db.query_row("PRAGMA max_page_count", [], |row| row.get(0))?;
        assert_eq!(page_size, 65536);
        assert!(
            pages * page_size <= 8 * 1024 * 1024 * 1024,
            "Private snapshot cap must bound bytes, not assume4096-byte pages"
        );
        let snapshot = capture(
            &path,
            scope(Kind::Node),
            Query::default(),
            Arc::new(AtomicBool::new(false)),
            permit().expect("The grouped view budget is initially available"),
        )?;
        let private_path = snapshot.file.path().to_owned();
        assert!(private_path.is_file());
        assert!(
            permit().is_none(),
            "A retained view keeps its snapshot budget"
        );
        drop(snapshot);
        assert!(!private_path.exists());
        assert!(permit().is_some());
        let db = Connection::open_in_memory()?;
        deadline(&db, Arc::new(AtomicBool::new(true)))?;
        let result=db.query_row("WITH RECURSIVE work(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM work WHERE n<1000000) SELECT sum(n) FROM work",[],|row|row.get::<_,i64>(0));
        assert!(
            matches!(result,Err(rusqlite::Error::SqliteFailure(error,_)) if error.code==rusqlite::ErrorCode::OperationInterrupted)
        );
        Ok(())
    }

    #[tokio::test]
    async fn source_scopes_group_all_retained_pages_and_keep_latest_ties_frozen_and_missing_values_distinct()
    -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("history.sqlite3");
        let worker = Worker::start(&path, 7, 64)?;
        let time = history::now_nanos() - 10000;
        for block in 0..3 {
            for node in [NODE, "owned-other-node"] {
                for start in (0..650).step_by(32) {
                    worker.history.ingest(
                        node.into(),
                        (start..(start + 32).min(650))
                            .map(|index| event(index, block, time))
                            .collect(),
                        "owned grouped fixture",
                    );
                    worker.history.flush().await?;
                }
            }
        }
        assert_eq!(worker.history.health.dropped.load(Ordering::Relaxed), 0);
        let stop = Arc::new(AtomicBool::new(false));
        for kind in [
            Kind::Node,
            Kind::Rule,
            Kind::Host,
            Kind::Application,
            Kind::Address,
            Kind::Port,
            Kind::User,
            Kind::Protocol,
        ] {
            let snapshot = capture(
                &path,
                scope(kind),
                Query::default(),
                stop.clone(),
                owned_permit(),
            )?;
            let one_node = matches!(kind, Kind::Node | Kind::Rule);
            assert_eq!(
                snapshot.groups,
                if one_node { 650 } else { 1300 },
                "{kind:?}"
            );
            assert_eq!(
                snapshot.events,
                if one_node { 1950 } else { 3900 },
                "{kind:?}"
            );
            let ascending = rows(&snapshot, (Column::Event(Field::Pid), true))?;
            let descending = rows(&snapshot, (Column::Event(Field::Pid), false))?;
            assert_eq!(ascending.len(), snapshot.groups);
            assert!(
                ascending
                    .iter()
                    .all(|row| row.hits == 3 && row.event.metadata.source_port == Some(5002))
            );
            assert!(
                ascending
                    .windows(2)
                    .all(|pair| pair[0].event.metadata.pid <= pair[1].event.metadata.pid)
            );
            assert!(
                descending
                    .windows(2)
                    .all(|pair| pair[0].event.metadata.pid >= pair[1].event.metadata.pid)
            );
            assert_eq!(ascending[0].event.metadata.pid, Some(1000));
            assert_eq!(ascending.last().unwrap().event.metadata.pid, Some(1649));
            assert_eq!(ascending[0].event.unixnano, time + 100);
            if one_node {
                assert!(ascending.iter().all(|row| row.event.node == NODE));
            }
            assert!(
                snapshot
                    .page(snapshot.groups, (Column::Hits, false), stop.clone())?
                    .is_empty()
            );
        }
        let snapshot = Arc::new(capture(
            &path,
            scope(Kind::Host),
            Query::default(),
            stop.clone(),
            owned_permit(),
        )?);
        let sort = (Column::Event(Field::Pid), true);
        let ordered = rows(&snapshot, sort)?;
        let mut selection = snapshot.selection(sort);
        selection.toggle(10)?;
        selection.extend_to(210)?;
        selection.toggle(150)?;
        selection.toggle(600)?;
        assert_eq!(selection.count(), 201);
        let output = selection.export(vec![Column::Hits, Column::Event(Field::Pid)])?;
        let (json, report) = output.copy(export::Format::JsonLines, stop.clone())?;
        assert_eq!(report.rows, 201);
        assert_eq!(report.bytes, json.len());
        let values = json
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let expected = ordered
            .iter()
            .enumerate()
            .filter(|(index, _)| ((10..=210).contains(index) && *index != 150) || *index == 600)
            .map(|(_, row)| row)
            .collect::<Vec<_>>();
        for (value, row) in values.iter().zip(expected) {
            assert_eq!(value.as_object().unwrap().len(), 2);
            assert_eq!(value["retained_hits"], 3);
            assert_eq!(value["pid"], row.event.metadata.pid.unwrap().to_string());
        }
        let (csv, _) = output.copy(export::Format::Csv, stop.clone())?;
        let target = dir.path().join("selected-groups.csv");
        assert_eq!(
            output
                .save(&target, export::Format::Csv, false, stop.clone())?
                .rows,
            201
        );
        assert_eq!(std::fs::read_to_string(&target)?, csv);
        assert!(
            output
                .save(&target, export::Format::JsonLines, false, stop.clone())
                .is_err()
        );
        assert!(
            output
                .save(
                    &target,
                    export::Format::JsonLines,
                    true,
                    Arc::new(AtomicBool::new(true))
                )
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&target)?, csv);
        assert!(output.with_columns(vec![]).is_err());
        assert!(
            output
                .with_columns(vec![Column::Hits, Column::Hits])
                .is_err()
        );
        assert!(
            output
                .save(&path, export::Format::JsonLines, true, stop.clone())
                .is_err()
        );
        selection.clear();
        assert_eq!(selection.count(), 0);
        assert_eq!(
            output.copy(export::Format::JsonLines, stop.clone())?.0,
            json,
            "Output keeps captured marks after the working selection changes"
        );
        for index in 0..output::MAX_MARKS {
            selection.toggle(index)?;
        }
        assert!(selection.toggle(output::MAX_MARKS).is_err());
        assert_eq!(selection.count(), output::MAX_MARKS);
        selection.select_all();
        selection.toggle(0)?;
        assert_eq!(selection.count(), 1299);
        assert_eq!(
            selection
                .export(vec![Column::Hits])?
                .copy(export::Format::JsonLines, stop.clone())?
                .1
                .rows,
            1299
        );
        assert!(selection.toggle(snapshot.groups).is_err());
        assert_eq!(selection.count(), 1299);
        let original = rows(&snapshot, (Column::Hits, false))?
            .iter()
            .map(|row| row.event.id)
            .collect::<Vec<_>>();
        let ceiling = snapshot.ceiling;
        worker
            .history
            .ingest(NODE.into(), vec![event(9999, 3, time)], "new after capture");
        worker.history.flush().await?;
        assert_eq!(
            rows(&snapshot, (Column::Hits, false))?
                .iter()
                .map(|row| row.event.id)
                .collect::<Vec<_>>(),
            original
        );
        let query = Query {
            ceiling: Some(ceiling),
            ..Default::default()
        };
        assert_eq!(
            capture(
                &path,
                scope(Kind::Host),
                query,
                stop.clone(),
                owned_permit()
            )?
            .groups,
            1300
        );
        let db = Connection::open(&path)?;
        db.execute(
            "UPDATE events SET metadata=json_set(metadata,'$.user',NULL) WHERE id=1",
            [],
        )?;
        db.execute(
            "UPDATE events SET metadata=json_set(metadata,'$.user',0) WHERE id=1301",
            [],
        )?;
        let missing = capture(
            &path,
            scope(Kind::Node),
            Query {
                ceiling: Some(ceiling),
                ..Default::default()
            },
            stop.clone(),
            owned_permit(),
        )?;
        assert_eq!(missing.groups, 652);
        assert_eq!(missing.events, 1950);
        let page = missing.page(0, (Column::Event(Field::User), true), stop.clone())?;
        assert_eq!(page[0].event.metadata.user, None);
        assert_eq!(page[0].hits, 1);
        assert_eq!(page[1].event.metadata.user, Some(0));
        assert_eq!(page[1].hits, 1);
        assert!(
            capture(
                &path,
                Scope {
                    kind: Kind::Port,
                    node: String::new(),
                    value: "65536".into()
                },
                Query::default(),
                stop.clone(),
                owned_permit()
            )
            .is_err()
        );
        assert!(
            capture(
                &path,
                Scope {
                    kind: Kind::Rule,
                    node: String::new(),
                    value: RULE.into()
                },
                Query::default(),
                stop.clone(),
                owned_permit()
            )
            .is_err()
        );
        assert!(
            capture(
                &path,
                scope(Kind::Host),
                Query::default(),
                Arc::new(AtomicBool::new(true)),
                owned_permit()
            )
            .is_err()
        );
        assert!(
            snapshot
                .page(0, (Column::Hits, false), Arc::new(AtomicBool::new(true)))
                .is_err()
        );
        assert_eq!(worker.history.health.dropped.load(Ordering::Relaxed), 0);
        let representative = original[0];
        db.execute("DELETE FROM events", [])?;
        let detail = snapshot
            .detail(representative, stop.clone())?
            .expect("A captured representative keeps its private detail after retention");
        assert_eq!(detail.row.id, representative);
        assert_eq!(
            detail.event.unwrap().connection.unwrap().process_path,
            PROCESS
        );
        assert!(
            snapshot.detail(1, stop)?.is_none(),
            "Unselected source events are not grouped representatives"
        );
        Ok(())
    }
}
