//! One finite presentation profile per category, never per node/process/host.
use super::Sort;
use crate::history::{
    grouped,
    query::{FIELDS, Field},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DetailKind {
    Node,
    Rule,
    Host,
    Application,
    Address,
    Port,
    User,
    Protocol,
}
pub const DETAIL_KINDS: [DetailKind; 8] = [
    DetailKind::Node,
    DetailKind::Rule,
    DetailKind::Host,
    DetailKind::Application,
    DetailKind::Address,
    DetailKind::Port,
    DetailKind::User,
    DetailKind::Protocol,
];
impl DetailKind {
    pub fn from_grouped(kind: grouped::Kind) -> Self {
        match kind {
            grouped::Kind::Node => Self::Node,
            grouped::Kind::Rule => Self::Rule,
            grouped::Kind::Host => Self::Host,
            grouped::Kind::Application => Self::Application,
            grouped::Kind::Address => Self::Address,
            grouped::Kind::Port => Self::Port,
            grouped::Kind::User => Self::User,
            grouped::Kind::Protocol => Self::Protocol,
        }
    }
    pub fn grouped(self) -> grouped::Kind {
        match self {
            Self::Node => grouped::Kind::Node,
            Self::Rule => grouped::Kind::Rule,
            Self::Host => grouped::Kind::Host,
            Self::Application => grouped::Kind::Application,
            Self::Address => grouped::Kind::Address,
            Self::Port => grouped::Kind::Port,
            Self::User => grouped::Kind::User,
            Self::Protocol => grouped::Kind::Protocol,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DetailColumn {
    Hits,
    Event(Field),
}
impl DetailColumn {
    pub fn grouped(self) -> grouped::Column {
        match self {
            Self::Hits => grouped::Column::Hits,
            Self::Event(field) => grouped::Column::Event(field),
        }
    }
    pub fn from_grouped(column: grouped::Column) -> Self {
        match column {
            grouped::Column::Hits => Self::Hits,
            grouped::Column::Event(field) => Self::Event(field),
        }
    }
    pub fn key(self) -> String {
        match self {
            Self::Hits => "hits".into(),
            Self::Event(field) => field.key(),
        }
    }
    pub fn width(self) -> u16 {
        match self {
            Self::Hits => 130,
            Self::Event(field) => field.width().clamp(40., 2000.) as u16,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailWidth {
    pub column: DetailColumn,
    pub width: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailSettings {
    pub kind: DetailKind,
    pub search: String,
    pub sort: Sort<DetailColumn>,
    pub columns: Vec<DetailWidth>,
}
impl DetailSettings {
    pub fn new(kind: grouped::Kind) -> Self {
        use Field::*;
        let fields = [
            Time,
            Node,
            Action,
            SourceIp,
            SourcePort,
            DestinationIp,
            Host,
            Port,
            Protocol,
            User,
            Pid,
            Process,
            Command,
            Rule,
            Reason,
        ];
        let mut columns = vec![DetailWidth {
            column: DetailColumn::Hits,
            width: 130,
        }];
        columns.extend(fields.into_iter().map(|field| {
            let column = DetailColumn::Event(field);
            DetailWidth {
                column,
                width: column.width(),
            }
        }));
        Self {
            kind: DetailKind::from_grouped(kind),
            search: String::new(),
            sort: Sort {
                field: DetailColumn::Hits,
                ascending: false,
            },
            columns,
        }
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        use anyhow::ensure;
        ensure!(self.search.len() <= 256, "Detail search exceeds 256 bytes");
        ensure!(
            !self.columns.is_empty() && self.columns.len() <= FIELDS.len() + 1,
            "Choose 1–21 detail columns"
        );
        let mut seen = std::collections::BTreeSet::new();
        ensure!(
            self.columns
                .iter()
                .all(|column| (40..=2000).contains(&column.width)
                    && seen.insert(column.column.key())),
            "Invalid or duplicate detail columns"
        );
        Ok(())
    }
}
