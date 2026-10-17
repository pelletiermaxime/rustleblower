use super::launcher::{Launch, Process};
use super::{Engine, MAX_FRAME, Query, Status, View};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, Permissions},
    io::Read,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

enum Mode {
    Managed,
    #[cfg(test)]
    Fixture(Option<Launch>),
}

pub struct Client {
    updates: watch::Receiver<Arc<View>>,
    queries: watch::Sender<Query>,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}
impl Client {
    pub fn start(directory: PathBuf, uid: u32) -> Self {
        Self::start_mode(directory, uid, Mode::Managed)
    }
    #[cfg(test)]
    pub(crate) fn start_fixture(directory: PathBuf, uid: u32, launch: Option<Launch>) -> Self {
        Self::start_mode(directory, uid, Mode::Fixture(launch))
    }
    fn start_mode(directory: PathBuf, uid: u32, mode: Mode) -> Self {
        let (sender, updates) = watch::channel(Arc::new(View {
            status: Status::Starting,
            ..Default::default()
        }));
        let (queries, receiver) = watch::channel(Query::default());
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_done = done.clone();
        let errors = sender.clone();
        let result = std::thread::Builder::new()
            .name("usage-collector".into())
            .spawn(move || {
                if let Err(error) = run(&directory, uid, &sender, receiver, worker_stop, mode) {
                    let old = sender.borrow().clone();
                    let mut view = (*old).clone();
                    view.revision += 1;
                    view.status =
                        Status::Unavailable(format!("Usage collector unavailable: {error:#}"));
                    for row in &mut view.rows {
                        row.download = None;
                        row.upload = None;
                    }
                    sender.send_replace(Arc::new(view));
                }
                worker_done.store(true, Ordering::Release);
            });
        if let Err(error) = result {
            errors.send_replace(Arc::new(View {
                status: Status::Unavailable(error.to_string()),
                ..Default::default()
            }));
            done.store(true, Ordering::Release);
        }
        Self {
            updates,
            queries,
            stop,
            done,
        }
    }
    pub fn snapshot(&mut self) -> Option<Arc<View>> {
        self.updates
            .has_changed()
            .unwrap_or(true)
            .then(|| self.updates.borrow_and_update().clone())
    }
    pub fn query(&self, query: Query) {
        if query.search.len() <= 256 && query.offset <= super::MAX_RECORDS {
            self.queries.send_replace(query);
        }
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub fn finished(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop();
    }
}

struct OwnedSocket {
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl Drop for OwnedSocket {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.dev() == self.device && m.ino() == self.inode)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn listen(directory: &Path, uid: u32) -> Result<(UnixListener, OwnedSocket)> {
    let metadata =
        fs::symlink_metadata(directory).context("Cannot inspect usage socket directory")?;
    ensure!(
        metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o022 == 0,
        "Usage socket directory must belong to this user and disallow group/other writes"
    );
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce)?;
    let path = directory.join(format!(
        "usage-{}-{}.sock",
        std::process::id(),
        hex::encode(nonce)
    ));
    ensure!(
        path.is_absolute() && path.as_os_str().as_bytes().len() < 108,
        "Usage socket path is too long or not absolute"
    );
    // bind() refuses existing paths. Never unlink a socket to make room.
    let listener = UnixListener::bind(&path)?;
    let metadata = fs::symlink_metadata(&path)?;
    let owned = OwnedSocket {
        path: path.clone(),
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    fs::set_permissions(&path, Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok((listener, owned))
}
fn peer_uid(stream: &UnixStream) -> Result<u32> {
    use std::os::fd::AsRawFd;
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SO_PEERCRED supplies kernel credentials, not a value from the data stream.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    ensure!(
        result == 0 && length as usize == std::mem::size_of::<libc::ucred>(),
        "Cannot authenticate collector peer"
    );
    Ok(credentials.uid)
}
fn run(
    directory: &Path,
    uid: u32,
    sender: &watch::Sender<Arc<View>>,
    queries: watch::Receiver<Query>,
    stop: Arc<AtomicBool>,
    mode: Mode,
) -> Result<()> {
    let (listener, owned) = listen(directory, uid)?;
    let launch = match mode {
        Mode::Managed => Some(Launch::resolve(&owned.path, uid)?),
        #[cfg(test)]
        Mode::Fixture(mut launch) => {
            if let Some(launch) = launch.as_mut() {
                launch
                    .arguments
                    .extend([owned.path.as_os_str().to_owned(), uid.to_string().into()]);
            }
            launch
        }
    };
    let mut process = launch.map(|launch| launch.spawn()).transpose()?;
    let result = receive(
        &listener,
        &owned.path,
        uid,
        sender,
        queries,
        stop,
        &mut process,
    );
    // Remove the receiver before waiting for a privileged child, including one
    // racing authorization with Stop. It can no longer connect after cancellation.
    drop(listener);
    drop(owned);
    drop(process);
    result
}
fn receive(
    listener: &UnixListener,
    endpoint: &Path,
    uid: u32,
    sender: &watch::Sender<Arc<View>>,
    mut queries: watch::Receiver<Query>,
    stop: Arc<AtomicBool>,
    process: &mut Option<Process>,
) -> Result<()> {
    let mut engine = Engine::new(uid);
    let mut status = if process.is_some() {
        Status::Authorizing
    } else {
        Status::Waiting
    };
    let mut stream: Option<UnixStream> = None;
    let mut fresh = true;
    let mut frame = Vec::new();
    let mut frame_started: Option<Instant> = None;
    let mut last_good = Instant::now();
    let mut revision = 0;
    let mut query = Query::default();
    let mut dirty = true;
    let mut buffer = [0u8; 16384];
    let started = Instant::now();
    let mut exit = None;
    loop {
        if stop.load(Ordering::Acquire) {
            // Drop the collector stream before removing only our own endpoint.
            drop(stream.take());
            revision += 1;
            sender.send_replace(engine.view(
                &query,
                Status::Stopped,
                Some(endpoint.to_owned()),
                revision,
            ));
            break;
        }
        if let Some(process) = process.as_mut() {
            if exit.is_none() {
                exit = process.poll()?;
            }
            if fresh && started.elapsed() > Duration::from_secs(120) {
                revision += 1;
                sender.send_replace(engine.view(
                    &query,
                    Status::Unavailable(
                        "Timed out waiting for collector authorization or startup".into(),
                    ),
                    Some(endpoint.to_owned()),
                    revision,
                ));
                break;
            }
        }
        if queries.has_changed().unwrap_or(false) {
            query = queries.borrow_and_update().clone();
            dirty = true;
        }
        if dirty {
            revision += 1;
            sender.send_replace(engine.view(
                &query,
                status.clone(),
                Some(endpoint.to_owned()),
                revision,
            ));
            dirty = false;
        }
        if stream.is_none() {
            match listener.accept() {
                Ok((candidate, _)) => {
                    let peer = peer_uid(&candidate)?;
                    if peer != 0 && peer != uid {
                        continue;
                    }
                    candidate.set_read_timeout(Some(Duration::from_millis(100)))?;
                    stream = Some(candidate);
                    frame.clear();
                    frame_started = None;
                    fresh = true;
                    last_good = Instant::now();
                    status = Status::Waiting;
                    dirty = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if let Some(status) = exit.take() {
                        revision += 1;
                        sender.send_replace(engine.view(
                            &query,
                            status,
                            Some(endpoint.to_owned()),
                            revision,
                        ));
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(error) => return Err(error.into()),
            }
            continue;
        }
        let read = stream.as_mut().unwrap().read(&mut buffer);
        let mut failure = None;
        match read {
            Ok(0) => {
                status = if frame.is_empty() {
                    Status::Disconnected
                } else {
                    Status::Unavailable("Collector disconnected during a snapshot".into())
                };
                stream = None;
                frame.clear();
                frame_started = None;
                dirty = true;
            }
            Ok(count) => {
                for part in buffer[..count].split_inclusive(|byte| *byte == b'\n') {
                    frame_started.get_or_insert_with(Instant::now);
                    if frame.len() + part.len() > MAX_FRAME {
                        failure = Some("Usage snapshot exceeds 2 MiB".to_owned());
                        break;
                    }
                    frame.extend_from_slice(part);
                    if part.last() == Some(&b'\n') {
                        let parsed = if fresh {
                            let mut candidate = Engine::new(uid);
                            candidate.ingest(&frame).map(|_| {
                                engine = candidate;
                                fresh = false;
                            })
                        } else {
                            engine.ingest(&frame)
                        };
                        if let Err(error) = parsed {
                            failure = Some(format!("Usage snapshot rejected: {error}"));
                            break;
                        }
                        frame.clear();
                        frame_started = None;
                        last_good = Instant::now();
                        status = Status::Live;
                        dirty = true;
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => failure = Some(format!("Collector read failed: {error}")),
        }
        if frame_started.is_some_and(|start| start.elapsed() > Duration::from_secs(2)) {
            failure = Some("Collector did not finish its snapshot within two seconds".into());
        }
        if let Some(error) = failure {
            stream = None;
            frame.clear();
            frame_started = None;
            status = Status::Unavailable(error);
            dirty = true;
        } else if status == Status::Live && last_good.elapsed() > Duration::from_secs(3) {
            status = Status::Stale;
            dirty = true;
        }
    }
    Ok(())
}
