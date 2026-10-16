use rustleblower::appearance::{ColorMode, Store};
use std::{os::unix::fs::symlink, path::Path};
#[test]
fn absent_defaults_and_saved_modes_reopen_without_changing_other_preferences() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("history.sqlite3");
    let store = Store::new(&db);
    let popup = dir.path().join("history.sqlite3.preferences.json");
    std::fs::write(&popup, b"popup fixture").unwrap();
    let first = store.load().unwrap();
    assert_eq!(first.mode, ColorMode::System);
    assert!(first.warning.is_none() && !store.path.exists());
    let mut current = first;
    for mode in [ColorMode::Dark, ColorMode::Light, ColorMode::System] {
        current = store.save(&current, mode).unwrap();
        assert_eq!(store.load().unwrap().mode, mode);
    }
    assert_eq!(std::fs::read(&popup).unwrap(), b"popup fixture");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}
#[test]
fn malformed_future_and_unsupported_modes_fall_back_without_automatic_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(&dir.path().join("db"));
    for text in [
        "{bad",
        r#"{"version":2,"color_mode":"dark"}"#,
        r#"{"version":1,"color_mode":"unknown"}"#,
        r#"{"version":1,"color_mode":"dark","extra":true}"#,
    ] {
        std::fs::write(&store.path, text).unwrap();
        let snapshot = store.load().unwrap();
        assert_eq!(snapshot.mode, ColorMode::System);
        assert!(snapshot.warning.is_some());
        assert_eq!(std::fs::read_to_string(&store.path).unwrap(), text);
        let saved = store.save(&snapshot, ColorMode::Light).unwrap();
        assert_eq!(saved.mode, ColorMode::Light);
        assert!(saved.warning.is_none());
    }
}
#[test]
fn stale_oversized_symlink_and_failed_directory_saves_preserve_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(&dir.path().join("db"));
    let old = store.load().unwrap();
    let first = store.save(&old, ColorMode::Dark).unwrap();
    assert!(store.save(&old, ColorMode::Light).is_err());
    let external = std::fs::read(&store.path).unwrap();
    std::fs::write(&store.path, b"{changed}").unwrap();
    assert!(store.save(&first, ColorMode::Light).is_err());
    assert_eq!(std::fs::read(&store.path).unwrap(), b"{changed}");
    std::fs::write(&store.path, vec![b' '; 4097]).unwrap();
    assert!(store.load().is_err());
    std::fs::remove_file(&store.path).unwrap();
    let target = dir.path().join("keep");
    std::fs::write(&target, &external).unwrap();
    symlink(&target, &store.path).unwrap();
    assert!(store.load().is_err());
    assert_eq!(std::fs::read(&target).unwrap(), external);
    let parent = dir.path().join("not-directory");
    std::fs::write(&parent, b"keep").unwrap();
    let invalid = Store::new(&parent.join(Path::new("db")));
    assert!(invalid.load().is_err());
    assert_eq!(std::fs::read(parent).unwrap(), b"keep");
}
