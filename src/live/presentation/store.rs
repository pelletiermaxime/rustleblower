use super::*;
use anyhow::{Result, ensure};
const MAX_BYTES: usize = 8192;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use tokio::sync::oneshot;
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub ready: bool,
    pub settings: Settings,
    pub revision: Option<String>,
    pub warning: Option<String>,
}
pub enum Command {
    Reload,
    Save {
        expected_revision: Option<String>,
        settings: Settings,
    },
}
struct Request {
    command: Command,
    reply: oneshot::Sender<std::result::Result<Snapshot, String>>,
}
#[derive(Clone)]
pub struct Handle {
    queue: mpsc::SyncSender<Request>,
    snapshot: Arc<Mutex<Arc<Snapshot>>>,
    stop: Arc<AtomicBool>,
}
impl Handle {
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.lock().unwrap().clone()
    }
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Release);
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
            && (settings.validate().is_err()
                || expected_revision
                    .as_ref()
                    .is_some_and(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit())))
        {
            let _ = tx.send(Err("Invalid live view preferences or revision".into()));
            return rx;
        }
        if let Err(error) = self.queue.try_send(Request { command, reply: tx }) {
            let request = match error {
                mpsc::TrySendError::Full(request) | mpsc::TrySendError::Disconnected(request) => {
                    request
                }
            };
            let _ = request
                .reply
                .send(Err("Live view preferences are busy or unavailable".into()));
        }
        rx
    }
}
pub struct Worker {
    handle: Handle,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start(path: PathBuf) -> Self {
        let (queue, rx) = mpsc::sync_channel::<Request>(4);
        let snapshot = Arc::new(Mutex::new(Arc::new(Snapshot {
            ready: false,
            settings: Settings::default(),
            revision: None,
            warning: None,
        })));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = Handle {
            queue,
            snapshot: snapshot.clone(),
            stop: stop.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("live-view".into())
            .spawn(move || {
                reload(&path, &snapshot);
                while !stop.load(Ordering::Acquire) {
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(request) => {
                            let result = match request.command {
                                Command::Reload => {
                                    reload(&path, &snapshot);
                                    Ok(snapshot.lock().unwrap().as_ref().clone())
                                }
                                Command::Save {
                                    expected_revision,
                                    settings,
                                } => save(&path, &expected_revision, &settings).inspect(|state| {
                                    *snapshot.lock().unwrap() = Arc::new(state.clone());
                                }),
                            };
                            let _ = request.reply.send(result.map_err(|_| {
                                "Live view preferences save refused; reload and review the file"
                                    .into()
                            }));
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .ok();
        if thread.is_none() {
            let mut state = handle.snapshot.lock().unwrap();
            let state = Arc::make_mut(&mut state);
            state.ready = true;
            state.warning = Some("Live view preferences are unavailable".into());
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
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    settings: Settings,
}
fn read(path: &Path) -> Result<Snapshot> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Snapshot {
                ready: true,
                settings: Settings::default(),
                revision: None,
                warning: None,
            });
        }
        Err(e) => return Err(e.into()),
    };
    ensure!(
        file.metadata()?.is_file(),
        "Expected regular live-view file"
    );
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "Live view preferences file exceeds limit"
    );
    let document: Document = serde_json::from_slice(&bytes)?;
    document.settings.validate()?;
    ensure!(document.version == 1, "Unknown live-view version");
    Ok(Snapshot {
        ready: true,
        settings: document.settings,
        revision: Some(hex::encode(Sha256::digest(bytes))),
        warning: None,
    })
}
fn reload(path: &Path, state: &Mutex<Arc<Snapshot>>) {
    match read(path) {
        Ok(next) => *state.lock().unwrap() = Arc::new(next),
        Err(_) => {
            let mut current = state.lock().unwrap();
            let current = Arc::make_mut(&mut current);
            current.ready = true;
            current.warning = Some(
                "Live view preferences file is invalid or inaccessible; keeping the previous preferences".into(),
            );
        }
    }
}
fn save(path: &Path, expected: &Option<String>, settings: &Settings) -> Result<Snapshot> {
    settings.validate()?;
    ensure!(
        &read(path)?.revision == expected,
        "Live view preferences file changed"
    );
    let bytes = serde_json::to_vec(&Document {
        version: 1,
        settings: settings.clone(),
    })?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "Live view preferences file exceeds limit"
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    ensure!(
        &read(path)?.revision == expected,
        "Live view preferences file changed"
    );
    if expected.is_none() {
        temp.persist_noclobber(path).map_err(|e| e.error)?;
    } else {
        temp.persist(path).map_err(|e| e.error)?;
    }
    File::open(parent)?.sync_all()?;
    read(path)
}
