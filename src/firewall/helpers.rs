//! Guided constructors for the pinned UI's nftables statement families.
//! Pure, bounded draft work; these helpers never execute firewall commands.
use super::document::{Expression, Statement, Value};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    DestinationPort,
    SourcePort,
    DestinationAddress,
    SourceAddress,
    InputInterface,
    OutputInterface,
    SetConnectionMark,
    ConnectionMark,
    ConnectionState,
    SetPacketMark,
    PacketInformation,
    Icmp,
    IcmpV6,
    Log,
    Quota,
    Counter,
    Limit,
}
pub const KINDS: &[Kind] = &[
    Kind::DestinationPort,
    Kind::SourcePort,
    Kind::DestinationAddress,
    Kind::SourceAddress,
    Kind::InputInterface,
    Kind::OutputInterface,
    Kind::SetConnectionMark,
    Kind::ConnectionMark,
    Kind::ConnectionState,
    Kind::SetPacketMark,
    Kind::PacketInformation,
    Kind::Icmp,
    Kind::IcmpV6,
    Kind::Log,
    Kind::Quota,
    Kind::Counter,
    Kind::Limit,
];
pub const OPERATORS: &[&str] = &["==", "!=", ">=", ">", "<=", "<"];
pub const VERDICTS: &[&str] = &[
    "accept", "drop", "reject", "return", "queue", "dnat", "snat", "redirect", "tproxy", "stop",
];
pub const ICMP_TYPES: &[&str] = &[
    "echo-request",
    "echo-reply",
    "source-quench",
    "destination-unreachable",
    "router-advertisement",
    "router-solicitation",
    "redirect",
    "time-exceeded",
    "info-request",
    "info-reply",
    "parameter-problem",
    "timestamp-request",
    "timestamp-reply",
    "address-mask-request",
    "address-mask-reply",
    "packet-too-big",
    "neighbour-solicitation",
    "neighbour-advertisement",
];
const RATE_UNITS: &[&str] = &["bytes", "kbytes", "mbytes", "gbytes"];
const TIME_UNITS: &[&str] = &["second", "minute", "hour", "day"];

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::DestinationPort => "Destination port",
            Self::SourcePort => "Source port",
            Self::DestinationAddress => "Destination address",
            Self::SourceAddress => "Source address",
            Self::InputInterface => "Input interface",
            Self::OutputInterface => "Output interface",
            Self::SetConnectionMark => "Set connection mark",
            Self::ConnectionMark => "Match connection mark",
            Self::ConnectionState => "Connection states",
            Self::SetPacketMark => "Set packet mark",
            Self::PacketInformation => "Packet information",
            Self::Icmp => "ICMP types",
            Self::IcmpV6 => "ICMPv6 types",
            Self::Log => "Log message",
            Self::Quota => "Bandwidth quota",
            Self::Counter => "Named counter",
            Self::Limit => "Rate limit",
        }
    }
    pub fn options(self) -> &'static [&'static str] {
        match self {
            Self::DestinationPort | Self::SourcePort => {
                &["tcp,udp", "tcp", "udp", "udplite", "sctp", "dccp"]
            }
            Self::DestinationAddress | Self::SourceAddress => &["ip", "ip6"],
            Self::PacketInformation => {
                &["mark", "l4proto", "skuid", "skgid", "protocol", "priority"]
            }
            Self::Log => &[
                "warn", "emerg", "alert", "crit", "err", "notice", "info", "debug", "audit",
            ],
            Self::Quota | Self::Limit => &["over", "until"],
            _ => &["default"],
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::DestinationPort | Self::SourcePort => {
                "Enter a numeric port, inclusive range, comma list or local service name: 443, 80-1024, 80,443 or https."
            }
            Self::DestinationAddress | Self::SourceAddress => {
                "Address, ordered address range or canonical CIDR: 192.0.2.0/24. Networks become first–last without enumerating hosts."
            }
            Self::InputInterface | Self::OutputInterface => {
                "Interface name or wildcard: eth0 or en*. The name is encoded in the value key, as required by the daemon."
            }
            Self::SetConnectionMark | Self::ConnectionMark | Self::SetPacketMark => {
                "Unsigned decimal mark, 1–4294967295. Raw statements remain available for other representations."
            }
            Self::ConnectionState => {
                "One or more comma-separated states: new,established,related,invalid."
            }
            Self::PacketInformation => {
                "Decimal value for mark/UID/GID/protocol/priority; l4proto also accepts a lowercase protocol name. Zero is allowed."
            }
            Self::Icmp | Self::IcmpV6 => {
                "Comma-separated ICMP type names, for example echo-request,echo-reply or neighbour-solicitation."
            }
            Self::Log => {
                "Log prefix. Choose the logging level above. ASCII spaces are removed, matching the pinned UI constructor."
            }
            Self::Quota => {
                "Positive count / unit, for example 10/mbytes. Units: bytes, kbytes, mbytes, gbytes. Until omits the over flag."
            }
            Self::Counter => {
                "Counter name. The helper retains the source packets marker followed by the counter name."
            }
            Self::Limit => {
                "Positive count / unit / time, for example 10/mbytes/minute. Time: second, minute, hour, day."
            }
        }
    }
    pub fn has_operator(self) -> bool {
        matches!(
            self,
            Self::DestinationPort
                | Self::SourcePort
                | Self::DestinationAddress
                | Self::SourceAddress
                | Self::InputInterface
                | Self::OutputInterface
                | Self::ConnectionMark
                | Self::ConnectionState
                | Self::PacketInformation
                | Self::Icmp
                | Self::IcmpV6
        )
    }
}
fn error(message: &str) -> String {
    message.into()
}
fn bounded(text: &str) -> Result<String, String> {
    if text.len() > 1024 || text.contains('\0') || text.contains(['\n', '\r']) {
        return Err(error(
            "Helper values must be one line of at most 1024 bytes",
        ));
    }
    Ok(text.replace(' ', ""))
}
fn number(text: &str, max: u64, zero: bool) -> Result<u64, String> {
    if text.is_empty() || !text.bytes().all(|c| c.is_ascii_digit()) {
        return Err(error("Enter an unsigned decimal integer"));
    }
    let n: u64 = text.parse().map_err(|_| error("Integer is too large"))?;
    if n > max || (!zero && n == 0) {
        return Err(error("Integer is outside the helper's supported range"));
    }
    Ok(n)
}
fn member(value: &str, choices: &[&str]) -> Result<(), String> {
    if choices.contains(&value) {
        Ok(())
    } else {
        Err(format!("Unsupported helper option: {value}"))
    }
}
fn ports(text: &str) -> Result<(), String> {
    if text.split(',').count() > 128 {
        return Err(error("Use at most 128 port terms"));
    }
    for term in text.split(',') {
        if let Some((a, b)) = term.split_once('-') {
            if number(a, 65535, false)? > number(b, 65535, false)? {
                return Err(error("Port range is reversed"));
            }
        } else {
            number(term, 65535, false)?;
        }
    }
    Ok(())
}
fn address(text: &str, family: &str) -> Result<String, String> {
    let parse = |s: &str| -> Result<IpAddr, String> {
        let ip: IpAddr = s.parse().map_err(|_| error("Enter a valid IP address"))?;
        if ip.is_ipv4() != (family == "ip") {
            return Err(error("Address does not match the selected IP family"));
        }
        Ok(ip)
    };
    if let Some((start, end)) = text.split_once('-') {
        let a = parse(start)?;
        let b = parse(end)?;
        if a > b {
            return Err(error("Address range is reversed"));
        }
        return Ok(format!("{a}-{b}"));
    }
    if let Some((ip, prefix)) = text.split_once('/') {
        let ip = parse(ip)?;
        let prefix = number(prefix, if ip.is_ipv4() { 32 } else { 128 }, true)? as u32;
        let (first, last) = match ip {
            IpAddr::V4(ip) => {
                let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
                let first = u32::from(ip) & mask;
                if first != u32::from(ip) {
                    return Err(error(
                        "CIDR must use its network address (host bits are set)",
                    ));
                }
                (
                    IpAddr::V4(Ipv4Addr::from(first)),
                    IpAddr::V4(Ipv4Addr::from(first | !mask)),
                )
            }
            IpAddr::V6(ip) => {
                let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
                let first = u128::from(ip) & mask;
                if first != u128::from(ip) {
                    return Err(error(
                        "CIDR must use its network address (host bits are set)",
                    ));
                }
                (
                    IpAddr::V6(Ipv6Addr::from(first)),
                    IpAddr::V6(Ipv6Addr::from(first | !mask)),
                )
            }
        };
        return Ok(format!("{first}-{last}"));
    }
    Ok(parse(text)?.to_string())
}
/// Build exact ordered values from the pinned UI's `form_to_protobuf` contract.
pub fn statement(kind: Kind, option: &str, op: &str, text: &str) -> Result<Expression, String> {
    member(option, kind.options())?;
    member(op, OPERATORS)?;
    let text = bounded(text)?;
    if text.is_empty() || (text == "0" && kind != Kind::PacketInformation) {
        return Err(error("Enter a nonempty, nonzero helper value"));
    }
    let mut values = Vec::with_capacity(4);
    let mut add = |key: &str, value: &str| {
        values.push(Value {
            key: key.into(),
            value: value.into(),
        })
    };
    let name = match kind {
        Kind::DestinationPort | Kind::SourcePort => {
            ports(&text)?;
            let key = if kind == Kind::DestinationPort {
                "dport"
            } else {
                "sport"
            };
            if option == "tcp,udp" {
                add("l4proto", option);
                add(key, &text);
                "meta"
            } else {
                add(key, &text);
                option
            }
        }
        Kind::DestinationAddress | Kind::SourceAddress => {
            let ip = address(&text, option)?;
            add(
                if kind == Kind::DestinationAddress {
                    "daddr"
                } else {
                    "saddr"
                },
                &ip,
            );
            option
        }
        Kind::InputInterface | Kind::OutputInterface => {
            if text.len() > 15
                || !text
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.*:-".contains(&c))
            {
                return Err(error(
                    "Use an interface name or wildcard of at most 15 ASCII characters",
                ));
            }
            add(&text, "");
            if kind == Kind::InputInterface {
                "iifname"
            } else {
                "oifname"
            }
        }
        Kind::SetConnectionMark | Kind::ConnectionMark | Kind::SetPacketMark => {
            number(&text, u32::MAX.into(), false)?;
            if kind != Kind::ConnectionMark {
                add("set", "");
            }
            add("mark", &text);
            if kind == Kind::SetPacketMark {
                "meta"
            } else {
                "ct"
            }
        }
        Kind::ConnectionState => {
            for state in text.split(',') {
                member(state, &["new", "established", "related", "invalid"])?;
            }
            add("state", &text);
            "ct"
        }
        Kind::PacketInformation => {
            if option == "l4proto" && !text.bytes().all(|c| c.is_ascii_digit()) {
                member(
                    &text,
                    &[
                        "tcp", "udp", "udplite", "sctp", "dccp", "icmp", "icmpv6", "ah",
                        "ethernet", "gre", "ip", "ipip", "l2tp", "comp", "igmp", "esp", "raw",
                        "encap",
                    ],
                )?;
            } else {
                number(&text, u32::MAX.into(), true)?;
            }
            add(option, &text);
            "meta"
        }
        Kind::Icmp | Kind::IcmpV6 => {
            for value in text.split(',') {
                member(value, ICMP_TYPES)?;
            }
            add("type", &text);
            if kind == Kind::Icmp { "icmp" } else { "icmpv6" }
        }
        Kind::Log => {
            add("level", option);
            add("prefix", &text);
            "log"
        }
        Kind::Quota | Kind::Limit => {
            let parts = text.split('/').collect::<Vec<_>>();
            let expected = if kind == Kind::Quota { 2 } else { 3 };
            if parts.len() != expected {
                return Err(error(
                    "Use count/unit for quota or count/unit/time for limit",
                ));
            }
            number(parts[0], u64::MAX, false)?;
            member(parts[1], RATE_UNITS)?;
            if option == "over" {
                add("over", "");
            }
            if kind == Kind::Quota {
                add(parts[1], parts[0]);
                "quota"
            } else {
                member(parts[2], TIME_UNITS)?;
                add("units", parts[0]);
                add("rate-units", parts[1]);
                add("time-units", parts[2]);
                "limit"
            }
        }
        Kind::Counter => {
            add("packets", "");
            add("name", &text);
            "counter"
        }
    };
    Ok(Expression {
        statement: Some(Statement {
            name: name.into(),
            op: if kind.has_operator() {
                op.into()
            } else {
                "==".into()
            },
            values,
        }),
    })
}

/// Guided verdicts use the selected chain's hook; never move the rule implicitly.
pub fn verdict(target: &str, parameter: &str, hook: &str) -> Result<(String, String), String> {
    member(target, VERDICTS)?;
    let parameter = bounded(parameter)?;
    // The pinned UI forces these choices to output unless already prerouting.
    // A captured native chain must never be changed implicitly, so refuse instead.
    if matches!(target, "redirect" | "dnat") && !matches!(hook, "output" | "prerouting") {
        return Err(error(
            "DNAT and redirect require an output or prerouting chain; choose that chain explicitly",
        ));
    }
    if matches!(target, "redirect" | "tproxy" | "dnat") && matches!(hook, "input" | "postrouting") {
        return Err(error(
            "This verdict cannot be used in an input or postrouting chain",
        ));
    }
    if target == "snat" && hook != "postrouting" {
        return Err(error("SNAT requires a postrouting chain"));
    }
    let parameters = match target {
        "queue" => {
            number(&parameter, 65535, true)?;
            format!("num {parameter}")
        }
        "redirect" | "tproxy" | "snat" | "dnat" => {
            if parameter.is_empty() || !parameter.contains(':') {
                return Err(error("Enter a target in :port or IP:port format"));
            }
            let (ip, port) = parameter.rsplit_once(':').unwrap();
            number(port, 65535, false)?;
            if !ip.is_empty() {
                ip.trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<IpAddr>()
                    .map_err(|_| error("Enter a valid target IP address"))?;
            }
            format!("to {parameter}")
        }
        _ => {
            if !parameter.is_empty() {
                return Err(error(
                    "This guided verdict takes no parameters; retain advanced parameters in the raw editor",
                ));
            }
            String::new()
        }
    };
    Ok((target.into(), parameters))
}
