//! AppImage staging and rollback adapted from FerryDock (MIT).
//! See LICENSES/FerryDock-MIT.txt.
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::UnixDatagram,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

const MAX_IMAGE: u64 = 512 * 1024 * 1024;
const MAX_CHECKSUMS: u64 = 64 * 1024;
const ASSET_ROOT: &str = "https://github.com/pelletiermaxime/rustleblower/releases/download";
const READY_SOCKET: &str = "RUSTLEBLOWER_UPDATE_READY_SOCKET";
const READY: &[u8] = b"rustleblower-ready";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Downloading(String),
    Ready(String),
    Restarting,
    Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
}

fn identity(metadata: fs::Metadata) -> Result<Identity> {
    ensure!(metadata.is_file(), "The AppImage must be a regular file");
    Ok(Identity {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        modified: (metadata.mtime(), metadata.mtime_nsec()),
    })
}

pub(super) struct Installation {
    path: PathBuf,
    identity: Identity,
}

impl Installation {
    pub(super) fn detect() -> Result<Self> {
        ensure!(
            cfg!(all(target_os = "linux", target_arch = "x86_64")),
            "AppImage updates require Linux x86-64"
        );
        let runtime = std::env::var_os("APPDIR").context("Missing AppImage runtime directory")?;
        ensure!(
            std::env::current_exe()?.canonicalize()?
                == Path::new(&runtime)
                    .join("usr/bin/rustleblower")
                    .canonicalize()?,
            "Rustleblower was launched outside its AppImage"
        );
        Self::open(PathBuf::from(
            std::env::var_os("APPIMAGE").context("Missing original AppImage path")?,
        ))
    }

    pub(super) fn open(path: PathBuf) -> Result<Self> {
        let path = path.canonicalize()?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)?;
        let identity = identity(file.metadata()?)?;
        validate_header(&mut file)?;
        Ok(Self { path, identity })
    }

    fn unchanged(&self) -> Result<()> {
        ensure!(
            identity(fs::symlink_metadata(&self.path)?)? == self.identity,
            "The installed AppImage changed. Restart Rustleblower before updating"
        );
        Ok(())
    }

    fn parent(&self) -> &Path {
        // Installation paths are canonical absolute file paths.
        self.path.parent().expect("AppImage parent")
    }
}

#[derive(Deserialize)]
pub(super) struct Asset {
    name: String,
    size: u64,
}

pub(super) struct Candidate {
    pub(super) version: String,
    name: String,
    url: String,
    size: u64,
    checksums_size: u64,
}

impl Candidate {
    pub(super) fn new(tag: &str, version: String, assets: &[Asset]) -> Result<Self> {
        ensure!(
            tag == format!("v{version}"),
            "Unexpected AppImage release tag"
        );
        let name = format!("rustleblower-{version}-linux-x86_64.AppImage");
        let size = |name: &str, limit| -> Result<u64> {
            let mut matches = assets.iter().filter(|asset| asset.name == name);
            let asset = matches
                .next()
                .context("The release is missing an AppImage or SHA256SUMS")?;
            ensure!(
                matches.next().is_none() && (1..=limit).contains(&asset.size),
                "The release contains invalid update assets"
            );
            Ok(asset.size)
        };
        Ok(Self {
            size: size(&name, MAX_IMAGE)?,
            checksums_size: size("SHA256SUMS", MAX_CHECKSUMS)?,
            url: format!("{ASSET_ROOT}/{tag}"),
            version,
            name,
        })
    }
}

fn validate_header(file: &mut File) -> Result<()> {
    let mut header = [0; 12];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut header)?;
    ensure!(
        &header[..4] == b"\x7fELF" && &header[8..11] == b"AI\x02",
        "The file is not a type-2 AppImage"
    );
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

fn checksum(bytes: &[u8], name: &str) -> Result<[u8; 32]> {
    let mut found = None;
    for line in bytes.split(|&byte| byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.len() >= 66
            && matches!(&line[64..66], b"  " | b" *")
            && &line[66..] == name.as_bytes()
        {
            ensure!(found.is_none(), "Duplicate AppImage checksum");
            let mut digest = [0; 32];
            hex::decode_to_slice(&line[..64], &mut digest).context("Invalid AppImage SHA-256")?;
            found = Some(digest);
        }
    }
    found.context("SHA256SUMS has no checksum for this AppImage")
}

struct Workspace {
    path: PathBuf,
    _lock: File,
}

impl Workspace {
    fn acquire(installation: &Installation) -> Result<Self> {
        let hash = Sha256::digest(installation.path.as_os_str().as_encoded_bytes());
        let path = installation.parent().join(format!(
            ".rustleblower-updates-{}",
            hex::encode(&hash[..12])
        ));
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("Cannot stage the update beside the AppImage"),
        }
        let metadata = fs::symlink_metadata(&path)?;
        let user = rustix::process::geteuid().as_raw();
        ensure!(
            metadata.is_dir() && metadata.uid() == user && metadata.mode() & 0o777 == 0o700,
            "The update staging directory must be private and owned by you"
        );
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path.join("lock"))?;
        ensure!(
            lock.metadata()?.is_file() && lock.metadata()?.uid() == user,
            "Invalid update staging lock"
        );
        lock.try_lock()
            .context("Another Rustleblower instance is staging an update")?;
        let candidate = path.join("candidate.AppImage");
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.is_file() && metadata.uid() == user => {
                fs::remove_file(candidate)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => bail!("Unexpected file in the AppImage staging directory"),
        }
        Ok(Self { path, _lock: lock })
    }
}

pub(super) struct Pending {
    file: tempfile::NamedTempFile,
    pub(super) version: String,
    size: u64,
    checksum: [u8; 32],
    _workspace: Workspace,
}

impl Pending {
    fn verify(&mut self) -> Result<()> {
        ensure!(
            self.file.as_file().metadata()?.len() == self.size,
            "The AppImage size differs from the release"
        );
        validate_header(self.file.as_file_mut())?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let count = self.file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        ensure!(
            hash.finalize().as_slice() == self.checksum,
            "AppImage SHA-256 verification failed"
        );
        ensure!(
            identity(self.file.as_file().metadata()?)?
                == identity(fs::symlink_metadata(self.file.path())?)?,
            "The staged AppImage changed"
        );
        Ok(())
    }

    pub(super) fn prepare(mut self, installation: std::sync::Arc<Installation>) -> Result<Restart> {
        installation.unchanged()?;
        self.verify()?;
        self.file
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
        self.file.as_file().sync_all()?;
        let backup = tempfile::tempdir_in(&self._workspace.path)?;
        fs::hard_link(&installation.path, backup.path().join("previous.AppImage"))
            .context("Cannot preserve the current AppImage for rollback")?;
        let ready_dir = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()?;
        let ready = UnixDatagram::bind(ready_dir.path().join("ready.sock"))?;
        ready.set_read_timeout(Some(Duration::from_millis(100)))?;
        Ok(Restart {
            pending: self,
            installation,
            backup,
            ready_dir,
            ready,
        })
    }
}

fn asset_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .https_only(true)
        .user_agent(concat!("Rustleblower/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 4
                && attempt.url().scheme() == "https"
                && matches!(
                    attempt.url().host_str(),
                    Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com")
                )
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()?)
}

async fn get(
    client: &reqwest::Client,
    url: String,
    size: u64,
    timeout: Duration,
) -> Result<reqwest::Response> {
    let response = client
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| e.without_url())?;
    ensure!(
        response.status().is_success(),
        "GitHub returned {} for the update asset",
        response.status()
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length == size),
        "The asset length differs from the release"
    );
    Ok(response)
}

pub(super) async fn download(
    installation: &Installation,
    candidate: Candidate,
    endpoints: super::Endpoints,
) -> Result<Pending> {
    #[cfg(test)]
    if let super::Endpoints::Fixture(base) = endpoints {
        let mut candidate = candidate;
        candidate.url = base;
        return download_with(
            &reqwest::Client::builder().no_proxy().build()?,
            installation,
            candidate,
        )
        .await;
    }
    let _ = endpoints;
    download_with(&asset_client()?, installation, candidate).await
}

async fn download_with(
    client: &reqwest::Client,
    installation: &Installation,
    candidate: Candidate,
) -> Result<Pending> {
    installation.unchanged()?;
    let workspace = Workspace::acquire(installation)?;
    let mut sums = get(
        client,
        format!("{}/SHA256SUMS", candidate.url),
        candidate.checksums_size,
        Duration::from_secs(30),
    )
    .await?
    .bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = sums.next().await {
        let chunk = chunk.map_err(|e| e.without_url())?;
        ensure!(
            bytes.len().saturating_add(chunk.len()) as u64 <= candidate.checksums_size,
            "SHA256SUMS exceeds the release size"
        );
        bytes.extend_from_slice(&chunk);
    }
    ensure!(
        bytes.len() as u64 == candidate.checksums_size,
        "SHA256SUMS is incomplete"
    );
    let checksum = checksum(&bytes, &candidate.name)?;
    let mut stream = get(
        client,
        format!("{}/{}", candidate.url, candidate.name),
        candidate.size,
        Duration::from_secs(600),
    )
    .await?
    .bytes_stream();
    let mut file = tempfile::Builder::new()
        .prefix("candidate.AppImage")
        .rand_bytes(0)
        .tempfile_in(&workspace.path)?;
    let mut received = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.without_url())?;
        received += chunk.len() as u64;
        ensure!(
            received <= candidate.size,
            "The AppImage download exceeds the release size"
        );
        file.write_all(&chunk)?;
    }
    file.as_file().sync_all()?;
    let mut pending = Pending {
        file,
        version: candidate.version,
        size: candidate.size,
        checksum,
        _workspace: workspace,
    };
    pending.verify()?;
    Ok(pending)
}

pub struct Restart {
    pending: Pending,
    installation: std::sync::Arc<Installation>,
    backup: tempfile::TempDir,
    ready_dir: tempfile::TempDir,
    ready: UnixDatagram,
}

impl Restart {
    /// Consume the old backend before starting any replacement process.
    pub fn after_shutdown(self, backend: crate::backend::Backend, socket: &Path) -> Result<()> {
        let args = std::env::args_os().skip(1).collect::<Vec<_>>();
        self.after_shutdown_with_args(backend, socket, &args)
            .map(drop)
    }

    fn after_shutdown_with_args(
        self,
        backend: crate::backend::Backend,
        socket: &Path,
        args: &[OsString],
    ) -> Result<Child> {
        backend.join();
        let endpoint = crate::transport::Endpoint::parse(socket)?;
        self.install(&endpoint, args)
    }

    fn install(
        mut self,
        endpoint: &crate::transport::Endpoint,
        args: &[OsString],
    ) -> Result<Child> {
        let installation = &self.installation;
        require_released(endpoint)?;
        installation.unchanged()?;
        self.pending.verify()?;
        let previous = self.backup.path().join("previous.AppImage");
        ensure!(
            identity(fs::symlink_metadata(&previous)?)? == installation.identity,
            "The AppImage rollback copy changed"
        );
        self.pending.file.persist(&installation.path)?;

        let ready_path = self.ready_dir.path().join("ready.sock");
        let mut command = relaunch(&installation.path, args);
        let failed = match command.env(READY_SOCKET, ready_path).spawn() {
            Ok(mut child) => match wait_for_startup(&mut child, &self.ready) {
                Ok(()) => return Ok(child),
                Err(error) if stop_failed_child(&mut child) => error,
                Err(error) => {
                    let retained = self.backup.keep().join("previous.AppImage");
                    bail!(
                        "{error}. The new process is still running. Previous AppImage retained at {}",
                        retained.display()
                    );
                }
            },
            Err(error) => anyhow::Error::new(error).context("Cannot launch the updated AppImage"),
        };
        if let Err(error) = fs::rename(&previous, &installation.path) {
            let retained = self.backup.keep().join("previous.AppImage");
            bail!(
                "{failed:#}. Rollback failed: {error}. Previous AppImage retained at {}",
                retained.display()
            );
        }
        require_released(endpoint)
            .context("Restored the previous AppImage. Could not restart it")?;
        let child = relaunch(&installation.path, args)
            .spawn()
            .context("Restored the previous AppImage. Could not restart it")?;
        eprintln!("{failed:#}. Restored and relaunched the previous AppImage.");
        Ok(child)
    }
}

fn require_released(endpoint: &crate::transport::Endpoint) -> Result<()> {
    if let crate::transport::Endpoint::Unix(path) = endpoint {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context("Could not confirm the OpenSnitch socket was released");
            }
            Ok(_) => {
                bail!("The OpenSnitch socket is still present. Restart stopped without touching it")
            }
        }
    }
    Ok(())
}

fn relaunch(path: &Path, args: &[OsString]) -> Command {
    let mut command = Command::new(path);
    for name in [
        "APPIMAGE",
        "APPDIR",
        "ARGV0",
        "OWD",
        "LD_LIBRARY_PATH",
        "RUSTLEBLOWER_ORIGINAL_LD_LIBRARY_PATH",
        READY_SOCKET,
    ] {
        command.env_remove(name);
    }
    if let Some(original) =
        std::env::var_os("RUSTLEBLOWER_ORIGINAL_LD_LIBRARY_PATH").filter(|value| !value.is_empty())
    {
        command.env("LD_LIBRARY_PATH", original);
    }
    command
        .process_group(0)
        .env("APPIMAGE_EXTRACT_AND_RUN", "1")
        .args(args);
    command
}

fn wait_for_startup(child: &mut Child, ready: &UnixDatagram) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut bytes = [0; 64];
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            bail!("The updated AppImage exited during startup ({status})");
        }
        match ready.recv(&mut bytes) {
            Ok(size) if &bytes[..size] == READY && child.try_wait()?.is_none() => return Ok(()),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    bail!("The updated AppImage did not finish starting")
}

fn stop_failed_child(child: &mut Child) -> bool {
    let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) else {
        return false;
    };
    // The AppImage runtime can start a separate application process. Only the
    // newly spawned group is ours, and both processes must release the socket.
    if child.try_wait().is_ok_and(|status| status.is_none()) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::TERM);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().is_ok_and(|status| status.is_some())
            && rustix::process::test_kill_process_group(pid) == Err(rustix::io::Errno::SRCH)
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

pub fn signal_ready() {
    if let Some(path) = std::env::var_os(READY_SOCKET) {
        let _ = UnixDatagram::unbound().and_then(|socket| socket.send_to(READY, path));
    }
}

#[cfg(test)]
mod tests;
