use super::model::*;
use futures_util::StreamExt;
use std::{
    io::{BufWriter, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const NORMALIZE_BATCH: usize = 64 * 1024;

async fn normalize_chunk(
    validator: &mut super::format::Normalizer,
    output: &mut impl Write,
    chunk: &[u8],
    cancel: &CancellationToken,
) -> Result<(), String> {
    // HTTP may deliver several ready chunks in one poll. Bound synchronous
    // parsing/writing work and explicitly give control requests a turn.
    for part in chunk.chunks(NORMALIZE_BATCH) {
        if cancel.is_cancelled() {
            return Err("Cancelled".into());
        }
        validator.push(part, output)?;
        tokio::task::yield_now().await;
    }
    Ok(())
}
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
    // Normalized lists contain many short lines. Buffer them instead of making
    // two filesystem writes per entry on the subscription runtime's only thread.
    let mut output = BufWriter::with_capacity(NORMALIZE_BATCH, file.as_file_mut());
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
        normalize_chunk(&mut validator, &mut output, &chunk, &cancel).await?;
        bytes.store(count, Ordering::Relaxed);
    }
    validator.finish(&mut output)?;
    if cancel.is_cancelled() {
        return Err("Cancelled".into());
    }
    output.flush().map_err(|e| e.to_string())?;
    drop(output);
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    Ok(Download::Updated {
        file,
        bytes: validator.bytes,
        etag,
        modified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn a_ready_large_chunk_yields_and_observes_cancellation_before_its_end() {
        let data = b"0.0.0.0 bounded.example\n".repeat(20_000);
        let mut validator = super::super::format::Normalizer::new(Format::Hosts, data.len() as u64);
        let mut output = Vec::new();
        let cancel = CancellationToken::new();
        let (result, ()) = tokio::join!(
            normalize_chunk(&mut validator, &mut output, &data, &cancel),
            async {
                tokio::task::yield_now().await;
                cancel.cancel();
            }
        );
        assert_eq!(result.unwrap_err(), "Cancelled");
        assert!(!output.is_empty());
        assert!(output.len() < data.len());
    }
}
