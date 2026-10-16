use rustleblower::window_state::{Mode, Page, Rect, State, Store};
use std::os::unix::fs::symlink;
fn state() -> State {
    State {
        version: 1,
        bounds: Rect {
            x: 20.,
            y: 40.,
            width: 1100.,
            height: 780.,
        },
        mode: Mode::Windowed,
        page: Page::History,
    }
}
#[test]
fn all_pages_and_window_modes_roundtrip_without_changing_sibling_settings() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(&dir.path().join("history.sqlite3"));
    let first = store.load().unwrap();
    assert!(first.state.is_none() && !store.path.exists());
    let sibling = dir.path().join("history.sqlite3.preferences.json");
    std::fs::write(&sibling, b"keep popup settings").unwrap();
    let mut previous = first;
    for page in [
        Page::History,
        Page::Rules,
        Page::Statistics,
        Page::Nodes,
        Page::Alerts,
        Page::Live,
        Page::Subscriptions,
    ] {
        for mode in [Mode::Windowed, Mode::Maximized, Mode::Fullscreen] {
            let s = State {
                page,
                mode,
                ..state()
            };
            previous = store.save(&previous, s).unwrap();
            assert_eq!(store.load().unwrap().state, Some(s));
        }
    }
    assert_eq!(std::fs::read(sibling).unwrap(), b"keep popup settings");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}
#[test]
fn disconnected_negative_and_small_displays_keep_restore_bounds_reachable() {
    let monitors = [
        Rect {
            x: 0.,
            y: 0.,
            width: 1920.,
            height: 1080.,
        },
        Rect {
            x: -1280.,
            y: 0.,
            width: 1280.,
            height: 1024.,
        },
    ];
    let s = State {
        bounds: Rect {
            x: -1400.,
            y: 800.,
            width: 1200.,
            height: 900.,
        },
        mode: Mode::Maximized,
        ..state()
    };
    let fitted = s.fit(&monitors).unwrap();
    assert_eq!(
        fitted.bounds,
        Rect {
            x: -1280.,
            y: 124.,
            width: 1200.,
            height: 900.
        }
    );
    assert_eq!(fitted.mode, Mode::Maximized);
    let gone = s.fit(&monitors[..1]).unwrap();
    assert_eq!(gone.bounds.x, 0.);
    assert_eq!(gone.bounds.y, 180.);
    let small = s
        .fit(&[Rect {
            x: 0.,
            y: 0.,
            width: 640.,
            height: 480.,
        }])
        .unwrap();
    assert_eq!(
        small.bounds,
        Rect {
            x: 0.,
            y: 0.,
            width: 640.,
            height: 480.
        }
    );
    assert!(s.fit(&[]).is_err());
    assert!(s.fit(&[monitors[0]; 17]).is_err());
    let mut invalid = state();
    invalid.bounds.width = f32::NAN;
    assert!(invalid.validate().is_err());
    invalid.bounds.width = 0.;
    assert!(invalid.validate().is_err());
}
#[test]
fn stale_corrupt_oversized_and_symlink_files_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(&dir.path().join("db"));
    let first = store.load().unwrap();
    let saved = store.save(&first, state()).unwrap();
    assert!(store.save(&first, state()).is_err());
    let original = std::fs::read(&store.path).unwrap();
    for bad in [
        "{bad".to_owned(),
        String::from_utf8(original.clone())
            .unwrap()
            .replace("\"version\":1", "\"version\":2"),
        "x".repeat(4097),
    ] {
        std::fs::write(&store.path, &bad).unwrap();
        assert!(store.load().is_err());
        assert!(store.save(&saved, state()).is_err());
        assert_eq!(std::fs::read_to_string(&store.path).unwrap(), bad);
    }
    std::fs::remove_file(&store.path).unwrap();
    let target = dir.path().join("keep");
    std::fs::write(&target, &original).unwrap();
    symlink(&target, &store.path).unwrap();
    assert!(store.load().is_err());
    assert_eq!(std::fs::read(target).unwrap(), original);
}
