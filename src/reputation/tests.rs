use super::*;
use std::os::unix::fs::PermissionsExt;
const KEY: &str = "owned-fixture-key-0123456789abcdef";
fn payload(malicious: u64) -> String {
    format!(
        r#"{{"data":{{"attributes":{{"last_analysis_stats":{{"malicious":{malicious},"suspicious":2,"harmless":60,"undetected":3}},"total_votes":{{"malicious":4,"harmless":1}},"reputation":-3,"echo":"{KEY}"}}}}}}"#
    )
}
fn settings(directory: &Path) -> Settings {
    let path = directory.join("api-key");
    std::fs::write(&path, KEY).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    Settings {
        enabled: true,
        api_key_file: Some(path),
        ..Settings::default()
    }
}
fn subject(index: u64) -> Subject {
    Subject::domain(&format!("host{index}.example")).unwrap()
}
fn query(index: u64) -> Query {
    Query {
        capture: Capture {
            session: 7,
            object_id: index,
        },
        subjects: vec![subject(index)],
    }
}
#[test]
fn settings_subjects_private_ranges_and_exclusions_are_bounded() {
    let mut settings = Settings::default();
    assert!(!settings.enabled);
    assert_eq!(settings.request_budget, 500);
    assert_eq!(settings.timeout_seconds, 1);
    assert!(settings.validate().is_ok());
    assert!(Subject::domain(&"a".repeat(1024)).is_err());
    assert!(Subject::domain("user@host.example/path").is_err());
    assert!(Subject::domain("127.0.0.1").is_err());
    assert_eq!(
        Subject::domain("Example.COM.").unwrap(),
        Subject::Domain("example.com".into())
    );
    for len in [32, 40, 64] {
        assert!(Subject::hash(&"A".repeat(len)).is_ok());
    }
    assert!(Subject::hash(&"x".repeat(64)).is_err());
    for ip in [
        "127.0.0.1",
        "10.0.0.1",
        "172.16.0.1",
        "192.168.0.1",
        "169.254.0.1",
        "224.0.0.1",
        "::1",
        "::",
        "fc00::1",
        "fe80::1",
        "ff02::1",
        "::ffff:192.168.0.1",
    ] {
        assert!(Subject::Ip(ip.parse().unwrap()).excluded(&settings));
    }
    assert!(!Subject::Ip("203.0.113.1".parse().unwrap()).excluded(&settings));
    settings.excluded_domains = vec![".LAN".into()];
    assert!(Subject::domain("nas.lan").unwrap().excluded(&settings));
    settings.excluded_domains = vec!["a".repeat(129)];
    assert!(settings.validate().is_err());
    settings.excluded_domains = vec!["x".into(); 33];
    assert!(settings.validate().is_err());
    assert_ne!(
        path_for(Path::new("history.db")),
        path_for(Path::new("history.sqlite3"))
    );
}
#[test]
fn source_threshold_signed_votes_reputation_and_safe_recommendation_are_exact() {
    let mut settings = Settings {
        enabled: true,
        ..Settings::default()
    };
    let report = parse(subject(1), payload(1).as_bytes(), &settings);
    assert_eq!(report.verdict, Verdict::Warning);
    assert_eq!(
        report.suggested_default,
        Some(crate::config::Decision::Reject)
    );
    settings.minimum_threshold = 10;
    settings.use_suspicious = true;
    settings.use_community_score = true;
    settings.use_reputation = true;
    let report = parse(subject(1), payload(2).as_bytes(), &settings);
    assert_eq!(report.score, Some(10));
    assert_eq!(report.verdict, Verdict::Warning);
    settings.warning_action = WarningAction::None;
    assert_eq!(
        parse(subject(1), payload(2).as_bytes(), &settings).suggested_default,
        None
    );
    settings.enabled = false;
    settings.warning_action = WarningAction::RecommendReject;
    assert_eq!(
        parse(subject(1), payload(2).as_bytes(), &settings).suggested_default,
        None
    );
    let report = parse(subject(1), br#"{"data":{"attributes":{"last_analysis_stats":{"malicious":0,"harmless":0,"undetected":0}}}}"#, &Settings::default());
    assert_eq!(report.verdict, Verdict::Unknown);
    assert_eq!(report.status, Status::NoData);
    assert_eq!(
        parse(subject(1), b"{}", &Settings::default()).verdict,
        Verdict::Unknown
    );
    assert_eq!(
        parse(subject(1), payload(0).as_bytes(), &Settings::default()).verdict,
        Verdict::NotFlagged
    );
}
#[test]
fn secret_file_and_preferences_refuse_disclosure_or_destructive_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let settings = settings(directory.path());
    let key_path = settings.api_key_file.as_ref().unwrap();
    let secret = store::key(Some(key_path)).unwrap();
    assert_eq!(format!("{secret:?}"), "Secret([redacted])");
    std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store::key(Some(key_path)).is_err());
    std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(key_path, &link).unwrap();
    assert!(store::key(Some(&link)).is_err());
    assert!(store::key(Some(directory.path())).is_err());
    let path = directory.path().join("settings.json");
    let saved = store::save(&path, &None, &settings).unwrap();
    assert!(!format!("{saved:?}").contains(KEY));
    assert!(!std::fs::read_to_string(&path).unwrap().contains(KEY));
    assert!(store::save(&path, &None, &settings).is_err());
    std::fs::write(&path, b"invalid private config").unwrap();
    assert!(store::save(&path, &saved.revision, &settings).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"invalid private config");
    assert!(!store::load(&path).settings.enabled);
    std::fs::write(key_path, vec![b'x'; 259]).unwrap();
    assert!(store::key(Some(key_path)).is_err());
}
#[path = "test_network.rs"]
mod network;
