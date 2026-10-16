//! Explicit, cancellable URL diagnostics. Never publishes a file or changes a subscription.
use super::{Format, Handle, Reply, Request};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
pub const MAX_SAMPLE_BYTES: usize = 64 * 1024;
pub const MAX_SAMPLE_LINES: usize = 200;
#[derive(Clone, Debug, serde::Serialize)]
pub struct Report {
    pub final_url: String,
    pub status: u16,
    pub redirected: bool,
    pub get_fallback: bool,
    pub sample_bytes: usize,
    pub sample_lines: usize,
    pub sample_capped: bool,
    pub short_sample: bool,
    pub hosts_like: Option<bool>,
}
impl Report {
    pub fn message(&self) -> String {
        let mut message = format!(
            "URL reachable{} (HTTP {}).",
            if self.redirected { " via redirect" } else { "" },
            self.status
        );
        if self.hosts_like == Some(false) {
            message.push_str(" The sample does not resemble hosts format.");
        } else if self.hosts_like == Some(true) {
            message.push_str(if self.short_sample {
                " Short sample: hosts format is not conclusive."
            } else {
                " The sample resembles hosts format."
            });
        }
        if self.sample_capped {
            message.push_str(" Sample limited to 200 lines / 64 KiB.");
        }
        if self.get_fallback {
            message.push_str(" HEAD was refused; GET succeeded.");
        }
        if self.redirected {
            message.push(' ');
            message.push_str(&self.final_url);
        }
        message
    }
}
pub struct Input {
    pub(super) url: String,
    pub(super) format: Format,
    pub(super) cancel: CancellationToken,
}
pub struct Ticket {
    cancel: CancellationToken,
    result: oneshot::Receiver<Result<Reply, String>>,
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl Ticket {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub fn try_result(&mut self) -> Option<Result<Report, String>> {
        match self.result.try_recv() {
            Ok(Ok(Reply::UrlTest(report))) => Some(Ok(report)),
            Ok(Err(e)) => Some(Err(e)),
            Ok(Ok(_)) => Some(Err("Unexpected URL-test response".into())),
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => Some(Err("URL test stopped".into())),
        }
    }
    pub async fn result(&mut self) -> Result<Report, String> {
        match (&mut self.result).await.map_err(|_| "URL test stopped")?? {
            Reply::UrlTest(report) => Ok(report),
            _ => Err("Unexpected URL-test response".into()),
        }
    }
}
impl Handle {
    pub fn test_url(&self, url: String, format: Format) -> Result<Ticket, String> {
        validate_url(&url)?;
        let cancel = CancellationToken::new();
        let result = self.request(Request::TestUrl(Input {
            url,
            format,
            cancel: cancel.clone(),
        }))?;
        Ok(Ticket { cancel, result })
    }
}
pub fn validate_url(url: &str) -> Result<(), String> {
    if url.is_empty() || url.len() > 2048 || url.chars().any(char::is_control) {
        return Err("Enter an HTTP(S) URL of at most 2048 bytes".into());
    }
    let value = reqwest::Url::parse(url).map_err(|_| "Enter a valid HTTP or HTTPS URL")?;
    if !matches!(value.scheme(), "http" | "https")
        || value.host_str().is_none()
        || !value.username().is_empty()
        || value.password().is_some()
        || value.fragment().is_some()
    {
        return Err("Use an HTTP(S) URL without embedded credentials or a fragment".into());
    }
    Ok(())
}
/// Pinned OpenSnitch is_hosts_file_like heuristic, including its permissive
/// <=10 non-comment-line case. This diagnostic does not validate a whole list.
pub fn hosts_like(lines: &[String]) -> (bool, bool) {
    let mut total = 0;
    let mut valid = 0;
    for line in lines.iter().take(MAX_SAMPLE_LINES) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        total += 1;
        let mut parts = line.split_whitespace();
        let first = parts.next().unwrap_or("");
        let second = parts.next();
        if second.is_some_and(|second| {
            matches!(first, "0.0.0.0" | "127.0.0.1" | "::")
                && second.contains('.')
                && !second.contains('/')
        }) || (second.is_none() && first.contains('.'))
        {
            valid += 1;
        }
    }
    (total <= 10 || valid * 100 >= total * 60, total <= 10)
}
async fn run(input: &Input, user_agent: &str) -> Result<Report, String> {
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .connect_timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("URL test exceeds five redirects");
            }
            if validate_url(attempt.url().as_str()).is_err() {
                return attempt.error("Redirect target is not a supported HTTP(S) URL");
            }
            attempt.follow()
        }))
        .user_agent(user_agent)
        .build()
        .map_err(|e| format!("Cannot start URL test: {e}"))?;
    let head = client
        .head(&input.url)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map_err(|e| format!("URL test failed: {e}"))?;
    let fallback = matches!(head.status().as_u16(), 403 | 405);
    if (head.status().is_client_error() || head.status().is_server_error()) && !fallback {
        return Err(format!("URL test failed: HTTP {}", head.status().as_u16()));
    }
    let mut report = Report {
        final_url: head.url().to_string(),
        status: head.status().as_u16(),
        redirected: false,
        get_fallback: fallback,
        sample_bytes: 0,
        sample_lines: 0,
        sample_capped: false,
        short_sample: false,
        hosts_like: None,
    };
    drop(head);
    let hosts = input.format == Format::Hosts;
    if fallback || hosts {
        let response = client
            .get(&input.url)
            .timeout(Duration::from_secs(if hosts { 5 } else { 3 }))
            .send()
            .await
            .map_err(|e| format!("URL test failed: {e}"))?;
        if response.status().is_client_error() || response.status().is_server_error() {
            return Err(format!(
                "URL test failed: HTTP {}",
                response.status().as_u16()
            ));
        }
        report.final_url = response.url().to_string();
        report.status = response.status().as_u16();
        if hosts {
            let mut bytes = Vec::new();
            let mut lines = 0usize;
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| format!("URL sample failed: {e}"))?;
                for b in chunk {
                    if bytes.len() == MAX_SAMPLE_BYTES || lines == MAX_SAMPLE_LINES {
                        report.sample_capped = true;
                        break;
                    }
                    bytes.push(b);
                    if b == b'\n' {
                        lines += 1;
                    }
                }
                if bytes.len() == MAX_SAMPLE_BYTES || lines == MAX_SAMPLE_LINES {
                    report.sample_capped = true;
                    break;
                }
            }
            // Ignore invalid UTF-8 as the pinned sample heuristic does.
            let mut text = String::new();
            let mut remaining = bytes.as_slice();
            loop {
                match std::str::from_utf8(remaining) {
                    Ok(valid) => {
                        text.push_str(valid);
                        break;
                    }
                    Err(error) => {
                        let n = error.valid_up_to();
                        text.push_str(std::str::from_utf8(&remaining[..n]).unwrap());
                        remaining =
                            &remaining[n + error.error_len().unwrap_or(remaining.len() - n)..];
                    }
                }
            }
            let sample = text
                .lines()
                .take(MAX_SAMPLE_LINES)
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let (like, short) = hosts_like(&sample);
            report.hosts_like = Some(like);
            report.short_sample = short;
            report.sample_bytes = bytes.len();
            report.sample_lines = sample.len();
        }
    }
    validate_url(&report.final_url)?;
    report.redirected = report.final_url != input.url;
    Ok(report)
}
pub(super) async fn fetch(input: Input, user_agent: String) -> Result<Report, String> {
    tokio::select! {
        biased;
        _=input.cancel.cancelled()=>Err("URL test cancelled".into()),
        result=tokio::time::timeout(Duration::from_secs(12),run(&input,&user_agent))=>result.map_err(|_|"URL test exceeded its 12-second deadline".to_owned())?.map_err(|e|crate::history::bounded(&e,1024)),
    }
}
pub(super) struct Active {
    pub cancel: CancellationToken,
    pub task: tokio::task::JoinHandle<Result<Report, String>>,
    pub reply: oneshot::Sender<Result<Reply, String>>,
}
