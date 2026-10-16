//! Best-effort local desktop icons. Neither lookup nor decoding runs on GPUI.
use freedesktop_desktop_entry::DesktopEntry;
use gpui::{App, RenderImage};
use std::{
    collections::{HashSet, VecDeque},
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, oneshot};

const SIDE: u32 = 64;
const MAX_ICON: usize = 1024 * 1024;
const MAX_ENTRY: usize = 128 * 1024;
const MAX_SCAN_BYTES: usize = 8 * 1024 * 1024;
static WORKERS: Semaphore = Semaphore::const_new(2);
type CachedIcon = (String, Instant, Option<Arc<RenderImage>>);
static CACHE: Mutex<VecDeque<CachedIcon>> = Mutex::new(VecDeque::new());

pub(super) struct Probe {
    path: String,
    pending: Option<oneshot::Receiver<Option<Arc<RenderImage>>>>,
    done: bool,
    pub image: Option<Arc<RenderImage>>,
}
impl Probe {
    pub fn new(path: &str) -> Self {
        let usable = path.len() <= 4096 && Path::new(path).is_absolute();
        Self {
            path: if usable { path.into() } else { String::new() },
            pending: None,
            done: !usable,
            image: None,
        }
    }
    pub fn poll(&mut self, cx: &App) {
        if let Some(rx) = &mut self.pending {
            match rx.try_recv() {
                Ok(image) => {
                    self.image = image;
                    self.done = true;
                    self.pending = None;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.done = true;
                    self.pending = None;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        if self.done || self.pending.is_some() {
            return;
        }
        let Ok(permit) = WORKERS.try_acquire() else {
            return;
        };
        let (tx, rx) = oneshot::channel();
        self.pending = Some(rx);
        let path = self.path.clone();
        cx.background_executor()
            .spawn(async move {
                let _permit = permit;
                let _ = tx.send(cached(&path));
            })
            .detach();
    }
}

fn cached(path: &str) -> Option<Arc<RenderImage>> {
    {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|(_, at, _)| at.elapsed() < Duration::from_secs(60));
        if let Some((_, _, image)) = cache.iter().find(|(key, _, _)| key == path) {
            return image.clone();
        }
    }
    let applications = data_dirs()
        .into_iter()
        .map(|p| p.join("applications"))
        .collect::<Vec<_>>();
    let search_path = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .filter(|p| p.is_absolute())
                .take(32)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let icon = find_icon(Path::new(path), &applications, &search_path, |name| {
        let path = Path::new(name);
        if path.is_absolute() {
            Some(path.to_owned())
        } else if !name.contains('/') && !name.contains('\\') && name.len() <= 256 {
            freedesktop_icons::lookup(name)
                .with_size(SIDE as u16)
                .find()
        } else {
            None
        }
    })
    .and_then(|p| decode(&p))
    .map(Arc::new);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= 128 {
        cache.pop_front();
    }
    cache.push_back((path.into(), Instant::now(), icon.clone()));
    icon
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
    {
        dirs.push(home);
    }
    let system =
        std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for p in std::env::split_paths(&system)
        .filter(|p| p.is_absolute())
        .take(16)
    {
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    dirs
}

fn bounded(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= limit).then_some(bytes)
}

// Never execute a desktop command, consult process-supplied environment values,
// or identify a different app solely by its executable's basename.
fn command_path(command: &str, search: &[PathBuf]) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.is_absolute() {
        path.canonicalize().ok()
    } else if path.components().count() == 1 {
        search
            .iter()
            .find_map(|dir| dir.join(path).canonicalize().ok())
    } else {
        None
    }
}

fn matches(entry: &DesktopEntry, executable: &Path, search: &[PathBuf]) -> bool {
    // Desktop Exec may quote only the executable, followed by field codes.
    // Tokenize without expansion or execution; the entry crate's parse_exec
    // treats a leading quote as quoting the entire line.
    let args = entry.exec().and_then(shlex::split).unwrap_or_default();
    let mut words = args.iter().map(String::as_str);
    let mut command = words.next();
    if command.is_some_and(|s| Path::new(s).file_name().is_some_and(|name| name == "env")) {
        command = words.find(|s| !s.contains('='));
    }
    let Some(command) = command.or_else(|| entry.try_exec()) else {
        return false;
    };
    let stem = Path::new(command)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    // Interpreters and shared runtimes cannot identify which hosted app is active.
    if [
        "sh", "bash", "python", "python3", "perl", "node", "java", "electron", "flatpak", "wine",
    ]
    .contains(&stem)
        || stem.starts_with("python3.")
    {
        return false;
    }
    let Some(candidate) = command_path(command, search) else {
        return false;
    };
    if candidate == executable {
        return true;
    }
    // Some launchers (e.g. Vivaldi) are scripts next to the real *-bin binary.
    // Require both a matching desktop class and a private shared install folder.
    let name = executable
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let private_dir = executable.parent().is_some_and(|p| {
        ![
            Path::new("/usr/bin"),
            Path::new("/usr/local/bin"),
            Path::new("/bin"),
        ]
        .contains(&p)
    });
    private_dir
        && candidate.parent() == executable.parent()
        && entry.startup_wm_class().is_some_and(|class| {
            class.eq_ignore_ascii_case(name.strip_suffix("-bin").unwrap_or(name))
        })
}

fn find_icon(
    executable: &Path,
    applications: &[PathBuf],
    search: &[PathBuf],
    lookup: impl Fn(&str) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let executable = executable.canonicalize().ok()?;
    let started = Instant::now();
    let mut remaining = MAX_SCAN_BYTES;
    let mut count = 0;
    let mut seen = HashSet::new();
    let mut icons = HashSet::new();
    for root in applications.iter().take(17) {
        let Ok(entries) = root.read_dir() else {
            continue;
        };
        for entry in entries {
            count += 1;
            if count > 2048 || started.elapsed() > Duration::from_millis(500) {
                return None;
            }
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            if path.extension().is_none_or(|s| s != "desktop") || !seen.insert(entry.file_name()) {
                continue;
            }
            let Some(bytes) = bounded(&path, MAX_ENTRY.min(remaining)) else {
                continue;
            };
            remaining = remaining.checked_sub(bytes.len())?;
            if remaining == 0 {
                return None;
            }
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let Ok(entry) = DesktopEntry::from_str::<&str>(&path, text, Some(&[])) else {
                continue;
            };
            if entry.desktop_entry("Type") != Some("Application")
                || entry.desktop_entry("Hidden") == Some("true")
                || !matches(&entry, &executable, search)
            {
                continue;
            }
            let icon = entry.icon().filter(|s| s.len() <= 4096).and_then(&lookup)?;
            icons.insert(icon.canonicalize().ok()?);
            if icons.len() > 1 {
                return None;
            }
        }
    }
    icons.into_iter().next()
}

fn decode(path: &Path) -> Option<RenderImage> {
    let bytes = bounded(path, MAX_ICON)?;
    let mut pixels = if path.extension().is_some_and(|s| s == "svg") {
        let options = resvg::usvg::Options {
            image_href_resolver: resvg::usvg::ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            ..Default::default()
        };
        let tree = resvg::usvg::Tree::from_data(&bytes, &options).ok()?;
        let size = tree.size();
        let scale = (SIDE as f32 / size.width()).min(SIDE as f32 / size.height());
        let mut pixmap = resvg::tiny_skia::Pixmap::new(SIDE, SIDE)?;
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(
            (SIDE as f32 - size.width() * scale) / 2.,
            (SIDE as f32 - size.height() * scale) / 2.,
        );
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        // tiny-skia is premultiplied; GPUI's image frames use straight BGRA.
        let mut pixels = pixmap.take();
        for p in pixels.as_chunks_mut::<4>().0 {
            if p[3] != 0 {
                for channel in 0..3 {
                    p[channel] = ((u32::from(p[channel]) * 255 + u32::from(p[3]) / 2)
                        / u32::from(p[3]))
                    .min(255) as u8;
                }
            }
        }
        image::RgbaImage::from_raw(SIDE, SIDE, pixels)?
    } else {
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(2048);
        limits.max_image_height = Some(2048);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        reader
            .decode()
            .ok()?
            .resize(SIDE, SIDE, image::imageops::FilterType::Triangle)
            .into_rgba8()
    };
    if pixels.pixels().all(|p| p[3] == 0) {
        return None;
    }
    for p in pixels.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    Some(RenderImage::new([image::Frame::new(pixels)]))
}

#[cfg(all(test, feature = "headless-tests"))]
pub(super) fn hold_workers() -> tokio::sync::SemaphorePermit<'static> {
    WORKERS.try_acquire_many(2).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _dir: tempfile::TempDir,
        user: PathBuf,
        system: PathBuf,
        app: PathBuf,
        icon: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let user = dir.path().join("user/applications");
            let system = dir.path().join("system/applications");
            std::fs::create_dir_all(&user).unwrap();
            std::fs::create_dir_all(&system).unwrap();
            let app = dir.path().join("Example App");
            std::fs::write(&app, "not executed").unwrap();
            let icon = dir.path().join("icon.png");
            image::RgbaImage::from_pixel(2, 2, image::Rgba([240, 20, 30, 255]))
                .save(&icon)
                .unwrap();
            Self {
                _dir: dir,
                user,
                system,
                app,
                icon,
            }
        }
        fn entry(&self, root: &Path, name: &str, extra: &str) {
            std::fs::write(root.join(name), format!("[Desktop Entry]\nType=Application\nName=Example\nExec=\"{}\" %U\nIcon=example-icon\n{extra}", self.app.display())).unwrap();
        }
        fn resolve(&self, executable: &Path) -> Option<PathBuf> {
            find_icon(
                executable,
                &[self.user.clone(), self.system.clone()],
                &[],
                |name| {
                    assert_eq!(name, "example-icon");
                    Some(self.icon.clone())
                },
            )
        }
    }
    #[test]
    fn quoted_exec_symlinks_and_main_group_resolve_without_running_commands() {
        let f = Fixture::new();
        f.entry(
            &f.system,
            "example.desktop",
            "\n[Desktop Action Other]\nExec=/not/the/app\nIcon=wrong-icon\n",
        );
        let link = f.app.with_file_name("linked-app");
        std::os::unix::fs::symlink(&f.app, &link).unwrap();
        assert_eq!(f.resolve(&link), Some(f.icon.canonicalize().unwrap()));
        let unrelated = f.app.parent().unwrap().join("different/Example App");
        std::fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
        std::fs::write(&unrelated, "different executable").unwrap();
        assert!(
            f.resolve(&unrelated).is_none(),
            "same basename is insufficient"
        );
    }
    #[test]
    fn hidden_user_override_masks_system_entry() {
        let f = Fixture::new();
        f.entry(&f.system, "example.desktop", "");
        f.entry(&f.user, "example.desktop", "Hidden=true\n");
        assert!(f.resolve(&f.app).is_none());
    }
    #[test]
    fn conflicting_desktop_identities_and_shared_interpreters_use_fallback() {
        let f = Fixture::new();
        f.entry(&f.system, "one.desktop", "");
        let second = f.icon.with_file_name("second.png");
        std::fs::copy(&f.icon, &second).unwrap();
        std::fs::write(
            f.system.join("two.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nExec=\"{}\"\nIcon={}\n",
                f.app.display(),
                second.display()
            ),
        )
        .unwrap();
        assert!(
            find_icon(&f.app, std::slice::from_ref(&f.system), &[], |name| {
                if name == "example-icon" {
                    Some(f.icon.clone())
                } else {
                    Some(PathBuf::from(name))
                }
            })
            .is_none()
        );
        let python = f.app.with_file_name("python3.14");
        std::fs::write(&python, "interpreter").unwrap();
        std::fs::write(
            f.user.join("script.desktop"),
            format!(
                "[Desktop Entry]\nType=Application\nExec={} other-script.py\nIcon=example-icon\n",
                python.display()
            ),
        )
        .unwrap();
        assert!(f.resolve(&python).is_none());
    }
    #[test]
    fn launcher_sibling_needs_matching_class_and_private_directory() {
        let f = Fixture::new();
        let real = f.app.with_file_name("example-bin");
        std::fs::write(&real, "binary").unwrap();
        f.entry(&f.system, "example.desktop", "StartupWMClass=example\n");
        assert_eq!(f.resolve(&real), Some(f.icon.canonicalize().unwrap()));
        f.entry(&f.system, "example.desktop", "StartupWMClass=different\n");
        assert!(f.resolve(&real).is_none());
    }
    #[test]
    fn oversized_metadata_and_images_and_corrupt_images_fall_back() {
        let f = Fixture::new();
        std::fs::write(f.system.join("large.desktop"), vec![b'x'; MAX_ENTRY + 1]).unwrap();
        assert!(f.resolve(&f.app).is_none());
        let oversized = f.icon.with_file_name("oversized.png");
        image::RgbaImage::from_pixel(2049, 1, image::Rgba([1, 2, 3, 255]))
            .save(&oversized)
            .unwrap();
        assert!(decode(&oversized).is_none());
        std::fs::write(&oversized, "corrupt image").unwrap();
        assert!(decode(&oversized).is_none());
        std::fs::write(&oversized, vec![0; MAX_ICON + 1]).unwrap();
        assert!(decode(&oversized).is_none());
    }
    #[test]
    fn png_and_translucent_svg_are_bounded_bgra_and_svg_cannot_load_other_files() {
        let f = Fixture::new();
        let png = decode(&f.icon).unwrap();
        assert_eq!(png.as_bytes(0).unwrap().len(), (SIDE * SIDE * 4) as usize);
        assert_eq!(&png.as_bytes(0).unwrap()[..4], &[30, 20, 240, 255]);
        let svg = f.icon.with_file_name("icon.svg");
        std::fs::write(&svg,r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><path fill="#f0141e" opacity="0.5" d="M0 0h64v64H0z"/></svg>"##).unwrap();
        let rendered = decode(&svg).unwrap();
        let pixel = &rendered.as_bytes(0).unwrap()[..4];
        for (value, expected) in pixel.iter().zip([30u8, 20, 240, 128]) {
            assert!(value.abs_diff(expected) <= 2);
        }
        std::fs::write(&svg,format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><image width="64" height="64" href="{}"/></svg>"#,f.icon.display())).unwrap();
        assert!(
            decode(&svg).is_none(),
            "external SVG image references are disabled"
        );
    }
}
