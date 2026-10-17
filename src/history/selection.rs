//! Bounded cross-page selection descriptors. A selection captures filters,
//! ordering and a maximum event ID; it never expands Select all into row IDs.
//! Actual row reads/export remain on the history/background worker.
use super::{
    Query, Row,
    query::{self, Cursor, Field},
};
use anyhow::{Result, ensure};
use rusqlite::types::Value;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

pub const MAX_OVERRIDES: usize = 1024;
pub const MAX_BYTES: usize = 128 * 1024;
const MAX_KEY_BYTES: usize = 8192;

#[derive(Clone)]
pub struct Selection {
    query: Query,
    ceiling: i64,
    all: bool,
    range: Option<(Cursor, Cursor)>,
    overrides: BTreeMap<i64, (Cursor, bool)>,
    anchor: Option<Cursor>,
}
impl Selection {
    pub fn capture(mut query: Query, ceiling: i64) -> Result<Self> {
        ensure!(ceiling >= 0, "Invalid history selection ceiling");
        query.before = None;
        query.cursor = None;
        query.ceiling = Some(query.ceiling.unwrap_or(ceiling).min(ceiling));
        let ceiling = query.ceiling.unwrap();
        ensure!(ceiling >= 0, "Invalid history selection ceiling");
        query::sql(&query, ceiling, 0)?;
        Ok(Self {
            query,
            ceiling,
            all: false,
            range: None,
            overrides: BTreeMap::new(),
            anchor: None,
        })
    }
    pub fn query(&self) -> &Query {
        &self.query
    }
    pub fn ceiling(&self) -> i64 {
        self.ceiling
    }
    pub fn all_matching(&self) -> bool {
        self.all
    }
    pub fn override_count(&self) -> usize {
        self.overrides.len()
    }
    pub fn has_range(&self) -> bool {
        self.range.is_some()
    }
    pub fn is_empty(&self) -> bool {
        !self.all && self.range.is_none() && !self.overrides.values().any(|(_, value)| *value)
    }
    pub fn clear(&mut self) {
        self.all = false;
        self.range = None;
        self.overrides.clear();
        self.anchor = None;
    }
    pub fn select_all(&mut self) {
        self.clear();
        self.all = true;
    }
    fn cursor(&self, row: &Row) -> Result<Cursor> {
        ensure!(
            row.id > 0 && row.id <= self.ceiling,
            "This event is outside the captured history selection"
        );
        let value = self
            .query
            .sort
            .map_or(Value::Integer(row.id), |(field, _)| field.value(row));
        ensure!(
            !matches!(&value, Value::Text(text) if text.len() > MAX_KEY_BYTES),
            "History selection sort key exceeds 8 KiB"
        );
        Ok(Cursor { id: row.id, value })
    }
    fn in_range(&self, cursor: &Cursor) -> bool {
        self.range.as_ref().is_some_and(|(low, high)| {
            compare(low, cursor) != Ordering::Greater && compare(cursor, high) != Ordering::Greater
        })
    }
    /// The caller supplies a row from this captured query. Membership does not
    /// execute filters again on the UI thread or consult live storage.
    pub fn contains(&self, row: &Row) -> bool {
        let Ok(cursor) = self.cursor(row) else {
            return false;
        };
        self.overrides
            .get(&row.id)
            .map_or(self.all || self.in_range(&cursor), |(_, selected)| {
                *selected
            })
    }
    pub fn set(&mut self, row: &Row, selected: bool) -> Result<()> {
        let cursor = self.cursor(row)?;
        let mut next = self.clone();
        if selected == (next.all || next.in_range(&cursor)) {
            next.overrides.remove(&row.id);
        } else {
            next.overrides.insert(row.id, (cursor.clone(), selected));
        }
        next.anchor = Some(cursor);
        next.validate_size()?;
        *self = next;
        Ok(())
    }
    pub fn toggle(&mut self, row: &Row) -> Result<()> {
        self.set(row, !self.contains(row))
    }
    /// Shift replaces the active range while preserving separate manual marks.
    /// Range endpoints are sort-value/ID pairs, including ties and reverse order.
    pub fn extend_to(&mut self, row: &Row) -> Result<()> {
        let cursor = self.cursor(row)?;
        let Some(anchor) = self.anchor.clone() else {
            return self.set(row, true);
        };
        let (low, high) = if compare(&anchor, &cursor) == Ordering::Greater {
            (cursor, anchor)
        } else {
            (anchor, cursor)
        };
        let mut next = self.clone();
        next.all = false;
        next.overrides.retain(|_, (cursor, _)| {
            compare(&low, cursor) == Ordering::Greater
                || compare(cursor, &high) == Ordering::Greater
        });
        next.range = Some((low, high));
        next.validate_size()?;
        *self = next;
        Ok(())
    }
    fn validate_size(&self) -> Result<()> {
        let bytes = |cursor: &Cursor| {
            std::mem::size_of::<Cursor>()
                + match &cursor.value {
                    Value::Text(text) => text.capacity(),
                    _ => 0,
                }
        };
        let retained = self
            .overrides
            .values()
            .map(|(cursor, _)| bytes(cursor) + 64)
            .sum::<usize>()
            + self
                .range
                .as_ref()
                .map_or(0, |(low, high)| bytes(low) + bytes(high))
            + self.anchor.as_ref().map_or(0, bytes);
        ensure!(
            self.overrides.len() <= MAX_OVERRIDES && retained <= MAX_BYTES,
            "History selection is full; select a range or all matching events instead"
        );
        Ok(())
    }
    /// Explicit selected IDs must still be retained when a copy/export starts.
    /// Range/all membership otherwise covers the retained captured query.
    pub(crate) fn required_ids(&self) -> BTreeSet<i64> {
        self.overrides
            .iter()
            .filter_map(|(id, (_, selected))| selected.then_some(*id))
            .collect()
    }
    pub(crate) fn sql(&self) -> Result<(String, Vec<Value>)> {
        self.validate_size()?;
        let (mut sql, mut values) = query::sql(&self.query, self.ceiling, 0)?;
        let (sort, _) = query::sort_sql(&self.query);
        let mut included = vec![];
        if self.all {
            included.push("1".to_owned());
        }
        if let Some((low, high)) = &self.range {
            included.push(format!(
                "(({sort}>? OR ({sort}=? AND id>=?)) AND ({sort}<? OR ({sort}=? AND id<=?)))"
            ));
            values.extend([
                low.value.clone(),
                low.value.clone(),
                Value::Integer(low.id),
                high.value.clone(),
                high.value.clone(),
                Value::Integer(high.id),
            ]);
        }
        let yes = self
            .overrides
            .iter()
            .filter_map(|(id, (_, selected))| selected.then_some(*id))
            .collect::<Vec<_>>();
        let no = self
            .overrides
            .iter()
            .filter_map(|(id, (_, selected))| (!selected).then_some(*id))
            .collect::<Vec<_>>();
        if !yes.is_empty() {
            included.push(format!("id IN ({})", vec!["?"; yes.len()].join(",")));
            values.extend(yes.into_iter().map(Value::Integer));
        }
        let mut membership = if included.is_empty() {
            "0".into()
        } else {
            format!("({})", included.join(" OR "))
        };
        if !no.is_empty() {
            membership.push_str(&format!(
                " AND id NOT IN ({})",
                vec!["?"; no.len()].join(",")
            ));
            values.extend(no.into_iter().map(Value::Integer));
        }
        let insertion = sql
            .rfind(" ORDER BY ")
            .expect("fixed history query order clause");
        sql.insert_str(insertion, &format!(" AND ({membership})"));
        Ok((
            sql.strip_suffix(" LIMIT 0")
                .expect("fixed history query limit")
                .to_owned(),
            values,
        ))
    }
}
fn compare(left: &Cursor, right: &Cursor) -> Ordering {
    let order = match (&left.value, &right.value) {
        (Value::Integer(left), Value::Integer(right)) => left.cmp(right),
        (Value::Text(left), Value::Text(right)) => left.cmp(right),
        _ => unreachable!("a selection uses one fixed typed sort"),
    };
    order.then_with(|| left.id.cmp(&right.id))
}

impl Field {
    /// Canonical value of the same SQL sort expression, before display formatting.
    /// In particular yes/no, RFC3339 strings and missing numeric values must not
    /// be used as selection range boundaries for numeric SQLite ordering.
    pub fn value(self, row: &Row) -> Value {
        let m = &row.metadata;
        let number = match self {
            Self::Time => Some(row.unixnano),
            Self::Port => Some(row.port),
            Self::SourcePort => m.source_port.map(i64::from),
            Self::User => m.user.map(i64::from),
            Self::Pid => m.pid.map(i64::from),
            Self::Logged => m.logged.map(i64::from),
            Self::Priority => m.priority.map(i64::from),
            Self::Truncated => m.available.then_some(i64::from(m.truncated)),
            _ => return Value::Text(self.text(row)),
        };
        Value::Integer(number.unwrap_or(-1))
    }
}
