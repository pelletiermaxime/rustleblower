//! Typed, bound-parameter history filters shared by pages and exports.
mod formula;
use super::Row;
use anyhow::{Result, ensure};
pub use formula::Formula;
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};

/// Bounded cached identities, including nodes whose daemon is now offline.
/// Call on a background executor after the history writer's fence.
pub fn node_choices(database: &std::path::Path) -> Result<Vec<String>> {
    let db = rusqlite::Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    db.busy_timeout(std::time::Duration::from_millis(250))?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-512; PRAGMA mmap_size=0;")?;
    let mut statement = db.prepare("SELECT node FROM events WHERE node<>'' UNION SELECT name FROM node_info WHERE name<>'' ORDER BY 1 LIMIT 33")?;
    let mut names = Vec::new();
    for row in statement.query_map([], |row| row.get::<_, String>(0))? {
        let name = row?;
        ensure!(name.len() <= 128, "History node exceeds 128 bytes");
        names.push(name);
    }
    ensure!(
        names.len() <= crate::nodes::MAX_NODES,
        "Cached node choices are unavailable"
    );
    Ok(names)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Time,
    Node,
    Action,
    SourcePort,
    SourceIp,
    DestinationIp,
    Host,
    Port,
    Protocol,
    User,
    Pid,
    Process,
    Command,
    Cwd,
    Rule,
    Duration,
    Logged,
    Priority,
    Reason,
    Truncated,
}
pub const FIELDS: &[Field] = &[
    Field::Time,
    Field::Node,
    Field::Action,
    Field::SourcePort,
    Field::SourceIp,
    Field::DestinationIp,
    Field::Host,
    Field::Port,
    Field::Protocol,
    Field::User,
    Field::Pid,
    Field::Process,
    Field::Command,
    Field::Cwd,
    Field::Rule,
    Field::Duration,
    Field::Logged,
    Field::Priority,
    Field::Reason,
    Field::Truncated,
];
impl Field {
    pub fn key(self) -> String {
        serde_json::to_value(self).unwrap().as_str().unwrap().into()
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Time => "Time (UTC)",
            Self::Node => "Node",
            Self::Action => "Action",
            Self::SourcePort => "Source port",
            Self::SourceIp => "Source IP",
            Self::DestinationIp => "Destination IP",
            Self::Host => "Destination host",
            Self::Port => "Destination port",
            Self::Protocol => "Protocol",
            Self::User => "UID",
            Self::Pid => "PID",
            Self::Process => "Executable",
            Self::Command => "Command line",
            Self::Cwd => "Working directory",
            Self::Rule => "Matched rule",
            Self::Duration => "Rule duration",
            Self::Logged => "Logging",
            Self::Priority => "Priority",
            Self::Reason => "Event reason",
            Self::Truncated => "Snapshot shortened",
        }
    }
    pub fn needs_metadata(self) -> bool {
        !matches!(
            self,
            Self::Time
                | Self::Node
                | Self::Action
                | Self::Port
                | Self::Protocol
                | Self::Process
                | Self::Reason
        )
    }
    pub fn numeric(self) -> bool {
        matches!(
            self,
            Self::Time
                | Self::SourcePort
                | Self::Port
                | Self::User
                | Self::Pid
                | Self::Logged
                | Self::Priority
                | Self::Truncated
        )
    }
    pub fn sql(self) -> &'static str {
        match self {
            Self::Time => "unixnano",
            Self::Node => "node",
            Self::Action => "action",
            Self::Port => "port",
            Self::Protocol => "protocol",
            Self::Process => "process",
            Self::Reason => "reason",
            Self::SourcePort => "json_extract(metadata,'$.source_port')",
            Self::SourceIp => "json_extract(metadata,'$.source')",
            Self::DestinationIp => "json_extract(metadata,'$.destination_ip')",
            Self::Host => "json_extract(metadata,'$.host')",
            Self::User => "json_extract(metadata,'$.user')",
            Self::Pid => "json_extract(metadata,'$.pid')",
            Self::Command => "json_extract(metadata,'$.command')",
            Self::Cwd => "nullif(json_extract(metadata,'$.cwd'),'')",
            Self::Rule => "json_extract(metadata,'$.rule')",
            Self::Duration => "json_extract(metadata,'$.duration')",
            Self::Logged => "json_extract(metadata,'$.logged')",
            Self::Priority => "json_extract(metadata,'$.priority')",
            Self::Truncated => {
                "CASE WHEN json_extract(metadata,'$.available') THEN json_extract(metadata,'$.truncated') END"
            }
        }
    }
    pub fn text(self, row: &Row) -> String {
        let m = &row.metadata;
        match self {
            Self::Time => chrono::DateTime::from_timestamp_nanos(row.unixnano)
                .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
            Self::Node => row.node.clone(),
            Self::Action => row.action.clone(),
            Self::Port => row.port.to_string(),
            Self::Protocol => row.protocol.clone(),
            Self::Process => row.process.clone(),
            Self::Reason => row.reason.clone(),
            Self::SourcePort => m.source_port.map(|n| n.to_string()).unwrap_or_default(),
            Self::SourceIp => m.source.clone(),
            Self::DestinationIp => m.destination_ip.clone(),
            Self::Host => m.host.clone(),
            Self::User => m.user.map(|n| n.to_string()).unwrap_or_default(),
            Self::Pid => m.pid.map(|n| n.to_string()).unwrap_or_default(),
            Self::Command => m.command.clone(),
            Self::Cwd => m.cwd.clone(),
            Self::Rule => m.rule.clone(),
            Self::Duration => m.duration.clone(),
            Self::Logged => m
                .logged
                .map(|v| if v { "yes" } else { "no" }.into())
                .unwrap_or_default(),
            Self::Priority => m
                .priority
                .map(|v| if v { "yes" } else { "no" }.into())
                .unwrap_or_default(),
            Self::Truncated => if !m.available {
                ""
            } else if m.truncated {
                "yes"
            } else {
                "no"
            }
            .into(),
        }
    }
    pub fn width(self) -> f32 {
        match self {
            Self::Process | Self::Command | Self::Cwd => 280.,
            Self::Time => 240.,
            Self::SourceIp | Self::DestinationIp | Self::Host | Self::Rule => 200.,
            _ => 120.,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    Equals,
    NotEquals,
    Exact,
    Before,
    After,
    AtMost,
    AtLeast,
    Missing,
    Present,
}
pub const COMPARISONS: &[(Comparison, &str)] = &[
    (Comparison::Contains, "Contains"),
    (Comparison::NotContains, "Does not contain"),
    (Comparison::StartsWith, "Starts with"),
    (Comparison::EndsWith, "Ends with"),
    (Comparison::Equals, "Equals"),
    (Comparison::Exact, "Exact match"),
    (Comparison::NotEquals, "Does not equal"),
    (Comparison::Before, "Less than / before"),
    (Comparison::After, "Greater than / after"),
    (Comparison::AtMost, "At most / at or before"),
    (Comparison::AtLeast, "At least / at or after"),
    (Comparison::Missing, "Not reported"),
    (Comparison::Present, "Reported"),
];
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub field: Field,
    pub comparison: Comparison,
    pub value: String,
}
impl Filter {
    pub fn validate(&self) -> Result<()> {
        self.value()?;
        Ok(())
    }
    fn value(&self) -> Result<Value> {
        ensure!(
            self.value.len() <= 256,
            "Filter values are limited to 256 bytes"
        );
        if matches!(self.comparison, Comparison::Missing | Comparison::Present) {
            return Ok(Value::Null);
        }
        if self.field.numeric()
            && !matches!(
                self.comparison,
                Comparison::Contains
                    | Comparison::NotContains
                    | Comparison::StartsWith
                    | Comparison::EndsWith
            )
        {
            let n = if self.field == Field::Time {
                self.value
                    .parse::<i64>()
                    .ok()
                    .or_else(|| {
                        chrono::DateTime::parse_from_rfc3339(&self.value)
                            .ok()?
                            .timestamp_nanos_opt()
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("Enter a UTC RFC3339 time or integer Unix nanoseconds")
                    })?
            } else {
                self.value.parse::<i64>().map_err(|_| {
                    anyhow::anyhow!("Enter a nonnegative integer; boolean fields use 0 or 1")
                })?
            };
            if self.field != Field::Time {
                ensure!(
                    (0..=u32::MAX as i64).contains(&n),
                    "Numeric filter is out of range"
                );
            }
            if matches!(self.field, Field::Port | Field::SourcePort) {
                ensure!(n <= 65535, "Ports must be between 0 and 65535");
            }
            if matches!(
                self.field,
                Field::Logged | Field::Priority | Field::Truncated
            ) {
                ensure!(n <= 1, "Boolean filters use 0 or 1");
            }
            Ok(Value::Integer(n))
        } else {
            ensure!(
                !self.value.is_empty(),
                "Enter a filter value or choose Not reported"
            );
            Ok(Value::Text(self.value.clone()))
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Match {
    #[default]
    All,
    Any,
}
impl Match {
    fn sql(self) -> &'static str {
        match self {
            Self::All => " AND ",
            Self::Any => " OR ",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All (AND)",
            Self::Any => "Any (OR)",
        }
    }
    pub fn toggled(self) -> Self {
        match self {
            Self::All => Self::Any,
            Self::Any => Self::All,
        }
    }
}
/// A group consumes the next `count` conditions in their displayed order.
/// Counts partition the existing filter vector: no recursion, references or
/// duplicated leaves can inflate the established eight-condition budget.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub count: u8,
    pub mode: Match,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Logic {
    /// Optional bounded condition-number expression. Empty keeps the original
    /// group controls and legacy all/any behavior.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub formula: String,
    #[serde(default)]
    pub mode: Match,
    #[serde(default)]
    pub groups: Vec<Group>,
}
impl Logic {
    pub fn validate(&self, conditions: usize) -> Result<()> {
        ensure!(
            self.formula.len() <= 256,
            "A formula is limited to 256 bytes"
        );
        if !self.formula.trim().is_empty() {
            Formula::parse(&self.formula, conditions)?;
        }
        ensure!(
            conditions <= 8 && self.groups.len() <= 8,
            "Use at most eight conditions and groups"
        );
        if self.groups.is_empty() {
            return Ok(());
        }
        ensure!(
            self.groups.iter().all(|g| g.count > 0),
            "Condition groups must not be empty"
        );
        ensure!(
            self.groups
                .iter()
                .map(|g| usize::from(g.count))
                .sum::<usize>()
                == conditions,
            "Condition groups must partition all conditions exactly once"
        );
        Ok(())
    }
    pub fn normalized(mut self, conditions: usize) -> Self {
        if self.groups.is_empty() && conditions > 0 && conditions <= 8 {
            self.groups.push(Group {
                count: conditions as u8,
                mode: self.mode,
            });
        }
        self
    }
    pub fn append(&mut self, new_group: bool, existing: usize) {
        self.formula.clear();
        *self = self.clone().normalized(existing);
        if new_group || self.groups.is_empty() {
            self.groups.push(Group {
                count: 1,
                mode: Match::All,
            });
        } else if let Some(group) = self.groups.last_mut() {
            group.count += 1;
        }
    }
    pub fn remove(&mut self, index: usize, existing: usize) {
        self.formula.clear();
        *self = self.clone().normalized(existing);
        let mut start = 0;
        for group in &mut self.groups {
            if (start..start + usize::from(group.count)).contains(&index) {
                group.count -= 1;
                break;
            }
            start += usize::from(group.count);
        }
        self.groups.retain(|g| g.count > 0);
    }
}

fn filter_sql(filter: &Filter, values: &mut Vec<Value>) -> Result<String> {
    let column = filter.field.sql();
    let value = filter.value()?;
    let result = match filter.comparison {
        Comparison::Missing => format!("({column} IS NULL OR CAST({column} AS TEXT)='')"),
        Comparison::Present => format!("({column} IS NOT NULL AND CAST({column} AS TEXT)!='')"),
        Comparison::Contains | Comparison::NotContains | Comparison::StartsWith => {
            let suffix = match filter.comparison {
                Comparison::Contains => ">0",
                Comparison::NotContains => "=0",
                _ => "=1",
            };
            values.push(value);
            format!("instr(lower(CAST({column} AS TEXT)),lower(?)){suffix}")
        }
        Comparison::EndsWith => {
            values.extend([value.clone(), value]);
            format!("substr(lower(CAST({column} AS TEXT)),-length(?))=lower(?)")
        }
        comparison => {
            let op = match comparison {
                Comparison::Equals | Comparison::Exact => "=",
                Comparison::NotEquals => "!=",
                Comparison::Before => "<",
                Comparison::After => ">",
                Comparison::AtMost => "<=",
                Comparison::AtLeast => ">=",
                _ => unreachable!(),
            };
            values.push(value);
            if filter.field.numeric() || comparison == Comparison::Exact {
                format!("{column}{op}?")
            } else {
                format!("lower({column}){op}lower(?)")
            }
        }
    };
    Ok(result)
}

#[derive(Clone, Debug)]
pub struct Cursor {
    pub id: i64,
    pub value: Value,
}

pub(super) fn sql(
    query: &super::Query,
    ceiling: i64,
    limit: usize,
) -> Result<(String, Vec<Value>)> {
    ensure!(query.node.len() <= 128, "History node exceeds 128 bytes");
    ensure!(
        query.search.len() <= 256 && query.filters.len() <= 8,
        "Use at most eight filters and 256 bytes of search text"
    );
    query.logic.validate(query.filters.len())?;
    ensure!(
        matches!(query.action.as_str(), "" | "allow" | "deny" | "reject"),
        "Unsupported history action"
    );
    let mut terms = vec!["id<=?".to_string()];
    let mut values = vec![Value::Integer(ceiling)];
    if !query.node.is_empty() {
        terms.push("node=?".into());
        values.push(Value::Text(query.node.clone()));
    }
    if let Some(before) = query.before {
        terms.push("id<?".into());
        values.push(Value::Integer(before));
    }
    if !query.action.is_empty() {
        terms.push("action=?".into());
        values.push(Value::Text(query.action.clone()));
    }
    if !query.search.is_empty() {
        terms.push(
            "(instr(lower(process),lower(?))>0 OR instr(lower(destination),lower(?))>0)".into(),
        );
        values.extend([
            Value::Text(query.search.clone()),
            Value::Text(query.search.clone()),
        ]);
    }
    if !query.logic.formula.trim().is_empty() {
        let formula = Formula::parse(&query.logic.formula, query.filters.len())?;
        terms.push(formula.sql(&mut |index| filter_sql(&query.filters[index], &mut values))?);
    } else if query.logic.groups.is_empty() {
        if !query.filters.is_empty() {
            let parts = query
                .filters
                .iter()
                .map(|filter| filter_sql(filter, &mut values))
                .collect::<Result<Vec<_>>>()?;
            terms.push(format!("({})", parts.join(query.logic.mode.sql())));
        }
    } else {
        let mut start = 0;
        let mut groups = Vec::new();
        for group in &query.logic.groups {
            let end = start + usize::from(group.count);
            let parts = query.filters[start..end]
                .iter()
                .map(|f| filter_sql(f, &mut values))
                .collect::<Result<Vec<_>>>()?;
            groups.push(format!("({})", parts.join(group.mode.sql())));
            start = end;
        }
        terms.push(format!("({})", groups.join(query.logic.mode.sql())));
    }
    let (sort, ascending) = sort_sql(query);
    if let Some(cursor) = &query.cursor {
        let op = if ascending { ">" } else { "<" };
        terms.push(format!("({sort}{op}? OR ({sort}=? AND id{op}?))"));
        values.extend([
            cursor.value.clone(),
            cursor.value.clone(),
            Value::Integer(cursor.id),
        ]);
    }
    let direction = if ascending { "ASC" } else { "DESC" };
    Ok((
        format!(
            "SELECT id,unixnano,action,protocol,process,destination,port,reason,node,coalesce(metadata,'{{}}'),{sort} FROM events WHERE {} ORDER BY {sort} {direction},id {direction} LIMIT {limit}",
            terms.join(" AND ")
        ),
        values,
    ))
}

pub(super) fn sort_sql(query: &super::Query) -> (String, bool) {
    query
        .sort
        .map_or(("id".to_string(), false), |(field, ascending)| {
            (
                format!(
                    "coalesce({}, {})",
                    field.sql(),
                    if field.numeric() { "-1" } else { "''" }
                ),
                ascending,
            )
        })
}
