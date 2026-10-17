use super::*;
fn group(directory: &Path, remote: &str) -> Group {
    Group {
        name: "owned resources".into(),
        interval: "60".into(),
        units: Unit::Seconds,
        urls: vec![Resource {
            name: "resource".into(),
            enabled: true,
            remote: remote.into(),
            localfile: directory.join("resource.bin"),
            extra: Default::default(),
        }],
        notify: serde_json::json!({"success":{"desktop":"Published resources"},"error":{"desktop":"Resources failed"}}),
        extra: Default::default(),
    }
}
fn settings(group: Group) -> Settings {
    Settings {
        groups: vec![group],
        ..Default::default()
    }
}
#[test]
fn source_profile_unknown_metadata_and_disabled_import_roundtrip() {
    let directory = tempfile::tempdir().unwrap();
    let source = serde_json::json!({"name":"downloadersActions","vendor":{"opaque":184467440737095516160_u128.to_string()},"actions":{"another":{"enabled":true,"unknown":"retained"},"downloader":{"enabled":true,"future":"kept","config":[{"name":"owned","interval":"0.5","units":"minutes","opaque-group":[1,2],"urls":[{"name":"one","enabled":true,"remote":"https://example.com/raw?token=private","localfile":directory.path().join("a.bin"),"opaque-url":{"x":3}}],"notify":{"success":{"desktop":"done","other":4},"custom":"kept"}}]}}});
    let imported = import_source(&source.to_string()).unwrap();
    assert!(!imported.enabled);
    assert_eq!(
        imported.groups[0].duration().unwrap(),
        Duration::from_secs(30)
    );
    assert!(!format!("{imported:?}").contains("private"));
    let exported: serde_json::Value =
        serde_json::from_str(&export_source(&imported).unwrap()).unwrap();
    assert_eq!(exported["vendor"], source["vendor"]);
    assert_eq!(exported["actions"]["another"], source["actions"]["another"]);
    assert_eq!(exported["actions"]["downloader"]["future"], "kept");
    assert_eq!(exported["actions"]["downloader"]["enabled"], false);
    assert_eq!(
        exported["actions"]["downloader"]["config"][0]["opaque-group"],
        source["actions"]["downloader"]["config"][0]["opaque-group"]
    );
    assert_eq!(
        exported["actions"]["downloader"]["config"][0]["urls"][0]["opaque-url"],
        serde_json::json!({"x":3})
    );
    let mut settings = imported;
    settings.groups[0].interval = "NaN".into();
    assert!(settings.validate().is_err());
    settings.groups[0].interval = "1".into();
    settings.groups[0].urls[0].remote = "https://user:secret@example.com/a".into();
    assert!(settings.validate().is_err());
    assert!(import_source(&" ".repeat(MAX_CONFIG + 1)).is_err());
}
#[test]
fn protected_destinations_aliases_and_stale_preferences_refuse_changes() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("history.sqlite3");
    std::fs::write(&database, b"owned db sentinel").unwrap();
    let guard = store::Guard::new(&database, &directory.path().join("ui.sock"));
    assert!(guard.target(&database, false).is_err());
    assert!(guard.target(&path_for(&database), false).is_err());
    assert!(guard.target(&journal_path(&database), false).is_err());
    assert!(
        guard
            .target(&crate::highlighting::path_for(&database), false)
            .is_err()
    );
    assert!(
        guard
            .target(&directory.path().join("subscriptions/raw"), false)
            .is_err()
    );
    let alias = directory.path().join("db-alias");
    std::fs::hard_link(&database, &alias).unwrap();
    assert!(guard.target(&alias, false).is_err());
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(&database, &link).unwrap();
    assert!(guard.target(&link, false).is_err());
    let settings = settings(group(directory.path(), "https://example.com/resource"));
    let path = path_for(&database);
    let saved = store::save(&path, &None, &settings).unwrap();
    assert!(store::save(&path, &None, &settings).is_err());
    std::fs::write(&path, b"invalid owned configuration").unwrap();
    assert!(store::save(&path, &saved.revision, &settings).is_err());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"invalid owned configuration"
    );
    assert!(!store::load(&path).settings.enabled);
}
#[test]
fn atomic_publication_changed_file_and_owned_stage_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("history.db");
    let guard = store::Guard::new(&database, &directory.path().join("ui.sock"));
    let target = directory.path().join("raw");
    std::fs::write(&target, b"last-good").unwrap();
    let initial = store::stamp(&target).unwrap();
    let mut stage = guard.stage(&target).unwrap();
    stage.write(b"staged").unwrap();
    std::fs::write(&target, b"external change").unwrap();
    assert!(stage.publish(&target, &initial).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"external change");
    let mut stage = guard.stage(&target).unwrap();
    stage.write(b"partial interrupted body").unwrap();
    let path = stage.leave_for_recovery().unwrap();
    let moved = directory.path().join("held-original-stage");
    std::fs::rename(&path, &moved).unwrap();
    std::fs::write(&path, b"foreign replacement").unwrap();
    assert!(guard.recover().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign replacement");
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&moved, &path).unwrap();
    guard.recover().unwrap();
    assert!(!path.exists());
    assert_eq!(std::fs::read(&target).unwrap(), b"external change");
    let expected = store::stamp(&target).unwrap();
    let mut stage = guard.stage(&target).unwrap();
    stage.write(&[0, 255, 1, 2, 3]).unwrap();
    assert_eq!(
        stage.publish(&target, &expected).unwrap(),
        Status::Published
    );
    assert_eq!(std::fs::read(&target).unwrap(), [0, 255, 1, 2, 3]);
    assert!(!journal_path(&database).exists());
}
#[path = "test_network.rs"]
mod network;

#[test]
fn source_file_export_never_clobbers_and_import_is_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("history.db");
    let guard = store::Guard::new(&database, &directory.path().join("ui.sock"));
    let settings = settings(group(directory.path(), "https://example.com/resource"));
    let profile = directory.path().join("profile.json");
    store::write_profile(&guard, &profile, &settings).unwrap();
    let before = std::fs::read(&profile).unwrap();
    assert!(store::write_profile(&guard, &profile, &settings).is_err());
    assert_eq!(std::fs::read(&profile).unwrap(), before);
    assert!(!store::profile(&profile).unwrap().enabled);
    std::fs::write(&profile, vec![b' '; MAX_CONFIG + 1]).unwrap();
    assert!(store::profile(&profile).is_err());
    assert!(store::write_profile(&guard, &database, &settings).is_err());
}

#[test]
fn live_stage_and_journal_replacements_survive_cancel_and_publication_refusal() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("history.db");
    let guard = store::Guard::new(&database, &directory.path().join("ui.sock"));
    let target = directory.path().join("raw");
    std::fs::write(&target, b"last-good").unwrap();
    let stage = guard.stage(&target).unwrap();
    let journal = journal_path(&database);
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
    let stage_path = PathBuf::from(record["path"].as_str().unwrap());
    let held = directory.path().join("held-owned-stage");
    std::fs::rename(&stage_path, &held).unwrap();
    std::fs::write(&stage_path, b"foreign-stage").unwrap();
    drop(stage);
    assert_eq!(std::fs::read(&stage_path).unwrap(), b"foreign-stage");
    assert!(journal.exists());
    std::fs::remove_file(&stage_path).unwrap();
    std::fs::rename(&held, &stage_path).unwrap();
    guard.recover().unwrap();
    let mut stage = guard.stage(&target).unwrap();
    stage.write(b"new-body").unwrap();
    let record = std::fs::read(&journal).unwrap();
    std::fs::write(&journal, b"foreign-journal").unwrap();
    let expected = store::stamp(&target).unwrap();
    assert!(stage.publish(&target, &expected).is_err());
    assert_eq!(std::fs::read(&journal).unwrap(), b"foreign-journal");
    assert_eq!(std::fs::read(&target).unwrap(), b"last-good");
    std::fs::write(&journal, record).unwrap();
    guard.recover().unwrap();
}
