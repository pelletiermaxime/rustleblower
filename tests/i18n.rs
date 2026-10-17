use rustleblower::{
    appearance::{ColorMode, DensityScale, PaletteId, Store},
    i18n::{Catalogue, LanguageChoice, LocaleId, MAX_BYTES, system_locale},
};
use serde_json::json;
fn locale(id: &str) -> LocaleId {
    LocaleId::try_from(id.to_owned()).unwrap()
}
fn choice(id: &str) -> LanguageChoice {
    LanguageChoice::Locale(locale(id))
}
fn document(id: &str, messages: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"version":1,"locale":id,"name":"Native name","messages":messages}))
        .unwrap()
}
#[test]
fn locale_system_precedence_normalization_and_fallback() {
    assert_eq!(locale("FR_ca").as_str(), "fr-CA");
    assert_eq!(locale("zh_hans_CN").as_str(), "zh-Hans-CN");
    for invalid in [
        "",
        "../fr",
        "fr--CA",
        "é",
        "a",
        "fr/FR",
        "fr-123456789",
        "system",
    ] {
        assert!(LocaleId::try_from(invalid.to_owned()).is_err(), "{invalid}");
    }
    assert_eq!(
        system_locale(Some(""), Some("fr_CA.UTF-8"), Some("de_DE")).as_str(),
        "fr-CA"
    );
    assert_eq!(
        system_locale(Some("C.UTF-8"), Some("fr_FR"), None).as_str(),
        "en"
    );
    assert_eq!(system_locale(None, None, Some("POSIX")).as_str(), "en");
    assert_eq!(
        system_locale(None, None, Some("zh_Hans_CN@modifier")).as_str(),
        "zh-Hans-CN"
    );
    let catalogue = Catalogue::bundled();
    let active = catalogue.activate(&LanguageChoice::System, &locale("fr-CA"));
    assert_eq!(active.locale.as_str(), "fr");
    assert_eq!(active.text("language.title"), "Langue");
    assert_eq!(
        active
            .format("language.active", &[("language", "%{error}")])
            .unwrap(),
        "Langue actuelle : %{error}"
    );
    assert!(active.format("language.active", &[]).is_err());
    assert!(
        active
            .format("language.active", &[("language", "a"), ("language", "b")])
            .is_err()
    );
    assert!(active.format("language.active", &[("error", "x")]).is_err());
    assert!(
        active
            .format("language.active", &[("language", &"x".repeat(4097))])
            .is_err()
    );
    let missing = catalogue.activate(&choice("xx-YY"), &locale("fr"));
    assert_eq!(missing.locale.as_str(), "en");
    assert!(missing.warning.is_some());
    assert_eq!(missing.text("future.missing.key"), "future.missing.key");
    assert!(
        catalogue
            .languages
            .iter()
            .filter(|l| matches!(l.id.as_str(), "en" | "fr"))
            .all(|l| l.translated == l.total)
    );
}
#[test]
fn bounded_discovery_partial_fallback_and_selected_snapshot_recheck() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("de-DE.json");
    std::fs::write(
        &path,
        document("de-DE", json!({"language.title":"Sprache"})),
    )
    .unwrap();
    let catalogue = Catalogue::discover(dir.path()).unwrap();
    assert_eq!(
        catalogue.languages.len(),
        Catalogue::bundled().languages.len() + 1
    );
    let active = catalogue.activate(&choice("de-DE"), &locale("en"));
    assert_eq!(active.locale.as_str(), "de-DE");
    assert_eq!(active.text("language.title"), "Sprache");
    assert_eq!(active.text("language.system"), "System");
    std::fs::write(
        &path,
        document("de-DE", json!({"language.title":"Changed"})),
    )
    .unwrap();
    let stale = catalogue.activate(&choice("de-DE"), &locale("en"));
    assert_eq!(stale.locale.as_str(), "en");
    assert!(stale.warning.unwrap().contains("changed"));
    let fresh = Catalogue::discover(dir.path()).unwrap();
    assert_eq!(
        fresh
            .activate(&choice("de-DE"), &locale("en"))
            .text("language.title"),
        "Changed"
    );
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        fresh
            .activate(&choice("de-DE"), &locale("en"))
            .locale
            .as_str(),
        "en"
    );
    for n in 0..33 {
        std::fs::write(dir.path().join(format!("x{n}.json")), b"{}").unwrap();
    }
    assert!(Catalogue::discover(dir.path()).is_err());
}
#[test]
fn rejects_duplicate_unknown_keys_placeholders_bounds_and_symlinks() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let invalid=[
        document("de",json!({"language.active":"Bad %{error}"})),
        document("de",json!({"language.active":"Bad %{language"})),
        document("de",json!({"unknown.key":"Bad"})),
        document("de",json!({"language.title":"x".repeat(2049)})),
        br#"{"version":1,"locale":"de","name":"Deutsch","messages":{"language.title":"A","language.title":"B"}}"#.to_vec(),
        vec![b'x';MAX_BYTES+1],
    ];
    for bytes in invalid {
        std::fs::write(dir.path().join("de.json"), bytes).unwrap();
        let catalogue = Catalogue::discover(dir.path()).unwrap();
        assert_eq!(
            catalogue.languages.len(),
            Catalogue::bundled().languages.len()
        );
        assert!(catalogue.warning.is_some());
    }
    std::fs::remove_file(dir.path().join("de.json")).unwrap();
    symlink("/etc/passwd", dir.path().join("de.json")).unwrap();
    assert!(Catalogue::discover(dir.path()).unwrap().warning.is_some());
    let directory_link = dir.path().join("link");
    symlink(dir.path(), &directory_link).unwrap();
    assert!(Catalogue::discover(&directory_link).is_err());
}
#[test]
fn language_saves_preserve_appearance_and_stale_file_protection() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(&dir.path().join("db"));
    let first = store.load().unwrap();
    assert_eq!(first.language, LanguageChoice::System);
    let custom = store
        .save_appearance(
            &first,
            ColorMode::Dark,
            DensityScale::new(7).unwrap(),
            Some(PaletteId::try_from("custom".to_owned()).unwrap()),
        )
        .unwrap();
    let french = store.save_language(&custom, choice("fr")).unwrap();
    assert_eq!(french.language, choice("fr"));
    assert_eq!(french.mode, ColorMode::Dark);
    assert_eq!(french.density.value(), 7);
    assert_eq!(french.palette, custom.palette);
    let light = store.save(&french, ColorMode::Light).unwrap();
    assert_eq!(light.language, choice("fr"));
    let before = std::fs::read(&store.path).unwrap();
    assert!(
        store
            .save_language(&french, LanguageChoice::System)
            .is_err()
    );
    assert_eq!(std::fs::read(&store.path).unwrap(), before);
    let system = store.save_language(&light, LanguageChoice::System).unwrap();
    assert_eq!(system.palette, custom.palette);
    assert_eq!(system.density.value(), 7);
    std::fs::write(&store.path, br#"{"version":1,"color_mode":"dark"}"#).unwrap();
    assert_eq!(store.load().unwrap().language, LanguageChoice::System);
    std::fs::write(
        &store.path,
        br#"{"version":1,"color_mode":"dark","language":"../../fr"}"#,
    )
    .unwrap();
    let invalid = store.load().unwrap();
    assert!(invalid.warning.is_some());
    assert_eq!(invalid.mode, ColorMode::Dark);
    assert_eq!(invalid.language, LanguageChoice::System);
}
#[cfg(feature = "desktop")]
#[test]
fn pinned_public_kit_extension_translates_all_116_keys_without_vendoring() {
    rustleblower::i18n::register_kit();
    rustleblower::i18n::register_kit();
    rustleblower::i18n::install(Catalogue::bundled().activate(&choice("fr"), &locale("en")));
    let keys: serde_json::Value =
        serde_json::from_slice(include_bytes!("../locales/fr.json")).unwrap();
    let messages = keys["messages"].as_object().unwrap();
    let mut count = 0;
    for (key, value) in messages {
        if let Some(kit_key) = key.strip_prefix("gpui_component.") {
            assert_eq!(
                gpui_kit::component::_rust_i18n_translate("fr", kit_key).as_ref(),
                value.as_str().unwrap()
            );
            count += 1;
        }
    }
    assert_eq!(count, 116);
    assert_eq!(&*gpui_kit::component::locale(), "fr");
    use gpui_kit::component::searchable_list::SearchableListItem;
    use rustleblower::{
        history::query::{Comparison, Field},
        i18n::LocalizedOption,
    };
    let cwd = LocalizedOption::new(Field::Cwd, Field::Cwd.label());
    let action = LocalizedOption::new(rustleblower::rules::query::Field::Action, "Action");
    let comparison = LocalizedOption::new(Comparison::AtLeast, "At least / at or after");
    assert_eq!(cwd.title().as_ref(), "Dossier de travail");
    assert_eq!(*cwd.value(), Field::Cwd);
    assert_eq!(*action.value(), rustleblower::rules::query::Field::Action);
    assert_eq!(comparison.title().as_ref(), "Au moins / à partir de");
    assert_eq!(*comparison.value(), Comparison::AtLeast);
    for (id, bytes) in [
        ("en", include_bytes!("../locales/en.json").as_slice()),
        ("de", include_bytes!("../locales/de.json").as_slice()),
        ("es", include_bytes!("../locales/es.json").as_slice()),
        ("ja", include_bytes!("../locales/ja.json").as_slice()),
    ] {
        rustleblower::i18n::install(Catalogue::bundled().activate(&choice(id), &locale("en")));
        let doc: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        let mut count = 0;
        for (key, value) in doc["messages"].as_object().unwrap() {
            if let Some(kit_key) = key.strip_prefix("gpui_component.") {
                assert_eq!(
                    gpui_kit::component::_rust_i18n_translate(id, kit_key).as_ref(),
                    value.as_str().unwrap()
                );
                count += 1;
            }
        }
        assert_eq!(count, 116, "{id}");
        assert_eq!(&*gpui_kit::component::locale(), id);
    }
    rustleblower::i18n::install(Default::default());
    assert_eq!(cwd.title().as_ref(), "Working directory");
    assert_eq!(*cwd.value(), Field::Cwd);
}

#[test]
fn presentation_translates_only_reviewed_contracts_and_preserves_arguments() {
    let active = Catalogue::bundled().activate(&choice("fr"), &locale("en"));
    assert_eq!(
        active.presentation("Waiting to queue the task stop; retrying: literal %{count} /owned/é"),
        "En attente de la mise en file de l’arrêt de la tâche ; nouvelle tentative : literal %{count} /owned/é"
    );
    assert_eq!(
        active.presentation("Wait for the counter page to finish loading"),
        "Attendez la fin du chargement de la page des compteurs"
    );
    let oversized = format!(
        "Waiting to queue the task stop; retrying: {}",
        "x".repeat(4097)
    );
    assert_eq!(active.presentation(&oversized), oversized);
    assert_eq!(
        active.presentation("Ports must be between 0 and 65535"),
        "Les ports doivent être compris entre 0 et 65535"
    );
    assert_eq!(
        active.presentation("Ignored 3 invalid or unreadable native palette file(s)."),
        "3 fichier(s) de palette native invalide(s) ou illisible(s) ignoré(s)."
    );
    let raw = "%{reason}: /tmp/Utilisateur/My file.json";
    assert_eq!(
        active.presentation(&format!("Invalid firewall JSON: {raw}")),
        format!("JSON de pare-feu invalide : {raw}")
    );
    for source in [
        "Ignored x invalid or unreadable native palette file(s).".to_owned(),
        "Ignored -1 invalid or unreadable native palette file(s).".to_owned(),
        "Invalid firewall JSON: ".to_owned(),
        format!("Invalid firewall JSON: {}", "x".repeat(4097)),
        "Unknown diagnostic: preserve me".to_owned(),
    ] {
        assert_eq!(active.presentation(&source), source);
    }
    let english = Catalogue::bundled().activate(&choice("en"), &locale("fr"));
    assert_eq!(
        english.presentation("Invalid firewall JSON: original detail"),
        "Invalid firewall JSON: original detail"
    );
}

#[test]
fn bundled_catalogues_report_complete_alias_coverage_and_preserve_fallback() {
    let catalogue = Catalogue::bundled();
    let english: serde_json::Value =
        serde_json::from_slice(include_bytes!("../locales/en.json")).unwrap();
    let schema = english["messages"].as_object().unwrap();
    assert_eq!(catalogue.languages.len(), 5);
    for (id, alias, title, bytes) in [
        (
            "de",
            "de-DE",
            "Sprache",
            include_bytes!("../locales/de.json").as_slice(),
        ),
        (
            "es",
            "es-MX",
            "Idioma",
            include_bytes!("../locales/es.json").as_slice(),
        ),
        (
            "ja",
            "ja-JP",
            "言語",
            include_bytes!("../locales/ja.json").as_slice(),
        ),
    ] {
        let doc: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        let messages = doc["messages"].as_object().unwrap();
        let descriptor = catalogue
            .languages
            .iter()
            .find(|l| l.id.as_str() == id)
            .unwrap();
        assert_eq!(descriptor.translated, messages.len());
        assert_eq!(descriptor.total, schema.len());
        assert_eq!(descriptor.translated, descriptor.total);
        let active = catalogue.activate(&choice(alias), &locale("en"));
        assert_eq!(active.locale.as_str(), id);
        assert_eq!(active.coverage(), (messages.len(), schema.len()));
        assert_eq!(active.text("language.title"), title);
        assert!(active.warning.is_none());
        for (key, expected) in messages {
            assert!(schema.contains_key(key));
            assert_eq!(active.text(key), expected.as_str().unwrap(), "{id}: {key}");
        }
        let system = catalogue.activate(&LanguageChoice::System, &locale(alias));
        assert_eq!(system.locale, active.locale);
        assert_eq!(system.coverage(), active.coverage());
    }
    let fallback = catalogue.activate(&choice("zz-JP"), &locale("ja"));
    assert_eq!(fallback.locale.as_str(), "en");
    assert_eq!(fallback.coverage(), (schema.len(), schema.len()));
    assert!(fallback.warning.is_some());
    // A file with a bundled ID cannot silently replace the reviewed bundled data.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("de.json"),
        document("de", json!({"language.title":"Override"})),
    )
    .unwrap();
    let discovered = Catalogue::discover(dir.path()).unwrap();
    assert!(discovered.warning.is_some());
    assert_eq!(
        discovered
            .activate(&choice("de-DE"), &locale("en"))
            .text("language.title"),
        "Sprache"
    );
}

#[test]
fn uncatalogued_source_templates_format_english_without_hiding_argument_mistakes() {
    let active = Catalogue::bundled().activate(&choice("fr"), &locale("en"));
    let source = "Future feature saved %{items} items in %{path}.";
    assert_eq!(
        active.format_source(source, &[("items", "2"), ("path", "/owned/%{items}/é")]),
        "Future feature saved 2 items in /owned/%{items}/é."
    );
    // Key lookup remains distinct from formatting a new English source.
    assert_eq!(
        active.text("future.feature.missing"),
        "future.feature.missing"
    );
    assert!(
        active
            .format("future.feature.missing", &[("items", "2")])
            .is_err()
    );
    // A known translated source with the wrong argument names stays visibly raw.
    let known = "Current language: %{language}";
    assert_eq!(active.format_source(known, &[("wrong", "x")]), known);
    assert_eq!(active.format_source(source, &[("items", "2")]), source);
    assert_eq!(
        active.format_source(source, &[("items", "2"), ("items", "3"), ("path", "x")]),
        source
    );
    assert_eq!(
        active.format_source("Broken %{name", &[("name", "x")]),
        "Broken %{name"
    );
    assert_eq!(
        active.format_source("Invalid %{bad-name}", &[("bad-name", "x")]),
        "Invalid %{bad-name}"
    );
    assert_eq!(
        active.format_source(source, &[("items", "2"), ("path", &"x".repeat(4097))]),
        source
    );
    let repeated = "%{value}".repeat(5);
    assert_eq!(
        active.format_source(&repeated, &[("value", &"x".repeat(4096))]),
        repeated
    );
    let too_long = "x".repeat(16385);
    assert_eq!(active.format_source(&too_long, &[]), too_long);
    assert_eq!(
        active.format_source("Unknown plain English", &[]),
        "Unknown plain English"
    );
}
