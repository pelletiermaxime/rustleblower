//! Opt-in release checks. This worker never installs software or participates in
//! daemon permission admission. URLs are fixed; settings and payloads are bounded.
use anyhow::{Result, ensure};
use futures_util::StreamExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, oneshot, watch};

const SETTINGS_BYTES: usize = 4096;
const RESPONSE_BYTES: usize = 256 * 1024;
const RELEASES: usize = 30;
const TIMEOUT: Duration = Duration::from_secs(10);
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    #[default]
    Application,
    SelectedDaemon,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Seconds,
    Minutes,
    #[default]
    Hours,
    Days,
}
impl Unit {
    fn seconds(self) -> u64 {
        match self {
            Self::Seconds => 1,
            Self::Minutes => 60,
            Self::Hours => 3600,
            Self::Days => 86400,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub check_on_start: bool,
    pub interval: u32,
    pub unit: Unit,
    pub target: Target,
    pub include_prereleases: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            check_on_start: true,
            interval: 12,
            unit: Unit::Hours,
            target: Target::Application,
            include_prereleases: false,
        }
    }
}
impl Settings {
    pub fn duration(&self) -> Result<Duration> {
        let seconds = u64::from(self.interval) * self.unit.seconds();
        ensure!(
            (1..=31_536_000).contains(&seconds),
            "interval must be between one second and 365 days"
        );
        Ok(Duration::from_secs(seconds))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetVersion {
    pub node: String,
    pub version: String,
}
impl TargetVersion {
    fn bounded(self) -> Option<Self> {
        (self.node.len() <= 128
            && self.version.len() <= 128
            && !self
                .node
                .chars()
                .chain(self.version.chars())
                .any(char::is_control))
        .then_some(self)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Disabled,
    Idle,
    Checking,
    Current,
    Available,
    NoStableRelease,
    NoComparableRelease,
    UnknownCurrentVersion,
    RateLimited,
    Cancelled,
    Failed(String),
}
impl Outcome {
    pub fn label(&self) -> String {
        match self {
            Self::Disabled => "Automatic update checks are off".into(),
            Self::Idle => "Ready to check".into(),
            Self::Checking => "Checking official releases…".into(),
            Self::Current => "No newer release for these settings".into(),
            Self::Available => "A newer release is available".into(),
            Self::NoStableRelease => "No published stable release is available".into(),
            Self::NoComparableRelease => "No release with a recognized version was found".into(),
            Self::UnknownCurrentVersion => {
                "The selected daemon has not reported a recognized version".into()
            }
            Self::RateLimited => "GitHub rate limit reached; try again later".into(),
            Self::Cancelled => "Check cancelled".into(),
            Self::Failed(message) => message.clone(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct State {
    /// Initial preference read has completed; inspecting/opening a view never
    /// needs to Reload or cancel an already running check.
    pub ready: bool,
    pub settings: Settings,
    pub revision: Option<String>,
    pub warning: Option<String>,
    pub outcome: Outcome,
    pub busy: bool,
    pub current: Option<TargetVersion>,
    pub latest: Option<String>,
    pub release_url: Option<String>,
    pub checked_at: Option<u64>,
    pub completed_checks: u64,
    pub next_due: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct Notice {
    pub target: Target,
    pub node: String,
    pub current: String,
    pub latest: String,
    pub release_url: String,
}
pub enum Command {
    CheckNow,
    Cancel,
    Reload,
    Save {
        expected_revision: Option<String>,
        settings: Settings,
    },
}
struct Request {
    command: Command,
    reply: oneshot::Sender<std::result::Result<State, String>>,
}
#[derive(Clone)]
pub struct Handle {
    commands: mpsc::Sender<Request>,
    daemon: watch::Sender<Option<TargetVersion>>,
    stop: watch::Sender<bool>,
    state: Arc<Mutex<State>>,
}
impl Handle {
    pub fn request(
        &self,
        command: Command,
    ) -> oneshot::Receiver<std::result::Result<State, String>> {
        let (reply, rx) = oneshot::channel();
        if let Command::Save {
            expected_revision: Some(revision),
            ..
        } = &command
            && (revision.len() != 64 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            let _ = reply.send(Err(
                "Invalid preferences revision; reload before saving".into()
            ));
            return rx;
        }
        if let Err(error) = self.commands.try_send(Request { command, reply }) {
            let _ = error
                .into_inner()
                .reply
                .send(Err("Update worker is busy or stopped".into()));
        }
        rx
    }
    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }
    /// Latest-only bounded input; daemon-reported metadata, never a package guess.
    pub fn set_daemon(&self, daemon: Option<TargetVersion>) {
        let daemon = daemon.and_then(TargetVersion::bounded);
        self.daemon.send_if_modified(|current| {
            if *current == daemon {
                false
            } else {
                *current = daemon;
                true
            }
        });
    }
    pub fn shutdown(&self) {
        self.stop.send_replace(true);
    }
}
pub struct Worker {
    handle: Handle,
    thread: Option<std::thread::JoinHandle<()>>,
}
type Notify = Arc<dyn Fn(Notice) + Send + Sync>;
impl Worker {
    /// `path` is a dedicated preferences file, normally `path_for(database)`.
    /// The callback must use a nonblocking bounded mailbox; it runs on this worker.
    pub fn start(path: PathBuf, notify: Option<Notify>) -> Self {
        Self::start_at(path, notify, Endpoints::Official)
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(crate) fn start_fixture(path: PathBuf, notify: Option<Notify>, base: &str) -> Result<Self> {
        require_fixture_namespace()?;
        ensure!(base.len() <= 128, "fixture origin too long");
        let url = reqwest::Url::parse(base)?;
        let ip: std::net::IpAddr = url
            .host_str()
            .unwrap_or("")
            .trim_matches(['[', ']'])
            .parse()?;
        ensure!(
            url.scheme() == "http"
                && ip.is_loopback()
                && url.port().is_some_and(|port| port != 0)
                && url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none(),
            "fixture must be a plain owned loopback origin"
        );
        Ok(Self::start_at(
            path,
            notify,
            Endpoints::Fixture(base.trim_end_matches('/').into()),
        ))
    }
    fn start_at(path: PathBuf, notify: Option<Notify>, endpoints: Endpoints) -> Self {
        let (commands, rx) = mpsc::channel(8);
        let (daemon, daemon_rx) = watch::channel(None);
        let (stop, stop_rx) = watch::channel(false);
        let state = Arc::new(Mutex::new(State {
            ready: false,
            settings: Settings::default(),
            revision: None,
            warning: None,
            outcome: Outcome::Disabled,
            busy: false,
            current: None,
            latest: None,
            release_url: None,
            checked_at: None,
            completed_checks: 0,
            next_due: None,
        }));
        let handle = Handle {
            commands,
            daemon,
            stop,
            state: state.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("release-checks".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .max_blocking_threads(2)
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => {
                        runtime.block_on(run(
                            path,
                            notify,
                            endpoints,
                            rx,
                            daemon_rx,
                            stop_rx,
                            state.clone(),
                        ));
                        // Cancelling HTTP cannot interrupt a running OS DNS
                        // lookup. Bound the join wait; at most two resolver
                        // blocking threads may finish after this worker exits.
                        runtime.shutdown_timeout(Duration::from_millis(250));
                    }
                    Err(_) => {
                        let mut state = state.lock().unwrap();
                        state.ready = true;
                        state.outcome = Outcome::Failed("Could not start update worker".into());
                    }
                }
            })
            .ok();
        if thread.is_none() {
            let mut state = handle.state.lock().unwrap();
            state.ready = true;
            state.outcome = Outcome::Failed("Could not start update worker".into());
        }
        Self { handle, thread }
    }
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }
    pub fn join(mut self) {
        self.handle.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.handle.shutdown();
    }
}
pub fn path_for(database: &Path) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(".updates.json");
    PathBuf::from(path)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    settings: Settings,
}
fn read(path: &Path) -> Result<(Settings, Option<String>)> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Settings::default(), None));
        }
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.is_file(),
        "preferences must be a regular file"
    );
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(SETTINGS_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= SETTINGS_BYTES, "preferences exceed 4 KiB");
    let revision = Some(hex::encode(Sha256::digest(&bytes)));
    let document: Document = serde_json::from_slice(&bytes)?;
    ensure!(document.version == 1, "unsupported preferences version");
    document.settings.duration()?;
    Ok((document.settings, revision))
}
fn save(path: &Path, expected: &Option<String>, settings: &Settings) -> Result<Option<String>> {
    settings.duration()?;
    let (_, current) = read(path)?;
    ensure!(
        &current == expected,
        "preferences changed on disk; reload before saving"
    );
    let bytes = serde_json::to_vec_pretty(&Document {
        version: 1,
        settings: settings.clone(),
    })?;
    ensure!(bytes.len() <= SETTINGS_BYTES, "preferences exceed 4 KiB");
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    // Recheck after preparing publication; first creation must never replace a
    // file another process created during the save.
    let (_, current) = read(path)?;
    ensure!(
        &current == expected,
        "preferences changed on disk; reload before saving"
    );
    if expected.is_none() {
        temp.persist_noclobber(path).map_err(|error| error.error)?;
    } else {
        temp.persist(path).map_err(|error| error.error)?;
    }
    File::open(parent)?.sync_all()?;
    Ok(Some(hex::encode(Sha256::digest(bytes))))
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn current(settings: &Settings, daemon: &Option<TargetVersion>) -> Option<TargetVersion> {
    match settings.target {
        Target::Application => Some(TargetVersion {
            node: "Rustleblower".into(),
            version: APP_VERSION.into(),
        }),
        Target::SelectedDaemon => daemon.clone(),
    }
}
fn version(text: &str) -> Option<Version> {
    (text.len() <= 128)
        .then(|| Version::parse(text.trim().strip_prefix('v').unwrap_or(text.trim())).ok())
        .flatten()
}
#[derive(Clone)]
enum Endpoints {
    Official,
    #[cfg(test)]
    Fixture(String),
}
impl Endpoints {
    fn url(&self, settings: &Settings) -> String {
        #[cfg(test)]
        if let Self::Fixture(base) = self {
            return format!(
                "{base}/{}",
                if settings.include_prereleases {
                    "releases?per_page=30"
                } else {
                    "releases/latest"
                }
            );
        }
        let repo = match settings.target {
            Target::Application => "pelletiermaxime/rustleblower",
            Target::SelectedDaemon => "evilsocket/opensnitch",
        };
        format!(
            "https://api.github.com/repos/{repo}/{}",
            if settings.include_prereleases {
                "releases?per_page=30"
            } else {
                "releases/latest"
            }
        )
    }
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}
struct Checked {
    outcome: Outcome,
    latest: Option<String>,
    url: Option<String>,
}
impl Checked {
    fn outcome(outcome: Outcome) -> Self {
        Self {
            outcome,
            latest: None,
            url: None,
        }
    }
}
async fn check(
    client: &reqwest::Client,
    settings: Settings,
    target: TargetVersion,
    endpoints: Endpoints,
) -> Checked {
    let result = tokio::time::timeout(TIMEOUT, fetch(client, &settings, &target, &endpoints)).await;
    match result {
        Ok(Ok(result)) => result,
        Ok(Err(error))
            if error
                .downcast_ref::<reqwest::Error>()
                .is_some_and(reqwest::Error::is_timeout) =>
        {
            Checked::outcome(Outcome::Failed("Release check timed out".into()))
        }
        Ok(Err(_)) => Checked::outcome(Outcome::Failed(
            "Could not read release information from the official service".into(),
        )),
        Err(_) => Checked::outcome(Outcome::Failed("Release check timed out".into())),
    }
}
async fn fetch(
    client: &reqwest::Client,
    settings: &Settings,
    target: &TargetVersion,
    endpoints: &Endpoints,
) -> Result<Checked> {
    let Some(installed) = version(&target.version) else {
        return Ok(Checked::outcome(Outcome::UnknownCurrentVersion));
    };
    let response = client
        .get(endpoints.url(settings))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    let status = response.status();
    if status.as_u16() == 404 && !settings.include_prereleases {
        return Ok(Checked::outcome(Outcome::NoStableRelease));
    }
    if status.as_u16() == 429
        || (status.as_u16() == 403
            && response
                .headers()
                .get("x-ratelimit-remaining")
                .is_some_and(|v| v == "0"))
    {
        return Ok(Checked::outcome(Outcome::RateLimited));
    }
    ensure!(status.is_success(), "release endpoint refused request");
    ensure!(
        response
            .content_length()
            .is_none_or(|len| len <= RESPONSE_BYTES as u64),
        "release body too large"
    );
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        ensure!(
            body.len().saturating_add(chunk.len()) <= RESPONSE_BYTES,
            "release body too large"
        );
        body.extend_from_slice(&chunk);
    }
    let releases: Vec<Release> = if settings.include_prereleases {
        let releases: Vec<Release> = serde_json::from_slice(&body)?;
        ensure!(releases.len() <= RELEASES, "too many releases");
        releases
    } else {
        vec![serde_json::from_slice(&body)?]
    };
    let latest = releases
        .into_iter()
        .filter(|r| !r.draft && r.tag_name.len() <= 128)
        .filter_map(|r| {
            let v = version(&r.tag_name)?;
            (settings.include_prereleases || (!r.prerelease && v.pre.is_empty()))
                .then_some((v, r.tag_name))
        })
        .max_by(|a, b| a.0.cmp_precedence(&b.0));
    let Some((latest, tag)) = latest else {
        return Ok(Checked::outcome(Outcome::NoComparableRelease));
    };
    let repo = match settings.target {
        Target::Application => "pelletiermaxime/rustleblower",
        Target::SelectedDaemon => "evilsocket/opensnitch",
    };
    // Construct the URL from a validated semantic tag; never trust an API-supplied link.
    let mut url = reqwest::Url::parse(&format!("https://github.com/{repo}/releases/tag/"))?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("invalid release base"))?
        .pop_if_empty()
        .push(&tag);
    Ok(Checked {
        outcome: if latest.cmp_precedence(&installed).is_gt() {
            Outcome::Available
        } else {
            Outcome::Current
        },
        latest: Some(latest.to_string()),
        url: Some(url.to_string()),
    })
}
type Inflight = std::pin::Pin<Box<dyn std::future::Future<Output = Checked> + Send>>;
async fn run(
    path: PathBuf,
    notify: Option<Notify>,
    endpoints: Endpoints,
    mut rx: mpsc::Receiver<Request>,
    mut daemon: watch::Receiver<Option<TargetVersion>>,
    mut stop: watch::Receiver<bool>,
    state: Arc<Mutex<State>>,
) {
    let mut client: Option<reqwest::Client> = None;
    let mut model = state.lock().unwrap().clone();
    load(&path, &mut model);
    let mut due = schedule(&model.settings, model.settings.check_on_start);
    let mut inflight: Option<Inflight> = None;
    let mut context: Option<TargetVersion> = None;
    let mut last_notice: Option<(Target, String, String, String)> = None;
    loop {
        model.current = current(&model.settings, &daemon.borrow());
        model.next_due = due.map(|when| {
            now().saturating_add(
                when.saturating_duration_since(tokio::time::Instant::now())
                    .as_secs(),
            )
        });
        *state.lock().unwrap() = model.clone();
        let wake =
            due.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(31_536_000));
        tokio::select! {
            biased;
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break; } }
            changed = daemon.changed() => {
                if changed.is_err() { break; }
                if model.settings.target == Target::SelectedDaemon {
                    inflight = None; model.busy = false; model.latest = None; model.release_url = None;
                    model.outcome = if model.settings.enabled { Outcome::Idle } else { Outcome::Disabled };
                    due = schedule(&model.settings, model.settings.check_on_start);
                }
            }
            request = rx.recv() => {
                let Some(request) = request else { break; };
                let result: Result<()> = match request.command {
                    Command::CheckNow => {
                        if model.busy { Err(anyhow::anyhow!("A release check is already running")) }
                        else { due = Some(tokio::time::Instant::now()); Ok(()) }
                    }
                    Command::Cancel => { inflight = None; model.busy = false; model.outcome = Outcome::Cancelled; due = schedule(&model.settings, false); Ok(()) }
                    Command::Reload => { inflight = None; model.busy = false; load(&path, &mut model); due = schedule(&model.settings, false); Ok(()) }
                    Command::Save { expected_revision, settings } => {
                        match save(&path, &expected_revision, &settings) {
                            Ok(revision) => { inflight = None; model.busy = false; model.settings = settings; model.revision = revision; model.warning = None; model.latest = None; model.release_url = None; model.outcome = if model.settings.enabled { Outcome::Idle } else { Outcome::Disabled }; due = schedule(&model.settings, false); Ok(()) }
                            Err(error) => Err(error),
                        }
                    }
                };
                let _ = request.reply.send(result.map(|_| model.clone()).map_err(|e| safe_error(&e)));
            }
            checked = async { match inflight.as_mut() { Some(job) => job.await, None => std::future::pending().await } } => {
                inflight = None; model.busy = false; model.checked_at = Some(now());
                model.completed_checks = model.completed_checks.saturating_add(1);
                model.outcome = checked.outcome; model.latest = checked.latest; model.release_url = checked.url;
                if model.outcome == Outcome::Available && let (Some(target), Some(latest), Some(url)) = (&context, &model.latest, &model.release_url) {
                    let key = (model.settings.target, target.node.clone(), target.version.clone(), latest.clone());
                    if last_notice.as_ref() != Some(&key) {
                        if let Some(callback) = &notify {
                            let notice = Notice { target: key.0, node: key.1.clone(), current: key.2.clone(), latest: key.3.clone(), release_url: url.clone() };
                            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(notice)));
                        }
                        last_notice = Some(key);
                    }
                }
                due = schedule(&model.settings, false);
                if model.outcome == Outcome::RateLimited && let Some(next) = &mut due {
                    *next = (*next).max(tokio::time::Instant::now() + Duration::from_secs(60));
                }
            }
            _ = tokio::time::sleep_until(wake), if inflight.is_none() && due.is_some() => {
                due = None;
                context = model.current.clone();
                match context.clone().filter(|v| version(&v.version).is_some()) {
                    Some(target) => { model.busy = true; model.outcome = Outcome::Checking; model.latest = None; model.release_url = None;
                        if client.is_none() {
                            match release_client() {
                                Ok(created) => client = Some(created),
                                Err(_) => { model.busy = false; model.outcome = Outcome::Failed("Could not initialize release client".into()); due = schedule(&model.settings, false); continue; }
                            }
                        }
                        let client = client.as_ref().unwrap().clone(); let settings = model.settings.clone(); let endpoints = endpoints.clone();
                        inflight = Some(Box::pin(async move { check(&client, settings, target, endpoints).await }));
                    }
                    None => { model.outcome = Outcome::UnknownCurrentVersion; model.checked_at = Some(now()); model.completed_checks = model.completed_checks.saturating_add(1); due = schedule(&model.settings, false); }
                }
            }
        }
    }
    model.busy = false;
    if model.outcome == Outcome::Checking {
        model.outcome = Outcome::Cancelled;
    }
    model.next_due = None;
    *state.lock().unwrap() = model;
}
fn release_client() -> std::result::Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(concat!("Rustleblower/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(5))
        .timeout(TIMEOUT)
        .pool_max_idle_per_host(1)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 3
                && attempt.url().scheme() == "https"
                && attempt.url().host_str() == Some("api.github.com")
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
}
#[cfg(test)]
fn require_fixture_namespace() -> Result<()> {
    let mapping = std::fs::read_to_string("/proc/self/uid_map")?;
    let fields: Vec<_> = mapping.split_whitespace().collect();
    ensure!(
        fields.len() == 3 && fields[0] == "0" && fields[1] != "0" && fields[2] == "1",
        "use scripts/owned_net.py"
    );
    let output = std::process::Command::new("ip")
        .args(["-j", "link", "show"])
        .output()?;
    ensure!(output.status.success(), "cannot inspect owned namespace");
    let links: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)?;
    ensure!(
        links.len() == 1 && links[0]["ifname"] == "lo",
        "fixture namespace must contain only loopback"
    );
    Ok(())
}
fn safe_error(error: &anyhow::Error) -> String {
    // Filesystem/JSON errors may include file contents or paths. Return only
    // our own actionable messages; never interpolate untrusted error strings.
    let text = error.to_string();
    if text == "preferences changed on disk; reload before saving"
        || text == "interval must be between one second and 365 days"
        || text == "A release check is already running"
    {
        text
    } else {
        "Could not read or save update preferences; reload to inspect the file".into()
    }
}
fn load(path: &Path, model: &mut State) {
    model.ready = true;
    match read(path) {
        Ok((settings, revision)) => {
            model.settings = settings;
            model.revision = revision;
            model.warning = None;
        }
        Err(_) => {
            model.settings = Settings::default();
            model.revision = None;
            model.warning = Some(
                "Update preferences are invalid or inaccessible; existing file preserved".into(),
            );
        }
    }
    model.outcome = if model.settings.enabled {
        Outcome::Idle
    } else {
        Outcome::Disabled
    };
    model.latest = None;
    model.release_url = None;
}
fn schedule(settings: &Settings, immediate: bool) -> Option<tokio::time::Instant> {
    settings.enabled.then(|| {
        tokio::time::Instant::now()
            + if immediate {
                Duration::ZERO
            } else {
                settings.duration().unwrap_or(Duration::from_secs(43200))
            }
    })
}

#[cfg(test)]
mod tests;
