use super::*;
fn rule(duration: &str, created: i64) -> Rule {
    Rule {
        enabled: true,
        duration: duration.into(),
        created,
        ..Default::default()
    }
}
#[test]
fn exact_ordered_threshold_omits_only_known_presets() {
    let default = Settings::default();
    for preset in PRESETS {
        assert!(default.includes(preset.duration()));
    }
    for threshold in PRESETS {
        let settings = Settings {
            ignore_temporary: true,
            threshold,
        };
        for duration in PRESETS {
            assert_eq!(
                settings.includes(duration.duration()),
                duration.index() >= threshold.index()
            );
        }
        for duration in ["always", "2m", "500ms", "invalid", ""] {
            assert!(settings.includes(duration));
        }
    }
    assert!(
        !Settings {
            ignore_temporary: true,
            ..Default::default()
        }
        .includes("once")
    );
}
#[test]
fn cached_status_keeps_daemon_truth_and_explicit_origin() {
    let settings = Settings::default();
    let mut source = rule("until restart", 100);
    let original = source.clone();
    assert_eq!(
        derive(settings, &source, Origin::Restored, 200).status,
        Status::ExpiredAtUiRestart
    );
    assert_eq!(
        derive(
            settings,
            &source,
            Origin::ObservedThisUiRun { received_unix: 190 },
            200
        )
        .status,
        Status::Active
    );
    assert_eq!(source, original);
    source.enabled = false;
    let display = derive(settings, &source, Origin::Restored, 200);
    assert!(!display.daemon_enabled);
    assert_eq!(display.status, Status::DaemonDisabled);
    assert_eq!(
        derive(settings, &rule("30s", 100), Origin::Restored, 130).status,
        Status::Active
    );
    let display = derive(settings, &rule("30s", 100), Origin::Restored, 131);
    assert!(display.daemon_enabled);
    assert_eq!(display.status, Status::ExpiredByAge);
    assert_eq!(
        derive(
            settings,
            &rule("once", 90),
            Origin::ObservedThisUiRun { received_unix: 100 },
            100
        )
        .status,
        Status::Active
    );
    assert_eq!(
        derive(
            settings,
            &rule("once", 90),
            Origin::ObservedThisUiRun { received_unix: 100 },
            101
        )
        .status,
        Status::ExpiredByAge
    );
}
#[test]
fn invalid_clock_duration_and_overflow_do_not_invent_expiry() {
    let settings = Settings::default();
    for value in ["invalid", "0s", "-1m", "99999999999999999999h", "1.2.3s"] {
        assert_eq!(
            derive(settings, &rule(value, 100), Origin::Restored, 200).status,
            Status::Unknown
        );
    }
    assert_eq!(duration_nanos("1h30m"), Some(5_400_000_000_000));
    assert_eq!(duration_nanos("1.5ms"), Some(1_500_000));
    assert_eq!(duration_nanos("1d"), Some(86_400_000_000_000));
    assert_eq!(duration_nanos("1w"), Some(604_800_000_000_000));
    assert_eq!(
        derive(settings, &rule("30s", 0), Origin::Restored, 100).status,
        Status::Unknown
    );
    assert_eq!(
        derive(settings, &rule("30s", 200), Origin::Restored, 100).status,
        Status::Unknown
    );
    assert_eq!(
        derive(
            settings,
            &rule("once", 90),
            Origin::ObservedThisUiRun { received_unix: 0 },
            100
        )
        .status,
        Status::Unknown
    );
    assert_eq!(
        derive(
            settings,
            &rule("1h", i64::MAX - 1),
            Origin::Restored,
            i64::MAX
        )
        .status,
        Status::Unknown
    );
}
#[tokio::test]
async fn off_thread_preferences_reopen_and_preserve_invalid_stale_and_symlink_sources() {
    let dir = tempfile::tempdir().unwrap();
    let path = path_for(&dir.path().join("history.sqlite3"));
    let worker = Worker::start(path.clone());
    let handle = worker.handle();
    let state = handle.request(Command::Reload).await.unwrap().unwrap();
    assert_eq!(state.settings, Settings::default());
    let settings = Settings {
        ignore_temporary: true,
        threshold: Preset::UntilRestart,
    };
    let saved = handle
        .request(Command::Save {
            expected_revision: state.revision,
            settings,
        })
        .await
        .unwrap()
        .unwrap();
    let reopened = handle.request(Command::Reload).await.unwrap().unwrap();
    assert_eq!(reopened.settings, settings);
    assert_eq!(reopened.revision, saved.revision);
    std::fs::write(&path, b"foreign-invalid").unwrap();
    assert!(
        handle
            .request(Command::Save {
                expected_revision: saved.revision,
                settings: Settings::default()
            })
            .await
            .unwrap()
            .is_err()
    );
    let retained = handle.request(Command::Reload).await.unwrap().unwrap();
    assert_eq!(retained.settings, settings);
    assert!(retained.warning.is_some());
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign-invalid");
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(store::test_save(&link, &None, &settings).is_err());
    worker.join();
}
