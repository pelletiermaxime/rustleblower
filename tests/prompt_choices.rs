use prost::Message;
use rustleblower::{
    config::Decision,
    prompts::{
        self, Prompts,
        choice::{self, Choice},
    },
    protocol::Connection,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn connection() -> Connection {
    Connection {
        process_path: "/usr/bin/example".into(),
        process_args: vec!["example".into(), "argument with spaces".into()],
        process_id: 4567,
        user_id: 1000,
        src_ip: "192.0.2.10".into(),
        src_port: 50000,
        dst_ip: "198.51.100.20".into(),
        dst_host: "api.example.test".into(),
        dst_port: 443,
        protocol: "tcp".into(),
        process_checksums: [("process.hash.md5".into(), "f".repeat(32))].into(),
        ..Default::default()
    }
}

#[test]
fn durations_actions_and_advanced_constraints_produce_complete_compatible_rules() {
    for (duration, _) in choice::DURATIONS {
        for decision in [Decision::Allow, Decision::Deny, Decision::Reject] {
            let choice = Choice {
                duration: (*duration).into(),
                target: "process.command".into(),
                constraints: vec![
                    "dest.host".into(),
                    "dest.port".into(),
                    "user.id".into(),
                    "process.hash.md5".into(),
                ],
                priority: true,
                logging: false,
            };
            let rule = choice::rule(&connection(), decision, 1, &choice).unwrap();
            assert_eq!(rule.duration, *duration);
            assert_eq!(rule.action, decision.action());
            assert!(rule.precedence && rule.nolog && rule.enabled);
            assert!(rule.encoded_len() <= prompts::MAX_PROMPT_BYTES);
            let op = rule.operator.unwrap();
            assert_eq!(op.r#type, "list");
            let json: serde_json::Value = serde_json::from_str(&op.data).unwrap();
            assert_eq!(json.as_array().unwrap().len(), op.list.len());
            assert!(
                op.list
                    .iter()
                    .any(|op| op.operand == "process.path" && op.data == "/usr/bin/example"),
                "relative commands retain the executable constraint"
            );
            for field in [
                "process.command",
                "dest.host",
                "dest.port",
                "user.id",
                "process.hash.md5",
            ] {
                assert!(op.list.iter().any(|op| op.operand == field), "{field}");
            }
        }
    }
}

#[test]
fn generated_domain_network_appimage_and_checksum_targets_match_the_observed_context() {
    let mut con = connection();
    con.dst_ip = "2001:db8:1234:5678::42".into();
    let targets = choice::targets(&con);
    assert!(
        targets
            .iter()
            .any(|target| target.key == "network:2001:db8:1234:5678::/64")
    );
    let domain = targets
        .iter()
        .find(|target| target.key == "domain:example.test")
        .unwrap();
    let regex = regex::Regex::new(&domain.operators[0].data).unwrap();
    for host in ["api.example.test", "example.test", "deep.api.example.test"] {
        assert!(regex.is_match(host));
    }
    assert!(!regex.is_match("exampleXtest") && !regex.is_match("example.test.attacker.invalid"));
    con.process_path = "/tmp/.mount_ExampleAb1234/usr/bin/example".into();
    let targets = choice::targets(&con);
    let appimage = targets
        .iter()
        .find(|target| target.key == "appimage")
        .unwrap();
    let regex = regex::Regex::new(&appimage.operators[0].data).unwrap();
    assert!(
        regex.is_match(&con.process_path)
            && regex.is_match("/tmp/.mount_ExampleCd5678/usr/bin/example")
    );
    assert!(!regex.is_match("/tmp/.mount_ExampleCd5678/usr/bin/other"));
    con.process_path = "/tmp/.mount_éééé/usr/bin/example".into();
    let _ = choice::targets(&con);
    con = connection();
    con.dst_ip = "192.168.1.20".into();
    let targets = choice::targets(&con);
    let alias = targets
        .iter()
        .find(|target| target.key == "alias:LAN")
        .unwrap();
    assert!(
        alias
            .operators
            .iter()
            .any(|op| op.operand == "process.path")
    );
}

#[tokio::test]
async fn invalid_expired_and_racing_choices_cannot_resolve_the_wrong_request() {
    let prompts = Arc::new(Prompts::default());
    let (prompt, mut reply) = prompts
        .insert(1, connection(), Instant::now() + Duration::from_secs(2))
        .unwrap();
    let invalid = Choice {
        duration: "invalid".into(),
        ..Default::default()
    };
    assert!(
        prompts
            .choose(prompt.id, Decision::Allow, &invalid)
            .is_err()
    );
    assert!(reply.try_recv().is_err());
    let a = prompts.clone();
    let b = prompts.clone();
    let id = prompt.id;
    let first =
        std::thread::spawn(move || a.choose(id, Decision::Allow, &Choice::default()).unwrap());
    let second =
        std::thread::spawn(move || b.choose(id, Decision::Reject, &Choice::default()).unwrap());
    assert_ne!(first.join().unwrap(), second.join().unwrap());
    let result = reply.await.unwrap();
    assert!(result.rule.is_some());
    assert!(prompts.snapshot().is_empty());
    let (expired, _) = prompts.insert(1, connection(), Instant::now()).unwrap();
    assert!(!prompts.review(expired.id));
    assert!(
        !prompts
            .choose(expired.id, Decision::Allow, &Choice::default())
            .unwrap()
    );
    prompts.remove(expired.id);
    let (fresh, _) = prompts
        .insert(2, connection(), Instant::now() + Duration::from_secs(2))
        .unwrap();
    assert!(
        !prompts
            .choose(id, Decision::Allow, &Choice::default())
            .unwrap()
    );
    assert_eq!(prompts.snapshot()[0].id, fresh.id);
    assert!(!prompts.review(id));
    assert!(!prompts.snapshot()[0].reviewing);
}

#[test]
fn tiny_encoded_messages_with_large_retained_allocations_cannot_fill_the_prompt_queue() {
    let mut con = connection();
    con.process_args = Vec::with_capacity(100_000);
    assert!(con.encoded_len() < prompts::MAX_PROMPT_BYTES);
    assert!(!prompts::valid_connection(&con));
    assert!(
        Prompts::default()
            .insert(1, con, Instant::now() + Duration::from_secs(1))
            .is_none()
    );
    let mut con = connection();
    con.process_path = "x".repeat(60 * 1024);
    let rule = prompts::once_rule(&con, Decision::Deny, 1);
    assert_eq!(rule.duration, "once");
    assert!(rule.encoded_len() < prompts::MAX_PROMPT_BYTES);
}
