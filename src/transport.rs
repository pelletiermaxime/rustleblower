//! Explicit bounded UI-server transports. Daemons remain clients and own policy.
use anyhow::{Context, Result, ensure};
use clap::{Args, ValueEnum};
use futures_util::Stream;
use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::{self, Read},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    pin::Pin,
    task::{Context as TaskContext, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpStream, UnixStream},
};
use tokio_stream::wrappers::{TcpListenerStream, UnixListenerStream};
use tonic::transport::{Certificate, Identity, ServerTlsConfig};

pub const MAX_CONNECTIONS: usize = 64;
pub const MAX_INITIAL_CONNECTIONS: usize = 8;
pub const INITIAL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
pub const MAX_CREDENTIAL_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum Auth {
    #[default]
    Simple,
    TlsSimple,
    TlsMutual,
}
impl Auth {
    pub fn key(self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::TlsSimple => "tls-simple",
            Self::TlsMutual => "tls-mutual",
        }
    }
}
#[derive(Clone, Debug, Default, Args)]
pub struct Settings {
    #[arg(long,value_enum,default_value_t=Auth::Simple)]
    pub socket_auth: Auth,
    #[arg(long)]
    pub tls_ca_cert: Option<PathBuf>,
    #[arg(long)]
    pub tls_cert: Option<PathBuf>,
    #[arg(long)]
    pub tls_key: Option<PathBuf>,
}
impl Settings {
    /// Called before GPUI startup; no credential reads occur on the UI/RPC path.
    pub fn tls(&self, endpoint: &Endpoint) -> Result<Option<ServerTlsConfig>> {
        if self.socket_auth == Auth::Simple {
            ensure!(
                self.tls_ca_cert.is_none() && self.tls_cert.is_none() && self.tls_key.is_none(),
                "TLS credential flags require --socket-auth tls-simple or tls-mutual"
            );
            return Ok(None);
        }
        ensure!(
            matches!(endpoint, Endpoint::Tcp(_)),
            "TLS modes require a TCP listener"
        );
        let certificate = read_credential(
            self.tls_cert
                .as_deref()
                .context("TLS requires --tls-cert")?,
        )?;
        let key = read_credential(self.tls_key.as_deref().context("TLS requires --tls-key")?)?;
        let mut tls = ServerTlsConfig::new().identity(Identity::from_pem(certificate, key));
        match self.socket_auth {
            Auth::TlsMutual => {
                let ca = read_credential(
                    self.tls_ca_cert
                        .as_deref()
                        .context("Mutual TLS requires --tls-ca-cert")?,
                )?;
                tls = tls
                    .client_ca_root(Certificate::from_pem(ca))
                    .client_auth_optional(false);
            }
            Auth::TlsSimple => ensure!(
                self.tls_ca_cert.is_none(),
                "Use tls-mutual with --tls-ca-cert to require verified client certificates"
            ),
            Auth::Simple => unreachable!(),
        }
        Ok(Some(tls))
    }
}
fn read_credential(path: &Path) -> Result<Vec<u8>> {
    ensure!(
        path.as_os_str().len() <= 4096,
        "TLS credential path exceeds 4096 bytes"
    );
    // O_NONBLOCK avoids blocking on a FIFO substituted before open; O_NOFOLLOW
    // prevents a symlink from redirecting a configured credential read.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("Cannot open TLS credential {}", path.display()))?;
    ensure!(
        file.metadata()?.is_file(),
        "TLS credentials must be regular files"
    );
    let mut bytes = Vec::new();
    file.take(MAX_CREDENTIAL_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= MAX_CREDENTIAL_BYTES,
        "Each TLS credential must contain 1–65536 bytes"
    );
    Ok(bytes)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Unix(PathBuf),
    Tcp(SocketAddr),
}
impl Endpoint {
    pub fn parse(value: &Path) -> Result<Self> {
        let text = value.to_str().context("Listener address must be UTF-8")?;
        ensure!(
            text.len() <= 4096 && !text.chars().any(char::is_control),
            "Listener address is invalid or exceeds 4096 bytes"
        );
        if let Some(path) = text.strip_prefix("unix://") {
            ensure!(
                Path::new(path).is_absolute(),
                "Unix socket path must be absolute"
            );
            return Ok(Self::Unix(path.into()));
        }
        if value.is_absolute() {
            return Ok(Self::Unix(value.into()));
        }
        let address = text
            .strip_prefix("tcp://")
            .unwrap_or(text)
            .parse::<SocketAddr>()
            .context("Use an absolute Unix socket path, IPv4:port or [IPv6]:port")?;
        Ok(Self::Tcp(address))
    }
    pub fn label(&self) -> String {
        match self {
            Self::Unix(path) => path.display().to_string(),
            Self::Tcp(address) => address.to_string(),
        }
    }
}

/// Trusted socket identity, independent of the daemon's self-reported name.
#[derive(Clone, Debug)]
pub struct Peer {
    pub address: Option<IpAddr>,
    pub local: bool,
    scope_id: u32,
}
impl Peer {
    pub fn unix() -> Self {
        Self {
            address: None,
            local: true,
            scope_id: 0,
        }
    }
    pub fn tcp(address: IpAddr, local: &BTreeSet<IpAddr>) -> Self {
        let address = normalized(address);
        // Link-local addresses identify a host only within an interface's zone.
        // The unscoped interface snapshot cannot establish their locality.
        let link_local = matches!(address,IpAddr::V6(ip) if ip.is_unicast_link_local());
        Self {
            address: Some(address),
            local: !link_local && (address.is_loopback() || local.contains(&address)),
            scope_id: 0,
        }
    }
    pub fn tcp_socket(address: SocketAddr, local: &BTreeSet<IpAddr>) -> Self {
        let mut peer = Self::tcp(address.ip(), local);
        if let SocketAddr::V6(address) = address {
            peer.scope_id = address.scope_id();
        }
        peer
    }
    pub fn node_key(&self, reported: &str) -> Result<String> {
        ensure!(
            reported.len() <= 128 && !reported.chars().any(char::is_control),
            "Node name must be at most 128 bytes without control characters"
        );
        let name = if reported.is_empty() {
            "local"
        } else {
            reported
        };
        let key = match self.address {
            None => name.to_owned(),
            Some(address) if self.scope_id != 0 => format!("{address}%{}/{name}", self.scope_id),
            Some(address) => format!("{address}/{name}"),
        };
        ensure!(
            key.len() <= 128,
            "Node name plus TCP peer address exceeds 128 bytes; choose a shorter daemon name"
        );
        Ok(key)
    }
}
fn normalized(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(address),
        _ => address,
    }
}

/// Snapshot at server startup. Failure conservatively admits only loopback as
/// local; a daemon's name/configuration can never elevate a remote peer.
pub fn local_addresses() -> Result<BTreeSet<IpAddr>> {
    let mut head = std::ptr::null_mut();
    // SAFETY: libc initializes this owned linked list, released by Guard on all paths.
    ensure!(
        unsafe { libc::getifaddrs(&mut head) } == 0,
        "Cannot read local interface addresses"
    );
    struct Guard(*mut libc::ifaddrs);
    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: pointer is returned by successful getifaddrs and freed once.
            unsafe { libc::freeifaddrs(self.0) };
        }
    }
    let _guard = Guard(head);
    let mut addresses = BTreeSet::new();
    let mut current = head;
    let mut count = 0;
    while !current.is_null() {
        count += 1;
        ensure!(count <= 1024, "Interface snapshot exceeds 1024 entries");
        // SAFETY: each pointer belongs to the live getifaddrs list. Family is
        // checked before interpreting its sockaddr with the corresponding layout.
        unsafe {
            let address = (*current).ifa_addr;
            if !address.is_null() {
                match i32::from((*address).sa_family) {
                    libc::AF_INET => {
                        let value = &*address.cast::<libc::sockaddr_in>();
                        addresses.insert(IpAddr::V4(Ipv4Addr::from(
                            value.sin_addr.s_addr.to_ne_bytes(),
                        )));
                    }
                    libc::AF_INET6 => {
                        let value = &*address.cast::<libc::sockaddr_in6>();
                        addresses.insert(normalized(IpAddr::V6(Ipv6Addr::from(
                            value.sin6_addr.s6_addr,
                        ))));
                    }
                    _ => {}
                }
            }
            current = (*current).ifa_next;
        }
    }
    Ok(addresses)
}

pub enum Listener {
    Unix(std::os::unix::net::UnixListener),
    Tcp(std::net::TcpListener),
}
pub enum Io {
    Unix(UnixStream),
    Tcp(TcpStream),
}
pub type Incoming = Pin<Box<dyn Stream<Item = io::Result<Io>> + Send>>;
impl Listener {
    pub fn incoming(self) -> io::Result<Incoming> {
        use futures_util::StreamExt;
        Ok(match self {
            Self::Unix(listener) => Box::pin(
                UnixListenerStream::new(tokio::net::UnixListener::from_std(listener)?)
                    .map(|s| s.map(Io::Unix)),
            ),
            Self::Tcp(listener) => Box::pin(
                TcpListenerStream::new(tokio::net::TcpListener::from_std(listener)?)
                    .map(|s| s.map(Io::Tcp)),
            ),
        })
    }
}
impl AsyncRead for Io {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Unix(s) => Pin::new(s).poll_read(cx, buf),
            Self::Tcp(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}
impl AsyncWrite for Io {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut *self {
            Self::Unix(s) => Pin::new(s).poll_write(cx, buf),
            Self::Tcp(s) => Pin::new(s).poll_write(cx, buf),
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Unix(s) => Pin::new(s).poll_flush(cx),
            Self::Tcp(s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Unix(s) => Pin::new(s).poll_shutdown(cx),
            Self::Tcp(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Admission runs outside generated gRPC decoding. Each method has an independent
/// shared pool; no waiter/task queue is allocated when a pool is full.
/// Notifications releases its admission permit at response headers. Its incoming
/// message stream remains bounded by the existing one-per-session /32-node gate.
#[derive(Clone)]
pub struct Admission<S> {
    inner: S,
    pools: std::sync::Arc<[std::sync::Arc<tokio::sync::Semaphore>; 5]>,
}
pub const RPC_CAPACITIES: [usize; 5] = [40, 4, 2, 4, 4];
impl<S> Admission<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            pools: std::sync::Arc::new(
                RPC_CAPACITIES.map(|cap| std::sync::Arc::new(tokio::sync::Semaphore::new(cap))),
            ),
        }
    }
}
impl<S: tonic::server::NamedService> tonic::server::NamedService for Admission<S> {
    const NAME: &'static str = S::NAME;
}
impl<S> tower::Service<http::Request<tonic::body::Body>> for Admission<S>
where
    S: tower::Service<
            http::Request<tonic::body::Body>,
            Response = http::Response<tonic::body::Body>,
        >,
    S::Future: Send + 'static,
    S::Error: 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future =
        Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }
    fn call(&mut self, request: http::Request<tonic::body::Body>) -> Self::Future {
        let index = match request.uri().path() {
            "/protocol.UI/AskRule" => 0,
            "/protocol.UI/Ping" => 1,
            "/protocol.UI/Subscribe" => 2,
            "/protocol.UI/PostAlert" => 3,
            "/protocol.UI/Notifications" => 4,
            _ => {
                return Box::pin(async {
                    Ok(tonic::Status::unimplemented("Unknown UI method").into_http())
                });
            }
        };
        let permit = match self.pools[index].clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                return Box::pin(async {
                    Ok(tonic::Status::resource_exhausted(
                        "UI method admission capacity reached; retry later",
                    )
                    .into_http())
                });
            }
        };
        let response = self.inner.call(request);
        Box::pin(async move {
            let _permit = permit;
            response.await
        })
    }
}
