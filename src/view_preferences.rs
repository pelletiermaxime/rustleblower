//! Finite local presentation state for native Nodes, Rules and Alerts tables.
mod details;
mod store;
pub use details::{DETAIL_KINDS, DetailColumn, DetailKind, DetailSettings, DetailWidth};
pub const MAX_DOCUMENT_BYTES: usize = 32 * 1024;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub use store::{Command, Handle, Snapshot, Worker};
macro_rules! fields {($name:ident,$constant:ident,[$($variant:ident => ($key:literal,$label:literal,$width:literal)),+ $(,)?])=>{
#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord,Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum $name {$($variant),+}
pub const $constant:&[$name]=&[$($name::$variant),+];
impl $name {pub fn key(self)-> &'static str {match self {$($name::$variant=>$key),+}}pub fn label(self)-> &'static str {match self {$($name::$variant=>$label),+}}pub fn width(self)->u16{match self {$($name::$variant=>$width),+}}}
}}
fields!(NodeField,NODE_FIELDS,[Name=>("name","Node",200),Version=>("version","Daemon version",160),Status=>("status","Connection",150),Running=>("interception","Interception",150),LogLevel=>("log-level","Log level",100),Updated=>("updated","Updated",180)]);
fields!(RuleField,RULE_FIELDS,[Name=>("name","Name",240),Action=>("action","Action",100),Enabled=>("enabled","Enabled",100),Duration=>("duration","Duration",140),Created=>("created","Created",180),Received=>("received","Received",180),Priority=>("priority","Priority",100),Logging=>("logging","Logging",100),Process=>("process","Process",280),Description=>("description","Description",280)]);
fields!(AlertField,ALERT_FIELDS,[Received=>("time","Received",180),Node=>("node","Node",180),Severity=>("severity","Severity",110),Priority=>("priority","Priority",100),Cause=>("cause","Cause",120),Action=>("action","Action",100),Kind=>("kind","Kind",120),Summary=>("summary","Summary",360),Read=>("read","Read",80),SourceId=>("source-id","Source ID",180)]);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub key: String,
    pub width: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sort<F> {
    pub field: F,
    pub ascending: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSettings {
    pub search: String,
    pub sort: Sort<NodeField>,
    pub columns: Vec<Column>,
}
impl Default for NodeSettings {
    fn default() -> Self {
        Self {
            search: String::new(),
            sort: Sort {
                field: NodeField::Name,
                ascending: true,
            },
            columns: NODE_FIELDS
                .iter()
                .map(|f| Column {
                    key: f.key().into(),
                    width: f.width(),
                })
                .collect(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSettings {
    pub search: String,
    pub temporary: Option<bool>,
    pub filters: Vec<crate::rules::query::Filter>,
    pub logic: crate::history::query::Logic,
    pub sort: Sort<RuleField>,
    pub columns: Vec<Column>,
}
impl Default for RuleSettings {
    fn default() -> Self {
        let mut columns = RULE_FIELDS
            .iter()
            .map(|f| Column {
                key: f.key().into(),
                width: f.width(),
            })
            .collect::<Vec<_>>();
        columns.push(Column {
            key: "cache_status".into(),
            width: 190,
        });
        Self {
            search: String::new(),
            temporary: None,
            filters: Vec::new(),
            logic: Default::default(),
            sort: Sort {
                field: RuleField::Name,
                ascending: true,
            },
            columns,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertSettings {
    pub search: String,
    pub node: String,
    pub severity: Option<i32>,
    pub unread: bool,
    pub sort: Sort<AlertField>,
    pub columns: Vec<Column>,
}
impl Default for AlertSettings {
    fn default() -> Self {
        Self {
            search: String::new(),
            node: String::new(),
            severity: None,
            unread: false,
            sort: Sort {
                field: AlertField::Received,
                ascending: false,
            },
            columns: ALERT_FIELDS
                .iter()
                .map(|f| Column {
                    key: f.key().into(),
                    width: f.width(),
                })
                .collect(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub nodes: NodeSettings,
    pub rules: RuleSettings,
    pub alerts: AlertSettings,
    #[serde(default = "default_details")]
    pub details: Vec<DetailSettings>,
}
fn default_details() -> Vec<DetailSettings> {
    DETAIL_KINDS
        .into_iter()
        .map(|kind| DetailSettings::new(kind.grouped()))
        .collect()
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            nodes: Default::default(),
            rules: Default::default(),
            alerts: Default::default(),
            details: default_details(),
        }
    }
}
fn columns(columns: &[Column], keys: &[&str]) -> anyhow::Result<()> {
    use anyhow::ensure;
    ensure!(
        !columns.is_empty() && columns.len() <= keys.len(),
        "Choose at least one available table column"
    );
    let mut seen = std::collections::BTreeSet::new();
    ensure!(
        columns.iter().all(|c| c.key.len() <= 32
            && keys.contains(&c.key.as_str())
            && (40..=2000).contains(&c.width)
            && seen.insert(c.key.as_str())),
        "Invalid table column or width"
    );
    Ok(())
}
impl Settings {
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        ensure!(
            self.nodes.search.len() <= 256
                && self.rules.search.len() <= 256
                && self.alerts.search.len() <= 256
                && self.alerts.node.len() <= 128,
            "Saved view filters exceed their text limits"
        );
        ensure!(
            self.rules.filters.len() <= 8,
            "Use at most eight saved rule conditions"
        );
        for filter in &self.rules.filters {
            filter.validate()?;
        }
        self.rules.logic.validate(self.rules.filters.len())?;
        ensure!(
            self.alerts
                .severity
                .is_none_or(|value| (0..=2).contains(&value)),
            "Unsupported saved alert severity"
        );
        columns(
            &self.nodes.columns,
            &NODE_FIELDS.iter().map(|f| f.key()).collect::<Vec<_>>(),
        )?;
        let mut rule_keys = RULE_FIELDS.iter().map(|f| f.key()).collect::<Vec<_>>();
        rule_keys.push("cache_status");
        columns(&self.rules.columns, &rule_keys)?;
        columns(
            &self.alerts.columns,
            &ALERT_FIELDS.iter().map(|f| f.key()).collect::<Vec<_>>(),
        )?;
        ensure!(
            self.details.len() == DETAIL_KINDS.len(),
            "Use exactly eight detail presentation profiles"
        );
        let mut seen = std::collections::BTreeSet::new();
        for detail in &self.details {
            ensure!(
                seen.insert(detail.kind),
                "Duplicate detail presentation profile"
            );
            detail.validate()?;
        }
        ensure!(
            serde_json::to_vec(self)?.len() + 64 <= MAX_DOCUMENT_BYTES,
            "Saved table views exceed 32 KiB"
        );
        Ok(())
    }
    pub fn detail(&self, kind: crate::history::grouped::Kind) -> Option<&DetailSettings> {
        let kind = DetailKind::from_grouped(kind);
        self.details.iter().find(|profile| profile.kind == kind)
    }
}
pub fn path_for(database: &Path) -> PathBuf {
    let mut path = database.as_os_str().to_owned();
    path.push(".views.json");
    PathBuf::from(path)
}
