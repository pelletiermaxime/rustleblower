use rustleblower::appearance::{ColorMode, DensityScale, PaletteColor, PaletteId, Store};
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
    assert_eq!(first.density, DensityScale::default());
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
fn legacy_color_only_documents_keep_their_mode_and_default_density() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(&dir.path().join("db"));
    let legacy = br#"{"version":1,"color_mode":"dark"}"#;
    std::fs::write(&store.path, legacy).unwrap();
    let snapshot = store.load().unwrap();
    assert_eq!(snapshot.mode, ColorMode::Dark);
    assert_eq!(snapshot.density, DensityScale::default());
    assert!(snapshot.warning.is_none());
    assert_eq!(std::fs::read(&store.path).unwrap(), legacy);
    let saved = store.save(&snapshot, ColorMode::Light).unwrap();
    assert_eq!(saved.density, DensityScale::default());
    assert_eq!(saved.mode, ColorMode::Light);
}
#[test]
fn source_density_choices_roundtrip_independently_and_reject_out_of_range_values() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(&dir.path().join("db"));
    let mut snapshot = store.load().unwrap();
    for value in DensityScale::MIN..=DensityScale::MAX {
        let density = DensityScale::new(value).unwrap();
        snapshot = store
            .save_preferences(&snapshot, ColorMode::Dark, density)
            .unwrap();
        let reopened = store.load().unwrap();
        assert_eq!(
            (reopened.mode, reopened.density.value()),
            (ColorMode::Dark, value)
        );
        assert_eq!(density.percent(), (100 + i16::from(value) * 2) as u8);
    }
    let light = store.save(&snapshot, ColorMode::Light).unwrap();
    assert_eq!(light.density.value(), DensityScale::MAX);
    let reset = store
        .save_preferences(&light, light.mode, DensityScale::default())
        .unwrap();
    assert_eq!(reset.mode, ColorMode::Light);
    assert_eq!(reset.density.percent(), 100);
    assert!(DensityScale::new(-21).is_none());
    assert!(DensityScale::new(21).is_none());
    for value in ["-21", "21", "9999", "1.5", "\"1\"", "null"] {
        let bytes = format!(r#"{{"version":1,"color_mode":"dark","density_scale":{value}}}"#);
        std::fs::write(&store.path, &bytes).unwrap();
        let invalid = store.load().unwrap();
        assert_eq!(
            (invalid.mode, invalid.density),
            (ColorMode::System, DensityScale::default())
        );
        assert!(invalid.warning.is_some());
        assert_eq!(std::fs::read_to_string(&store.path).unwrap(), bytes);
    }
}
#[test]
fn stale_density_save_preserves_external_mode_and_density_then_reload_can_save() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(&dir.path().join("db"));
    let first = store.load().unwrap();
    let saved = store
        .save_preferences(&first, ColorMode::Dark, DensityScale::new(-7).unwrap())
        .unwrap();
    let external = store
        .save_preferences(&saved, ColorMode::Light, DensityScale::new(12).unwrap())
        .unwrap();
    let bytes = std::fs::read(&store.path).unwrap();
    assert!(
        store
            .save_preferences(&saved, ColorMode::Dark, DensityScale::new(1).unwrap())
            .is_err()
    );
    assert_eq!(std::fs::read(&store.path).unwrap(), bytes);
    assert!(
        store
            .save_preferences(&first, ColorMode::System, DensityScale::default())
            .is_err()
    );
    let reloaded = store.load().unwrap();
    assert_eq!(
        (reloaded.mode, reloaded.density),
        (external.mode, external.density)
    );
    let reset = store
        .save_preferences(&reloaded, reloaded.mode, DensityScale::default())
        .unwrap();
    assert_eq!(
        (reset.mode, reset.density.percent()),
        (ColorMode::Light, 100)
    );
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

fn palette_document() -> serde_json::Value {
    let mut colors = serde_json::Map::new();
    for name in [
        "background",
        "foreground",
        "primary",
        "primary_foreground",
        "secondary",
        "secondary_foreground",
        "muted",
        "muted_foreground",
        "accent",
        "accent_foreground",
        "border",
        "danger",
        "danger_foreground",
    ] {
        colors.insert(name.into(), serde_json::Value::String("#123456".into()));
    }
    serde_json::json!({"version":1,"name":"Ocean","mode":"dark","colors":colors})
}
#[test]
fn native_palette_catalogue_and_selection_reopen_preserve_density_and_external_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(&dir.path().join("history.sqlite3"));
    assert_eq!(
        store.themes_path(),
        dir.path().join("history.sqlite3.themes")
    );
    assert!(store.catalogue().unwrap().palettes.is_empty());
    std::fs::create_dir(store.themes_path()).unwrap();
    let path = store.themes_path().join("ocean.json");
    let bytes = serde_json::to_vec(&palette_document()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let catalogue = store.catalogue().unwrap();
    assert!(catalogue.warning.is_none());
    assert_eq!(catalogue.palettes.len(), 1);
    let id = catalogue.palettes[0].id.clone();
    assert_eq!(id.as_str(), "ocean");
    assert_eq!(
        catalogue.palettes[0].document.colors.background.rgb(),
        0x123456
    );
    let initial = store.load().unwrap();
    let saved = store
        .save_appearance(
            &initial,
            ColorMode::Dark,
            DensityScale::new(-8).unwrap(),
            Some(id.clone()),
        )
        .unwrap();
    let recolored = store.save(&saved, ColorMode::Light).unwrap();
    assert_eq!(recolored.palette, Some(id.clone()));
    assert_eq!(recolored.density.value(), -8);
    let resized = store
        .save_preferences(&recolored, recolored.mode, DensityScale::new(9).unwrap())
        .unwrap();
    assert_eq!(resized.palette, Some(id));
    std::fs::remove_file(&path).unwrap();
    assert!(store.catalogue().unwrap().palettes.is_empty());
    assert_eq!(store.load().unwrap().palette, resized.palette);
    assert_eq!(store.load().unwrap().density.value(), 9);
    std::fs::write(&path, &bytes).unwrap();
    let reset = store
        .save_appearance(&resized, ColorMode::System, resized.density, None)
        .unwrap();
    assert_eq!(
        (reset.mode, reset.density.value(), reset.palette),
        (ColorMode::System, 9, None)
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}
#[test]
fn palette_catalogue_rejects_nondata_invalid_oversized_symlink_and_unbounded_catalogues() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(&dir.path().join("db"));
    std::fs::create_dir(store.themes_path()).unwrap();
    let path = store.themes_path().join("invalid.json");
    let valid = palette_document();
    let mut documents = Vec::new();
    for (key, value) in [
        ("mode", serde_json::json!("system")),
        ("version", serde_json::json!(2)),
        ("name", serde_json::json!("bad\nname")),
        ("name", serde_json::json!("x".repeat(81))),
        ("script", serde_json::json!("execute")),
    ] {
        let mut doc = valid.clone();
        doc[key] = value;
        documents.push(doc);
    }
    for value in ["url(file:///etc/passwd)", "#ffff", "#+00000", "#00000000"] {
        let mut doc = valid.clone();
        doc["colors"]["background"] = serde_json::json!(value);
        documents.push(doc);
    }
    for document in documents {
        let bytes = serde_json::to_vec(&document).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let catalogue = store.catalogue().unwrap();
        assert!(catalogue.palettes.is_empty() && catalogue.warning.is_some());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    std::fs::write(&path, vec![b' '; 16 * 1024 + 1]).unwrap();
    assert!(store.catalogue().unwrap().palettes.is_empty());
    std::fs::remove_file(&path).unwrap();
    let target = dir.path().join("preserved.json");
    std::fs::write(&target, serde_json::to_vec(&valid).unwrap()).unwrap();
    symlink(&target, &path).unwrap();
    assert!(store.catalogue().unwrap().palettes.is_empty());
    assert!(target.exists());
    std::fs::remove_file(path).unwrap();
    for index in 0..33 {
        std::fs::write(
            store.themes_path().join(format!("palette-{index}.json")),
            serde_json::to_vec(&valid).unwrap(),
        )
        .unwrap();
    }
    assert!(store.catalogue().unwrap_err().to_string().contains("32"));
    for index in 0..32 {
        std::fs::write(store.themes_path().join(format!("other-{index}")), b"keep").unwrap();
    }
    assert!(store.catalogue().unwrap_err().to_string().contains("64"));
    for id in ["../escape", "a/b", "", "bad.name", "file:theme"] {
        assert!(PaletteId::try_from(id.to_owned()).is_err());
    }
    assert!(PaletteColor::try_from("#00aaFF".to_owned()).is_ok());
    let mut invalid = serde_json::json!({"version":1,"color_mode":"dark","palette_id":"../escape"});
    std::fs::write(&store.path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    assert!(store.load().unwrap().warning.is_some());
    invalid["palette_id"] = serde_json::json!("safe-id");
    std::fs::write(&store.path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    assert_eq!(store.load().unwrap().palette.unwrap().as_str(), "safe-id");
}
