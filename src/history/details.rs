//! On-demand historical metadata. Live environment/checksum/process inspection
//! is deliberately separate; no unbounded connection payload is retained here.
use super::{Row, bounded};
use crate::protocol::{Connection, Event, Operator, Rule};
use prost::Message;

pub const MAX_DETAIL_BYTES: usize = 24 * 1024;
pub const PATH: u32 = 1;
pub const CWD: u32 = 2;
pub const ARGUMENTS: u32 = 4;
pub const DESTINATION: u32 = 8;
pub const SOURCE: u32 = 16;
pub const MATCHED_RULE: u32 = 32;
pub const TIME: u32 = 64;

#[derive(Clone, Debug)]
pub struct Detail {
    pub row: Row,
    /// Missing for records written before schema 2.
    pub event: Option<Event>,
    pub truncated: u32,
}
#[derive(Clone, Copy)]
pub enum DraftKind {
    Application,
    Destination,
}

pub fn endpoint(address: &str, port: u32) -> String {
    if address.is_empty() {
        return if port == 0 {
            "Not reported".into()
        } else {
            format!("Port {port} · address not reported")
        };
    }
    if address.contains(':') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

fn trim(value: &mut String, max: usize, flag: u32, flags: &mut u32) {
    if value.len() > max {
        *flags |= flag;
        *value = bounded(value, max);
    }
}
pub(super) fn encode(mut event: Event) -> (Vec<u8>, u32) {
    let mut flags = 0;
    trim(&mut event.time, 128, TIME, &mut flags);
    if let Some(con) = event.connection.as_mut() {
        trim(&mut con.protocol, 32, DESTINATION, &mut flags);
        trim(&mut con.src_ip, 64, SOURCE, &mut flags);
        trim(&mut con.dst_ip, 64, DESTINATION, &mut flags);
        trim(&mut con.dst_host, 1024, DESTINATION, &mut flags);
        trim(&mut con.process_path, 4096, PATH, &mut flags);
        trim(&mut con.process_cwd, 4096, CWD, &mut flags);
        let mut bytes = 0;
        let mut args = Vec::new();
        for mut arg in std::mem::take(&mut con.process_args) {
            if args.len() >= 64 || bytes >= 8192 {
                flags |= ARGUMENTS;
                break;
            }
            trim(&mut arg, 1024.min(8192 - bytes), ARGUMENTS, &mut flags);
            bytes += arg.len();
            args.push(arg);
        }
        con.process_args = args;
        con.process_env.clear();
        con.process_checksums.clear();
        con.process_tree.clear();
    }
    if let Some(rule) = event.rule.as_mut() {
        trim(&mut rule.name, 1024, MATCHED_RULE, &mut flags);
        trim(&mut rule.action, 32, MATCHED_RULE, &mut flags);
        trim(&mut rule.duration, 64, MATCHED_RULE, &mut flags);
        rule.description.clear();
        rule.operator = None;
    }
    let bytes = event.encode_to_vec();
    debug_assert!(bytes.len() <= MAX_DETAIL_BYTES);
    (bytes, flags)
}

impl Detail {
    pub fn connection(&self) -> Option<&Connection> {
        self.event.as_ref()?.connection.as_ref()
    }
    pub fn matched_rule(&self) -> Option<&Rule> {
        self.event.as_ref()?.rule.as_ref()
    }
    pub fn omissions(&self) -> Vec<&'static str> {
        [
            (PATH, "executable path"),
            (CWD, "working directory"),
            (ARGUMENTS, "arguments"),
            (DESTINATION, "destination/protocol"),
            (SOURCE, "source address"),
            (MATCHED_RULE, "matched rule"),
            (TIME, "reported time"),
        ]
        .into_iter()
        .filter_map(|(flag, label)| (self.truncated & flag != 0).then_some(label))
        .collect()
    }
    pub fn draft(&self, kind: DraftKind) -> Result<Rule, String> {
        let con = self.connection().ok_or(
            "This older event has no detailed connection snapshot; create a rule manually",
        )?;
        if con.process_path.is_empty() || self.truncated & PATH != 0 {
            return Err("A complete executable path is needed to create this rule".into());
        }
        let mut conditions = vec![Operator {
            r#type: "simple".into(),
            operand: "process.path".into(),
            data: con.process_path.clone(),
            sensitive: true,
            ..Default::default()
        }];
        if matches!(kind, DraftKind::Destination) {
            if self.truncated & DESTINATION != 0 {
                return Err(
                    "The recorded destination was truncated; create its condition manually".into(),
                );
            }
            let (field, value) = if con.dst_host.is_empty() {
                ("dest.ip", &con.dst_ip)
            } else {
                ("dest.host", &con.dst_host)
            };
            if value.is_empty() {
                return Err("The connection has no recorded destination".into());
            }
            conditions.push(Operator {
                r#type: "simple".into(),
                operand: field.into(),
                data: value.clone(),
                sensitive: true,
                ..Default::default()
            });
            if con.dst_port > 0 {
                conditions.push(Operator {
                    r#type: "simple".into(),
                    operand: "dest.port".into(),
                    data: con.dst_port.to_string(),
                    sensitive: true,
                    ..Default::default()
                });
            }
            if !con.protocol.is_empty() {
                conditions.push(Operator {
                    r#type: "simple".into(),
                    operand: "protocol".into(),
                    data: con.protocol.clone(),
                    sensitive: true,
                    ..Default::default()
                });
            }
        }
        let operator = if conditions.len() == 1 {
            conditions.remove(0)
        } else {
            Operator {
                r#type: "list".into(),
                operand: "list".into(),
                list: conditions,
                sensitive: true,
                ..Default::default()
            }
        };
        Ok(Rule {
            name: format!(
                "rustleblower.history.{}.{}",
                self.row.id,
                super::now_nanos()
            ),
            description: format!(
                "Created from recorded connection {} on {}",
                self.row.id, self.row.node
            ),
            created: super::now_seconds(),
            enabled: true,
            action: "allow".into(),
            duration: "always".into(),
            operator: Some(operator),
            ..Default::default()
        })
    }
}
