use super::*;
use futures_util::StreamExt;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
struct Job {
    query: Query,
    generation: u64,
    cancel: CancellationToken,
    results: watch::Sender<ResultSet>,
}
struct Control {
    command: Command,
    reply: oneshot::Sender<std::result::Result<Snapshot, String>>,
}
#[derive(Clone)]
pub struct Handle {
    queue: mpsc::Sender<Job>,
    commands: mpsc::Sender<Control>,
    snapshot: Arc<Mutex<Snapshot>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    stop: CancellationToken,
}
pub struct Ticket {
    receiver: watch::Receiver<ResultSet>,
    cancel: CancellationToken,
}
impl Ticket {
    pub fn snapshot(&self) -> ResultSet {
        self.receiver.borrow().clone()
    }
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl Handle {
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().unwrap().clone()
    }
    pub fn shutdown(&self) {
        self.stop.cancel();
    }
    pub fn request(
        &self,
        command: Command,
    ) -> oneshot::Receiver<std::result::Result<Snapshot, String>> {
        let (tx, rx) = oneshot::channel();
        if let Command::Save {
            expected_revision,
            settings,
        } = &command
            && (expected_revision
                .as_ref()
                .is_some_and(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
                || settings.validate().is_err())
        {
            let _ = tx.send(Err(
                "Invalid reputation preferences; review the fields and reload".into(),
            ));
            return rx;
        }
        if let Err(error) = self.commands.try_send(Control { command, reply: tx }) {
            let _ = error
                .into_inner()
                .reply
                .send(Err("Reputation checks are busy or unavailable".into()));
        }
        rx
    }
    pub fn lookup(&self, query: Query) -> Ticket {
        let (snapshot, generation) = self.captured();
        self.lookup_at(query, snapshot, generation)
    }
    fn captured(&self) -> (Snapshot, u64) {
        let snapshot = self.snapshot.lock().unwrap();
        (
            snapshot.clone(),
            self.generation.load(std::sync::atomic::Ordering::Acquire),
        )
    }
    fn lookup_at(&self, query: Query, snapshot: Snapshot, generation: u64) -> Ticket {
        let invalid = query.subjects.is_empty()
            || query.subjects.len() > 3
            || query.subjects.iter().any(|s| s.validate().is_err());
        let status = if invalid {
            Status::NoData
        } else if !snapshot.settings.enabled {
            Status::Disabled
        } else {
            Status::Queued
        };
        let (results, receiver) = watch::channel(ResultSet {
            capture: query.capture,
            settings_revision: snapshot.revision.clone(),
            status: status.clone(),
            reports: Vec::new(),
        });
        let cancel = CancellationToken::new();
        let ticket = Ticket {
            receiver,
            cancel: cancel.clone(),
        };
        if status != Status::Queued {
            return ticket;
        }
        let job = Job {
            query,
            generation,
            cancel,
            results,
        };
        if let Err(error) = self.queue.try_send(job) {
            set_status(&error.into_inner(), Status::Busy);
        }
        ticket
    }
    /// Captures only request identity and three small reported values; never
    /// retains the full Connection, opens a binary or waits for disk/network.
    pub fn lookup_connection(
        &self,
        capture: Capture,
        connection: &crate::protocol::Connection,
    ) -> Ticket {
        let (snapshot, generation) = self.captured();
        let mut subjects = Vec::new();
        let ip = connection.dst_ip.parse::<IpAddr>().ok();
        let host = if connection.dst_host.len() <= 254 {
            connection
                .dst_host
                .trim_end_matches('.')
                .to_ascii_lowercase()
        } else {
            String::new()
        };
        let excluded = connection.dst_host.len() > 254
            || connection.dst_ip.len() > 64
            || host.parse::<IpAddr>().ok().is_some_and(private)
            || ip.is_some_and(private)
            || snapshot
                .settings
                .excluded_ips
                .iter()
                .any(|p| connection.dst_ip.contains(p))
            || snapshot
                .settings
                .excluded_domains
                .iter()
                .any(|p| host.contains(&p.to_ascii_lowercase()))
            || ["www.virustotal.com", "virustotal.com"]
                .iter()
                .any(|candidate| host == *candidate);
        if !excluded {
            if let Ok(domain) = Subject::domain(&connection.dst_host) {
                subjects.push(domain);
            }
            if let Some(ip) = ip {
                subjects.push(Subject::Ip(ip));
            }
            for algorithm in [
                "process.hash.sha256",
                "process.hash.sha1",
                "process.hash.md5",
            ] {
                if let Some(hash) = connection
                    .process_checksums
                    .get(algorithm)
                    .and_then(|v| Subject::hash(v).ok())
                {
                    subjects.push(hash);
                    break;
                }
            }
        }
        if excluded {
            let (_, receiver) = watch::channel(ResultSet {
                capture,
                settings_revision: snapshot.revision,
                status: Status::Excluded,
                reports: Vec::new(),
            });
            return Ticket {
                receiver,
                cancel: CancellationToken::new(),
            };
        }
        self.lookup_at(Query { capture, subjects }, snapshot, generation)
    }
}
pub struct Worker {
    handle: Handle,
    thread: Option<std::thread::JoinHandle<()>>,
}
#[derive(Clone)]
enum Endpoint {
    Official,
    #[cfg(test)]
    Fixture(String),
}
impl Worker {
    pub fn start(path: PathBuf) -> Self {
        Self::start_at(path, Endpoint::Official)
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(crate) fn start_fixture(path: PathBuf, base: &str) -> Result<Self> {
        Self::fixture(path, base)
    }
    #[cfg(test)]
    pub(super) fn fixture(path: PathBuf, base: &str) -> Result<Self> {
        fixture_guard(base)?;
        Ok(Self::start_at(
            path,
            Endpoint::Fixture(base.trim_end_matches('/').into()),
        ))
    }
    fn start_at(path: PathBuf, endpoint: Endpoint) -> Self {
        let (queue, rx) = mpsc::channel(MAX_QUEUE);
        let (commands, control_rx) = mpsc::channel(MAX_QUEUE);
        let snapshot = Arc::new(Mutex::new(Snapshot {
            ready: false,
            settings: Settings::default(),
            revision: None,
            key_status: KeyStatus::Missing,
            warning: None,
            dispatched: 0,
        }));
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let stop = CancellationToken::new();
        let handle = Handle {
            queue,
            commands,
            snapshot: snapshot.clone(),
            generation: generation.clone(),
            stop: stop.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("reputation".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .max_blocking_threads(2)
                    .enable_all()
                    .build();
                if let Ok(runtime) = runtime {
                    runtime.block_on(run(
                        path,
                        endpoint,
                        (rx, control_rx),
                        snapshot.clone(),
                        generation,
                        stop,
                    ));
                    runtime.shutdown_timeout(Duration::from_millis(250));
                } else {
                    let mut snapshot = snapshot.lock().unwrap();
                    snapshot.ready = true;
                    snapshot.warning = Some("Reputation checks are unavailable".into());
                }
            })
            .ok();
        if thread.is_none() {
            let mut snapshot = handle.snapshot.lock().unwrap();
            snapshot.ready = true;
            snapshot.warning = Some("Reputation checks are unavailable".into());
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
fn set_status(job: &Job, status: Status) {
    job.results.send_modify(|r| r.status = status);
}
struct Cache {
    subject: Subject,
    report: Report,
    checked: Instant,
    credential: [u8; 32],
}
fn apply(
    control: Control,
    path: &Path,
    snapshot: &Arc<Mutex<Snapshot>>,
    generation: &std::sync::atomic::AtomicU64,
    cache: &mut VecDeque<Cache>,
) -> bool {
    let spent = snapshot.lock().unwrap().dispatched;
    let loaded = match control.command {
        Command::Reload => Ok(store::load(path)),
        Command::Save {
            expected_revision,
            settings,
        } => store::save(path, &expected_revision, &settings),
    };
    let result = loaded
        .map(|mut loaded| {
            loaded.dispatched = spent;
            let mut target = snapshot.lock().unwrap();
            generation.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            cache.clear();
            *target = loaded.clone();
            loaded
        })
        .map_err(|_| "Could not save reputation preferences; check the key file and reload".into());
    let changed = result.is_ok();
    let _ = control.reply.send(result);
    changed
}
async fn run(
    path: PathBuf,
    endpoint: Endpoint,
    channels: (mpsc::Receiver<Job>, mpsc::Receiver<Control>),
    snapshot: Arc<Mutex<Snapshot>>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    stop: CancellationToken,
) {
    let (mut rx, mut controls) = channels;
    *snapshot.lock().unwrap() = store::load(&path);
    let mut cache = VecDeque::<Cache>::new();
    let mut client = None;
    let mut limited_until = None;
    loop {
        let job = tokio::select! {biased;
            _=stop.cancelled()=>break,
            control=controls.recv()=>{if let Some(control)=control{apply(control,&path,&snapshot,&generation,&mut cache);continue;}else{break;}},
            job=rx.recv()=>match job{Some(job)=>job,None=>break},
        };
        if job.cancel.is_cancelled() {
            set_status(&job, Status::Cancelled);
            continue;
        }
        if job.generation != generation.load(std::sync::atomic::Ordering::Acquire) {
            set_status(&job, Status::Stale);
            continue;
        }
        let settings = snapshot.lock().unwrap().settings.clone();
        if !settings.enabled {
            set_status(&job, Status::Disabled);
            continue;
        }
        let secret = match store::key(settings.api_key_file.as_deref()) {
            Ok(secret) => {
                snapshot.lock().unwrap().key_status = KeyStatus::Ready;
                secret
            }
            Err(_) => {
                let status = if settings.api_key_file.is_some() {
                    snapshot.lock().unwrap().key_status = KeyStatus::Invalid;
                    Status::InvalidKey
                } else {
                    Status::MissingKey
                };
                set_status(&job, status);
                continue;
            }
        };
        use sha2::Digest;
        let credential: [u8; 32] = sha2::Sha256::digest(secret.bytes()).into();
        if client.is_none() {
            client = reqwest::Client::builder()
                .user_agent(concat!("Rustleblower/", env!("CARGO_PKG_VERSION")))
                .pool_max_idle_per_host(1)
                .connect_timeout(Duration::from_secs(3))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .ok();
        }
        let Some(client) = client.as_ref() else {
            set_status(&job, Status::Failed);
            continue;
        };
        set_status(&job, Status::Checking);
        let mut reports = Vec::new();
        let mut interrupted = false;
        for subject in job.query.subjects.iter().cloned() {
            if job.cancel.is_cancelled() {
                set_status(&job, Status::Cancelled);
                interrupted = true;
                break;
            }
            if subject.excluded(&settings) {
                reports.push(Report::status(subject, Status::Excluded));
                continue;
            }
            cache.retain(|entry| entry.checked.elapsed() < Duration::from_secs(86400));
            if let Some(entry) = cache
                .iter()
                .find(|entry| entry.subject == subject && entry.credential == credential)
            {
                reports.push(entry.report.clone());
                continue;
            }
            if limited_until.is_some_and(|until| Instant::now() < until) {
                reports.push(Report::status(subject, Status::RateLimited));
                continue;
            }
            if snapshot.lock().unwrap().dispatched >= settings.request_budget {
                reports.push(Report::status(subject, Status::QuotaExceeded));
                continue;
            }
            snapshot.lock().unwrap().dispatched += 1;
            let request = fetch(client, &endpoint, &subject, &settings, &secret);
            tokio::pin!(request);
            let report = loop {
                tokio::select! {biased;
                    _=stop.cancelled()=>{set_status(&job,Status::Cancelled);interrupted=true;break None;}
                    _=job.cancel.cancelled()=>{set_status(&job,Status::Cancelled);interrupted=true;break None;}
                    control=controls.recv()=>{
                        if let Some(control)=control{if apply(control,&path,&snapshot,&generation,&mut cache){set_status(&job,Status::Stale);interrupted=true;break None;}}
                        else{set_status(&job,Status::Cancelled);interrupted=true;break None;}
                    }
                    report=&mut request=>break Some(report),
                }
            };
            let Some(report) = report else {
                break;
            };
            if report.status == Status::RateLimited {
                limited_until = Some(Instant::now() + Duration::from_secs(300));
            }
            if report.status == Status::InvalidKey {
                snapshot.lock().unwrap().key_status = KeyStatus::Invalid;
            }
            if matches!(
                report.status,
                Status::Complete | Status::NoData | Status::NotFound
            ) {
                if cache.len() == MAX_CACHE {
                    cache.pop_front();
                }
                cache.push_back(Cache {
                    subject: subject.clone(),
                    report: report.clone(),
                    checked: Instant::now(),
                    credential,
                });
            }
            reports.push(report);
        }
        if !interrupted {
            let status = if reports.iter().any(|r| r.status == Status::Complete) {
                Status::Complete
            } else {
                reports
                    .first()
                    .map(|r| r.status.clone())
                    .unwrap_or(Status::NoData)
            };
            job.results.send_modify(|r| {
                r.status = status;
                r.reports = reports;
            });
        }
    }
    while let Ok(job) = rx.try_recv() {
        set_status(&job, Status::Cancelled);
    }
}
async fn fetch(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    subject: &Subject,
    settings: &Settings,
    secret: &store::Secret,
) -> Report {
    let mut url = match endpoint {
        Endpoint::Official => reqwest::Url::parse("https://www.virustotal.com/api/v3/").unwrap(),
        #[cfg(test)]
        Endpoint::Fixture(base) => reqwest::Url::parse(&format!("{base}/api/v3/")).unwrap(),
    };
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .push(subject.category())
        .push(&subject.value());
    let mut key = match reqwest::header::HeaderValue::from_bytes(secret.bytes()) {
        Ok(key) => key,
        Err(_) => return Report::status(subject.clone(), Status::InvalidKey),
    };
    key.set_sensitive(true);
    let result = tokio::time::timeout(
        Duration::from_secs(u64::from(settings.timeout_seconds)),
        async {
            let response = client
                .get(url)
                .header("x-apikey", key)
                .send()
                .await
                .map_err(|_| Status::Failed)?;
            match response.status().as_u16() {
                200 => {}
                401 => return Err(Status::InvalidKey),
                403 | 204 | 429 => return Err(Status::RateLimited),
                404 => return Err(Status::NotFound),
                _ => return Err(Status::Failed),
            }
            if response
                .content_length()
                .is_some_and(|len| len > MAX_BODY as u64)
            {
                return Err(Status::Failed);
            }
            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| Status::Failed)?;
                if body.len().saturating_add(chunk.len()) > MAX_BODY {
                    return Err(Status::Failed);
                }
                body.extend_from_slice(&chunk);
            }
            let mut report = parse(subject.clone(), &body, settings);
            // A service/fixture echo must not turn a secret header into UI text.
            let key = std::str::from_utf8(secret.bytes()).map_err(|_| Status::InvalidKey)?;
            report.details = report.details.replace(key, "*");
            Ok(report)
        },
    )
    .await;
    match result {
        Ok(Ok(report)) => report,
        Ok(Err(status)) => Report::status(subject.clone(), status),
        Err(_) => Report::status(subject.clone(), Status::TimedOut),
    }
}
#[cfg(test)]
pub(super) fn fixture_guard(base: &str) -> Result<()> {
    let map = std::fs::read_to_string("/proc/self/uid_map")?;
    let fields: Vec<_> = map.split_whitespace().collect();
    ensure!(
        fields.len() == 3 && fields[0] == "0" && fields[1] != "0" && fields[2] == "1",
        "Use scripts/owned_net.py"
    );
    let output = std::process::Command::new("ip")
        .args(["-j", "link", "show"])
        .output()?;
    let links: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)?;
    ensure!(
        output.status.success() && links.len() == 1 && links[0]["ifname"] == "lo",
        "Use owned loopback namespace"
    );
    ensure!(base.len() <= 128, "Fixture origin too long");
    let url = reqwest::Url::parse(base)?;
    let ip: IpAddr = url
        .host_str()
        .unwrap_or("")
        .trim_matches(['[', ']'])
        .parse()?;
    ensure!(
        url.scheme() == "http"
            && ip.is_loopback()
            && url.port().is_some_and(|p| p != 0)
            && url.username().is_empty()
            && url.password().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none(),
        "Invalid fixture origin"
    );
    Ok(())
}
