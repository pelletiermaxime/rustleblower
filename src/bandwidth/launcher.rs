//! The desktop authentication agent owns passwords. Only this worker owns the child.
use super::Status;
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsString,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::{Child, ChildStderr, Command, Stdio},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) struct Launch {
    pub prepared: Option<Arc<tempfile::TempDir>>,
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
}
impl Launch {
    pub(super) fn resolve(endpoint: &Path, uid: u32) -> Result<Self> {
        let executable = std::env::current_exe()?.canonicalize()?;
        let parent = executable
            .parent()
            .context("Application path has no parent")?;
        let mut directories = vec![
            parent.join("libexec/rustleblower"),
            parent.join("../libexec/rustleblower"),
            parent.join("../bandwidth"),
        ];
        if cfg!(debug_assertions) {
            directories.insert(
                0,
                Path::new(env!("CARGO_MANIFEST_DIR")).join("target/bandwidth"),
            );
        }
        let directory = directories.into_iter().find(|p| p.join("collector").is_file())
            .context("Collector is not installed. Build it with scripts/bandwidth/build.py for this source preview")?
            .canonicalize()?;
        // A root helper cannot rely on access to a user's FUSE AppImage mount.
        // Keep a bounded private copy (including its $ORIGIN libraries) alive
        // until the collector exits. This also keeps one immutable build pair.
        let prepared = Arc::new(prepare(&directory, uid)?);
        let helper = prepared.path().join("collector");
        let object = prepared.path().join("accounting.bpf.o");
        Ok(Self {
            prepared: Some(prepared),
            program: "/usr/bin/pkexec".into(),
            arguments: vec![
                "--disable-internal-agent".into(),
                helper.into_os_string(),
                "--object".into(),
                object.into_os_string(),
                "--uid".into(),
                uid.to_string().into(),
                "--seconds".into(),
                "3600".into(),
                "--connect".into(),
                endpoint.as_os_str().into(),
            ],
        })
    }
    pub(super) fn spawn(&self) -> Result<Process> {
        let mut child = Command::new(&self.program).args(&self.arguments)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped())
            .spawn().context("Cannot open system authorization. Install pkexec and a desktop polkit authentication agent")?;
        let stderr = child
            .stderr
            .take()
            .context("Collector error stream missing")?;
        let mut process = Process {
            _prepared: self.prepared.clone(),
            child,
            stderr,
            detail: Vec::new(),
        };
        let fd = process.stderr.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        ensure!(
            flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } >= 0,
            "Cannot prepare collector diagnostics"
        );
        process.drain();
        Ok(process)
    }
}

/// Read regular, non-shared-writable sources without following symlinks.
/// All output is owned by the caller in a fresh mode-0700 directory.
pub(super) fn prepare(directory: &Path, uid: u32) -> Result<tempfile::TempDir> {
    let target = tempfile::Builder::new()
        .prefix("rustleblower-collector-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()?;
    let mut remaining = 32 * 1024 * 1024_u64;
    let mut copy = |source: &Path, destination: &Path, executable: bool| -> Result<()> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(source)?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && (metadata.uid() == uid || metadata.uid() == 0)
                && metadata.mode() & 0o022 == 0,
            "Collector files must belong to this user or root and disallow group/other writes"
        );
        let limit = remaining.min(16 * 1024 * 1024);
        ensure!(metadata.len() <= limit, "Collector bundle is too large");
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(if executable { 0o700 } else { 0o600 })
            .open(destination)?;
        let count = std::io::copy(&mut file.take(limit + 1), &mut output)?;
        ensure!(count <= limit, "Collector bundle is too large");
        output.flush()?;
        remaining -= count;
        Ok(())
    };
    copy(
        &directory.join("collector"),
        &target.path().join("collector"),
        true,
    )?;
    copy(
        &directory.join("accounting.bpf.o"),
        &target.path().join("accounting.bpf.o"),
        false,
    )?;
    let libraries = directory.join("lib");
    if libraries.exists() {
        std::fs::create_dir(target.path().join("lib"))?;
        std::fs::set_permissions(
            target.path().join("lib"),
            std::fs::Permissions::from_mode(0o700),
        )?;
        for (index, entry) in std::fs::read_dir(libraries)?.enumerate() {
            ensure!(index < 16, "Too many collector libraries");
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("Invalid collector library name")?;
            ensure!(
                name.starts_with("lib")
                    && name.contains(".so")
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
                "Invalid collector library name"
            );
            copy(&entry.path(), &target.path().join("lib").join(name), false)?;
        }
    }
    Ok(target)
}

pub(super) struct Process {
    _prepared: Option<Arc<tempfile::TempDir>>,
    child: Child,
    stderr: ChildStderr,
    detail: Vec<u8>,
}
impl Process {
    fn drain(&mut self) {
        let mut buffer = [0; 4096];
        // Drain bounded work per tick and retain at most 4 KiB of diagnostics.
        for _ in 0..4 {
            match self.stderr.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    let retained = count.min(4096 - self.detail.len());
                    self.detail.extend_from_slice(&buffer[..retained]);
                }
            }
        }
    }
    pub fn poll(&mut self) -> Result<Option<Status>> {
        self.drain();
        let Some(exit) = self.child.try_wait()? else {
            return Ok(None);
        };
        self.drain();
        Ok(Some(match exit.code() {
            Some(0) => Status::Ended,
            Some(126) => Status::Cancelled,
            Some(127) => Status::Unavailable("System authorization failed. Check pkexec and the desktop polkit authentication agent".into()),
            _ => {
                let detail: String = String::from_utf8_lossy(&self.detail).chars()
                    .map(|c| if c.is_control() { ' ' } else { c }).collect();
                Status::Unavailable(format!("Collector exited ({exit}): {}", detail.trim()))
            }
        }))
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        // Pending pkexec belongs to us and can be killed. Once it has become
        // root, closing the receiver makes the collector exit. Reap off the UI.
        let _ = self.child.kill();
        loop {
            self.drain();
            match self.child.try_wait() {
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
                _ => break,
            }
        }
    }
}
