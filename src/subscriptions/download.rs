use super::model::*;
use futures_util::StreamExt;
use std::{
    io::Write,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;
pub(super) enum Download {
    Updated {
        file: tempfile::NamedTempFile,
        bytes: u64,
        etag: String,
        modified: String,
    },
    Unchanged,
}
pub(super) async fn fetch(
    client: reqwest::Client,
    row: Row,
    settings: Settings,
    tmp: &Path,
    force: bool,
    cancel: CancellationToken,
    bytes: Arc<AtomicU64>,
) -> Result<Download, String> {
    let (_, timeout, max) = effective(&row.spec, &settings)?;
    let mut request = client
        .get(&row.spec.url)
        .timeout(Duration::from_secs(u64::from(timeout)))
        .header(reqwest::header::USER_AGENT, &settings.user_agent);
    let has_file = row.publication.is_some()
        && row
            .file
            .as_ref()
            .is_some_and(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file()));
    if !force && has_file {
        if !row.etag.is_empty() {
            request = request.header(reqwest::header::IF_NONE_MATCH, &row.etag);
        }
        if !row.modified.is_empty() {
            request = request.header(reqwest::header::IF_MODIFIED_SINCE, &row.modified);
        }
    }
    let response = tokio::select! {_ = cancel.cancelled()=>return Err("Cancelled".into()), response=request.send()=>response.map_err(|e|format!("HTTP request failed: {e}"))?};
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return if has_file {
            Ok(Download::Unchanged)
        } else {
            Err("Server returned 304 without a retained local list".into())
        };
    }
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    if response.content_length().is_some_and(|n| n > max) {
        return Err("Content-Length exceeds the configured byte limit".into());
    }
    let header = |name| -> Result<String, String> {
        let value = response
            .headers()
            .get(name)
            .map(|v| v.to_str().map(str::to_owned))
            .transpose()
            .map_err(|_| "Invalid HTTP cache validator")?
            .unwrap_or_default();
        if value.len() > 1024 {
            return Err("HTTP cache validator exceeds 1 KiB".into());
        }
        Ok(value)
    };
    let etag = header(reqwest::header::ETAG)?;
    let modified = header(reqwest::header::LAST_MODIFIED)?;
    let mut file = tempfile::Builder::new()
        .prefix("download-")
        .tempfile_in(tmp)
        .map_err(|e| e.to_string())?;
    let mut validator = super::format::Normalizer::new(row.spec.format.clone(), max);
    let mut stream = response.bytes_stream();
    let mut count = 0;
    loop {
        let chunk = tokio::select! {_ = cancel.cancelled()=>return Err("Cancelled".into()),chunk=stream.next()=>chunk};
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(|e| format!("Download stream failed: {e}"))?;
        count += chunk.len() as u64;
        if count > max {
            return Err("Stream exceeds the configured byte limit".into());
        }
        validator.push(&chunk, &mut file)?;
        bytes.store(count, Ordering::Relaxed);
    }
    validator.finish(&mut file)?;
    if cancel.is_cancelled() {
        return Err("Cancelled".into());
    }
    file.flush()
        .and_then(|_| file.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    Ok(Download::Updated {
        file,
        bytes: validator.bytes,
        etag,
        modified,
    })
}
