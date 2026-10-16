//! Typed, bound-parameter history filters shared by pages and exports.
use super::Row;
use anyhow::{Result, ensure};
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};

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
            Self::Process | Self::Command => 280.,
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
    Equals,
    NotEquals,
    Exact,
    Before,
    After,
    Missing,
    Present,
}
pub const COMPARISONS: &[(Comparison, &str)] = &[
    (Comparison::Contains, "Contains"),
    (Comparison::Equals, "Equals"),
    (Comparison::Exact, "Exact match"),
    (Comparison::NotEquals, "Does not equal"),
    (Comparison::Before, "Less than / before"),
    (Comparison::After, "Greater than / after"),
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
        if self.field.numeric() && self.comparison != Comparison::Contains {
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
    ensure!(
        query.search.len() <= 256 && query.filters.len() <= 8,
        "Use at most eight filters and 256 bytes of search text"
    );
    ensure!(
        matches!(query.action.as_str(), "" | "allow" | "deny" | "reject"),
        "Unsupported history action"
    );
    let mut terms = vec!["id<=?".to_string()];
    let mut values = vec![Value::Integer(ceiling)];
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
    for filter in &query.filters {
        let column = filter.field.sql();
        let value = filter.value()?;
        match filter.comparison {
            Comparison::Missing => {
                terms.push(format!("({column} IS NULL OR CAST({column} AS TEXT)='')"))
            }
            Comparison::Present => terms.push(format!(
                "({column} IS NOT NULL AND CAST({column} AS TEXT)!='')"
            )),
            Comparison::Contains => {
                terms.push(format!("instr(lower(CAST({column} AS TEXT)),lower(?))>0"));
                values.push(value);
            }
            comparison => {
                let op = match comparison {
                    Comparison::Equals | Comparison::Exact => "=",
                    Comparison::NotEquals => "!=",
                    Comparison::Before => "<",
                    Comparison::After => ">",
                    _ => unreachable!(),
                };
                if filter.field.numeric() || comparison == Comparison::Exact {
                    terms.push(format!("{column}{op}?"));
                } else {
                    terms.push(format!("lower({column}){op}lower(?)"));
                }
                values.push(value);
            }
        }
    }
    let (sort, ascending) = query
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
        });
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
