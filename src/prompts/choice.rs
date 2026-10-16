//! Match choices are derived from the pending connection, never from a stale
//! popup's fields. A persistent reply uses the same daemon-owned AskRule path.
use crate::{
    config::Decision,
    protocol::{Connection, Operator, Rule},
};
use std::net::IpAddr;

pub const DURATIONS: &[(&str, &str)] = &[
    ("once", "Once"),
    ("30s", "30 seconds"),
    ("5m", "5 minutes"),
    ("15m", "15 minutes"),
    ("30m", "30 minutes"),
    ("1h", "1 hour"),
    ("12h", "12 hours"),
    ("until restart", "Until daemon restart"),
    ("always", "Forever"),
];
#[derive(Clone, Debug)]
pub struct Target {
    pub key: String,
    pub label: String,
    pub operators: Vec<Operator>,
}
#[derive(Clone, Debug)]
pub struct Choice {
    pub duration: String,
    pub target: String,
    /// Additional source-derived target keys form an AND group.
    pub constraints: Vec<String>,
    pub priority: bool,
    pub logging: bool,
}
impl Default for Choice {
    fn default() -> Self {
        Self {
            duration: "once".into(),
            target: "connection".into(),
            constraints: Vec::new(),
            priority: false,
            logging: true,
        }
    }
}
fn op(kind: &str, field: &str, data: impl Into<String>) -> Operator {
    Operator {
        r#type: kind.into(),
        operand: field.into(),
        data: data.into(),
        sensitive: true,
        ..Default::default()
    }
}
fn simple(field: &str, data: impl Into<String>) -> Operator {
    op("simple", field, data)
}
fn escape_regex(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if "\\.+*?()|[]{}^$".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}
fn command(connection: &Connection) -> Vec<Operator> {
    let mut ops = vec![simple("process.command", connection.process_args.join(" "))];
    let first = connection
        .process_args
        .first()
        .map(String::as_str)
        .unwrap_or_default();
    if !first.starts_with('/') || first.starts_with("/proc/") {
        ops.push(simple("process.path", connection.process_path.clone()));
    }
    ops
}
pub fn targets(c: &Connection) -> Vec<Target> {
    let mut result = Vec::new();
    let mut add = |key: String, label: String, operators: Vec<Operator>| {
        if result.len() < 128 {
            result.push(Target {
                key,
                label,
                operators,
            });
        }
    };
    let mut exact = vec![simple("user.id", c.user_id.to_string())];
    if !c.process_path.is_empty() && c.process_path.len() <= 4096 {
        exact.push(simple("process.path", c.process_path.clone()));
    } else if c.process_id > 0 {
        exact.push(simple("process.id", c.process_id.to_string()));
    }
    if !c.dst_ip.is_empty() && c.dst_ip.len() <= 64 {
        exact.push(simple("dest.ip", c.dst_ip.clone()));
    }
    if c.dst_port > 0 {
        exact.push(simple("dest.port", c.dst_port.to_string()));
    }
    if !c.protocol.is_empty() && c.protocol.len() <= 32 {
        exact.push(simple("protocol", c.protocol.clone()));
    }
    add(
        "connection".into(),
        "Executable + destination".into(),
        exact,
    );
    if !c.process_path.is_empty() && c.process_path.len() <= 4096 {
        add(
            "process.path".into(),
            "This executable".into(),
            vec![simple("process.path", c.process_path.clone())],
        );
        if let Some(path) = c.process_path.strip_prefix("/tmp/.mount_")
            && let Some((mount, relative)) = path.split_once('/')
            && mount.len() > 6
            && mount.is_char_boundary(mount.len() - 6)
            && mount.as_bytes()[mount.len() - 6..]
                .iter()
                .all(u8::is_ascii_alphanumeric)
            && !relative.is_empty()
        {
            let name = &mount[..mount.len() - 6];
            add(
                "appimage".into(),
                "This AppImage across temporary mounts".into(),
                vec![op(
                    "regexp",
                    "process.path",
                    format!(
                        r"^/tmp/\.mount_{}[0-9A-Za-z]{{6,7}}/{}$",
                        escape_regex(name),
                        escape_regex(relative)
                    ),
                )],
            );
        }
    }
    let args_bytes = c.process_args.iter().map(String::len).sum::<usize>()
        + c.process_args.len().saturating_sub(1);
    let first = c
        .process_args
        .first()
        .map(String::as_str)
        .unwrap_or_default();
    if !first.is_empty()
        && args_bytes <= 8192
        && ((first.starts_with('/') && !first.starts_with("/proc/"))
            || (!c.process_path.is_empty() && c.process_path.len() <= 4096))
    {
        add(
            "process.command".into(),
            "This command line".into(),
            command(c),
        );
    }
    if c.process_id > 0 {
        add(
            "process.id".into(),
            format!("Process ID {}", c.process_id),
            vec![simple("process.id", c.process_id.to_string())],
        );
    }
    add(
        "user.id".into(),
        format!("User ID {}", c.user_id),
        vec![simple("user.id", c.user_id.to_string())],
    );
    for (field, label, value, max) in [
        ("dest.ip", "Destination IP", &c.dst_ip, 64),
        ("dest.host", "Destination host", &c.dst_host, 1024),
        ("source.ip", "Source IP", &c.src_ip, 64),
        ("protocol", "Protocol", &c.protocol, 32),
    ] {
        if !value.is_empty() && value.len() <= max {
            add(
                field.into(),
                format!("{label}: {value}"),
                vec![simple(field, value.clone())],
            );
        }
    }
    for (field, label, value) in [
        ("dest.port", "Destination port", c.dst_port),
        ("source.port", "Source port", c.src_port),
    ] {
        if value > 0 {
            add(
                field.into(),
                format!("{label}: {value}"),
                vec![simple(field, value.to_string())],
            );
        }
    }
    if let Ok(ip) = c.dst_ip.parse::<IpAddr>() {
        let alias = match ip {
            IpAddr::V4(v) if v.is_private() || v.is_loopback() => Some("LAN"),
            IpAddr::V6(v) if v.is_unique_local() || v.is_loopback() => Some("LAN"),
            ip if ip.is_multicast() => Some("MULTICAST"),
            _ => None,
        };
        if let Some(alias) = alias
            && !c.process_path.is_empty()
            && c.process_path.len() <= 4096
        {
            add(
                format!("alias:{alias}"),
                format!("This executable to {alias}"),
                vec![
                    simple("process.path", c.process_path.clone()),
                    op("network", "dest.network", alias),
                ],
            );
        }
        let prefixes: &[u8] = if ip.is_ipv4() {
            &[24, 16, 8]
        } else {
            &[64, 128]
        };
        for prefix in prefixes {
            let network = match ip {
                IpAddr::V4(v) => {
                    IpAddr::V4((u32::from(v) & (u32::MAX << (32 - u32::from(*prefix)))).into())
                }
                IpAddr::V6(v) => {
                    IpAddr::V6((u128::from(v) & (u128::MAX << (128 - u32::from(*prefix)))).into())
                }
            };
            let cidr = format!("{network}/{prefix}");
            add(
                format!("network:{cidr}"),
                format!("Destination network: {cidr}"),
                vec![op("network", "dest.network", cidr)],
            );
        }
    }
    if !c.dst_host.is_empty() && c.dst_host.len() <= 1024 && c.dst_host.parse::<IpAddr>().is_err() {
        let parts = c
            .dst_host
            .trim_end_matches('.')
            .split('.')
            .collect::<Vec<_>>();
        for index in 1..parts.len().saturating_sub(1).min(16) {
            let suffix = parts[index..].join(".");
            add(
                format!("domain:{suffix}"),
                format!("Destination domain: *.{suffix}"),
                vec![op(
                    "regexp",
                    "dest.host",
                    format!("^(|.*\\.){}\\.?$", escape_regex(&suffix)),
                )],
            );
        }
    }
    for (algorithm, label) in [("md5", "Executable MD5"), ("sha1", "Executable SHA1")] {
        // OpenSnitch sends checksum map keys as process.hash.*; tolerate older
        // fixtures using the short algorithm name as well.
        let field = format!("process.hash.{algorithm}");
        if let Some(hash) = c
            .process_checksums
            .get(&field)
            .or_else(|| c.process_checksums.get(algorithm))
            .filter(|hash| !hash.is_empty() && hash.len() <= 128)
            && !c.process_path.is_empty()
            && c.process_path.len() <= 4096
        {
            add(
                field.clone(),
                label.into(),
                vec![
                    simple("process.path", c.process_path.clone()),
                    simple(&field, hash.clone()),
                ],
            );
        }
    }
    // Simple destination scopes always keep the executable and deliberately
    // omit ports. A reported hostname is an exact host, never a wildcard domain.
    if !c.process_path.is_empty() && c.process_path.len() <= 4096 {
        if !c.dst_host.is_empty()
            && c.dst_host.len() <= 1024
            && c.dst_host.parse::<IpAddr>().is_err()
        {
            add(
                "app-host".into(),
                "This executable to the exact hostname".into(),
                vec![
                    simple("process.path", c.process_path.clone()),
                    simple("dest.host", c.dst_host.clone()),
                ],
            );
        }
        if c.dst_ip.parse::<IpAddr>().is_ok() && c.dst_ip.len() <= 64 {
            add(
                "app-address".into(),
                "This executable to the exact IP address".into(),
                vec![
                    simple("process.path", c.process_path.clone()),
                    simple("dest.ip", c.dst_ip.clone()),
                ],
            );
        }
    }
    result
}

pub fn rule(
    connection: &Connection,
    decision: Decision,
    id: u64,
    choice: &Choice,
) -> Result<Rule, String> {
    if !DURATIONS
        .iter()
        .any(|(duration, _)| *duration == choice.duration)
    {
        return Err("Choose a supported rule duration".into());
    }
    if choice.constraints.len() > 16 {
        return Err("At most 16 additional match constraints are supported".into());
    }
    let targets = targets(connection);
    let selected = targets
        .iter()
        .find(|target| target.key == choice.target)
        .ok_or("The selected match target is not available for this connection")?;
    let mut operators = selected.operators.clone();
    for key in &choice.constraints {
        let target = targets
            .iter()
            .find(|target| &target.key == key)
            .ok_or("An additional match field is no longer available")?;
        for op in &target.operators {
            if !operators.contains(op) {
                operators.push(op.clone());
            }
        }
    }
    if choice.duration != "once"
        && choice.target == "connection"
        && connection.process_path.is_empty()
        && connection.dst_ip.is_empty()
    {
        return Err("Choose a specific target before creating a lasting rule for an unidentified connection".into());
    }
    let operator = if operators.len() == 1 {
        operators.remove(0)
    } else {
        let data=serde_json::to_string(&operators.iter().map(|op|serde_json::json!({"type":op.r#type,"operand":op.operand,"data":op.data,"sensitive":op.sensitive})).collect::<Vec<_>>()).unwrap();
        Operator {
            r#type: "list".into(),
            operand: "list".into(),
            data,
            list: operators,
            sensitive: true,
        }
    };
    let rule = Rule {
        created: crate::history::now_seconds(),
        name: format!("rustleblower.prompt.{}.{id}", crate::history::now_nanos()),
        description: "Rustleblower permission choice".into(),
        enabled: true,
        precedence: choice.priority,
        nolog: !choice.logging,
        action: decision.action().into(),
        duration: choice.duration.clone(),
        operator: Some(operator),
    };
    crate::rules::document::validate_edit(&rule)?;
    Ok(rule)
}
