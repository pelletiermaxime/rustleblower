//! Bounded, single-pass expansion of the pinned UI's missed-popup template.
use super::{Mailbox, Notice, Target};
use crate::{
    preferences::Popup,
    protocol::{Connection, Rule},
};

pub const MAX_TEMPLATE: usize = 1024;
pub const MAX_BODY: usize = 512;
pub const DEFAULT_TEMPLATE: &str =
    "%rule.action% action applied %node.addr%\nCommand line: %conn.process_args%";
const EMPTY_TEMPLATE: &str =
    "%rule.action% action applied %node.addr%\nProcess: %conn.process_args%";

pub fn validate(template: &str) -> Result<(), &'static str> {
    if template.len() > MAX_TEMPLATE
        || template
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        Err(
            "Missed-popup message templates must be at most 1024 bytes with no control characters other than newlines or tabs",
        )
    } else {
        Ok(())
    }
}
/// The Kit single-line input has a validator; multiline text is stored unchanged.
pub fn encode_input(template: &str) -> String {
    template
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}
pub fn decode_input(input: &str) -> Result<String, &'static str> {
    if input.len() > MAX_TEMPLATE * 2 {
        return Err("Message template input exceeds 2048 bytes");
    }
    let mut out = String::new();
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' {
            match chars.next() {
                Some('n') => '\n',
                Some('t') => '\t',
                Some('\\') => '\\',
                _ => return Err("Use \\n for a new line, \\t for a tab or \\\\ for a backslash"),
            }
        } else {
            c
        });
        if out.len() > MAX_TEMPLATE {
            return Err("Message templates must be at most 1024 bytes");
        }
    }
    validate(&out)?;
    Ok(out)
}
struct Body {
    text: String,
    full: bool,
}
impl Body {
    fn len(&self) -> usize {
        if self.full { MAX_BODY } else { self.text.len() }
    }
}
fn append(out: &mut Body, text: &str, template: bool) {
    if out.full {
        return;
    }
    for mut c in text.chars() {
        if c.is_control() && !(template && matches!(c, '\n' | '\t')) {
            c = ' ';
        }
        if out.text.len() + c.len_utf8() > MAX_BODY {
            out.full = true;
            break;
        }
        out.text.push(c);
    }
}
/// Values never become template syntax, even when arguments contain `%rule.action%`.
pub fn render(
    template: &str,
    connection: &Connection,
    rule: &Rule,
    node: &str,
    hostname: &str,
) -> String {
    let mut rest = if template.is_empty() {
        EMPTY_TEMPLATE
    } else {
        template
    };
    let mut out = Body {
        text: String::with_capacity(MAX_BODY),
        full: false,
    };
    while !rest.is_empty() && out.len() < MAX_BODY {
        let Some(start) = rest.find('%') else {
            append(&mut out, rest, true);
            break;
        };
        append(&mut out, &rest[..start], true);
        rest = &rest[start + 1..];
        let Some(end) = rest.find('%') else {
            append(&mut out, "%", true);
            append(&mut out, rest, true);
            break;
        };
        let token = &rest[..end];
        let value = match token {
            "conn.srcip" => Some(connection.src_ip.as_str()),
            "conn.dstip" => Some(connection.dst_ip.as_str()),
            "conn.dsthost" => Some(connection.dst_host.as_str()),
            "conn.proto" => Some(connection.protocol.as_str()),
            "conn.process" => Some(connection.process_path.as_str()),
            "conn.process_cwd" => Some(connection.process_cwd.as_str()),
            "rule.action" | "conn.action" => Some(rule.action.as_str()),
            "node.addr" => Some(node),
            "node.hostname" => Some(hostname),
            _ => None,
        };
        if let Some(value) = value {
            append(&mut out, value, false);
        } else {
            match token {
                "conn.srcport" => append(&mut out, &connection.src_port.to_string(), false),
                "conn.dstport" => append(&mut out, &connection.dst_port.to_string(), false),
                "conn.pid" => append(&mut out, &connection.process_id.to_string(), false),
                "conn.uid" => append(&mut out, &connection.user_id.to_string(), false),
                "conn.process_args" => {
                    for (i, arg) in connection.process_args.iter().enumerate() {
                        if out.len() >= MAX_BODY {
                            break;
                        }
                        if i != 0 {
                            append(&mut out, " ", false);
                        }
                        append(&mut out, arg, false);
                    }
                }
                _ => {
                    append(&mut out, "%", true);
                    append(&mut out, token, true);
                    append(&mut out, "%", true);
                }
            }
        }
        rest = &rest[end + 1..];
    }
    out.text
}
pub fn post_timeout(
    mailbox: &Mailbox,
    preferences: &Popup,
    connection: &Connection,
    rule: &Rule,
    node: &str,
    hostname: &str,
    reason: &str,
) {
    if reason != "timeout"
        || !preferences.desktop_notifications
        || !preferences.missed_popup_notifications
    {
        return;
    }
    let name = if connection.process_path.is_empty() {
        let destination = if connection.dst_host.is_empty() {
            &connection.dst_ip
        } else {
            &connection.dst_host
        };
        format!(
            "{}:{} ({})",
            crate::history::bounded(destination, 128),
            connection.dst_port,
            crate::history::bounded(&connection.protocol, 32)
        )
    } else {
        crate::history::bounded(&connection.process_path, 128)
    };
    mailbox.enqueue(Notice {
        target: Target::Rules,
        node: crate::history::bounded(
            &name
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect::<String>(),
            128,
        ),
        summary: render(
            &preferences.missed_popup_template,
            connection,
            rule,
            node,
            hostname,
        ),
        severity: 2,
        priority: 1,
    });
}
