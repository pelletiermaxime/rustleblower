//! Bounded parsing on a background worker; no generic JSON tree for socket dumps.
use super::MAX_UPDATE;
use anyhow::{Result, ensure};
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::value::RawValue;
use std::{borrow::Cow, collections::BTreeMap, fmt, ops::Range, sync::Arc};
pub const MAX_ROWS: usize = 10_000;
pub const MAX_VIEW_BYTES: usize = 4 * 1024 * 1024;
pub const PAGE: usize = 200;
#[derive(Default)]
struct Rows<'a> {
    values: Vec<&'a RawValue>,
    shortened: bool,
}
impl<'de> Deserialize<'de> for Rows<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Rows<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a socket array")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut rows = Rows::default();
                while let Some(row) = seq.next_element::<Option<&'de RawValue>>()? {
                    if let Some(row) = row {
                        if rows.values.len() < MAX_ROWS {
                            rows.values.push(row);
                        } else {
                            rows.shortened = true;
                        }
                    }
                }
                Ok(rows)
            }
        }
        d.deserialize_seq(V)
    }
}
#[derive(Default)]
struct RawMap<'a, const N: usize> {
    values: BTreeMap<Cow<'a, str>, &'a RawValue>,
    shortened: bool,
}
impl<'de, const N: usize> Deserialize<'de> for RawMap<'de, N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<const N: usize>;
        impl<'de, const N: usize> Visitor<'de> for V<N> {
            type Value = RawMap<'de, N>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut result = RawMap::default();
                while let Some((key, value)) = map.next_entry::<Cow<'de, str>, &'de RawValue>()? {
                    if result.values.len() < N {
                        result.values.insert(key, value);
                    } else {
                        result.shortened = true;
                    }
                }
                Ok(result)
            }
        }
        d.deserialize_map(V::<N>)
    }
}
#[derive(Deserialize)]
#[serde(bound(deserialize = "'a: 'de"))]
struct Dump<'a> {
    #[serde(borrow, rename = "Table")]
    table: Option<Rows<'a>>,
    #[serde(borrow, rename = "Processes")]
    processes: Option<RawMap<'a, MAX_ROWS>>,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct Identity {
    source: Option<String>,
    destination: Option<String>,
    source_port: u16,
    destination_port: u16,
    interface: u32,
    cookie: [u32; 2],
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct Kernel {
    #[serde(rename = "ID")]
    id: Identity,
    #[serde(rename = "UID")]
    uid: u32,
    #[serde(rename = "INode")]
    inode: u32,
    family: u8,
    state: u8,
    timer: u8,
    retrans: u8,
    r_queue: u32,
    w_queue: u32,
    expires: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Wire {
    socket: Option<Kernel>,
    #[serde(rename = "PID")]
    pid: i64,
    #[serde(default)]
    iface: String,
    #[serde(default)]
    mark: u32,
    proto: u16,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct Process {
    path: String,
    comm: String,
}
pub struct SocketRow {
    pub pid: Option<u32>,
    pub title: String,
    pub endpoints: String,
    pub metadata: String,
    pub raw: Range<usize>,
}
impl SocketRow {
    pub fn matches(&self, search: &str) -> bool {
        let search = search.to_lowercase();
        [&self.title, &self.endpoints, &self.metadata]
            .iter()
            .any(|s| s.to_lowercase().contains(&search))
    }
}
pub struct Sockets {
    pub raw: Arc<str>,
    pub rows: Vec<SocketRow>,
    pub shortened: bool,
}
pub struct Field {
    pub name: String,
    pub text: String,
}
pub struct Fields {
    pub raw: Arc<str>,
    pub fields: Vec<Field>,
    pub shortened: bool,
}
pub fn protocol(id: u16) -> String {
    match id {
        1 => "ICMP".into(),
        2 => "IGMP".into(),
        6 => "TCP".into(),
        17 => "UDP".into(),
        33 => "DCCP".into(),
        58 => "ICMPv6".into(),
        132 => "SCTP".into(),
        136 => "UDPLite".into(),
        255 => "RAW".into(),
        _ => format!("Protocol {id}"),
    }
}
pub fn state(id: u8) -> String {
    [
        "All",
        "Established",
        "SYN sent",
        "SYN received",
        "FIN wait 1",
        "FIN wait 2",
        "Time wait",
        "Closed",
        "Close wait",
        "Last ACK",
        "Listen",
        "Closing",
        "New SYN received",
    ]
    .get(id as usize)
    .map(|v| (*v).into())
    .unwrap_or_else(|| format!("State {id}"))
}
fn short(s: &str) -> String {
    crate::history::bounded(s, 256)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
pub fn sockets(raw: Arc<str>) -> Result<Sockets> {
    ensure!(raw.len() <= MAX_UPDATE, "Live snapshot exceeds 1 MiB");
    let dump: Dump = serde_json::from_str(&raw)?;
    let table = dump
        .table
        .ok_or_else(|| anyhow::anyhow!("Daemon socket result has no Table array"))?;
    let processes = dump.processes.unwrap_or_default();
    let mut process_summaries = BTreeMap::new();
    for (pid, raw) in &processes.values {
        let proc: Process = serde_json::from_str(raw.get())?;
        process_summaries.insert(
            pid.as_ref(),
            Process {
                path: short(&proc.path),
                comm: short(&proc.comm),
            },
        );
    }
    let mut rows = Vec::with_capacity(table.values.len());
    let mut bytes = raw.len() + rows.capacity() * std::mem::size_of::<SocketRow>();
    let mut shortened = table.shortened || processes.shortened;
    for value in table.values {
        let wire: Wire = serde_json::from_str(value.get())?;
        let Some(s) = wire.socket else { continue };
        let unavailable = Process::default();
        let proc = process_summaries
            .get(wire.pid.to_string().as_str())
            .unwrap_or(&unavailable);
        let family = match s.family {
            2 => "IPv4".into(),
            10 => "IPv6".into(),
            17 => "Packet".into(),
            44 => "XDP".into(),
            n => format!("Family {n}"),
        };
        let title = format!(
            "{} · {} · {} · PID {} · UID {} · {}",
            protocol(wire.proto),
            state(s.state),
            family,
            if wire.pid >= 0 {
                wire.pid.to_string()
            } else {
                "unavailable".into()
            },
            s.uid,
            short(&proc.comm)
        );
        let endpoints = format!(
            "{} → {} · {}",
            crate::history::details::endpoint(
                &short(s.id.source.as_deref().unwrap_or("?")),
                s.id.source_port.into()
            ),
            crate::history::details::endpoint(
                &short(s.id.destination.as_deref().unwrap_or("?")),
                s.id.destination_port.into()
            ),
            short(&proc.path)
        );
        let metadata = format!(
            "Interface {} · inode {} · receive/send queues {}/{} · timer {} · expires {} · retransmits {} · mark {} · cookies {:?}",
            if wire.iface.is_empty() {
                s.id.interface.to_string()
            } else {
                short(&wire.iface)
            },
            s.inode,
            s.r_queue,
            s.w_queue,
            s.timer,
            s.expires,
            s.retrans,
            wire.mark,
            s.id.cookie
        );
        let start = value.get().as_ptr() as usize - raw.as_ptr() as usize;
        bytes += title.capacity() + endpoints.capacity() + metadata.capacity();
        if bytes > MAX_VIEW_BYTES {
            shortened = true;
            break;
        }
        rows.push(SocketRow {
            pid: u32::try_from(wire.pid).ok().filter(|p| *p > 0),
            title,
            endpoints,
            metadata,
            raw: start..start + value.get().len(),
        });
    }
    rows.sort_by(|a, b| (&a.endpoints, a.pid).cmp(&(&b.endpoints, b.pid)));
    Ok(Sockets {
        raw,
        rows,
        shortened,
    })
}
pub fn fields(raw: Arc<str>) -> Result<Fields> {
    ensure!(raw.len() <= MAX_UPDATE, "Live snapshot exceeds 1 MiB");
    let map: RawMap<128> = serde_json::from_str(&raw)?;
    let mut fields = Vec::with_capacity(map.values.len());
    let mut shortened = map.shortened;
    for (name, value) in map.values {
        if name.len() > 128 {
            shortened = true;
        }
        fields.push(Field {
            name: crate::history::bounded(&name, 128),
            text: if value.get().starts_with('"') {
                serde_json::from_str(value.get())?
            } else {
                value.get().to_owned()
            },
        });
    }
    Ok(Fields {
        raw,
        fields,
        shortened,
    })
}

/// Readable map/array entries without expanding a general JSON tree. Copy retains
/// the original complete field. Preview limits apply after filtering.
pub fn preview(text: &str, search: &str) -> (Vec<String>, bool) {
    let search = search.to_lowercase();
    let mut lines = Vec::new();
    let mut shortened = false;
    let mut add = |line: &str, shortened: &mut bool| {
        if line.to_lowercase().contains(&search) {
            if lines.len() >= 2048 {
                *shortened = true;
            } else {
                if line.len() > 512 {
                    *shortened = true;
                }
                lines.push(crate::history::bounded(line, 512));
            }
        }
    };
    if text.starts_with('{')
        && let Ok(map) = serde_json::from_str::<RawMap<MAX_ROWS>>(text)
    {
        shortened = map.shortened;
        for (key, value) in map.values {
            let value = if value.get().starts_with('"') {
                serde_json::from_str::<String>(value.get())
                    .unwrap_or_else(|_| value.get().to_owned())
            } else {
                value.get().to_owned()
            };
            add(&format!("{key}: {value}"), &mut shortened);
        }
    } else if text.starts_with('[')
        && let Ok(rows) = serde_json::from_str::<Rows>(text)
    {
        shortened = rows.shortened;
        for (i, row) in rows.values.iter().enumerate() {
            add(&format!("{}: {}", i + 1, row.get()), &mut shortened);
        }
    } else {
        for line in text.lines() {
            add(line, &mut shortened);
        }
    }
    (lines, shortened)
}

pub fn resources(raw: &str) -> Result<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Info {
        uptime: i64,
        loads: [u64; 3],
        procs: u16,
        totalram: u64,
        freeram: u64,
        totalswap: u64,
        freeswap: u64,
        unit: u32,
    }
    let info: Info = serde_json::from_str(raw)?;
    ensure!(info.unit > 0, "Memory unit unavailable");
    let mib = |value: u64| u128::from(value) * u128::from(info.unit) / 1024 / 1024;
    Ok(format!(
        "Uptime {}s · {} processes · load {:.2}/{:.2}/{:.2} · RAM free/total {} / {} MiB · swap free/total {} / {} MiB",
        info.uptime,
        info.procs,
        info.loads[0] as f64 / 65536.,
        info.loads[1] as f64 / 65536.,
        info.loads[2] as f64 / 65536.,
        mib(info.freeram),
        mib(info.totalram),
        mib(info.freeswap),
        mib(info.totalswap)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sockets_preserve_unknowns_nulls_ipv6_queues_and_process_identity() -> Result<()> {
        let raw:Arc<str>=r#"{"Table":[null,{"Socket":null,"PID":-1,"Proto":6},{"PID":42,"Proto":6,"Iface":"eth0","Mark":4,"Socket":{"ID":{"Source":"2001:db8::1","Destination":"2001:db8::2","SourcePort":32100,"DestinationPort":443,"Cookie":[1,2]},"UID":1000,"INode":567,"Family":10,"State":1,"RQueue":7,"WQueue":9},"Future":"kept"}],"Processes":{"42":{"Path":"/bin/Ω","Comm":"fixture","Env":{"IGNORED":"not retained in summaries"}}}}"#.into();
        let sockets = sockets(raw)?;
        assert_eq!(sockets.rows.len(), 1);
        let row = &sockets.rows[0];
        assert_eq!(row.pid, Some(42));
        assert!(row.endpoints.contains("[2001:db8::2]:443"));
        assert!(row.title.contains("UID 1000"));
        assert!(row.endpoints.contains("/bin/Ω"));
        assert!(row.metadata.contains("7/9"));
        assert!(sockets.raw[row.raw.clone()].contains("Future"));
        assert!(row.matches("established"));
        assert!(!sockets.shortened);
        assert!(super::sockets("{}".into()).is_err());
        assert!(
            super::sockets("{\"Table\":[]}".into())
                .unwrap()
                .rows
                .is_empty()
        );
        Ok(())
    }
    #[test]
    fn raw_collections_preview_and_payload_are_bounded() -> Result<()> {
        let row = r#"{"PID":-1,"Proto":255,"Socket":{"Family":44,"State":99}}"#;
        let raw = format!(
            "{{\"Table\":[{}]}}",
            std::iter::repeat_n(row, MAX_ROWS + 1)
                .collect::<Vec<_>>()
                .join(",")
        );
        let s = sockets(raw.into())?;
        assert!(s.shortened);
        assert!(s.rows.len() <= MAX_ROWS);
        assert!(s.rows[0].pid.is_none());
        assert!(s.rows[0].title.contains("State 99"));
        let fields=fields(r#"{"Status":"Name:\tfixture\nPid:\t42","Env":{"TOKEN":"requested explicitly"},"Future":18446744073709551615}"#.into())?;
        assert!(
            fields
                .fields
                .iter()
                .find(|f| f.name == "Status")
                .unwrap()
                .text
                .contains('\n')
        );
        assert_eq!(
            fields
                .fields
                .iter()
                .find(|f| f.name == "Future")
                .unwrap()
                .text,
            "18446744073709551615"
        );
        assert!(super::fields("x".repeat(MAX_UPDATE + 1).into()).is_err());
        Ok(())
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    #[test]
    fn escaped_map_names_readable_fields_and_preview_limits_preserve_full_values() -> Result<()> {
        let raw: Arc<str> =
            r#"{"Env":{"quoted\"name":"Ω"},"Future\u0020Field":18446744073709551615}"#.into();
        let fields = fields(raw.clone())?;
        assert!(fields.fields.iter().any(|f| f.name == "Future Field"));
        assert!(fields.raw.contains("Future\\u0020Field"));
        let env = &fields.fields.iter().find(|f| f.name == "Env").unwrap().text;
        let (lines, shortened) = preview(env, "");
        assert_eq!(lines, ["quoted\"name: Ω"]);
        assert!(!shortened);
        let text = std::iter::repeat_n("abcdef", 2050)
            .collect::<Vec<_>>()
            .join("\n");
        let (lines, shortened) = preview(&text, "abc");
        assert_eq!(lines.len(), 2048);
        assert!(shortened);
        assert!(preview(&text, "missing").0.is_empty());
        assert!(preview(&"Ω".repeat(1000), "").1);
        Ok(())
    }
    #[test]
    fn sysinfo_units_and_load_are_explicit_and_large_memory_does_not_overflow() -> Result<()> {
        let value = r#"{"Uptime":3600,"Loads":[65536,32768,16384],"Procs":123,"Totalram":8388608,"Freeram":4194304,"Totalswap":2097152,"Freeswap":1048576,"Unit":1024}"#;
        let summary = resources(value)?;
        assert!(summary.contains("load 1.00/0.50/0.25"));
        assert!(summary.contains("4096 / 8192 MiB"));
        assert!(summary.contains("1024 / 2048 MiB"));
        assert!(resources(&value.replace("\"Unit\":1024", "\"Unit\":0")).is_err());
        assert!(resources("{}").is_err());
        Ok(())
    }
}
