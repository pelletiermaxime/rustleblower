//! Patch known daemon settings without discarding unknown or unreported values.
use super::{CONFIG_BYTES, validate_config};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    General,
    Logging,
    Firewall,
    Statistics,
    Connection,
}
pub const GROUPS: &[(Group, &str)] = &[
    (Group::General, "General"),
    (Group::Logging, "Logging"),
    (Group::Firewall, "Firewall & memory"),
    (Group::Statistics, "Statistics"),
    (Group::Connection, "Connection"),
];
#[derive(Clone, Copy)]
pub enum Kind {
    Text(usize),
    Choice(&'static [(&'static str, &'static str)]),
    Boolean,
    Unsigned(u64),
    Integer,
    Interval,
}
#[derive(Clone, Copy)]
pub struct Field {
    pub id: &'static str,
    pub path: &'static str,
    pub label: &'static str,
    pub group: Group,
    pub kind: Kind,
    pub hint: &'static str,
}
const ACTIONS: &[(&str, &str)] = &[("deny", "Deny"), ("allow", "Allow"), ("reject", "Reject")];
const DURATIONS: &[(&str, &str)] = &[
    ("once", "Once"),
    ("until restart", "Until daemon restart"),
    ("always", "Forever"),
];
const MONITORS: &[(&str, &str)] = &[("proc", "proc"), ("ebpf", "eBPF"), ("audit", "audit")];
const LEVELS: &[(&str, &str)] = &[
    ("0", "Debug"),
    ("1", "Info"),
    ("2", "Important"),
    ("3", "Warning"),
    ("4", "Error"),
    ("5", "Fatal"),
];
const AUTH: &[(&str, &str)] = &[
    ("simple", "Plain connection"),
    ("tls-simple", "TLS"),
    ("tls-mutual", "Mutual TLS"),
];
const CERT_AUTH: &[(&str, &str)] = &[
    ("no-client-cert", "No client certificate"),
    ("req-cert", "Request client certificate"),
    ("req-any-cert", "Require any client certificate"),
    ("verify-cert", "Verify certificate if given"),
    ("req-and-verify-cert", "Require and verify certificate"),
];
use Group::*;
use Kind::*;
pub const FIELDS: &[Field] = &[
    Field {
        id: "daemon-default-action",
        path: "/DefaultAction",
        label: "Default action",
        group: General,
        kind: Choice(ACTIONS),
        hint: "Fallback policy owned by this daemon.",
    },
    Field {
        id: "daemon-default-duration",
        path: "/DefaultDuration",
        label: "Default duration",
        group: General,
        kind: Choice(DURATIONS),
        hint: "Duration reported in the daemon configuration; popup defaults are separate.",
    },
    Field {
        id: "daemon-monitor",
        path: "/ProcMonitorMethod",
        label: "Process monitor",
        group: General,
        kind: Choice(MONITORS),
        hint: "The daemon must support the chosen monitor on its own machine.",
    },
    Field {
        id: "daemon-intercept-unknown",
        path: "/InterceptUnknown",
        label: "Intercept connections with an unknown process",
        group: General,
        kind: Boolean,
        hint: "",
    },
    Field {
        id: "daemon-rules-path",
        path: "/Rules/Path",
        label: "Rules directory",
        group: General,
        kind: Text(4096),
        hint: "Path on the daemon's machine.",
    },
    Field {
        id: "daemon-checksums",
        path: "/Rules/EnableChecksums",
        label: "Enable executable checksums",
        group: General,
        kind: Boolean,
        hint: "",
    },
    Field {
        id: "daemon-log-level",
        path: "/LogLevel",
        label: "Log level",
        group: Logging,
        kind: Choice(LEVELS),
        hint: "",
    },
    Field {
        id: "daemon-log-file",
        path: "/Server/LogFile",
        label: "Log file",
        group: Logging,
        kind: Text(4096),
        hint: "Path on the daemon's machine. An empty value is preserved as empty.",
    },
    Field {
        id: "daemon-log-utc",
        path: "/LogUTC",
        label: "Use UTC in daemon logs",
        group: Logging,
        kind: Boolean,
        hint: "",
    },
    Field {
        id: "daemon-log-micro",
        path: "/LogMicro",
        label: "Include microseconds in daemon logs",
        group: Logging,
        kind: Boolean,
        hint: "",
    },
    Field {
        id: "daemon-queue-bypass",
        path: "/FwOptions/QueueBypass",
        label: "Let traffic pass when the packet queue is unavailable",
        group: Firewall,
        kind: Boolean,
        hint: "",
    },
    Field {
        id: "daemon-firewall-interval",
        path: "/FwOptions/MonitorInterval",
        label: "Firewall monitor interval",
        group: Firewall,
        kind: Interval,
        hint: "Positive duration, for example 15s, 500ms or 1m.",
    },
    Field {
        id: "daemon-queue-number",
        path: "/FwOptions/QueueNum",
        label: "Packet queue number",
        group: Firewall,
        kind: Unsigned(65535),
        hint: "0–65535.",
    },
    Field {
        id: "daemon-firewall-name",
        path: "/FwOptions/Firewall",
        label: "Firewall backend option",
        group: Firewall,
        kind: Text(256),
        hint: "Keep the daemon's reported backend unless changing it intentionally.",
    },
    Field {
        id: "daemon-firewall-path",
        path: "/FwOptions/ConfigPath",
        label: "System firewall configuration path",
        group: Firewall,
        kind: Text(4096),
        hint: "Path on the daemon's machine.",
    },
    Field {
        id: "daemon-flush-connections",
        path: "/Internal/FlushConnsOnStart",
        label: "Flush connections when the daemon starts",
        group: Firewall,
        kind: Boolean,
        hint: "",
    },
    Field {
        id: "daemon-gc-percent",
        path: "/Internal/GCPercent",
        label: "Garbage collection target (%)",
        group: Firewall,
        kind: Integer,
        hint: "Integer from the daemon configuration.",
    },
    Field {
        id: "daemon-max-events",
        path: "/Stats/MaxEvents",
        label: "Daemon event buffer limit",
        group: Statistics,
        kind: Unsigned(i32::MAX as u64),
        hint: "Controls the daemon's event buffer, separately from UI history retention.",
    },
    Field {
        id: "daemon-max-stats",
        path: "/Stats/MaxStats",
        label: "Daemon keys per statistics category",
        group: Statistics,
        kind: Unsigned(i32::MAX as u64),
        hint: "Zero retains the daemon's own default behavior.",
    },
    Field {
        id: "daemon-stats-workers",
        path: "/Stats/Workers",
        label: "Statistics workers",
        group: Statistics,
        kind: Unsigned(i32::MAX as u64),
        hint: "Zero selects the daemon's default worker count.",
    },
    Field {
        id: "daemon-server-address",
        path: "/Server/Address",
        label: "UI server address",
        group: Connection,
        kind: Text(1024),
        hint: "Changing this address can reconnect the daemon to a different UI server.",
    },
    Field {
        id: "daemon-auth-type",
        path: "/Server/Authentication/Type",
        label: "Connection authentication",
        group: Connection,
        kind: Choice(AUTH),
        hint: "These configure the daemon's client; Rustleblower's server transport is configured separately.",
    },
    Field {
        id: "daemon-ca-cert",
        path: "/Server/Authentication/TLSOptions/CACert",
        label: "CA certificate",
        group: Connection,
        kind: Text(4096),
        hint: "Certificate path on the daemon's machine.",
    },
    Field {
        id: "daemon-server-cert",
        path: "/Server/Authentication/TLSOptions/ServerCert",
        label: "Server certificate",
        group: Connection,
        kind: Text(4096),
        hint: "Reported daemon TLS option; kept verbatim.",
    },
    Field {
        id: "daemon-server-key",
        path: "/Server/Authentication/TLSOptions/ServerKey",
        label: "Server key",
        group: Connection,
        kind: Text(4096),
        hint: "Reported daemon TLS option; kept verbatim.",
    },
    Field {
        id: "daemon-client-cert",
        path: "/Server/Authentication/TLSOptions/ClientCert",
        label: "Client certificate",
        group: Connection,
        kind: Text(4096),
        hint: "Certificate path on the daemon's machine.",
    },
    Field {
        id: "daemon-client-key",
        path: "/Server/Authentication/TLSOptions/ClientKey",
        label: "Client key",
        group: Connection,
        kind: Text(4096),
        hint: "Key path on the daemon's machine.",
    },
    Field {
        id: "daemon-client-auth",
        path: "/Server/Authentication/TLSOptions/ClientAuthType",
        label: "Client certificate policy",
        group: Connection,
        kind: Choice(CERT_AUTH),
        hint: "",
    },
    Field {
        id: "daemon-skip-verify",
        path: "/Server/Authentication/TLSOptions/SkipVerify",
        label: "Skip server certificate verification",
        group: Connection,
        kind: Boolean,
        hint: "Disables certificate verification in the daemon's TLS client.",
    },
];
/// Only user-edited fields enter the patch. Missing fields are never defaulted.
pub type Edits = BTreeMap<&'static str, Option<Value>>;
pub fn field(path: &str) -> Option<Field> {
    FIELDS.iter().copied().find(|field| field.path == path)
}
pub fn parse(field: Field, text: &str) -> Result<Option<Value>, String> {
    let fail = |reason: &str| format!("{}: {reason}", field.label);
    let value = match field.kind {
        Text(limit) => {
            if text.len() > limit || text.contains('\0') {
                return Err(fail(
                    "value exceeds its text limit or contains a null character",
                ));
            }
            Value::String(text.into())
        }
        Boolean => return Err(fail("use the checkbox for this setting")),
        Choice(options) => {
            if text.is_empty() {
                return Ok(None);
            }
            if !options.iter().any(|(key, _)| *key == text) {
                return Err(fail("choose a supported value or edit Complete JSON"));
            }
            if field.path == "/LogLevel" {
                Value::from(text.parse::<u32>().unwrap())
            } else {
                Value::String(text.into())
            }
        }
        Unsigned(max) => {
            if text.is_empty() {
                return Ok(None);
            }
            let number = text
                .parse::<u64>()
                .map_err(|_| fail("enter a nonnegative whole number"))?;
            if number > max {
                return Err(fail(&format!("enter a value from 0 to {max}")));
            }
            Value::from(number)
        }
        Integer => {
            if text.is_empty() {
                return Ok(None);
            }
            Value::from(
                text.parse::<i64>()
                    .map_err(|_| fail("enter a whole number in the signed 64-bit range"))?,
            )
        }
        Interval => {
            if text.is_empty() {
                return Ok(None);
            }
            if !positive_interval(text) {
                return Err(fail("enter a positive duration such as 15s or 1m"));
            }
            Value::String(text.into())
        }
    };
    Ok(Some(value))
}
pub fn display(value: Option<&Value>) -> String {
    match value {
        None => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(value) => value.to_string(),
    }
}
pub fn patch(text: &str, edits: &Edits) -> Result<String, String> {
    if edits.is_empty() {
        validate_config(text)?;
        return Ok(text.into());
    }
    let mut document = validate_config(text)?;
    for (path, value) in edits {
        let field = field(path).ok_or("Unknown daemon preference")?;
        if let Some(value) = value {
            match field.kind {
                Boolean if value.is_boolean() => {}
                Boolean => return Err(format!("{} must be a boolean", field.label)),
                _ => {
                    let parsed = parse(field, &display(Some(value)))?;
                    if parsed.as_ref() != Some(value) {
                        return Err(format!("{} has the wrong JSON type", field.label));
                    }
                }
            }
        }
        fn put(
            object: &mut Map<String, Value>,
            keys: &[&str],
            value: &Option<Value>,
        ) -> Result<(), String> {
            if keys.len() == 1 {
                if let Some(value) = value {
                    object.insert(keys[0].into(), value.clone());
                } else {
                    object.remove(keys[0]);
                }
                return Ok(());
            }
            if !object.contains_key(keys[0]) {
                if value.is_none() {
                    return Ok(());
                }
                object.insert(keys[0].into(), Value::Object(Map::new()));
            }
            let child = object
                .get_mut(keys[0])
                .and_then(Value::as_object_mut)
                .ok_or_else(|| {
                    format!(
                        "{} is not an object; use Complete JSON to review it",
                        keys[0]
                    )
                })?;
            put(child, &keys[1..], value)
        }
        let keys = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
        put(document.as_object_mut().unwrap(), &keys, value)?;
    }
    let text = serde_json::to_string(&document).map_err(|error| error.to_string())?;
    if text.len() > CONFIG_BYTES {
        return Err("Edited daemon configuration exceeds 64 KiB".into());
    }
    Ok(text)
}
pub const MAX_REVIEW: usize = 200;
#[derive(Clone, Debug)]
pub struct Delta {
    pub path: String,
    pub before: String,
    pub after: String,
}
pub struct Review {
    pub total: usize,
    pub rows: Vec<Delta>,
}
fn shortened(text: &str) -> String {
    let mut result = crate::history::bounded(text, 256);
    if result.len() < text.len() {
        result.push('…');
    }
    result
}
fn describe(value: Option<&Value>) -> String {
    match value {
        None => "Not reported".into(),
        Some(value) => shortened(&value.to_string()),
    }
}
pub fn review(before: &str, after: &str) -> Result<Review, String> {
    let before = validate_config(before)?;
    let after = validate_config(after)?;
    let mut review = Review {
        total: 0,
        rows: Vec::new(),
    };
    fn visit(path: &str, before: Option<&Value>, after: Option<&Value>, review: &mut Review) {
        if before == after {
            return;
        }
        let a = before.and_then(Value::as_object);
        let b = after.and_then(Value::as_object);
        if (a.is_some() && after.is_none())
            || (b.is_some() && before.is_none())
            || (a.is_some() && b.is_some())
        {
            let keys = a
                .into_iter()
                .flat_map(|map| map.keys())
                .chain(b.into_iter().flat_map(|map| map.keys()))
                .collect::<BTreeSet<_>>();
            for key in keys {
                let key = key.replace('~', "~0").replace('/', "~1");
                let child = shortened(&format!("{path}/{key}"));
                let raw = key.replace("~1", "/").replace("~0", "~");
                visit(
                    &child,
                    a.and_then(|map| map.get(&raw)),
                    b.and_then(|map| map.get(&raw)),
                    review,
                );
            }
            return;
        }
        review.total += 1;
        if review.rows.len() < MAX_REVIEW {
            review.rows.push(Delta {
                path: shortened(path),
                before: describe(before),
                after: describe(after),
            });
        }
    }
    visit("", Some(&before), Some(&after), &mut review);
    Ok(review)
}
/// Go time.Duration's positive decimal units, including compound/fractional values.
fn positive_interval(text: &str) -> bool {
    if text.len() > 128 {
        return false;
    }
    let mut rest = text.strip_prefix('+').unwrap_or(text);
    let mut total = 0u128;
    while !rest.is_empty() {
        let length = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(rest.len());
        if length == 0 {
            return false;
        }
        let number = &rest[..length];
        rest = &rest[length..];
        let Some((unit, multiplier)) = [
            ("ns", 1u128),
            ("us", 1_000),
            ("µs", 1_000),
            ("μs", 1_000),
            ("ms", 1_000_000),
            ("s", 1_000_000_000),
            ("m", 60_000_000_000),
            ("h", 3_600_000_000_000),
        ]
        .into_iter()
        .find(|(unit, _)| rest.starts_with(unit)) else {
            return false;
        };
        let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
        if (whole.is_empty() && fraction.is_empty())
            || !whole
                .bytes()
                .chain(fraction.bytes())
                .all(|c| c.is_ascii_digit())
        {
            return false;
        }
        let Ok(whole) = (if whole.is_empty() {
            Ok(0)
        } else {
            whole.parse::<u128>()
        }) else {
            return false;
        };
        // Long multiplication computes floor(fraction * unit) exactly, without
        // losing precision near time.Duration's signed nanosecond boundary.
        let mut fractional = 0;
        for digit in fraction.bytes().rev() {
            fractional = (u128::from(digit - b'0') * multiplier + fractional) / 10;
        }
        let Some(value) = whole
            .checked_mul(multiplier)
            .and_then(|value| value.checked_add(fractional))
            .and_then(|value| total.checked_add(value))
        else {
            return false;
        };
        total = value;
        if total > i64::MAX as u128 {
            return false;
        }
        rest = &rest[unit.len()..];
    }
    total > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edit(path: &'static str, text: &str) -> Option<Value> {
        parse(field(path).unwrap(), text).unwrap()
    }
    #[test]
    fn typed_edits_preserve_unknown_fields_exact_numbers_and_unreported_defaults() {
        let text = r#"{"DefaultAction":"deny","Rules":{"Path":"/old","Future":7},"Server":{"Address":"unix:///owned.sock","Authentication":{"Type":"mutual-tls","TLSOptions":{"ClientKey":"/private/key","Unknown":"keep"}}},"Future":{"precise":1.2345678901234567890123456789,"large":184467440737095516159999}}"#;
        let edits = Edits::from([
            ("/DefaultAction", edit("/DefaultAction", "allow")),
            ("/Rules/EnableChecksums", Some(Value::Bool(true))),
            ("/Rules/Path", edit("/Rules/Path", "/new")),
        ]);
        let after = patch(text, &edits).unwrap();
        assert!(
            after.contains("1.2345678901234567890123456789")
                && after.contains("184467440737095516159999")
        );
        let before = validate_config(text).unwrap();
        let value = validate_config(&after).unwrap();
        assert_eq!(value["Future"], before["Future"]);
        assert_eq!(value["Server"], before["Server"]);
        assert_eq!(value["Rules"]["Future"], 7);
        assert!(value.get("Internal").is_none());
        assert!(value.get("DefaultDuration").is_none());
        let review = review(text, &after).unwrap();
        assert_eq!(review.total, 3);
        assert!(
            review
                .rows
                .iter()
                .any(|row| row.path == "/Rules/EnableChecksums" && row.before == "Not reported")
        );
        assert_eq!(patch(text, &Edits::new()).unwrap(), text);
    }
    #[test]
    fn removal_is_precise_and_malformed_parent_is_never_silently_replaced() {
        let text = r#"{"GCPercent":"unrelated root","Future":true}"#;
        assert_eq!(
            validate_config(&patch(text, &Edits::from([("/Internal/GCPercent", None)])).unwrap())
                .unwrap(),
            validate_config(text).unwrap()
        );
        assert!(
            patch(
                r#"{"Rules":"future syntax"}"#,
                &Edits::from([("/Rules/Path", edit("/Rules/Path", "/new"))])
            )
            .unwrap_err()
            .contains("not an object")
        );
        let value = validate_config(
            &patch(
                r#"{"Internal":{"GCPercent":100,"Future":true}}"#,
                &Edits::from([("/Internal/GCPercent", None)]),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(value["Internal"].get("GCPercent").is_none());
        assert_eq!(value["Internal"]["Future"], true);
    }
    #[test]
    fn bounds_and_field_types_reject_invalid_changes_before_any_command() {
        for (path, text) in [
            ("/Stats/MaxEvents", "-1"),
            ("/Stats/MaxStats", "2147483648"),
            ("/FwOptions/QueueNum", "65536"),
            ("/Internal/GCPercent", "9223372036854775808"),
            ("/FwOptions/MonitorInterval", "0s"),
            ("/FwOptions/MonitorInterval", "-1s"),
            ("/FwOptions/MonitorInterval", "nan"),
            ("/FwOptions/MonitorInterval", "999999999999999999999h"),
            ("/DefaultAction", "ask"),
            ("/FwOptions/MonitorInterval", "9223372036854775808ns"),
        ] {
            assert!(parse(field(path).unwrap(), text).is_err(), "{path} {text}");
        }
        for text in [
            "15s",
            "500ms",
            "1h2m3.5s",
            ".5s",
            "1µs",
            "1μs",
            "0.000000001s",
            "+15s",
            "9223372036854775807ns",
        ] {
            assert!(
                parse(field("/FwOptions/MonitorInterval").unwrap(), text).is_ok(),
                "{text}"
            );
        }
        assert!(parse(field("/Rules/Path").unwrap(), &"x".repeat(4097)).is_err());
        assert!(
            patch(
                "{}",
                &Edits::from([("/LogLevel", Some(Value::String("2".into())))])
            )
            .is_err()
        );
        let huge = format!("{{\"unknown\":\"{}\"}}", "x".repeat(CONFIG_BYTES - 20));
        assert!(
            patch(
                &huge,
                &Edits::from([("/Rules/Path", Some(Value::String("x".repeat(100))))])
            )
            .is_err()
        );
    }
    #[test]
    fn daemon_authentication_uses_the_installed_protocol_spellings() {
        for value in ["simple", "tls-simple", "tls-mutual"] {
            assert_eq!(
                parse(field("/Server/Authentication/Type").unwrap(), value).unwrap(),
                Some(Value::String(value.into()))
            );
        }
        for value in [
            "no-client-cert",
            "req-cert",
            "req-any-cert",
            "verify-cert",
            "req-and-verify-cert",
        ] {
            assert_eq!(
                parse(
                    field("/Server/Authentication/TLSOptions/ClientAuthType").unwrap(),
                    value
                )
                .unwrap(),
                Some(Value::String(value.into()))
            );
        }
        assert!(
            parse(
                field("/Server/Authentication/TLSOptions/ClientAuthType").unwrap(),
                "RequireAndVerifyClientCert"
            )
            .is_err()
        );
    }
    #[test]
    fn review_is_bounded_and_escapes_paths_without_losing_change_count() {
        let before = "{}";
        let mut map = Map::new();
        for i in 0..430 {
            map.insert(format!("key-{i}"), Value::from(i));
        }
        map.insert(
            "a/b~c".into(),
            serde_json::json!({"é".repeat(300):"x".repeat(1000)}),
        );
        let after = serde_json::to_string(&map).unwrap();
        let result = review(before, &after).unwrap();
        assert_eq!(result.total, 431);
        assert_eq!(result.rows.len(), 200);
        assert!(
            result.rows.iter().all(|row| row.path.len() <= 259
                && row.before.len() <= 259
                && row.after.len() <= 259)
        );
        assert!(
            result
                .rows
                .iter()
                .any(|row| row.path.starts_with("/a~1b~0c/"))
        );
    }
}
