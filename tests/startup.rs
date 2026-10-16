use rustleblower::{
    config::{Config, Decision},
    startup::{ENTRY, Launch, Manager, Paths},
};
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};
fn config(dir: &Path) -> Config {
    Config {
        socket: dir.join("ui $`% socket.sock"),
        database: dir.join("history % é.sqlite3"),
        prompt_timeout: 19,
        timeout_action: Decision::Reject,
        history_days: 7,
        history_mib: 16,
        headless: false,
    }
}
fn manager(dir: &Path) -> Manager {
    Manager {
        paths: Paths {
            user: dir.join("user/autostart").join(ENTRY),
            system: vec![
                dir.join("system-first/autostart").join(ENTRY),
                dir.join("system-second/autostart").join(ENTRY),
            ],
        },
        launch: Launch::new(PathBuf::from("/bin/true"), &config(dir)).unwrap(),
    }
}
fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
const ENABLED: &str =
    "[Desktop Entry]\nType=Application\nName=Rustleblower\nExec=rustleblower\nHidden=false\n";
#[test]
fn user_override_roundtrips_and_disables_system_startup_without_changing_system_files() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let empty = m.load().unwrap();
    assert!(!empty.enabled && empty.origin.is_none());
    write(&m.paths.system[0], ENABLED);
    write(&m.paths.system[1], "[Desktop Entry]\nHidden=true\n");
    let system = m.load().unwrap();
    assert!(system.enabled && !system.managed);
    assert_eq!(system.origin.as_ref(), Some(&m.paths.system[0]));
    let disabled = m.save(&system, false).unwrap();
    assert!(!disabled.enabled && disabled.managed);
    assert_eq!(
        std::fs::read_to_string(&m.paths.system[0]).unwrap(),
        ENABLED
    );
    let enabled = m.save(&disabled, true).unwrap();
    assert!(enabled.enabled && enabled.managed);
    assert_eq!(m.load().unwrap(), enabled);
    let text = std::fs::read_to_string(&m.paths.user).unwrap();
    assert!(
        text.contains("--socket")
            && text.contains("--database")
            && text.contains("--timeout-action")
    );
    assert!(!text.contains("--history-mib") && !text.contains("--headless"));
    m.save(&enabled, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(&m.paths.system[0]).unwrap(),
        ENABLED
    );
    assert_eq!(
        std::fs::read_dir(m.paths.user.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
}
#[test]
fn stale_foreign_symlink_oversized_and_invalid_entries_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let old = m.load().unwrap();
    write(&m.paths.user, ENABLED);
    assert!(m.save(&old, true).is_err());
    let foreign = m.load().unwrap();
    assert!(m.save(&foreign, false).is_err());
    assert_eq!(std::fs::read_to_string(&m.paths.user).unwrap(), ENABLED);
    std::fs::remove_file(&m.paths.user).unwrap();
    let enabled = m.save(&m.load().unwrap(), true).unwrap();
    let changed = std::fs::read_to_string(&m.paths.user).unwrap() + "# external change\n";
    write(&m.paths.user, &changed);
    assert!(m.save(&enabled, false).is_err());
    assert_eq!(std::fs::read_to_string(&m.paths.user).unwrap(), changed);
    write(
        &m.paths.user,
        "[Desktop Entry]\nType=Application\nHidden=true\nHidden=false\n",
    );
    assert!(m.load().is_err());
    write(&m.paths.user, &"x".repeat(65537));
    assert!(m.load().is_err());
    std::fs::remove_file(&m.paths.user).unwrap();
    let target = dir.path().join("keep.desktop");
    write(&target, ENABLED);
    symlink(&target, &m.paths.user).unwrap();
    assert!(m.load().is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), ENABLED);
}
#[test]
fn appimage_resolution_and_reserved_argument_fixtures() {
    let dir = tempfile::tempdir().unwrap();
    let current = dir.path().join("native");
    let appdir = dir.path().join("mount");
    std::fs::create_dir(&appdir).unwrap();
    let image = dir.path().join("My App.AppImage");
    assert_eq!(
        Launch::resolve(current.clone(), Some(appdir.clone()), Some(image.clone())).unwrap(),
        current
    );
    assert_eq!(
        Launch::resolve(
            appdir.join("usr/bin/rustleblower"),
            Some(appdir.clone()),
            Some(image.clone())
        )
        .unwrap(),
        image
    );
    assert!(Launch::resolve(appdir.join("rustleblower"), Some(appdir), None).is_err());
    let mut c = config(dir.path());
    c.socket = dir.path().join("bad\nargument");
    assert!(Launch::new(PathBuf::from("/bin/true"), &c).is_err());
    c.socket = PathBuf::from("relative.sock");
    let relative = Launch::new(PathBuf::from("/bin/true"), &c).unwrap();
    assert_eq!(
        PathBuf::from(&relative.arguments[1]),
        std::env::current_dir().unwrap().join("relative.sock")
    );
    if let Some(root) = std::env::var_os("RUSTLEBLOWER_STARTUP_EVIDENCE") {
        let root = PathBuf::from(root);
        std::fs::create_dir(&root).unwrap();
        let exe = root.join("Rustle blower % quote\"$`\\ é.py");
        write(
            &exe,
            "#!/usr/bin/python3\nimport json,sys\nfrom pathlib import Path\nPath(__file__).with_name('argv.json').write_text(json.dumps(sys.argv[1:]))\n",
        );
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        let launch = Launch::new(exe.clone(), &config(&root)).unwrap();
        let expected = launch.arguments.clone();
        let m = Manager {
            paths: Paths {
                user: root.join("xdg/autostart").join(ENTRY),
                system: vec![],
            },
            launch,
        };
        m.save(&m.load().unwrap(), true).unwrap();
        std::fs::write(root.join("fixture.json"),serde_json::to_vec_pretty(&serde_json::json!({"entry":m.paths.user,"executable":exe,"expected_arguments":expected,"argv":root.join("argv.json")})).unwrap()).unwrap();
    }
}
