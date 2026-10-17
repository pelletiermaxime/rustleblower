use super::*;
use futures_util::StreamExt;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
type Notify = Arc<dyn Fn(Notice) + Send + Sync>;
struct Request {
    command: Command,
    reply: oneshot::Sender<std::result::Result<Reply, String>>,
}
#[derive(Clone)]
pub struct Handle {
    queue: mpsc::Sender<Request>,
    snapshot: Arc<Mutex<Snapshot>>,
    active: Arc<Mutex<Option<CancellationToken>>>,
    stop: CancellationToken,
}
impl Handle {
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().unwrap().clone()
    }
    pub fn shutdown(&self) {
        self.stop.cancel();
        if let Some(active) = self.active.lock().unwrap().as_ref() {
            active.cancel();
        }
    }
    pub fn request(
        &self,
        command: Command,
    ) -> oneshot::Receiver<std::result::Result<Reply, String>> {
        let (reply, rx) = oneshot::channel();
        let invalid = match &command {
            Command::Save {
                expected_revision,
                settings,
            } => bad_revision(expected_revision) || settings.validate().is_err(),
            Command::Import {
                expected_revision,
                json,
            } => bad_revision(expected_revision) || json.len() > MAX_CONFIG,
            Command::RunNow {
                group,
                expected_revision,
            } => !name(group) || bad_revision(expected_revision),
            Command::ReadProfile { path } => super::path(path).is_err(),
            Command::WriteProfile {
                path,
                expected_revision,
            } => super::path(path).is_err() || bad_revision(expected_revision),
            _ => false,
        };
        if invalid {
            let _ = reply.send(Err("Invalid raw download configuration or revision".into()));
            return rx;
        }
        if matches!(
            command,
            Command::Cancel | Command::Reload | Command::Save { .. } | Command::Import { .. }
        ) && let Some(active) = self.active.lock().unwrap().as_ref()
        {
            active.cancel();
        }
        if let Err(error) = self.queue.try_send(Request { command, reply }) {
            let _ = error
                .into_inner()
                .reply
                .send(Err("Raw downloads are busy or unavailable".into()));
        }
        rx
    }
}
fn bad_revision(value: &Option<String>) -> bool {
    value
        .as_ref()
        .is_some_and(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub struct Worker {
    handle: Handle,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start(database: PathBuf, socket: PathBuf, notify: Option<Notify>) -> Self {
        let (queue, rx) = mpsc::channel(8);
        let snapshot = Arc::new(Mutex::new(Snapshot {
            completed_runs: 0,
            ready: false,
            settings: Settings::default(),
            revision: None,
            warning: None,
            busy: false,
            active_group: None,
            reports: Vec::new(),
        }));
        let active = Arc::new(Mutex::new(None));
        let stop = CancellationToken::new();
        let handle = Handle {
            queue,
            snapshot: snapshot.clone(),
            active: active.clone(),
            stop: stop.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("raw-downloads".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .max_blocking_threads(2)
                    .enable_all()
                    .build();
                if let Ok(runtime) = runtime {
                    runtime.block_on(
                        Engine {
                            completed: 0,
                            path: path_for(&database),
                            guard: store::Guard::new(&database, &socket),
                            snapshot,
                            active,
                            stop,
                            rx,
                            notify,
                            due: Vec::new(),
                            client: None,
                        }
                        .run(),
                    );
                    runtime.shutdown_timeout(Duration::from_millis(250));
                } else {
                    let mut snapshot = snapshot.lock().unwrap();
                    snapshot.ready = true;
                    snapshot.warning = Some("Raw downloads are unavailable".into());
                }
            })
            .ok();
        if thread.is_none() {
            let mut snapshot = handle.snapshot.lock().unwrap();
            snapshot.ready = true;
            snapshot.warning = Some("Raw downloads are unavailable".into());
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
struct Engine {
    completed: u64,
    path: PathBuf,
    guard: store::Guard,
    snapshot: Arc<Mutex<Snapshot>>,
    active: Arc<Mutex<Option<CancellationToken>>>,
    stop: CancellationToken,
    rx: mpsc::Receiver<Request>,
    notify: Option<Notify>,
    due: Vec<(String, tokio::time::Instant)>,
    client: Option<reqwest::Client>,
}
impl Engine {
    fn schedule(&mut self) {
        let snapshot = self.snapshot.lock().unwrap();
        self.due = if snapshot.settings.enabled && snapshot.warning.is_none() {
            snapshot
                .settings
                .groups
                .iter()
                .map(|group| {
                    (
                        group.name.clone(),
                        tokio::time::Instant::now() + group.duration().unwrap(),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
    }
    fn load(&mut self) {
        let mut snapshot = store::load(&self.path);
        snapshot.completed_runs = self.completed;
        if self.guard.recover().is_err() {
            snapshot.warning = Some(
                "An owned raw-download stage could not be recovered; existing files preserved"
                    .into(),
            );
        } else if self.guard.settings(&snapshot.settings).is_err() {
            snapshot.warning = Some(
                "A raw-download destination is protected or unsupported; review the configuration"
                    .into(),
            );
        }
        *self.snapshot.lock().unwrap() = snapshot;
        self.schedule();
    }
    fn command(&mut self, request: Request, running: bool) -> Option<String> {
        if running
            && matches!(
                request.command,
                Command::Reload | Command::Save { .. } | Command::Import { .. } | Command::Cancel
            )
            && let Some(active) = self.active.lock().unwrap().as_ref()
        {
            active.cancel();
        }
        let result: Result<Reply> = match request.command {
            Command::Reload => {
                self.load();
                Ok(Reply::State(self.handle_snapshot(running)))
            }
            Command::Save {
                expected_revision,
                settings,
            } => self.save(expected_revision, settings, running),
            Command::Import {
                expected_revision,
                json,
            } => import_source(&json)
                .and_then(|settings| self.save(expected_revision, settings, running)),
            Command::Export => {
                export_source(&self.snapshot.lock().unwrap().settings).map(Reply::Export)
            }
            Command::ReadProfile { path } => store::profile(&path).map(Reply::Profile),
            Command::WriteProfile {
                path,
                expected_revision,
            } => {
                let snapshot = self.snapshot.lock().unwrap().clone();
                if snapshot.revision == expected_revision
                    && store::read(&self.path)
                        .is_ok_and(|(_, revision)| revision == expected_revision)
                {
                    store::write_profile(&self.guard, &path, &snapshot.settings).map(Reply::Written)
                } else {
                    Err(anyhow::anyhow!(
                        "Configuration changed; reload before exporting"
                    ))
                }
            }
            Command::Cancel => Ok(Reply::State(self.handle_snapshot(running))),
            Command::RunNow {
                group,
                expected_revision,
            } => {
                let snapshot = self.snapshot.lock().unwrap();
                let exists = snapshot.revision == expected_revision
                    && snapshot.settings.groups.iter().any(|g| g.name == group);
                drop(snapshot);
                if running
                    || !exists
                    || !store::read(&self.path)
                        .is_ok_and(|(_, revision)| revision == expected_revision)
                {
                    Err(anyhow::anyhow!("Choose an existing idle group"))
                } else {
                    let _ = request
                        .reply
                        .send(Ok(Reply::State(self.handle_snapshot(false))));
                    return Some(group);
                }
            }
        };
        let _ = request.reply.send(result.map_err(|_| {
            "Raw download action refused; reload and review the configuration/destination".into()
        }));
        None
    }
    fn handle_snapshot(&self, running: bool) -> Snapshot {
        let mut snapshot = self.snapshot.lock().unwrap().clone();
        snapshot.busy = running;
        snapshot
    }
    fn save(
        &mut self,
        revision: Option<String>,
        settings: Settings,
        running: bool,
    ) -> Result<Reply> {
        self.guard.settings(&settings)?;
        let mut loaded = store::save(&self.path, &revision, &settings)?;
        loaded.completed_runs = self.completed;
        loaded.busy = running;
        *self.snapshot.lock().unwrap() = loaded;
        self.schedule();
        Ok(Reply::State(self.handle_snapshot(running)))
    }
    async fn run(mut self) {
        self.load();
        loop {
            let deadline = self.due.iter().min_by_key(|(_, due)| *due).cloned();
            let wake = deadline
                .as_ref()
                .map(|(_, due)| *due)
                .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(31_536_000));
            let group = tokio::select! {biased;_=self.stop.cancelled()=>break,request=self.rx.recv()=>match request{Some(request)=>self.command(request,false),None=>break},_=tokio::time::sleep_until(wake),if deadline.is_some()=>deadline.map(|(name,_)|name)};
            if let Some(name) = group {
                let group = self
                    .snapshot
                    .lock()
                    .unwrap()
                    .settings
                    .groups
                    .iter()
                    .find(|g| g.name == name)
                    .cloned();
                if let Some(group) = group {
                    self.batch(group).await;
                }
            }
        }
        let mut snapshot = self.snapshot.lock().unwrap();
        snapshot.busy = false;
        snapshot.active_group = None;
    }
    async fn batch(&mut self, group: Group) {
        let cancel = CancellationToken::new();
        *self.active.lock().unwrap() = Some(cancel.clone());
        {
            let mut snapshot = self.snapshot.lock().unwrap();
            snapshot.busy = true;
            snapshot.active_group = Some(group.name.clone());
        }
        let mut reports = Vec::new();
        let mut spent = 0;
        if self.client.is_none() {
            self.client = reqwest::Client::builder()
                .user_agent(concat!("Rustleblower/", env!("CARGO_PKG_VERSION")))
                .pool_max_idle_per_host(1)
                .connect_timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    let prior = attempt.previous().last();
                    if attempt.previous().len() < 3
                        && matches!(attempt.url().scheme(), "http" | "https")
                        && attempt.url().username().is_empty()
                        && attempt.url().password().is_none()
                        && !(prior.is_some_and(|url| url.scheme() == "https")
                            && attempt.url().scheme() == "http")
                    {
                        attempt.follow()
                    } else {
                        attempt.stop()
                    }
                }))
                .build()
                .ok();
        }
        for resource in group.urls.iter().filter(|resource| resource.enabled) {
            if cancel.is_cancelled() || self.stop.is_cancelled() {
                break;
            }
            let remaining = MAX_BATCH - spent;
            let report = if remaining == 0 {
                ResourceReport {
                    name: resource.name.clone(),
                    localfile: resource.localfile.clone(),
                    status: Status::TooLarge,
                    bytes: 0,
                }
            } else {
                let client = self.client.clone();
                let guard = self.guard.clone();
                let outcome = {
                    let request = download(client.as_ref(), &guard, resource, remaining, &cancel);
                    tokio::pin!(request);
                    loop {
                        tokio::select! {biased;
                            _=self.stop.cancelled()=>break None,
                            _=cancel.cancelled()=>break None,
                            request=self.rx.recv()=>{match request{Some(request)=>{self.command(request,true);if cancel.is_cancelled(){break None;}},None=>break None}},
                            report=&mut request=>break Some(report),
                        }
                    }
                };
                outcome.unwrap_or_else(|| ResourceReport {
                    name: resource.name.clone(),
                    localfile: resource.localfile.clone(),
                    status: Status::Cancelled,
                    bytes: 0,
                })
            };
            spent = spent.saturating_add(report.bytes).min(MAX_BATCH);
            reports.push(report);
            if cancel.is_cancelled() || self.stop.is_cancelled() {
                break;
            }
        }
        let cancelled = cancel.is_cancelled() || self.stop.is_cancelled();
        let success = !reports.is_empty() && reports.iter().all(|r| r.status == Status::Published);
        let status = if reports
            .iter()
            .any(|report| report.status == Status::RecoveryNeeded)
        {
            Status::RecoveryNeeded
        } else if cancelled {
            Status::Cancelled
        } else if reports.is_empty() {
            Status::NoEnabledResources
        } else if success {
            Status::Published
        } else {
            Status::Failed
        };
        if !cancelled
            && !reports.is_empty()
            && let (Some(callback), Some(message)) = (&self.notify, group.message(success))
        {
            let failed_count = reports
                .iter()
                .filter(|r| r.status != Status::Published)
                .count();
            let notice = Notice {
                group: group.name.clone(),
                success,
                message,
                failed_names: reports
                    .iter()
                    .filter(|r| r.status != Status::Published)
                    .take(8)
                    .map(|r| r.name.clone())
                    .collect(),
                failed_count,
            };
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(notice)));
        }
        {
            let mut snapshot = self.snapshot.lock().unwrap();
            self.completed = self.completed.saturating_add(1);
            snapshot.completed_runs = self.completed;
            snapshot.busy = false;
            snapshot.active_group = None;
            snapshot.reports.retain(|report| report.name != group.name);
            snapshot.reports.push(GroupReport {
                name: group.name.clone(),
                status,
                resources: reports,
            });
            if snapshot.reports.len() > 16 {
                snapshot.reports.remove(0);
            }
        }
        *self.active.lock().unwrap() = None;
        let duration = self
            .snapshot
            .lock()
            .unwrap()
            .settings
            .groups
            .iter()
            .find(|current| current.name == group.name)
            .map(Group::duration);
        if let (Some((_, due)), Some(Ok(duration))) = (
            self.due.iter_mut().find(|(name, _)| *name == group.name),
            duration,
        ) {
            *due = tokio::time::Instant::now() + duration;
        }
    }
}
async fn download(
    client: Option<&reqwest::Client>,
    guard: &store::Guard,
    resource: &Resource,
    budget: u64,
    cancel: &CancellationToken,
) -> ResourceReport {
    let mut written: u64 = 0;
    let status = match (client, guard.target(&resource.localfile, true)) {
        (Some(client), Ok(target)) => {
            let expected = store::stamp(&target);
            match expected {
                Ok(expected) => {
                    let result = tokio::time::timeout(Duration::from_secs(30), async {
                        guard.recover().map_err(|_| Status::RecoveryNeeded)?;
                        let mut stage = guard.stage(&target).map_err(|_| Status::RecoveryNeeded)?;
                        let response = client
                            .get(url(&resource.remote).map_err(|_| Status::Failed)?)
                            .send()
                            .await
                            .map_err(|_| Status::Failed)?;
                        if response.status() != reqwest::StatusCode::OK {
                            return Err(Status::Failed);
                        }
                        let limit = MAX_FILE.min(budget);
                        if response.content_length().is_some_and(|len| len > limit) {
                            return Err(Status::TooLarge);
                        }
                        let mut stream = response.bytes_stream();
                        while let Some(chunk) = stream.next().await {
                            let chunk = chunk.map_err(|_| Status::Failed)?;
                            if written.saturating_add(chunk.len() as u64) > limit {
                                return Err(Status::TooLarge);
                            }
                            for bytes in chunk.chunks(64 * 1024) {
                                if cancel.is_cancelled() {
                                    return Err(Status::Cancelled);
                                }
                                stage.write(bytes).map_err(|_| Status::Failed)?;
                                written += bytes.len() as u64;
                            }
                        }
                        if cancel.is_cancelled() {
                            return Err(Status::Cancelled);
                        }
                        if store::stamp(&target).map_err(|_| Status::Changed)? != expected {
                            return Err(Status::Changed);
                        }
                        let status = stage
                            .publish(&target, &expected)
                            .map_err(|_| Status::Failed)?;
                        Ok(status)
                    })
                    .await;
                    match result {
                        Ok(Ok(status)) => status,
                        Ok(Err(status)) => status,
                        Err(_) => Status::Failed,
                    }
                }
                Err(_) => Status::Protected,
            }
        }
        _ => Status::Protected,
    };
    ResourceReport {
        name: resource.name.clone(),
        localfile: resource.localfile.clone(),
        status,
        bytes: written,
    }
}
