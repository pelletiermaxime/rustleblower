use rustleblower::{
    firewall::{
        self,
        document::{Chain, Command, Document, Group, Rule},
        helpers::{self, Kind},
    },
    protocol,
};

// Expected wire tuples are from a1353848 ui/.../firewall_rule/dialog.py:416-557,
// not from Rust serialization. Order and empty marker values are significant.
#[test]
fn all_source_statement_families_have_exact_ordered_wire_values() {
    let cases = [
        (
            Kind::DestinationPort,
            "tcp,udp",
            "80,443",
            "meta",
            vec![("l4proto", "tcp,udp"), ("dport", "80,443")],
        ),
        (
            Kind::SourcePort,
            "udp",
            "1000-2000",
            "udp",
            vec![("sport", "1000-2000")],
        ),
        (
            Kind::DestinationAddress,
            "ip",
            "192.0.2.0/24",
            "ip",
            vec![("daddr", "192.0.2.0-192.0.2.255")],
        ),
        (
            Kind::SourceAddress,
            "ip6",
            "2001:db8::1",
            "ip6",
            vec![("saddr", "2001:db8::1")],
        ),
        (
            Kind::InputInterface,
            "default",
            "en*",
            "iifname",
            vec![("en*", "")],
        ),
        (
            Kind::OutputInterface,
            "default",
            "eth0",
            "oifname",
            vec![("eth0", "")],
        ),
        (
            Kind::SetConnectionMark,
            "default",
            "123",
            "ct",
            vec![("set", ""), ("mark", "123")],
        ),
        (
            Kind::ConnectionMark,
            "default",
            "123",
            "ct",
            vec![("mark", "123")],
        ),
        (
            Kind::ConnectionState,
            "default",
            "related,new",
            "ct",
            vec![("state", "related,new")],
        ),
        (
            Kind::SetPacketMark,
            "default",
            "123",
            "meta",
            vec![("set", ""), ("mark", "123")],
        ),
        (
            Kind::PacketInformation,
            "skuid",
            "0",
            "meta",
            vec![("skuid", "0")],
        ),
        (
            Kind::Icmp,
            "default",
            "echo-request,echo-reply",
            "icmp",
            vec![("type", "echo-request,echo-reply")],
        ),
        (
            Kind::IcmpV6,
            "default",
            "neighbour-solicitation",
            "icmpv6",
            vec![("type", "neighbour-solicitation")],
        ),
        (
            Kind::Log,
            "warn",
            "log prefix",
            "log",
            vec![("level", "warn"), ("prefix", "logprefix")],
        ),
        (
            Kind::Quota,
            "over",
            "10/mbytes",
            "quota",
            vec![("over", ""), ("mbytes", "10")],
        ),
        (
            Kind::Counter,
            "default",
            "web",
            "counter",
            vec![("packets", ""), ("name", "web")],
        ),
        (
            Kind::Limit,
            "over",
            "10/kbytes/minute",
            "limit",
            vec![
                ("over", ""),
                ("units", "10"),
                ("rate-units", "kbytes"),
                ("time-units", "minute"),
            ],
        ),
    ];
    for (kind, option, input, name, values) in cases {
        let expression = helpers::statement(kind, option, "!=", input).unwrap();
        let wire: protocol::Expressions = expression.into();
        let s = wire.statement.unwrap();
        assert_eq!(s.name, name, "{kind:?}");
        assert_eq!(
            s.op,
            if kind.has_operator() { "!=" } else { "==" },
            "{kind:?}"
        );
        assert_eq!(
            s.values
                .into_iter()
                .map(|v| (v.key, v.value))
                .collect::<Vec<_>>(),
            values
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect::<Vec<(String, String)>>(),
            "{kind:?}"
        );
    }
    for kind in [Kind::Quota, Kind::Limit] {
        let text = if kind == Kind::Quota {
            "1/bytes"
        } else {
            "1/bytes/second"
        };
        assert!(
            !helpers::statement(kind, "until", "==", text)
                .unwrap()
                .statement
                .unwrap()
                .values
                .iter()
                .any(|v| v.key == "over")
        );
    }
}

#[test]
fn networks_are_bounded_arithmetic_and_invalid_inputs_stay_out_of_drafts() {
    for (family, input, expected) in [
        ("ip", "0.0.0.0/0", "0.0.0.0-255.255.255.255"),
        ("ip6", "::/0", "::-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
        ("ip6", "2001:db8::/126", "2001:db8::-2001:db8::3"),
        ("ip6", "::1/128", "::1-::1"),
    ] {
        assert_eq!(
            helpers::statement(Kind::DestinationAddress, family, "==", input)
                .unwrap()
                .statement
                .unwrap()
                .values[0]
                .value,
            expected
        );
    }
    for (kind, option, input) in [
        (Kind::DestinationAddress, "ip6", "192.0.2.1"),
        (Kind::DestinationAddress, "ip", "192.0.2.1/24"),
        (Kind::DestinationAddress, "ip", "192.0.2.20-192.0.2.1"),
        (Kind::DestinationAddress, "ip6", "::/129"),
        (Kind::DestinationPort, "tcp", "0"),
        (Kind::DestinationPort, "tcp", "65536"),
        (Kind::DestinationPort, "tcp", "443-80"),
        (Kind::DestinationPort, "tcp", "80,,443"),
        (Kind::DestinationPort, "tcp", "https"),
        (Kind::SetConnectionMark, "default", "4294967296"),
        (Kind::ConnectionState, "default", "new,bogus"),
        (Kind::InputInterface, "default", "eth0;drop"),
        (Kind::Icmp, "default", "bogus"),
        (Kind::Quota, "over", "1/megabytes"),
        (Kind::Quota, "over", "-1/bytes"),
        (Kind::Limit, "over", "1/bytes/week"),
        (Kind::Limit, "over", "0/bytes/hour"),
        (Kind::PacketInformation, "skuid", "-1"),
    ] {
        assert!(
            helpers::statement(kind, option, "==", input).is_err(),
            "{kind:?} {input}"
        );
    }
    assert!(helpers::statement(Kind::Log, "warn", "==", &"x".repeat(1025)).is_err());
    assert!(helpers::statement(Kind::Log, "warn", "==", "first\nsecond").is_err());
    assert!(helpers::statement(Kind::Log, "warn", "in", "prefix").is_err());
    assert!(helpers::statement(Kind::SourcePort, "tcp", "==", &vec!["1"; 129].join(",")).is_err());
}

#[test]
fn guided_verdicts_check_hook_parameters_and_never_infer_a_new_chain() {
    for (target, parameter, hook, expected) in [
        ("queue", "0", "input", "num 0"),
        ("queue", "42", "output", "num 42"),
        ("dnat", "192.0.2.1:443", "prerouting", "to 192.0.2.1:443"),
        (
            "snat",
            "[2001:db8::1]:80",
            "postrouting",
            "to [2001:db8::1]:80",
        ),
        ("redirect", ":8080", "output", "to :8080"),
        ("tproxy", ":3128", "prerouting", "to :3128"),
        ("accept", "", "input", ""),
        ("reject", "", "input", ""),
        ("stop", "", "output", ""),
    ] {
        assert_eq!(
            helpers::verdict(target, parameter, hook).unwrap(),
            (target.into(), expected.into())
        );
    }
    for (target, parameter, hook) in [
        ("dnat", ":443", "input"),
        ("redirect", ":80", "postrouting"),
        ("tproxy", ":80", "input"),
        ("snat", ":80", "output"),
        ("queue", "65536", "input"),
        ("queue", "not-a-number", "input"),
        ("dnat", "192.0.2.1", "output"),
        ("dnat", "garbage:443", "output"),
        ("dnat", ":0", "output"),
        ("accept", "to :443", "input"),
    ] {
        assert!(helpers::verdict(target, parameter, hook).is_err());
    }
}

#[test]
fn upstream_nat_direction_reachability_is_refused_without_retargeting_chain() {
    for target in ["dnat", "redirect"] {
        for hook in ["input", "forward", "postrouting", "", "custom-hook"] {
            assert!(
                helpers::verdict(target, ":443", hook).is_err(),
                "{target} on {hook}"
            );
        }
        for hook in ["output", "prerouting"] {
            assert_eq!(
                helpers::verdict(target, ":443", hook).unwrap(),
                (target.into(), "to :443".into())
            );
        }
    }
    // The generic document model still retains advanced raw combinations.
    let rule = Rule {
        uuid: "raw-dnat".into(),
        target: "dnat".into(),
        target_parameters: "to :443".into(),
        ..Default::default()
    };
    let mut doc = Document {
        enabled: true,
        version: 1,
        system_rules: vec![Group {
            rule: None,
            chains: vec![Chain {
                name: "forward".into(),
                table: "opensnitch".into(),
                family: "inet".into(),
                kind: "filter".into(),
                hook: "forward".into(),
                policy: "accept".into(),
                ..Default::default()
            }],
        }],
    };
    doc.command(Command::Rule {
        group: 0,
        chain: 0,
        index: None,
        rule: rule.clone(),
    })
    .unwrap();
    assert_eq!(doc.chain(0, 0).unwrap().hook, "forward");
    assert_eq!(doc.chain(0, 0).unwrap().rules[0], rule);
}

#[test]
fn helper_drafts_preserve_complete_unknown_raw_fields_through_wire_roundtrip() {
    let unknown = rustleblower::firewall::document::Expression {
        statement: Some(rustleblower::firewall::document::Statement {
            name: "future-statement".into(),
            op: "custom".into(),
            values: vec![rustleblower::firewall::document::Value {
                key: "future-key".into(),
                value: "future-value".into(),
            }],
        }),
    };
    let original = Rule {
        uuid: "owned-rule".into(),
        enabled: true,
        position: u64::MAX,
        target: "accept".into(),
        parameters: "legacy untouched".into(),
        expressions: vec![unknown.clone()],
        ..Default::default()
    };
    let mut doc = Document {
        enabled: true,
        version: 1,
        system_rules: vec![Group {
            rule: None,
            chains: vec![Chain {
                name: "input".into(),
                table: "opensnitch".into(),
                family: "inet".into(),
                priority: "0".into(),
                kind: "filter".into(),
                hook: "input".into(),
                policy: "accept".into(),
                rules: vec![original.clone()],
            }],
        }],
    };
    let mut draft = original.clone();
    draft
        .expressions
        .push(helpers::statement(Kind::DestinationPort, "tcp,udp", "==", "80,443").unwrap());
    doc.command(Command::Rule {
        group: 0,
        chain: 0,
        index: Some(0),
        rule: draft.clone(),
    })
    .unwrap();
    let wire: protocol::SysFirewall = doc.clone().into();
    firewall::validate(&wire).unwrap();
    let roundtrip = Document::from(wire);
    assert_eq!(roundtrip, doc);
    let saved = &roundtrip.chain(0, 0).unwrap().rules[0];
    assert_eq!(saved.expressions[0], unknown);
    assert_eq!(saved.position, u64::MAX);
    assert_eq!(saved.parameters, original.parameters);
    assert_eq!(saved.uuid, original.uuid);
    assert_eq!(saved.expressions.len(), 2);
}
