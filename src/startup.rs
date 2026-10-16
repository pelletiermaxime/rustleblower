//! Explicit per-user XDG autostart control. No file is changed at application
//! startup; callers run discovery/read/write on an I/O executor.
use crate::config::Config;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
pub const ENTRY: &str = "dev.rustleblower.Rustleblower.desktop";
const LIMIT: usize = 64 * 1024;
#[derive(Clone)]
pub struct Paths {
    pub user: PathBuf,
    pub system: Vec<PathBuf>,
}
impl Paths {
    pub fn environment() -> Result<Self> {
        let home = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
            .context("Cannot locate the user configuration directory")?;
        let roots = std::env::var_os("XDG_CONFIG_DIRS")
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/etc/xdg".into());
        let mut system = Vec::new();
        for p in std::env::split_paths(&roots).filter(|p| p.is_absolute()) {
            ensure!(
                system.len() < 16,
                "Too many system configuration directories"
            );
            system.push(p.join("autostart").join(ENTRY));
        }
        let paths = Self {
            user: home.join("autostart").join(ENTRY),
            system,
        };
        paths.validate()?;
        Ok(paths)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.system.len() <= 16,
            "Too many system configuration directories"
        );
        for p in std::iter::once(&self.user).chain(&self.system) {
            ensure!(
                p.is_absolute() && p.as_os_str().len() <= 4096,
                "Invalid autostart path"
            );
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Launch {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
}
impl Launch {
    pub fn environment(config: &Config) -> Result<Self> {
        let current = std::env::current_exe()?;
        let exe = Self::resolve(
            current,
            std::env::var_os("APPDIR").map(PathBuf::from),
            std::env::var_os("APPIMAGE").map(PathBuf::from),
        )?;
        Self::new(exe, config)
    }
    pub fn resolve(
        current: PathBuf,
        appdir: Option<PathBuf>,
        image: Option<PathBuf>,
    ) -> Result<PathBuf> {
        if let Some(dir) = appdir.and_then(|p| p.canonicalize().ok())
            && current.starts_with(dir)
        {
            return image.context("AppImage location is unavailable; launch the original AppImage to configure startup");
        }
        Ok(current)
    }
    pub fn new(executable: PathBuf, config: &Config) -> Result<Self> {
        ensure!(
            executable.is_absolute()
                && executable.is_file()
                && std::fs::metadata(&executable)?.permissions().mode() & 0o111 != 0,
            "Autostart needs an existing absolute executable path"
        );
        let path = |p: &Path| -> Result<String> {
            let absolute = if p.is_absolute() {
                p.to_owned()
            } else {
                std::env::current_dir()?.join(p)
            };
            Ok(absolute
                .to_str()
                .context("Autostart paths must be UTF-8")?
                .to_owned())
        };
        let launch = Self {
            executable,
            arguments: vec![
                "--socket".into(),
                path(&config.socket)?,
                "--database".into(),
                path(&config.database)?,
                "--prompt-timeout".into(),
                config.prompt_timeout.to_string(),
                "--timeout-action".into(),
                config.timeout_action.action().into(),
            ],
        };
        launch.command()?;
        Ok(launch)
    }
    pub fn command(&self) -> Result<String> {
        let exe = self
            .executable
            .to_str()
            .context("Executable path must be UTF-8")?;
        ensure!(
            !exe.contains('='),
            "Desktop startup does not support '=' in executable paths"
        );
        ensure!(self.arguments.len() <= 16, "Too many startup arguments");
        // GIO checks the first executable before expanding literal %% codes.
        // Keep a percent-bearing path in an argument, without a shell.
        let mut args = if exe.contains('%') {
            ensure!(
                Path::new("/usr/bin/env").is_file(),
                "Percent-bearing executable paths need /usr/bin/env"
            );
            vec![quote("/usr/bin/env")?, "--".into(), quote(exe)?]
        } else {
            vec![quote(exe)?]
        };
        for arg in &self.arguments {
            args.push(quote(arg)?);
        }
        let cmd = args.join(" ");
        ensure!(cmd.len() <= 32 * 1024, "Startup command exceeds 32 KiB");
        Ok(cmd)
    }
}
fn quote(value: &str) -> Result<String> {
    ensure!(
        value.len() <= 4096 && !value.chars().any(char::is_control),
        "Startup argument contains unsupported control characters or exceeds 4096 bytes"
    );
    // First quote the argument, then escape backslashes for desktop string values.
    let mut quoted = String::from("\"");
    for c in value.chars() {
        match c {
            '%' => quoted.push_str("%%"),
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    Ok(quoted.replace('\\', "\\\\"))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub enabled: bool,
    pub managed: bool,
    pub origin: Option<PathBuf>,
    user_hash: Option<[u8; 32]>,
    effective_hash: Option<[u8; 32]>,
}
#[derive(Clone)]
pub struct Manager {
    pub paths: Paths,
    pub launch: Launch,
}
fn read(path: &Path, user: bool) -> Result<Option<String>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        !user || !meta.file_type().is_symlink(),
        "User autostart entry is a symbolic link; it was preserved"
    );
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "Autostart entry is not a regular file"
    );
    let mut text = String::new();
    file.take((LIMIT + 1) as u64).read_to_string(&mut text)?;
    ensure!(text.len() <= LIMIT, "Autostart entry exceeds 64 KiB");
    Ok(Some(text))
}
fn state(text: &str) -> Result<(bool, bool)> {
    let mut inside = false;
    let mut application = false;
    let mut hidden = false;
    let mut managed = false;
    let mut seen = std::collections::BTreeSet::new();
    let mut groups = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
            groups += usize::from(inside);
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            if ["Type", "Hidden", "X-Rustleblower-Managed"].contains(&key) {
                ensure!(
                    seen.insert(key),
                    "Duplicate autostart key; entry was preserved"
                );
            }
            match key {
                "Type" => application = value == "Application",
                "Hidden" => {
                    ensure!(
                        matches!(value, "true" | "false"),
                        "Invalid Hidden value; entry was preserved"
                    );
                    hidden = value == "true";
                }
                "X-Rustleblower-Managed" => managed = value == "true",
                _ => {}
            }
        }
    }
    ensure!(
        groups == 1 && (application || hidden),
        "Invalid application autostart entry; it was preserved"
    );
    Ok((!hidden, managed))
}
impl Manager {
    pub fn load(&self) -> Result<Snapshot> {
        self.paths.validate()?;
        if let Some(text) = read(&self.paths.user, true)? {
            let (enabled, managed) = state(&text)?;
            let hash = Sha256::digest(text.as_bytes()).into();
            return Ok(Snapshot {
                enabled,
                managed,
                origin: Some(self.paths.user.clone()),
                user_hash: Some(hash),
                effective_hash: Some(hash),
            });
        }
        for path in &self.paths.system {
            if let Some(text) = read(path, false)? {
                let (enabled, _) = state(&text)?;
                return Ok(Snapshot {
                    enabled,
                    managed: false,
                    origin: Some(path.clone()),
                    user_hash: None,
                    effective_hash: Some(Sha256::digest(text.as_bytes()).into()),
                });
            }
        }
        Ok(Snapshot {
            enabled: false,
            managed: false,
            origin: None,
            user_hash: None,
            effective_hash: None,
        })
    }
    pub fn save(&self, expected: &Snapshot, enabled: bool) -> Result<Snapshot> {
        let current = self.load()?;
        ensure!(
            &current == expected,
            "Autostart entry changed; reload before saving"
        );
        ensure!(
            current.user_hash.is_none() || current.managed,
            "Existing user autostart entry is not managed by Rustleblower; it was preserved"
        );
        let command = self.launch.command()?;
        let text = format!(
            "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Rustleblower\nComment=OpenSnitch firewall interface\nExec={command}\nIcon=dev.rustleblower.Rustleblower\nTerminal=false\nHidden={}\nX-Rustleblower-Managed=true\n",
            !enabled
        );
        ensure!(text.len() <= LIMIT, "Autostart entry exceeds its limit");
        let parent = self
            .paths
            .user
            .parent()
            .context("Invalid autostart directory")?;
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(text.as_bytes())?;
        file.as_file().sync_all()?;
        ensure!(
            self.load()? == current,
            "Autostart entry changed while saving; reload before retrying"
        );
        if current.user_hash.is_none() {
            file.persist_noclobber(&self.paths.user)
                .map_err(|e| e.error)?;
        } else {
            file.persist(&self.paths.user).map_err(|e| e.error)?;
        }
        File::open(parent)?.sync_all()?;
        self.load()
    }
}
