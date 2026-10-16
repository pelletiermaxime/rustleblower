//! OpenSnitch JSON interchange. Unknown UI fields never replace known wire fields.
use super::{MAX_RULE_BYTES, valid};
use crate::protocol::{Operator, Rule};
use serde_json::{Value, json};

pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;

fn text(value: &Value, key: &str, default: &str) -> Result<String, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(default.into()),
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(format!("{key} must be text")),
    }
}
fn flag(value: &Value, key: &str, default: bool) -> Result<bool, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(flag)) => Ok(*flag),
        _ => Err(format!("{key} must be true or false")),
    }
}

pub fn operator_json(op: &Operator) -> Value {
    json!({"type":op.r#type,"operand":op.operand,"data":op.data,"sensitive":op.sensitive,
        "list":op.list.iter().map(operator_json).collect::<Vec<_>>()})
}

fn parse_operator(value: &Value, depth: usize) -> Result<Operator, String> {
    if depth > 16 || !value.is_object() {
        return Err("Match operators must be objects with at most 16 nested levels".into());
    }
    let kind = text(value, "type", "simple")?;
    let mut data = text(value, "data", "")?;
    let mut children = match value.get("list") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| parse_operator(v, depth + 1))
            .collect::<Result<Vec<_>, _>>()?,
        None | Some(Value::Null) => Vec::new(),
        _ => return Err("operator.list must be an array".into()),
    };
    // Older exports stored an AND-list as JSON in data. Normalize it to the
    // protocol list used by current daemons, rather than silently losing it.
    if kind == "list" && children.is_empty() && !data.is_empty() {
        let legacy: Value =
            serde_json::from_str(&data).map_err(|e| format!("Invalid legacy match list: {e}"))?;
        children = legacy
            .as_array()
            .ok_or("Legacy match list must be an array")?
            .iter()
            .map(|v| parse_operator(v, depth + 1))
            .collect::<Result<Vec<_>, _>>()?;
        data.clear();
    }
    Ok(Operator {
        r#type: kind,
        operand: text(value, "operand", "")?,
        data,
        sensitive: flag(value, "sensitive", false)?,
        list: children,
    })
}

pub fn operator_from_text(text: &str) -> Result<Operator, String> {
    if text.len() > MAX_RULE_BYTES {
        return Err("Match JSON exceeds 64 KiB".into());
    }
    let value = serde_json::from_str(text).map_err(|e| format!("Invalid match JSON: {e}"))?;
    parse_operator(&value, 0)
}

pub fn rule_json(rule: &Rule) -> Value {
    let created = chrono::DateTime::from_timestamp(rule.created, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| rule.created.to_string());
    json!({"created":created,"name":rule.name,"description":rule.description,"enabled":rule.enabled,
        "precedence":rule.precedence,"nolog":rule.nolog,"action":rule.action,"duration":rule.duration,
        "operator":rule.operator.as_ref().map(operator_json)})
}

pub fn rule_from_json(value: &Value) -> Result<Rule, String> {
    if !value.is_object() {
        return Err("Each rule must be a JSON object".into());
    }
    let created = match value.get("created") {
        None | Some(Value::Null) => crate::history::now_seconds(),
        Some(Value::Number(n)) => n.as_i64().ok_or("created must be an integer timestamp")?,
        Some(Value::String(s)) => s
            .parse::<i64>()
            .or_else(|_| chrono::DateTime::parse_from_rfc3339(s).map(|t| t.timestamp()))
            .map_err(|_| "created must be a Unix timestamp or RFC3339 date")?,
        _ => return Err("created must be a timestamp or date".into()),
    };
    let operator = parse_operator(
        value
            .get("operator")
            .ok_or("Rule is missing its match operator")?,
        0,
    )?;
    let rule = Rule {
        created,
        name: text(value, "name", "")?,
        description: text(value, "description", "")?,
        enabled: flag(value, "enabled", false)?,
        precedence: flag(value, "precedence", false)?,
        nolog: flag(value, "nolog", false)?,
        action: text(value, "action", "")?,
        duration: text(value, "duration", "")?,
        operator: Some(operator),
    };
    validate_edit(&rule)?;
    Ok(rule)
}

pub fn validate_edit(rule: &Rule) -> Result<(), String> {
    valid(rule)?;
    if !matches!(rule.action.as_str(), "allow" | "deny" | "reject") {
        return Err("Choose allow, deny or reject".into());
    }
    if !valid_duration(&rule.duration) {
        return Err(
            "Choose once, until restart, always, or a positive duration such as 90s or 2h30m"
                .into(),
        );
    }
    let op = rule
        .operator
        .as_ref()
        .ok_or("Add at least one match condition")?;
    validate_operator(op, 0)
}

// Go's time.ParseDuration units, restricted to positive, finite deadlines.
// The daemon installs a rule before scheduling its expiry, so a malformed
// duration must be caught before sending a ChangeRule command.
fn valid_duration(value: &str) -> bool {
    if matches!(value, "once" | "until restart" | "always") {
        return true;
    }
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    let mut rest = value.strip_prefix('+').unwrap_or(value);
    let mut nanos = 0f64;
    while !rest.is_empty() {
        let length = rest
            .bytes()
            .take_while(|b| b.is_ascii_digit() || *b == b'.')
            .count();
        if length == 0 {
            return false;
        }
        let Ok(number) = rest[..length].parse::<f64>() else {
            return false;
        };
        rest = &rest[length..];
        let Some((unit, factor)) = [
            ("ns", 1.),
            ("us", 1_000.),
            ("µs", 1_000.),
            ("μs", 1_000.),
            ("ms", 1_000_000.),
            ("s", 1_000_000_000.),
            ("m", 60_000_000_000.),
            ("h", 3_600_000_000_000.),
        ]
        .into_iter()
        .find(|(unit, _)| rest.starts_with(unit)) else {
            return false;
        };
        nanos += number * factor;
        rest = &rest[unit.len()..];
    }
    nanos >= 1. && nanos < i64::MAX as f64
}

fn validate_operator(op: &Operator, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err("Match operators exceed 16 nested levels".into());
    }
    let network = matches!(op.operand.as_str(), "source.network" | "dest.network");
    let lists = matches!(
        op.operand.as_str(),
        "lists.domains" | "lists.domains_regexp" | "lists.ips" | "lists.nets" | "lists.hash.md5"
    );
    match op.r#type.as_str() {
        "range" => {
            if !matches!(op.operand.as_str(), "source.port" | "dest.port") {
                return Err("Port ranges require the Source port or Destination port field".into());
            }
            port_range(&op.data)?;
        }
        "list" => {
            if depth > 0 {
                return Err("This daemon protocol supports a flat AND group; flatten nested groups before saving".into());
            }
            if op.operand != "list" || op.list.is_empty() {
                return Err("AND groups need at least one condition; use Match all connections for a global rule".into());
            }
            for child in &op.list {
                validate_operator(child, depth + 1)?;
            }
        }
        "network" => {
            if !network || op.data.is_empty() {
                return Err("CIDR matching requires a source/destination network and a network or daemon alias".into());
            }
            if let Some((address, prefix)) = op.data.split_once('/') {
                let address: std::net::IpAddr =
                    address.parse().map_err(|_| "Invalid network IP address")?;
                let prefix: u8 = prefix
                    .parse()
                    .map_err(|_| "Invalid network prefix length")?;
                if prefix > if address.is_ipv4() { 32 } else { 128 } {
                    return Err("Network prefix is out of range".into());
                }
            }
        }
        "lists" => {
            if !lists || op.data.is_empty() {
                return Err(
                    "A watched list needs a list field and a directory on the daemon".into(),
                );
            }
        }
        "simple" | "regexp" => {
            if network || lists || op.operand.is_empty() || op.operand == "list" {
                return Err("Choose a compatible field and match type".into());
            }
        }
        _ => return Err(format!("Unsupported match type: {}", op.r#type)),
    }
    Ok(())
}

/// Exact compileRange grammar at the pinned daemon: remove ASCII spaces only,
/// parse unsigned decimal uint64 bounds and include both endpoints. Keep the
/// original spelling in the wire document; no regexp expansion or truncation.
pub fn port_range(data: &str) -> Result<(u64, u64), String> {
    let compact = data.replace(' ', "");
    let error = || {
        "Enter an inclusive port range as min-max with unsigned decimal bounds and min <= max"
            .to_string()
    };
    let (min, max) = compact.split_once('-').ok_or_else(error)?;
    let number = |value: &str| {
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(error());
        }
        value.parse::<u64>().map_err(|_| error())
    };
    let (min, max) = (number(min)?, number(max)?);
    if min > max {
        return Err(error());
    }
    Ok((min, max))
}

pub fn has_range(rule: &Rule) -> bool {
    fn contains(op: &Operator) -> bool {
        op.r#type == "range" || op.list.iter().any(contains)
    }
    rule.operator.as_ref().is_some_and(contains)
}

pub fn parse_document(text: &str) -> Result<Vec<Rule>, String> {
    super::transfer::Import::from_text(text)
        .map(|import| import.rules)
        .map_err(|error| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_roundtrip_keeps_operators_flags_and_utc_timestamp() {
        let rule = Rule {
            created: 123456789,
            name: "test".into(),
            description: "Unicode é / app".into(),
            enabled: true,
            precedence: true,
            nolog: true,
            action: "reject".into(),
            duration: "until restart".into(),
            operator: Some(Operator {
                r#type: "list".into(),
                operand: "list".into(),
                list: vec![Operator {
                    r#type: "network".into(),
                    operand: "source.network".into(),
                    data: "192.0.2.0/24".into(),
                    sensitive: true,
                    ..Default::default()
                }],
                ..Default::default()
            }),
        };
        assert_eq!(rule_from_json(&rule_json(&rule)).unwrap(), rule);
    }
    #[test]
    fn legacy_operator_json_and_created_dates_are_imported_without_losing_conditions() {
        let mut value = json!({"name":"legacy","created":"2024-01-01T00:00:00.000000Z","action":"allow","duration":"always",
            "operator":{"type":"list","operand":"list","data":"[{\"type\":\"simple\",\"operand\":\"process.path\",\"data\":\"/usr/bin/example\",\"sensitive\":true}]"}});
        let rule = rule_from_json(&value).unwrap();
        assert_eq!(rule.created, 1704067200);
        assert_eq!(
            rule.operator.as_ref().unwrap().list[0].data,
            "/usr/bin/example"
        );
        value["name"] = json!("../escape");
        assert!(rule_from_json(&value).is_err());
    }

    #[test]
    fn imports_keep_omitted_protobuf_flags_false_and_reject_unsafe_match_pairs() {
        let mut value = json!({"name":"test","action":"allow","duration":"always","operator":{"type":"simple","operand":"process.path","data":"/bin/example"}});
        let rule = rule_from_json(&value).unwrap();
        assert!(!rule.enabled && !rule.precedence && !rule.nolog);
        value["operator"]["type"] = json!("network");
        assert!(rule_from_json(&value).unwrap_err().contains("CIDR"));
        value["operator"]["operand"] = json!("dest.network");
        value["operator"]["data"] = json!("192.0.2.0/33");
        assert!(rule_from_json(&value).unwrap_err().contains("prefix"));
        value["operator"]["data"] = json!("192.0.2.0/24");
        assert!(rule_from_json(&value).is_ok());
    }

    #[test]
    fn durations_use_daemon_units_and_reject_unschedulable_values() {
        for duration in [
            "always",
            "once",
            "until restart",
            "90s",
            "2h30m",
            ".5h",
            "1µs",
            "+5m",
        ] {
            assert!(valid_duration(duration), "{duration}");
        }
        for duration in [
            "",
            "1d",
            "tomorrow",
            "0s",
            "-5m",
            "999999999999h",
            "1.2.3s",
            "1e3s",
            "5",
        ] {
            assert!(!valid_duration(duration), "{duration}");
        }
    }
}
