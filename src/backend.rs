use crate::{
    config::{Config, Decision},
    history::{self, Batch, History, LossReason, Worker},
    prompts::{self, PendingGuard, Prompts},
    protocol::{
        self,
        ui_server::{Ui, UiServer},
    },
    rules::Control,
};
use anyhow::{Context as _, Result};
use futures_util::{Stream, StreamExt};
use std::{
    collections::BTreeMap,
    future::Future,
    io,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        net::UnixListener as StdListener,
    },
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::{
    Request, Response, Status as RpcStatus,
    transport::{Server, server::Connected},
};

pub const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct Session {
    pub id: u64,
    pub name: String,
    pub reported_name: String,
    pub version: String,
    pub rules: usize,
    pub last_seen: Instant,
    pub notifications: bool,
    pub local: bool,
}

#[derive(Clone, Default)]
pub struct Status {
    pub session: Option<Session>,
    pub sessions: BTreeMap<u64, Session>,
    selected: u64,
    pub error: Option<String>,
    pub last_alert: Option<String>,
    pub timed_out: u64,
    pub overloaded: u64,
    pub answered: u64,
}

#[derive(Clone)]
pub struct Handle {
    pub listener: String,
    connections: Arc<Semaphore>,
    initial_connections: Arc<Semaphore>,
    pub prompts: Arc<Prompts>,
    pub control: Arc<Control>,
    pub history: History,
    pub status: Arc<Mutex<Status>>,
    pub cancel: CancellationToken,
    pub timeout_action: Decision,
    pub prompt_timeout: u64,
    pub preferences: Arc<crate::preferences::Store>,
    pub desktop_notices: Arc<crate::desktop_notifications::Mailbox>,
    pub live: Arc<crate::live::Tasks>,
    pub subscriptions: crate::subscriptions::Handle,
    pub updates: crate::updates::Handle,
    pub reputation: crate::reputation::Handle,
    pub raw_downloads: crate::raw_downloads::Handle,
    pub highlighting: crate::highlighting::Handle,
    pub catalogue_policy: crate::rules::catalogue_policy::Handle,
    pub catalogue_run: Option<crate::rules::catalogue_policy::RunId>,
    pub live_presentation: crate::live::presentation::Handle,
    pub view_preferences: crate::view_preferences::Handle,
}

impl Handle {
    pub fn connection_count(&self) -> usize {
        crate::transport::MAX_CONNECTIONS - self.connections.available_permits()
    }
    pub fn initial_connection_count(&self) -> usize {
        crate::transport::MAX_INITIAL_CONNECTIONS - self.initial_connections.available_permits()
    }
    pub fn popup_policy(&self) -> (crate::preferences::Popup, u64, Decision) {
        let preferences = self.preferences.snapshot();
        let timeout = preferences.timeout.unwrap_or(self.prompt_timeout);
        let fallback = preferences.fallback.unwrap_or(self.timeout_action);
        (preferences, timeout, fallback)
    }
    pub fn refresh_rules(&self, session: u64) -> Result<(), String> {
        if !self
            .session(session)
            .is_some_and(|current| current.notifications)
        {
            return Err("The daemon connection changed; refresh its current connection".into());
        }
        if self
            .prompts
            .snapshot()
            .iter()
            .any(|prompt| prompt.session == session)
        {
            return Err(
                "Answer pending permission requests before refreshing the connection".into(),
            );
        }
        self.control.refresh(session)
    }
    pub fn status(&self) -> Status {
        let mut snapshot = self.status.lock().unwrap().clone();
        snapshot.session = snapshot.sessions.get(&snapshot.selected).cloned();
        snapshot
    }
    pub fn decide(&self, id: u64, decision: Decision) -> bool {
        self.prompts.decide(id, decision)
    }
    pub fn shutdown(&self) {
        self.subscriptions.shutdown();
        self.updates.shutdown();
        self.reputation.shutdown();
        self.raw_downloads.shutdown();
        self.highlighting.shutdown();
        self.catalogue_policy.shutdown();
        self.live_presentation.shutdown();
        self.view_preferences.shutdown();
        self.cancel.cancel();
    }
    pub fn session(&self, id: u64) -> Option<Session> {
        self.status.lock().unwrap().sessions.get(&id).cloned()
    }
    pub fn node(&self, name: &str) -> Option<Session> {
        self.status
            .lock()
            .unwrap()
            .sessions
            .values()
            .find(|session| session.name == name)
            .cloned()
    }
    pub fn select_node(&self, id: u64) -> Result<(), String> {
        let mut state = self.status.lock().unwrap();
        if !state.sessions.contains_key(&id) {
            return Err("This node is disconnected".into());
        }
        state.selected = id;
        Ok(())
    }
    fn disconnect(&self, id: u64) {
        let mut state = self.status.lock().unwrap();
        state.sessions.remove(&id);
        if state.selected == id {
            state.selected = state.sessions.keys().next().copied().unwrap_or(0);
        }
        // Maintain the same status -> prompts lock order as Subscribe/AskRule.
        self.prompts
            .finish_session(id, self.timeout_action, "disconnected");
        self.control.disconnect(id);
        self.live.disconnect(id);
    }
}

pub struct Backend {
    pub handle: Handle,
    server: thread::JoinHandle<()>,
    history: Worker,
    subscriptions: crate::subscriptions::Worker,
    updates: crate::updates::Worker,
    reputation: crate::reputation::Worker,
    raw_downloads: crate::raw_downloads::Worker,
    highlighting: crate::highlighting::Worker,
    catalogue_policy: crate::rules::catalogue_policy::Worker,
    live_presentation: crate::live::presentation::Worker,
    view_preferences: crate::view_preferences::Worker,
}

impl Backend {
    pub fn start(config: Config) -> Result<Self> {
        Self::start_with_storage(config, Default::default())
    }
    pub fn start_with_storage(
        config: Config,
        overrides: crate::history::storage::Overrides,
    ) -> Result<Self> {
        Self::start_with_subscription_worker(
            config,
            overrides,
            crate::subscriptions::Worker::start_with_notices,
        )
    }
    fn start_with_subscription_worker(
        config: Config,
        overrides: crate::history::storage::Overrides,
        subscriptions_worker: impl FnOnce(
            &std::path::Path,
            Arc<crate::desktop_notifications::Mailbox>,
        ) -> crate::subscriptions::Worker,
    ) -> Result<Self> {
        // Desktop dependencies can also enable Ring. Select a default explicitly
        // before Tonic constructs TLS configs; preserve a caller's installed provider.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let endpoint = crate::transport::Endpoint::parse(&config.socket)?;
        let tls = config.transport.tls(&endpoint)?;
        let mut rpc_server = Server::builder();
        if let Some(tls) = tls {
            rpc_server = rpc_server
                .tls_config(tls)
                .context("Invalid TLS credentials")?;
        }
        let local_addresses = Arc::new(if matches!(endpoint, crate::transport::Endpoint::Tcp(_)) {
            crate::transport::local_addresses()?
        } else {
            Default::default()
        });
        let preferences = Arc::new(crate::preferences::Store::open(&config.database)?);
        // Bind without unlinking: never steal opensnitch-ui's endpoint.
        let (listener, guard, address) = match endpoint {
            crate::transport::Endpoint::Unix(path) => {
                let listener=StdListener::bind(&path).with_context(||format!("Cannot bind {}. Existing sockets are never replaced; choose --socket for testing.",path.display()))?;
                let guard = SocketGuard::new(path.clone())?;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
                listener.set_nonblocking(true)?;
                (
                    crate::transport::Listener::Unix(listener),
                    Some(guard),
                    path.display().to_string(),
                )
            }
            crate::transport::Endpoint::Tcp(address) => {
                let listener = std::net::TcpListener::bind(address)
                    .with_context(|| format!("Cannot bind TCP listener {address}"))?;
                let address = listener.local_addr()?.to_string();
                listener.set_nonblocking(true)?;
                (crate::transport::Listener::Tcp(listener), None, address)
            }
        };
        let history = Worker::start_with_overrides(
            &config.database,
            config.history_days,
            config.history_mib,
            overrides,
        )?;
        let desktop_notices = Arc::new(crate::desktop_notifications::Mailbox::default());
        let subscriptions = subscriptions_worker(
            &config
                .database
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("subscriptions"),
            desktop_notices.clone(),
        );
        let update_notices = desktop_notices.clone();
        let updates = crate::updates::Worker::start(
            crate::updates::path_for(&config.database),
            Some(Arc::new(move |notice| update_notices.post_update(&notice))),
        );
        let reputation =
            crate::reputation::Worker::start(crate::reputation::path_for(&config.database));
        let raw_notices = desktop_notices.clone();
        let raw_downloads = crate::raw_downloads::Worker::start(
            config.database.clone(),
            config.socket.clone(),
            Some(Arc::new(move |notice| {
                raw_notices.post_raw_download(&notice)
            })),
        );
        let highlighting =
            crate::highlighting::Worker::start(crate::highlighting::path_for(&config.database));
        // Startup runs before the UI; entropy failure leaves origin unknown.
        let catalogue_run = crate::rules::catalogue_policy::process_run_id();
        let catalogue_policy = crate::rules::catalogue_policy::Worker::start(
            crate::rules::catalogue_policy::path_for(&config.database),
        );
        let live_presentation = crate::live::presentation::Worker::start(
            crate::live::presentation::path_for(&config.database),
        );
        let view_preferences = crate::view_preferences::Worker::start(
            crate::view_preferences::path_for(&config.database),
        );
        let control = Arc::new(Control::new(history.history.health.catalogues.clone()));
        let live = Arc::new(crate::live::Tasks::new(control.clone()));
        let handle = Handle {
            listener: address,
            connections: Arc::new(Semaphore::new(crate::transport::MAX_CONNECTIONS)),
            initial_connections: Arc::new(Semaphore::new(
                crate::transport::MAX_INITIAL_CONNECTIONS,
            )),
            prompts: Arc::new(Prompts::default()),
            control,
            live,
            history: history.history.clone(),
            status: Arc::new(Mutex::new(Status::default())),
            cancel: CancellationToken::new(),
            timeout_action: config.timeout_action,
            prompt_timeout: config.prompt_timeout,
            preferences,
            desktop_notices,
            subscriptions: subscriptions.handle(),
            updates: updates.handle(),
            reputation: reputation.handle(),
            raw_downloads: raw_downloads.handle(),
            highlighting: highlighting.handle(),
            catalogue_policy: catalogue_policy.handle(),
            catalogue_run,
            live_presentation: live_presentation.handle(),
            view_preferences: view_preferences.handle(),
        };
        let service = Service {
            handle: handle.clone(),
        };
        let server_handle = handle.clone();
        let server = thread::Builder::new()
            .name("rustleblower-rpc".into())
            .spawn(move || {
                let _guard = guard;
                let result = (|| -> Result<()> {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .enable_all()
                        .build()?;
                    runtime.block_on(async {
                        let control = server_handle.control.clone();
                        let live = server_handle.live.clone();
                        let command_cancel = server_handle.cancel.clone();
                        let subscription_handle=server_handle.subscriptions.clone();
                        let subscription_status=server_handle.status.clone();
                        let update_handle=server_handle.updates.clone();
                        let pause_history=server_handle.history.clone();
                        tokio::spawn(async move {
                            let mut restoring: Vec<crate::rules::PauseRestore> = Vec::new();
                            loop {
                                tokio::select! {
                                    _ = command_cancel.cancelled() => break,
                                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                                        control.expire(Instant::now()); live.maintain(Instant::now());
                                        restoring.retain_mut(|(session,node,reply)|match reply.try_recv(){
                                            Ok(Ok(()))=>{control.finish_pause_restore(*session);false},
                                            Ok(Err(error))=>{control.finish_pause_restore(*session);subscription_status.lock().unwrap().last_alert=Some(format!("Could not restore saved pause for {node}: {error}"));false},
                                            Err(tokio::sync::oneshot::error::TryRecvError::Closed)=>{control.finish_pause_restore(*session);subscription_status.lock().unwrap().last_alert=Some(format!("Saved pause for {node} was not confirmed; reconnect to verify its state"));false},
                                            Err(tokio::sync::oneshot::error::TryRecvError::Empty)=>true,
                                        });
                                        if restoring.len()<crate::nodes::MAX_NODES {restoring.extend(control.restore_pauses(&pause_history,crate::nodes::MAX_NODES-restoring.len()));}
                                        subscription_handle.local_ready(subscription_status.lock().unwrap().sessions.values().any(|s|s.notifications&&s.local));
                                        let selected = {
                                            let status = subscription_status.lock().unwrap();
                                            status.sessions.get(&status.selected).filter(|s| !s.version.is_empty())
                                                .map(|s| crate::updates::TargetVersion { node: s.name.clone(), version: s.version.clone() })
                                        };
                                        update_handle.set_daemon(selected);
                                    },
                                }
                            }
                        });
                        let signal_cancel = server_handle.cancel.clone();
                        tokio::spawn(async move {
                            let Ok(mut terminate) = tokio::signal::unix::signal(
                                tokio::signal::unix::SignalKind::terminate(),
                            ) else {
                                return;
                            };
                            let Ok(mut interrupt) = tokio::signal::unix::signal(
                                tokio::signal::unix::SignalKind::interrupt(),
                            ) else {
                                return;
                            };
                            tokio::select! {
                                _ = terminate.recv() => signal_cancel.cancel(),
                                _ = interrupt.recv() => signal_cancel.cancel(),
                                _ = signal_cancel.cancelled() => {},
                            }
                        });
                        let incoming=listener.incoming()?;
                        let ids = Arc::new(AtomicU64::new(history::now_nanos() as u64));
                        let connection_history = server_handle.history.clone();
                        let connection_handle = server_handle.clone();
                        let permits=server_handle.connections.clone();
                        let initial_permits=server_handle.initial_connections.clone();
                        let incoming = incoming.filter_map(move |s| {
                            futures_util::future::ready(Some(s.and_then(|stream| {
                                let peer=match &stream {
                                    crate::transport::Io::Unix(_)=>crate::transport::Peer::unix(),
                                    crate::transport::Io::Tcp(tcp)=>{tcp.set_nodelay(true)?;crate::transport::Peer::tcp_socket(tcp.peer_addr()?,&local_addresses)},
                                };
                                let deadline=peer.address.map(|_|Box::pin(tokio::time::sleep(crate::transport::INITIAL_DEADLINE)));
                                let initial=if peer.address.is_some() {
                                    match initial_permits.clone().try_acquire_owned() {
                                        Ok(permit)=>Some(permit),
                                        Err(_)=>return Err(io::Error::new(io::ErrorKind::ConnectionAborted,"Initial connection capacity reached")),
                                    }
                                } else {None};
                                // Reject excess initial peers before counting a live connection.
                                // Otherwise a rejected candidate transiently inflates that count.
                                let permit=permits.clone().try_acquire_owned()
                                    .map_err(|_|io::Error::new(io::ErrorKind::ConnectionAborted,"Connection capacity reached"))?;
                                Ok(SessionIo {
                                stream,
                                deadline,
                                _permit:permit,
                                handle: connection_handle.clone(),
                                info: ConnectionId {
                                    initial: Arc::new(Mutex::new(initial)),
                                    peer,
                                    session: Arc::new(AtomicU64::new(0)),
                                    ids: ids.clone(),
                                    history: Arc::new(Mutex::new(ConnectionHistory {
                                        history: connection_history.clone(),
                                        subscribed: false,
                                        version: String::new(),
                                        early: Vec::new(),
                                        statistics: None,
                                    })),
                                },
                            })})))
                        });
                        rpc_server
                            .concurrency_limit_per_connection(64)
                            .max_concurrent_streams(64)
                            .add_service(
                                crate::transport::Admission::new(UiServer::new(service)
                                    .max_decoding_message_size(MAX_MESSAGE_BYTES)
                                    .max_encoding_message_size(MAX_MESSAGE_BYTES)),
                            )
                            .serve_with_incoming_shutdown(
                                incoming,
                                server_handle.cancel.clone().cancelled_owned(),
                            )
                            .await?;
                        Ok(())
                    })
                })();
                if let Err(e) = result {
                    server_handle.status.lock().unwrap().error =
                        Some(format!("gRPC server failed: {e:#}"));
                }
            })?;
        Ok(Self {
            handle,
            server,
            history,
            subscriptions,
            updates,
            reputation,
            raw_downloads,
            highlighting,
            catalogue_policy,
            live_presentation,
            view_preferences,
        })
    }

    pub fn join(self) {
        self.handle.shutdown();
        let _ = self.server.join();
        self.subscriptions.join();
        self.updates.join();
        self.reputation.join();
        self.raw_downloads.join();
        self.highlighting.join();
        self.catalogue_policy.join();
        self.live_presentation.join();
        self.view_preferences.join();
        drop(self.handle);
        self.history.join();
    }
}

struct SocketGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl SocketGuard {
    fn new(path: PathBuf) -> Result<Self> {
        let m = std::fs::symlink_metadata(&path)?;
        Ok(Self {
            path,
            device: m.dev(),
            inode: m.ino(),
        })
    }
}
impl Drop for SocketGuard {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.dev() == self.device && m.ino() == self.inode)
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

const MAX_EARLY_BATCHES: usize = 32;

struct ConnectionHistory {
    history: History,
    subscribed: bool,
    version: String,
    early: Vec<Batch>,
    statistics: Option<crate::statistics::Prepared>,
}

impl Drop for ConnectionHistory {
    fn drop(&mut self) {
        if self.statistics.is_some() {
            self.history
                .health
                .statistics_skipped
                .fetch_add(1, Ordering::Relaxed);
        }
        self.history.health.record_loss(
            LossReason::Connection,
            self.early.iter().map(Batch::len).sum(),
        );
    }
}

#[derive(Clone)]
struct ConnectionId {
    initial: Arc<Mutex<Option<OwnedSemaphorePermit>>>,
    peer: crate::transport::Peer,
    session: Arc<AtomicU64>,
    ids: Arc<AtomicU64>,
    history: Arc<Mutex<ConnectionHistory>>,
}
struct SessionIo {
    stream: crate::transport::Io,
    deadline: Option<Pin<Box<tokio::time::Sleep>>>,
    _permit: OwnedSemaphorePermit,
    handle: Handle,
    info: ConnectionId,
}
impl SessionIo {
    fn expired(&mut self, cx: &mut Context<'_>) -> bool {
        if self.info.session.load(Ordering::Acquire) != 0 {
            self.deadline = None;
            return false;
        }
        self.deadline
            .as_mut()
            .is_some_and(|timer| timer.as_mut().poll(cx).is_ready())
    }
    fn deadline_error() -> io::Error {
        io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "Initial TLS/subscription deadline exceeded",
        )
    }
}

impl Drop for SessionIo {
    fn drop(&mut self) {
        self.handle
            .disconnect(self.info.session.load(Ordering::Acquire));
    }
}
impl Connected for SessionIo {
    type ConnectInfo = ConnectionId;
    fn connect_info(&self) -> Self::ConnectInfo {
        self.info.clone()
    }
}
impl AsyncRead for SessionIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.expired(cx) {
            return Poll::Ready(Err(Self::deadline_error()));
        }
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for SessionIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.expired(cx) {
            return Poll::Ready(Err(Self::deadline_error()));
        }
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.expired(cx) {
            return Poll::Ready(Err(Self::deadline_error()));
        }
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[derive(Clone)]
struct Service {
    handle: Handle,
}
fn connection_id<T>(request: &Request<T>) -> u64 {
    request
        .extensions()
        .get::<ConnectionId>()
        .map_or(0, |info| info.session.load(Ordering::Acquire))
}

#[tonic::async_trait]
impl Ui for Service {
    async fn ping(
        &self,
        request: Request<protocol::PingRequest>,
    ) -> Result<Response<protocol::PingReply>, RpcStatus> {
        let session_id = connection_id(&request);
        let info = request.extensions().get::<ConnectionId>().cloned();
        let request = request.into_inner();
        if let Some(mut stats) = request.stats {
            // Reduce the payload and reserve its shared memory budget before
            // taking the session lock; neither path waits for disk or Subscribe.
            let batch = self
                .handle
                .history
                .prepare(std::mem::take(&mut stats.events), "observed");
            let version = history::bounded(&stats.daemon_version, 64);
            let statistics = self.handle.history.prepare_statistics(stats);
            let mut state = self.handle.status.lock().unwrap();
            let session_id = info
                .as_ref()
                .map_or(session_id, |info| info.session.load(Ordering::Acquire));
            if let Some(session) = state.sessions.get_mut(&session_id) {
                session.last_seen = Instant::now();
                self.handle
                    .control
                    .report_range_version(session_id, &version);
                session.version = version;
                if let Some(sample) = statistics {
                    self.handle
                        .history
                        .enqueue_statistics(session.name.clone(), sample);
                }
                if let Some(batch) = batch {
                    self.handle.history.enqueue(session.name.clone(), batch);
                }
            } else if let Some(info) = info {
                let mut pending = info.history.lock().unwrap();
                if !pending.subscribed {
                    pending.version = version;
                    if let Some(sample) = statistics
                        && pending.statistics.replace(sample).is_some()
                    {
                        self.handle
                            .history
                            .health
                            .statistics_skipped
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    if let Some(batch) = batch {
                        if pending.early.len() < MAX_EARLY_BATCHES {
                            pending.early.push(batch);
                        } else {
                            self.handle
                                .history
                                .health
                                .record_loss(LossReason::QueueFull, batch.len());
                        }
                    }
                } else if let Some(batch) = batch {
                    // A replaced or disconnected session cannot become a new
                    // pre-subscription buffer by sending another Ping.
                    self.handle
                        .history
                        .health
                        .record_loss(LossReason::Connection, batch.len());
                }
            } else if let Some(batch) = batch {
                self.handle
                    .history
                    .health
                    .record_loss(LossReason::Connection, batch.len());
            }
        } else if let Some(session) = self
            .handle
            .status
            .lock()
            .unwrap()
            .sessions
            .get_mut(&session_id)
        {
            session.last_seen = Instant::now();
        }
        Ok(Response::new(protocol::PingReply { id: request.id }))
    }

    async fn subscribe(
        &self,
        request: Request<protocol::ClientConfig>,
    ) -> Result<Response<protocol::ClientConfig>, RpcStatus> {
        let info = request
            .extensions()
            .get::<ConnectionId>()
            .cloned()
            .ok_or_else(|| RpcStatus::failed_precondition("Missing transport identity"))?;
        let previous = info.session.load(Ordering::Acquire);
        let id = info.ids.fetch_add(1, Ordering::Relaxed) + 1;
        let config = request.into_inner();
        let saved = crate::nodes::Saved::prepare(&config).map_err(RpcStatus::resource_exhausted)?;
        let name = info
            .peer
            .node_key(&config.name)
            .map_err(|e| RpcStatus::invalid_argument(e.to_string()))?;
        let mut state = self.handle.status.lock().unwrap();
        let replaced = state
            .sessions
            .iter()
            .filter_map(|(key, session)| (session.name == name || *key == previous).then_some(*key))
            .collect::<Vec<_>>();
        if state.sessions.len().saturating_sub(replaced.len()) >= crate::nodes::MAX_NODES {
            return Err(RpcStatus::resource_exhausted(
                "At most 32 nodes can be connected",
            ));
        }
        for old in replaced {
            state.sessions.remove(&old);
            self.handle.live.disconnect(old);
            self.handle
                .prompts
                .finish_session(old, self.handle.timeout_action, "reconnected");
        }
        state.sessions.insert(
            id,
            Session {
                id,
                name: name.clone(),
                reported_name: config.name.clone(),
                version: String::new(),
                rules: config.rules.len(),
                last_seen: Instant::now(),
                notifications: false,
                local: info.peer.local,
            },
        );
        if !state.sessions.contains_key(&state.selected) {
            state.selected = id;
        }
        self.handle.control.start_session(id, name.clone());
        self.handle
            .control
            .report_interception(id, config.is_firewall_running);
        self.handle
            .control
            .report_firewall(id, config.system_firewall.as_ref(), &config.config);
        self.handle
            .history
            .health
            .catalogues
            .begin(id, &name)
            .map_err(RpcStatus::resource_exhausted)?;
        if let Err(error) = self.handle.history.save_node(id, name.clone(), saved) {
            state.error = Some(error);
        }
        self.handle
            .history
            .replace_rules(id, name, config.rules.clone());
        info.session.store(id, Ordering::Release);
        info.initial.lock().unwrap().take();
        {
            let mut pending = info.history.lock().unwrap();
            pending.subscribed = true;
            let session = state.sessions.get_mut(&id).unwrap();
            session.version = std::mem::take(&mut pending.version);
            self.handle
                .control
                .report_range_version(id, &session.version);
            self.handle
                .control
                .report_range_inventory(id, &config.rules);
            if let Some(sample) = pending.statistics.take() {
                self.handle
                    .history
                    .enqueue_statistics(session.name.clone(), sample);
            }
            // Keep the same status -> connection lock order as Ping. Enqueue
            // early reports before later reports can acquire the status lock.
            for batch in pending.early.drain(..) {
                self.handle.history.enqueue(session.name.clone(), batch);
            }
        }
        // Return the daemon's configuration unchanged. Subscription must not alter policy.
        Ok(Response::new(config))
    }

    async fn ask_rule(
        &self,
        request: Request<protocol::Connection>,
    ) -> Result<Response<protocol::Rule>, RpcStatus> {
        let session_id = connection_id(&request);
        let rpc_timeout = request
            .metadata()
            .get("grpc-timeout")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_timeout);
        let (popup_preferences, seconds, fallback) = self.handle.popup_policy();
        let configured_timeout = Duration::from_secs(seconds);
        // OpenSnitch 1.7.2's AskRule client uses a 120-second deadline. Keep a
        // finite bound even for clients that omit metadata, and leave a reply
        // margin before any shorter RPC limit.
        let hard_timeout = rpc_timeout
            .unwrap_or(Duration::from_secs(120))
            .min(Duration::from_secs(120))
            .saturating_sub(Duration::from_millis(100));
        let admitted = Instant::now();
        let deadline = admitted + configured_timeout.min(hard_timeout);
        let hard_deadline = admitted + hard_timeout;
        let connection = request.into_inner();
        let oversized = !prompts::valid_connection(&connection);
        let popups_disabled = !popup_preferences.popups;
        let initial_choice = if oversized {
            Default::default()
        } else {
            popup_preferences.initial(&connection, fallback)
        };
        let (node, hostname, node_text, pending) = {
            let mut state = self.handle.status.lock().unwrap();
            let session = state.sessions.get(&session_id);
            let hostname = session.map(|s| s.reported_name.clone()).unwrap_or_default();
            let node_text = session
                .filter(|s| !s.local)
                .map(|s| {
                    // TCP node keys are the trusted address/scope followed by the
                    // reported name; never derive locality from the reported name.
                    let address = s.name.split_once('/').map_or(s.name.as_str(), |(a, _)| a);
                    format!("on node {address}")
                })
                .unwrap_or_default();
            let node = session.map(|session| session.name.clone());
            let pending = if node.is_some()
                && !oversized
                && !popups_disabled
                && !self.handle.cancel.is_cancelled()
            {
                self.handle.prompts.insert_timed(
                    session_id,
                    connection.clone(),
                    deadline,
                    hard_deadline,
                    initial_choice,
                )
            } else {
                None
            };
            if pending.is_none() && !popups_disabled {
                state.overloaded += 1;
            }
            (
                node.unwrap_or_else(|| "unregistered".into()),
                hostname,
                node_text,
                pending,
            )
        };
        let (decision, reason, id, chosen_rule) = if let Some((prompt, receiver)) = pending {
            let _guard = PendingGuard(self.handle.prompts.clone(), prompt.id);
            let decision = tokio::select! {
                biased;
                _=self.handle.cancel.cancelled()=> (fallback,"shutdown",None),
                result=receiver=>match result {Ok(d)=>(d.decision,d.reason,d.rule),Err(_)=>(fallback,"disconnected",None)},
                _=self.handle.prompts.wait_for_expiry(prompt.id)=> {
                    self.handle.status.lock().unwrap().timed_out+=1;
                    (fallback,"timeout",None)
                }
            };
            (decision.0, decision.1, prompt.id, decision.2)
        } else {
            (
                fallback,
                if popups_disabled {
                    "popups-disabled"
                } else {
                    "overload"
                },
                0,
                None,
            )
        };
        if reason == "decision" {
            self.handle.status.lock().unwrap().answered += 1;
        }
        // Never echo an oversized connection into a reply or history queue.
        let safe = if oversized {
            protocol::Connection::default()
        } else {
            connection
        };
        let rule = chosen_rule
            .map(|rule| *rule)
            .unwrap_or_else(|| prompts::once_rule(&safe, decision, id));
        if reason == "timeout" {
            // Notification preferences apply immediately; the rule/fallback
            // above remains captured. This bounded snapshot has no I/O.
            let notice_preferences = self.handle.preferences.snapshot();
            crate::desktop_notifications::missed::post_timeout(
                &self.handle.desktop_notices,
                &notice_preferences,
                &safe,
                &rule,
                &node_text,
                &hostname,
                reason,
            );
        }
        if reason == "decision"
            && rule.duration != "once"
            && !self
                .handle
                .history
                .propose_rule(session_id, node.clone(), rule.clone())
        {
            self.handle.status.lock().unwrap().last_alert=Some("Popup reply sent; the pending rule could not be cached. Reconnect to refresh the daemon inventory.".into());
        }
        self.handle.history.ingest(
            node,
            vec![protocol::Event {
                unixnano: history::now_nanos(),
                connection: Some(safe),
                rule: Some(rule.clone()),
                time: String::new(),
            }],
            reason,
        );
        Ok(Response::new(rule))
    }

    type NotificationsStream =
        Pin<Box<dyn Stream<Item = Result<protocol::Notification, RpcStatus>> + Send + 'static>>;
    async fn notifications(
        &self,
        request: Request<tonic::Streaming<protocol::NotificationReply>>,
    ) -> Result<Response<Self::NotificationsStream>, RpcStatus> {
        let id = connection_id(&request);
        {
            let mut state = self.handle.status.lock().unwrap();
            let Some(session) = state.sessions.get_mut(&id) else {
                return Err(RpcStatus::failed_precondition(
                    "Subscribe on this connection first",
                ));
            };
            if session.notifications {
                return Err(RpcStatus::already_exists(
                    "Notification stream already open",
                ));
            }
            session.notifications = true;
        }
        let mut input = request.into_inner();
        let (tx, rx) = mpsc::channel(8);
        self.handle.control.attach(id, tx.clone());
        let handle = self.handle.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _=handle.cancel.cancelled()=>break,
                    _=tx.closed()=>break,
                    reply=input.message()=>match reply {
                        Ok(Some(reply))=> {
                            if reply.code!=protocol::NotificationReplyCode::Ok as i32 {
                                handle.status.lock().unwrap().last_alert=Some(format!("Daemon notification {} failed: {}",reply.id,history::bounded(&reply.data,512)));
                            }
                            if let Some(reply)=handle.live.complete(id, reply) {
                                handle.control.complete(id, reply, &handle.history);
                            }
                        },
                        _=>break,
                    }
                }
            }
            handle.disconnect(id);
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }

    async fn post_alert(
        &self,
        request: Request<protocol::Alert>,
    ) -> Result<Response<protocol::MsgResponse>, RpcStatus> {
        let session = connection_id(&request);
        let node = self
            .handle
            .session(session)
            .map(|node| node.name)
            .unwrap_or_else(|| "Unidentified peer".into());
        let alert = request.into_inner();
        let id = alert.id;
        match protocol::alert::Action::try_from(alert.action) {
            Ok(protocol::alert::Action::ShowAlert) => {
                if self.handle.preferences.snapshot().desktop_notifications {
                    self.handle.desktop_notices.post(&node, &alert);
                }
                self.handle.status.lock().unwrap().last_alert =
                    Some(crate::alerts::summary(&alert));
                self.handle.history.alert(node, alert);
            }
            Ok(protocol::alert::Action::SaveToDb) => self.handle.history.alert(node, alert),
            Ok(protocol::alert::Action::None) => {}
            Err(_) => {
                self.handle
                    .history
                    .health
                    .alerts_skipped
                    .fetch_add(1, Ordering::Relaxed);
                *self.handle.history.health.alerts_error.lock().unwrap() = Some(format!(
                    "Unsupported daemon alert action {}; not recorded",
                    alert.action
                ));
            }
        }
        Ok(Response::new(protocol::MsgResponse { id }))
    }
}

fn parse_timeout(value: &str) -> Option<Duration> {
    if value.len() < 2 || value.len() > 9 {
        return None;
    }
    let (digits, unit) = value.split_at(value.len() - 1);
    let n = digits.parse::<u64>().ok()?;
    match unit {
        "H" => Some(Duration::from_secs(n.checked_mul(3600)?)),
        "M" => Some(Duration::from_secs(n.checked_mul(60)?)),
        "S" => Some(Duration::from_secs(n)),
        "m" => Some(Duration::from_millis(n)),
        "u" => Some(Duration::from_micros(n)),
        "n" => Some(Duration::from_nanos(n)),
        _ => None,
    }
}

#[cfg(test)]
mod subscription_tests;
