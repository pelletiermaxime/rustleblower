//! Bounded full-catalogue name/key index; protobuf documents stay unchanged.
use super::{MAX_CATALOG_BYTES, MAX_RULE_BYTES, MAX_RULES, Query, memory_bytes, valid};
use crate::{
    protocol::Rule,
    view_preferences::{RuleField, Sort},
};
use anyhow::{Result, ensure};
use prost::Message;
use rusqlite::Connection;
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Value {
    Text(String),
    Number(i64),
    Receipt(Option<i64>),
    Flag(bool),
    Duration(DurationKey),
}
impl Value {
    fn bytes(&self) -> usize {
        match self {
            Self::Text(value) => value.capacity(),
            Self::Duration(key) => key.text.capacity(),
            _ => 0,
        }
    }
}
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct DurationKey {
    rank: u8,
    nanos: u64,
    text: String,
}
impl DurationKey {
    fn new(text: &str) -> Self {
        let (rank, nanos) = match text {
            "once" => (0, 0),
            "until restart" => (1, 0),
            "always" => (2, 0),
            value => match super::catalogue_policy::duration_nanos(value) {
                Some(nanos) => (0, nanos),
                None => (3, 0),
            },
        };
        Self {
            rank,
            nanos,
            text: if rank == 3 {
                text.to_owned()
            } else {
                String::new()
            },
        }
    }
}
struct Entry {
    name: String,
    value: Value,
}
fn value(rule: &Rule, received: Option<i64>, field: RuleField) -> Value {
    match field {
        RuleField::Name => Value::Text(rule.name.clone()),
        RuleField::Action => Value::Text(rule.action.clone()),
        RuleField::Enabled => Value::Flag(rule.enabled),
        RuleField::Duration => Value::Duration(DurationKey::new(&rule.duration)),
        RuleField::Created => Value::Number(rule.created),
        RuleField::Received => Value::Receipt(received),
        RuleField::Priority => Value::Flag(rule.precedence),
        RuleField::Logging => Value::Flag(!rule.nolog),
        RuleField::Process => Value::Text(super::executable(rule).unwrap_or("").to_owned()),
        RuleField::Description => Value::Text(rule.description.clone()),
    }
}
/// Called only by background readers inside their frozen SQLite snapshot.
/// At most10,000 keys/names and8MiB are retained; one rule is decoded at a time.
pub(super) fn names(db: &Connection, query: &Query) -> Result<Vec<String>> {
    let predicate = super::query::Predicate::new(query)?;
    let sort = query.sort.unwrap_or(Sort {
        field: RuleField::Name,
        ascending: true,
    });
    let mut statement=db.prepare("WITH catalogue AS (SELECT node,name,wire FROM application_rules UNION ALL SELECT node,name,wire FROM prompt_rules) SELECT c.name,c.wire,o.received FROM catalogue c LEFT JOIN rule_observations o ON o.node=c.node AND o.name=c.name WHERE c.node=?1 ORDER BY c.name LIMIT 10001")?;
    let mut records = statement.query([&query.node])?;
    let (mut scanned, mut wire_bytes, mut strings) = (0usize, 0usize, 0usize);
    let mut entries: Vec<Entry> = Vec::new();
    while let Some(record) = records.next()? {
        scanned += 1;
        let wire = record.get_ref(1)?.as_blob()?;
        wire_bytes += wire.len();
        ensure!(
            scanned <= MAX_RULES && wire.len() <= MAX_RULE_BYTES && wire_bytes <= MAX_CATALOG_BYTES,
            "Rule ordering exceeds catalogue limits"
        );
        let rule = Rule::decode(wire)?;
        valid(&rule).map_err(anyhow::Error::msg)?;
        ensure!(
            memory_bytes(&rule) <= 256 * 1024 && record.get_ref(0)?.as_str()? == rule.name,
            "Invalid cached rule document"
        );
        let received = record.get::<_, Option<i64>>(2)?;
        if !predicate.matches_received(&query.node, &rule, received) {
            continue;
        }
        let value = value(&rule, received, sort.field);
        strings += rule.name.capacity() + value.bytes();
        entries.push(Entry {
            name: rule.name,
            value,
        });
        ensure!(
            strings + entries.capacity() * std::mem::size_of::<Entry>() <= 8 * 1024 * 1024,
            "Rule ordering index exceeds 8 MiB"
        );
    }
    entries.sort_by(|a, b| {
        let order = a.value.cmp(&b.value);
        let order = if sort.ascending {
            order
        } else {
            order.reverse()
        };
        order.then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries.into_iter().map(|entry| entry.name).collect())
}

/// Build at most201 name bindings after the global sort. The larger transient
/// ordering index is dropped before page protobufs are retained.
pub(super) fn page_sql(
    db: &Connection,
    query: &Query,
) -> Result<(String, Vec<rusqlite::types::Value>)> {
    let names = names(db, query)?;
    let start = if query.after.is_empty() {
        0
    } else {
        names
            .iter()
            .position(|name| name == &query.after)
            .ok_or_else(|| anyhow::anyhow!("Rule page anchor changed; return to the first page"))?
            + 1
    };
    let page = names.into_iter().skip(start).take(201).collect::<Vec<_>>();
    let catalogue = "WITH catalogue AS (SELECT *,0 AS awaiting FROM application_rules UNION ALL SELECT *,1 AS awaiting FROM prompt_rules)";
    let columns = "SELECT c.name,c.wire,c.awaiting,o.received,o.ui_run FROM catalogue c LEFT JOIN rule_observations o ON o.node=c.node AND o.name=c.name";
    if page.is_empty() {
        return Ok((format!("{catalogue} {columns} WHERE 0"), Vec::new()));
    }
    let mut values = vec![rusqlite::types::Value::Text(query.node.clone())];
    let positions = page
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            values.push(rusqlite::types::Value::Text(name));
            format!("(?{},{} )", index + 2, index)
        })
        .collect::<Vec<_>>()
        .join(",");
    Ok((
        format!(
            "{catalogue}, positions(name,ordinal) AS (VALUES {positions}) {columns} JOIN positions p ON p.name=c.name WHERE c.node=?1 ORDER BY p.ordinal"
        ),
        values,
    ))
}
