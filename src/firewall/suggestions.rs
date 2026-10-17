//! Bounded local display suggestions. Discovery runs off the UI thread and
//! never contacts a daemon, DNS service or firewall.
use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    sync::{Arc, OnceLock},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub const MAX_BYTES: usize = 256 * 1024;
pub const MAX_SERVICES: usize = 2048;
pub const MAX_INTERFACES: usize = 128;
const PROTOCOLS: &[&str] = &["tcp", "udp", "udplite", "sctp", "dccp"];

#[cfg(all(test, feature = "headless-tests"))]
static FIXTURE: std::sync::Mutex<Option<(std::path::PathBuf, std::path::PathBuf)>> =
    std::sync::Mutex::new(None);

#[cfg(all(test, feature = "headless-tests"))]
pub(crate) struct DiscoveryFixture(Option<(std::path::PathBuf, std::path::PathBuf)>);
#[cfg(all(test, feature = "headless-tests"))]
impl Drop for DiscoveryFixture {
    fn drop(&mut self) {
        *FIXTURE.lock().unwrap() = self.0.take();
    }
}
/// Only the GPU test binary can supply owned filesystem roots. Normal builds
/// have no runtime path/environment override for host discovery.
#[cfg(all(test, feature = "headless-tests"))]
pub(crate) fn discovery_fixture(
    services: std::path::PathBuf,
    interfaces: std::path::PathBuf,
) -> DiscoveryFixture {
    assert!(services.is_absolute() && interfaces.is_absolute());
    DiscoveryFixture(FIXTURE.lock().unwrap().replace((services, interfaces)))
}

pub fn read_job() -> Option<OwnedSemaphorePermit> {
    static JOB: OnceLock<Arc<Semaphore>> = OnceLock::new();
    JOB.get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
        .try_acquire_owned()
        .ok()
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Service {
    pub name: String,
    pub port: u16,
    pub protocol: String,
}
impl Service {
    pub fn label(&self) -> String {
        format!("{} — {}/{}", self.name, self.port, self.protocol)
    }
}

#[derive(Debug, Default)]
pub struct Catalogue {
    pub services: Vec<Service>,
    pub interfaces: Vec<String>,
    pub warning: Option<String>,
}
fn name(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"_.:-".contains(&v))
}
impl Catalogue {
    pub fn discover() -> Self {
        #[cfg(all(test, feature = "headless-tests"))]
        if let Some((services, interfaces)) = FIXTURE.lock().unwrap().clone() {
            return Self::load(&services, &interfaces);
        }
        let services = if std::fs::symlink_metadata("/etc/services")
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            Path::new("/usr/etc/services")
        } else {
            Path::new("/etc/services")
        };
        Self::load(services, Path::new("/sys/class/net"))
    }

    /// Explicit roots also allow fixtures to exercise the real discovery path.
    pub fn load(services: &Path, interfaces: &Path) -> Self {
        let mut result = Self::default();
        let read = (|| -> std::io::Result<Vec<u8>> {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(services)?;
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::other(
                    "service catalogue is not a regular file",
                ));
            }
            let mut bytes = Vec::new();
            file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
            Ok(bytes)
        })();
        match read {
            Ok(mut bytes) => {
                let truncated = bytes.len() > MAX_BYTES;
                if truncated {
                    bytes.truncate(MAX_BYTES);
                    // Never turn a partial final record into a valid suggestion.
                    if let Some(last) = bytes.iter().rposition(|v| *v == b'\n') {
                        bytes.truncate(last);
                    } else {
                        bytes.clear();
                    }
                }
                let (services, limited) = Self::parse_services(&String::from_utf8_lossy(&bytes));
                result.services = services;
                // The pinned UI also offers WireGuard when absent from the
                // distribution's service catalogue.
                if result.services.len() < MAX_SERVICES
                    && !result
                        .services
                        .iter()
                        .any(|s| s.name == "wireguard" && s.protocol == "udp")
                {
                    result.services.push(Service {
                        name: "wireguard".into(),
                        port: 51820,
                        protocol: "udp".into(),
                    });
                    result.services.sort();
                }
                if truncated || limited {
                    result.warning = Some("Only part of the local service catalogue is shown. You can still enter a numeric port.".into());
                }
            }
            Err(_) => result.warning = Some(
                "Local service suggestions are unavailable. You can still enter a numeric port."
                    .into(),
            ),
        }
        match std::fs::read_dir(interfaces) {
            Ok(entries) => {
                for (index, entry) in entries.enumerate() {
                    if index >= MAX_INTERFACES {
                        result.warning = Some("Local suggestions were limited. Manual values remain available.".into());
                        break;
                    }
                    if let Ok(entry) = entry
                        && let Some(value) = entry.file_name().to_str()
                        && name(value, 15)
                    {
                        result.interfaces.push(value.to_owned());
                    }
                }
                result.interfaces.sort();
                result.interfaces.dedup();
            }
            Err(_) => result.warning = Some("Local interface suggestions are unavailable. You can still enter an interface name.".into()),
        }
        result
    }

    /// Return unique aliases with their protocol, never an unqualified global
    /// name-to-port guess. A full catalogue remains finite even for hostile input.
    pub fn parse_services(text: &str) -> (Vec<Service>, bool) {
        let mut records = BTreeSet::new();
        let input = &text[..text.floor_char_boundary(text.len().min(MAX_BYTES))];
        for line in input.lines() {
            let mut fields = line
                .split('#')
                .next()
                .unwrap_or_default()
                .split_whitespace();
            let Some(primary) = fields.next() else {
                continue;
            };
            let Some((port, protocol)) = fields.next().and_then(|v| v.split_once('/')) else {
                continue;
            };
            let Ok(port) = port.parse::<u16>() else {
                continue;
            };
            if port == 0 || !PROTOCOLS.contains(&protocol) {
                continue;
            }
            for alias in std::iter::once(primary).chain(fields) {
                if !name(alias, 64) {
                    continue;
                }
                let service = Service {
                    name: alias.into(),
                    port,
                    protocol: protocol.into(),
                };
                if records.len() == MAX_SERVICES && !records.contains(&service) {
                    return (records.into_iter().collect(), true);
                }
                records.insert(service);
            }
        }
        (records.into_iter().collect(), text.len() > MAX_BYTES)
    }

    pub fn port(&self, name: &str, protocols: &str) -> Result<Option<u16>, String> {
        if !name.is_empty()
            && name
                .bytes()
                .all(|v| v.is_ascii_digit() || b",-".contains(&v))
        {
            return Ok(None);
        }
        if !self::name(name, 64) || !protocols.split(',').all(|p| PROTOCOLS.contains(&p)) {
            return Err("Enter a numeric port, range, list or local service name".into());
        }
        let mut resolved = None;
        for protocol in protocols.split(',') {
            let mut matches = self
                .services
                .iter()
                .filter(|s| s.name == name && s.protocol == protocol);
            let Some(first) = matches.next() else {
                return Err(format!(
                    "No local {protocol} service named {name}; enter a numeric port or choose another protocol"
                ));
            };
            if matches.any(|s| s.port != first.port) || resolved.is_some_and(|p| p != first.port) {
                return Err(
                    "This service name has different ports; enter the intended numeric port".into(),
                );
            }
            resolved = Some(first.port);
        }
        Ok(resolved)
    }
}
