//! Bounded typed rule predicates shared by pages and every filtered export.
//! Filtering never rewrites a cached protobuf document or executes user text.
use crate::{
    history::query::{Comparison, Formula, Logic, Match},
    protocol::Rule,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Received,
    Created,
    Name,
    Description,
    Node,
    Enabled,
    Action,
    Nolog,
    Priority,
    Duration,
    OperatorType,
    Operand,
    Data,
    AnyOperand,
    AnyData,
    Process,
}
pub const FIELDS: &[Field] = &[
    Field::Received,
    Field::Created,
    Field::Name,
    Field::Description,
    Field::Node,
    Field::Enabled,
    Field::Action,
    Field::Nolog,
    Field::Priority,
    Field::Duration,
    Field::OperatorType,
    Field::Operand,
    Field::Data,
    Field::AnyOperand,
    Field::AnyData,
    Field::Process,
];
impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Self::Received => "Last received by UI (UTC)",
            Self::Created => "Created (UTC)",
            Self::Name => "Rule name",
            Self::Description => "Description",
            Self::Node => "Node",
            Self::Enabled => "Enabled",
            Self::Action => "Action",
            Self::Nolog => "Logging disabled",
            Self::Priority => "Priority",
            Self::Duration => "Duration",
            Self::OperatorType => "Root operator type",
            Self::Operand => "Root operand",
            Self::Data => "Root operand data",
            Self::AnyOperand => "Any operator operand",
            Self::AnyData => "Any operator data",
            Self::Process => "Executable",
        }
    }
    fn numeric(self) -> bool {
        matches!(
            self,
            Self::Received | Self::Created | Self::Enabled | Self::Nolog | Self::Priority
        )
    }
    fn text<'a>(
        self,
        node: &'a str,
        rule: &'a Rule,
        received: Option<i64>,
    ) -> Option<Cow<'a, str>> {
        let text = match self {
            Self::Received => return received.map(|time| time.to_string().into()),
            Self::Created => return Some(rule.created.to_string().into()),
            Self::Enabled => return Some(if rule.enabled { "1" } else { "0" }.into()),
            Self::Nolog => return Some(if rule.nolog { "1" } else { "0" }.into()),
            Self::Priority => return Some(if rule.precedence { "1" } else { "0" }.into()),
            Self::Name => rule.name.as_str(),
            Self::Description => rule.description.as_str(),
            Self::Node => node,
            Self::Action => rule.action.as_str(),
            Self::Duration => rule.duration.as_str(),
            Self::OperatorType => rule.operator.as_ref()?.r#type.as_str(),
            Self::Operand => rule.operator.as_ref()?.operand.as_str(),
            Self::Data => rule.operator.as_ref()?.data.as_str(),
            Self::Process => super::executable(rule).unwrap_or(""),
            Self::AnyOperand | Self::AnyData => {
                unreachable!("tree fields are matched without flattening")
            }
        };
        (!text.is_empty()).then_some(Cow::Borrowed(text))
    }
    fn number(self, rule: &Rule, received: Option<i64>) -> i64 {
        match self {
            Self::Received => received.expect("presence checked before numeric comparison"),
            Self::Created => rule.created,
            Self::Enabled => i64::from(rule.enabled),
            Self::Nolog => i64::from(rule.nolog),
            Self::Priority => i64::from(rule.precedence),
            _ => unreachable!(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub field: Field,
    pub comparison: Comparison,
    pub value: String,
}
impl Filter {
    pub fn validate(&self) -> Result<()> {
        Prepared::new(self).map(|_| ())
    }
}
enum Value {
    Text(String),
    Number(i64),
    None,
}
struct Prepared {
    field: Field,
    comparison: Comparison,
    value: Value,
}
impl Prepared {
    fn new(filter: &Filter) -> Result<Self> {
        ensure!(
            filter.value.len() <= 256,
            "Rule filter values are limited to 256 bytes"
        );
        let text = matches!(
            filter.comparison,
            Comparison::Contains
                | Comparison::NotContains
                | Comparison::StartsWith
                | Comparison::EndsWith
        );
        let value = if matches!(filter.comparison, Comparison::Missing | Comparison::Present) {
            Value::None
        } else if filter.field.numeric() && !text {
            let number = if matches!(filter.field, Field::Received | Field::Created) {
                filter
                    .value
                    .parse::<i64>()
                    .ok()
                    .or_else(|| {
                        let date = chrono::DateTime::parse_from_rfc3339(&filter.value).ok()?;
                        (date.timestamp_subsec_nanos() == 0).then_some(date.timestamp())
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("Enter integer Unix seconds or a whole-second RFC3339 date")
                    })?
            } else {
                let number = filter
                    .value
                    .parse::<i64>()
                    .map_err(|_| anyhow::anyhow!("Boolean rule filters use 0 or 1"))?;
                ensure!((0..=1).contains(&number), "Boolean rule filters use 0 or 1");
                number
            };
            Value::Number(number)
        } else {
            ensure!(
                !filter.value.is_empty(),
                "Enter a rule filter value or choose Not reported"
            );
            Value::Text(if filter.comparison == Comparison::Exact {
                filter.value.clone()
            } else {
                filter.value.to_ascii_lowercase()
            })
        };
        Ok(Self {
            field: filter.field,
            comparison: filter.comparison,
            value,
        })
    }
    fn matches(&self, node: &str, rule: &Rule, received: Option<i64>) -> bool {
        if matches!(self.field, Field::AnyOperand | Field::AnyData) {
            return self.matches_tree(rule);
        }
        self.matches_actual(self.field.text(node, rule, received), rule, received)
    }
    fn matches_tree(&self, rule: &Rule) -> bool {
        fn any(
            op: &crate::protocol::Operator,
            test: &impl Fn(&crate::protocol::Operator) -> bool,
        ) -> bool {
            test(op) || op.list.iter().any(|child| any(child, test))
        }
        let negative = matches!(
            self.comparison,
            Comparison::NotEquals | Comparison::NotContains
        );
        let matched = rule.operator.as_ref().is_some_and(|op| {
            any(op, &|op| {
                let text = if self.field == Field::AnyOperand {
                    &op.operand
                } else {
                    &op.data
                };
                if text.is_empty() {
                    return false;
                }
                match self.comparison {
                    Comparison::Missing | Comparison::Present => true,
                    _ => self.matches_actual(Some(Cow::Borrowed(text)), rule, None) != negative,
                }
            })
        });
        if negative || self.comparison == Comparison::Missing {
            !matched
        } else {
            matched
        }
    }
    fn matches_actual(
        &self,
        actual: Option<Cow<'_, str>>,
        rule: &Rule,
        received: Option<i64>,
    ) -> bool {
        match self.comparison {
            Comparison::Missing => return actual.is_none(),
            Comparison::Present => return actual.is_some(),
            _ => {}
        }
        let Some(actual) = actual else { return false };
        match &self.value {
            Value::None => false,
            Value::Number(value) => compare(
                self.field.number(rule, received).cmp(value),
                self.comparison,
            ),
            Value::Text(value) => {
                let actual = if self.comparison == Comparison::Exact {
                    actual
                } else {
                    Cow::Owned(actual.to_ascii_lowercase())
                };
                match self.comparison {
                    Comparison::Contains => actual.contains(value),
                    Comparison::NotContains => !actual.contains(value),
                    Comparison::StartsWith => actual.starts_with(value),
                    Comparison::EndsWith => actual.ends_with(value),
                    comparison => compare(actual.as_ref().cmp(value.as_str()), comparison),
                }
            }
        }
    }
}
fn compare(order: std::cmp::Ordering, comparison: Comparison) -> bool {
    use std::cmp::Ordering::*;
    match comparison {
        Comparison::Equals | Comparison::Exact => order == Equal,
        Comparison::NotEquals => order != Equal,
        Comparison::Before => order == Less,
        Comparison::After => order == Greater,
        Comparison::AtMost => order != Greater,
        Comparison::AtLeast => order != Less,
        _ => false,
    }
}

pub struct Predicate {
    search: String,
    action: String,
    temporary: Option<bool>,
    filters: Vec<Prepared>,
    logic: Logic,
    formula: Option<Formula>,
    policy: Option<super::catalogue_policy::Context>,
}
impl Predicate {
    pub fn new(query: &super::Query) -> Result<Self> {
        ensure!(
            matches!(query.action.as_str(), "" | "allow" | "deny" | "reject"),
            "Unsupported rule action filter"
        );
        ensure!(
            query.node.len() <= 128 && query.after.len() <= 1024,
            "Rule node or cursor exceeds its limit"
        );
        ensure!(
            query.search.len() <= 256,
            "Rule search is limited to 256 bytes"
        );
        query.logic.validate(query.filters.len())?;
        let filters = query
            .filters
            .iter()
            .map(Prepared::new)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            policy: query.policy,
            search: query.search.to_ascii_lowercase(),
            action: query.action.clone(),
            temporary: query.temporary,
            filters,
            logic: query.logic.clone(),
            formula: (!query.logic.formula.trim().is_empty())
                .then(|| Formula::parse(&query.logic.formula, query.filters.len()))
                .transpose()?,
        })
    }
    /// Match a wire document without inventing local receipt metadata.
    pub fn matches(&self, node: &str, rule: &Rule) -> bool {
        self.matches_received(node, rule, None)
    }
    pub fn matches_received(&self, node: &str, rule: &Rule, received: Option<i64>) -> bool {
        if !self.action.is_empty() && self.action != rule.action {
            return false;
        }
        if self
            .policy
            .is_some_and(|context| !context.settings.includes(&rule.duration))
        {
            return false;
        }
        if self
            .temporary
            .is_some_and(|temporary| (rule.duration != "always") != temporary)
        {
            return false;
        }
        if !self.search.is_empty()
            && ![
                rule.name.as_str(),
                rule.description.as_str(),
                super::executable(rule).unwrap_or(""),
            ]
            .iter()
            .any(|text| text.to_ascii_lowercase().contains(&self.search))
        {
            return false;
        }
        let values = self
            .filters
            .iter()
            .map(|filter| filter.matches(node, rule, received))
            .collect::<Vec<_>>();
        let combine = |mode: Match, values: &[bool]| match mode {
            Match::All => values.iter().all(|v| *v),
            Match::Any => values.iter().any(|v| *v),
        };
        if let Some(formula) = &self.formula {
            return formula.evaluate(&values).unwrap_or(false);
        }
        if values.is_empty() {
            return true;
        }
        if self.logic.groups.is_empty() {
            return combine(self.logic.mode, &values);
        }
        let mut start = 0;
        let groups = self
            .logic
            .groups
            .iter()
            .map(|group| {
                let end = start + usize::from(group.count);
                let value = combine(group.mode, &values[start..end]);
                start = end;
                value
            })
            .collect::<Vec<_>>();
        combine(self.logic.mode, &groups)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quick_action_filters_scope_all_pages_and_exports_with_other_conditions() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("rules.sqlite3");
        let mut db = rusqlite::Connection::open(&path)?;
        crate::rules::schema(&db)?;
        let rules = (0..660)
            .map(|i| Rule {
                name: format!("app.{i:04}"),
                action: ["allow", "deny", "reject"][i % 3].into(),
                duration: if i % 2 == 0 { "always" } else { "5m" }.into(),
                enabled: true,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        crate::rules::replace(&mut db, "node", &rules)?;
        for action in ["allow", "deny", "reject"] {
            let mut query = crate::rules::Query {
                node: "node".into(),
                action: action.into(),
                ..Default::default()
            };
            let first = crate::rules::query(&db, &query)?;
            assert_eq!(first.rules.len(), 200);
            assert!(first.has_more && first.rules.iter().all(|rule| rule.action == action));
            query.after = first.rules.last().unwrap().name.clone();
            let second = crate::rules::query(&db, &query)?;
            assert_eq!(second.rules.len(), 20);
            assert!(!second.has_more && second.rules.iter().all(|rule| rule.action == action));
            query.after.clear();
            query.temporary = Some(false);
            let page = crate::rules::query(&db, &query)?;
            assert_eq!(page.rules.len(), 110);
            assert!(page.rules.iter().all(|rule| rule.duration == "always"));
            let tx = db.transaction()?;
            let mut count = 0;
            crate::rules::transfer::visit_snapshot(
                &tx,
                &crate::rules::transfer::ExportScope { query, names: None },
                |rule, _| {
                    assert_eq!(rule.action, action);
                    assert_eq!(rule.duration, "always");
                    count += 1;
                    Ok(())
                },
            )?;
            assert_eq!(count, 110);
        }
        assert!(
            Predicate::new(&crate::rules::Query {
                action: "invalid".into(),
                ..Default::default()
            })
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn grouped_export_pages_keep_one_snapshot_while_later_matches_change() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("rules.sqlite3");
        let mut writer = rusqlite::Connection::open(&path)?;
        writer.execute_batch("PRAGMA journal_mode=WAL;")?;
        crate::rules::schema(&writer)?;
        let original = (0..430)
            .map(|index| Rule {
                created: 1_700_000_000 + index,
                name: format!("app.{index:04}"),
                enabled: true,
                action: "allow".into(),
                duration: "always".into(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        crate::rules::replace(&mut writer, "node", &original)?;
        let query = crate::rules::Query {
            node: "node".into(),
            filters: vec![
                Filter {
                    field: Field::Created,
                    comparison: Comparison::AtLeast,
                    value: "1700000100".into(),
                },
                Filter {
                    field: Field::Action,
                    comparison: Comparison::Equals,
                    value: "allow".into(),
                },
                Filter {
                    field: Field::Enabled,
                    comparison: Comparison::Equals,
                    value: "1".into(),
                },
            ],
            logic: Logic {
                formula: String::new(),
                mode: Match::All,
                groups: vec![
                    crate::history::query::Group {
                        count: 1,
                        mode: Match::All,
                    },
                    crate::history::query::Group {
                        count: 2,
                        mode: Match::Any,
                    },
                ],
            },
            ..Default::default()
        };
        let mut reader = rusqlite::Connection::open(&path)?;
        let snapshot = reader.transaction()?;
        let mut seen = Vec::new();
        crate::rules::transfer::visit_snapshot(
            &snapshot,
            &crate::rules::transfer::ExportScope {
                query: query.clone(),
                names: None,
            },
            |rule, _| {
                if seen.is_empty() {
                    let mut changed = original[305].clone();
                    changed.action = "deny".into();
                    changed.enabled = false;
                    crate::rules::apply(
                        &mut writer,
                        "node",
                        crate::protocol::Action::ChangeRule,
                        &changed,
                    )?;
                }
                seen.push(rule.clone());
                Ok(())
            },
        )?;
        assert_eq!(seen, original[100..]);
        let predicate = Predicate::new(&query)?;
        let changed =
            crate::rules::transfer::find_confirmed_rule(&path, "node", "app.0305")?.unwrap();
        assert!(!predicate.matches("node", &changed));
        Ok(())
    }
}
