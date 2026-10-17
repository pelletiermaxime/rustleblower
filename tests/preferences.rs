use rustleblower::{
    preferences::{Popup, Store},
    protocol::Connection,
};

#[test]
fn preferences_roundtrip_validate_and_keep_last_good_memory_on_failed_save() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("history.sqlite3");
    let store = Store::open(&database)?;
    assert_eq!(store.snapshot(), Popup::default());
    let value = Popup {
        duration: "always".into(),
        target: "dest.host".into(),
        popups: false,
        desktop_notifications: false,
        priority: true,
        logging: false,
        ..Default::default()
    };
    store.save(value.clone())?;
    assert_eq!(Store::open(&database)?.snapshot(), value);
    let choice = value.choice(&Connection::default());
    assert_eq!(
        choice.target, "connection",
        "missing host has a usable fallback"
    );
    assert_eq!(choice.duration, "always");
    let pid = Popup {
        target: "process.id".into(),
        ..Popup::default()
    };
    pid.validate()?;
    assert_eq!(
        pid.choice(&Connection {
            process_id: 123,
            ..Default::default()
        })
        .target,
        "process.id"
    );
    let invalid = Popup {
        duration: "not-a-duration".into(),
        ..value.clone()
    };
    assert!(store.save(invalid).is_err());
    assert_eq!(store.snapshot(), value);
    let path = dir.path().join("history.sqlite3.preferences.json");
    std::fs::remove_file(&path)?;
    std::fs::create_dir(&path)?;
    assert!(store.save(Popup::default()).is_err());
    assert_eq!(
        store.snapshot(),
        value,
        "failed file replacement must not publish new memory state"
    );
    Ok(())
}

#[test]
fn malformed_oversized_and_future_preferences_fail_explicitly() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("history.sqlite3");
    let path = dir.path().join("history.sqlite3.preferences.json");
    for bytes in [vec![b' ';4097],b"{}".to_vec(),br#"{"version":6}"#.to_vec(),br#"{"version":1,"duration":"once","target":"unknown","popups":true,"priority":false,"logging":true}"#.to_vec()] {
        std::fs::write(&path,bytes)?;
        assert!(Store::open(&database).is_err());
    }
    Ok(())
}

#[test]
fn advanced_defaults_capture_reported_conditions_and_explain_unavailable_fields()
-> anyhow::Result<()> {
    use rustleblower::{config::Decision, prompts::choice};
    let prefs = Popup {
        duration: "always".into(),
        target: "process.path".into(),
        show_advanced: true,
        constraints: vec![
            "dest.ip".into(),
            "dest.port".into(),
            "user.id".into(),
            "checksum".into(),
        ],
        timeout: Some(2),
        fallback: Some(Decision::Reject),
        ..Default::default()
    };
    prefs.validate()?;
    let mut con = Connection {
        process_path: "/usr/bin/example".into(),
        dst_ip: "192.0.2.17".into(),
        dst_port: 443,
        user_id: 1000,
        process_checksums: [("process.hash.sha1".into(), "f".repeat(40))].into(),
        ..Default::default()
    };
    let initial = prefs.initial(&con, Decision::Reject);
    assert!(initial.expanded && initial.unavailable.is_empty());
    assert_eq!(initial.fallback, Some(Decision::Reject));
    assert_eq!(initial.choice.constraints.len(), 4);
    assert!(
        initial
            .choice
            .constraints
            .contains(&"process.hash.sha1".into())
    );
    let rule =
        choice::rule(&con, Decision::Allow, 1, &initial.choice).map_err(anyhow::Error::msg)?;
    let ops = rule.operator.unwrap().list;
    for key in [
        "process.path",
        "dest.ip",
        "dest.port",
        "user.id",
        "process.hash.sha1",
    ] {
        assert!(ops.iter().any(|op| op.operand == key), "{key}");
    }
    con.process_checksums
        .insert("process.hash.md5".into(), "f".repeat(32));
    assert!(
        prefs
            .choice(&con)
            .constraints
            .contains(&"process.hash.md5".into())
    );
    let missing = Popup {
        show_advanced: false,
        ..prefs
    }
    .initial(&Connection::default(), Decision::Deny);
    assert!(missing.expanded);
    assert_eq!(
        missing.unavailable,
        vec!["Destination IP", "Destination port", "Reported checksum"]
    );
    assert_eq!(missing.choice.constraints, vec!["user.id"]);
    Ok(())
}

#[test]
fn optional_preferences_migrate_old_files_and_reject_invalid_overrides() -> anyhow::Result<()> {
    use rustleblower::config::Decision;
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("history.sqlite3");
    let path = dir.path().join("history.sqlite3.preferences.json");
    let old = serde_json::json!({"version":1,"duration":"once","target":"connection","popups":true,"priority":false,"logging":true});
    std::fs::write(&path, serde_json::to_vec(&old)?)?;
    let store = Store::open(&database)?;
    let legacy = Popup {
        target: "connection".into(),
        ..Popup::default()
    };
    assert_eq!(store.snapshot(), legacy, "saved scope is preserved");
    let mut previous = old.clone();
    previous["version"] = serde_json::json!(2);
    std::fs::write(&path, serde_json::to_vec(&previous)?)?;
    assert_eq!(Store::open(&database)?.snapshot(), legacy);
    let new = Popup {
        timeout: Some(110),
        fallback: Some(Decision::Allow),
        show_advanced: true,
        constraints: vec!["checksum".into()],
        position: rustleblower::prompts::placement::Position::BottomRight,
        ..Default::default()
    };
    store.save(new.clone())?;
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(
        written["version"], 5,
        "old binaries must reject overrides they do not understand"
    );
    assert_eq!(Store::open(&database)?.snapshot(), new);
    for (field, value) in [
        ("timeout", serde_json::json!(0)),
        ("timeout", serde_json::json!(111)),
        ("timeout", serde_json::json!("5")),
        ("fallback", serde_json::json!("always")),
        ("constraints", serde_json::json!(["user.id", "user.id"])),
        ("constraints", serde_json::json!(["unknown"])),
        ("show_advanced", serde_json::json!(1)),
        ("position", serde_json::json!("invalid")),
        ("desktop_notifications", serde_json::json!(1)),
    ] {
        let mut bad = old.clone();
        bad[field] = value;
        std::fs::write(&path, serde_json::to_vec(&bad)?)?;
        assert!(Store::open(&database).is_err(), "{field}");
    }
    Ok(())
}
