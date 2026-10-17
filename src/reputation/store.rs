use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
};
const MAX_SETTINGS: usize = 16 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    settings: Settings,
}
pub(super) struct Secret(Vec<u8>);
impl Secret {
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([redacted])")
    }
}
pub(super) fn key(path: Option<&Path>) -> Result<Secret> {
    let path = path.ok_or_else(|| anyhow::anyhow!("No key file"))?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.len() <= 258
            && metadata.mode() & 0o777 == 0o600
            && metadata.uid() == unsafe { libc::geteuid() },
        "Key must be an owned regular file with mode 0600 and at most 256 bytes plus a line ending"
    );
    let mut bytes = Vec::new();
    Read::by_ref(&mut file).take(258).read_to_end(&mut bytes)?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    ensure!(
        !bytes.is_empty() && bytes.len() <= 256 && bytes.iter().all(|b| b.is_ascii_graphic()),
        "Key must be 1–256 visible ASCII bytes"
    );
    Ok(Secret(bytes))
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
        "Preferences must be a regular file"
    );
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_SETTINGS as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_SETTINGS, "Preferences exceed 16 KiB");
    let document: Document = serde_json::from_slice(&bytes)?;
    ensure!(document.version == 1, "Unsupported preferences version");
    document.settings.validate()?;
    Ok((document.settings, Some(hex::encode(Sha256::digest(bytes)))))
}
pub(super) fn load(path: &Path) -> Snapshot {
    match read(path) {
        Ok((settings, revision)) => {
            let key_status = if settings.api_key_file.is_none() {
                KeyStatus::Missing
            } else if key(settings.api_key_file.as_deref()).is_ok() {
                KeyStatus::Ready
            } else {
                KeyStatus::Invalid
            };
            Snapshot {
                ready: true,
                settings,
                revision,
                key_status,
                warning: None,
                dispatched: 0,
            }
        }
        Err(_) => Snapshot {
            ready: true,
            settings: Settings::default(),
            revision: None,
            key_status: KeyStatus::Missing,
            warning: Some(
                "Reputation preferences are invalid or inaccessible; existing file preserved"
                    .into(),
            ),
            dispatched: 0,
        },
    }
}
pub(super) fn save(
    path: &Path,
    expected: &Option<String>,
    settings: &Settings,
) -> Result<Snapshot> {
    settings.validate()?;
    if settings.enabled {
        let _key = key(settings.api_key_file.as_deref())?;
    }
    ensure!(
        &read(path)?.1 == expected,
        "Preferences changed; reload before saving"
    );
    let bytes = serde_json::to_vec_pretty(&Document {
        version: 1,
        settings: settings.clone(),
    })?;
    ensure!(bytes.len() <= MAX_SETTINGS, "Preferences exceed 16 KiB");
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    ensure!(
        &read(path)?.1 == expected,
        "Preferences changed; reload before saving"
    );
    if expected.is_none() {
        temp.persist_noclobber(path).map_err(|e| e.error)?;
    } else {
        temp.persist(path).map_err(|e| e.error)?;
    }
    File::open(parent)?.sync_all()?;
    Ok(load(path))
}
