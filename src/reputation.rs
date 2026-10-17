//! Optional GET-only VirusTotal lookups. No binary reads/uploads, policy handle,
//! permission reply or unbounded response/cache is owned by this module.
mod store;
mod worker;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    path::{Path, PathBuf},
};
pub use worker::{Handle, Ticket, Worker};

pub const MAX_BODY: usize = 256 * 1024;
pub const MAX_REPORT: usize = 16 * 1024;
pub const MAX_CACHE: usize = 128;
pub const MAX_QUEUE: usize = 8;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningAction {
    None,
    #[default]
    RecommendReject,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub api_key_file: Option<PathBuf>,
    pub timeout_seconds: u8,
    /// Dispatch budget for this worker lifetime, not a fabricated daily quota.
    pub request_budget: u32,
    pub domains: bool,
    pub ips: bool,
    pub hashes: bool,
    pub minimum_threshold: u32,
    pub use_suspicious: bool,
    pub use_community_score: bool,
    pub use_reputation: bool,
    pub warning_action: WarningAction,
    pub excluded_domains: Vec<String>,
    pub excluded_ips: Vec<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key_file: None,
            timeout_seconds: 1,
            request_budget: 500,
            domains: true,
            ips: true,
            hashes: true,
            minimum_threshold: 1,
            use_suspicious: false,
            use_community_score: false,
            use_reputation: false,
            warning_action: WarningAction::RecommendReject,
            excluded_domains: Vec::new(),
            excluded_ips: Vec::new(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=10).contains(&self.timeout_seconds),
            "Timeout must be 1–10 seconds"
        );
        ensure!(
            (1..=5000).contains(&self.request_budget),
            "Request budget must be 1–5000"
        );
        ensure!(
            (1..=1_000_000).contains(&self.minimum_threshold),
            "Warning threshold must be 1–1000000"
        );
        if let Some(path) = &self.api_key_file {
            let text = path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Key file path must be valid text"))?;
            ensure!(
                path.is_absolute() && text.len() <= 4096 && !text.chars().any(char::is_control),
                "Choose a bounded absolute key file path"
            );
        }
        for exclusions in [&self.excluded_domains, &self.excluded_ips] {
            ensure!(
                exclusions.len() <= 32,
                "Use at most 32 exclusions per category"
            );
            ensure!(
                exclusions
                    .iter()
                    .all(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)),
                "Exclusions must be 1–128 characters on one line"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyStatus {
    Missing,
    Ready,
    Invalid,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub ready: bool,
    pub settings: Settings,
    pub revision: Option<String>,
    pub key_status: KeyStatus,
    pub warning: Option<String>,
    pub dispatched: u32,
}
pub enum Command {
    Reload,
    Save {
        expected_revision: Option<String>,
        settings: Settings,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Capture {
    pub session: u64,
    pub object_id: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    Domain(String),
    Ip(IpAddr),
    Hash(String),
}
impl Subject {
    pub fn domain(value: &str) -> Result<Self> {
        ensure!(value.len() <= 254, "Domain name is too long");
        let value = value.trim_end_matches('.').to_ascii_lowercase();
        ensure!(
            !value.is_empty() && value.len() <= 253 && value.parse::<IpAddr>().is_err(),
            "Choose a domain name, not an IP address"
        );
        ensure!(
            value.split('.').all(|label| !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')),
            "Domain contains unsupported characters"
        );
        Ok(Self::Domain(value))
    }
    pub fn hash(value: &str) -> Result<Self> {
        ensure!(
            [32, 40, 64].contains(&value.len()) && value.bytes().all(|b| b.is_ascii_hexdigit()),
            "Use a reported MD5, SHA1 or SHA256 hash"
        );
        Ok(Self::Hash(value.to_ascii_lowercase()))
    }
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Domain(text) => {
                Self::domain(text)?;
            }
            Self::Hash(text) => {
                Self::hash(text)?;
            }
            Self::Ip(_) => {}
        }
        Ok(())
    }
    pub fn value(&self) -> String {
        match self {
            Self::Domain(value) | Self::Hash(value) => value.clone(),
            Self::Ip(ip) => ip.to_string(),
        }
    }
    pub fn category(&self) -> &'static str {
        match self {
            Self::Domain(_) => "domains",
            Self::Ip(_) => "ip_addresses",
            Self::Hash(_) => "files",
        }
    }
    pub fn excluded(&self, settings: &Settings) -> bool {
        match self {
            Self::Domain(value) => {
                !settings.domains
                    || value.eq_ignore_ascii_case("www.virustotal.com")
                    || value.eq_ignore_ascii_case("virustotal.com")
                    || settings.excluded_domains.iter().any(|part| {
                        value
                            .to_ascii_lowercase()
                            .contains(&part.to_ascii_lowercase())
                    })
            }
            Self::Ip(ip) => {
                !settings.ips
                    || private(*ip)
                    || settings
                        .excluded_ips
                        .iter()
                        .any(|part| ip.to_string().contains(part))
            }
            Self::Hash(_) => !settings.hashes,
        }
    }
    pub fn details_url(&self) -> String {
        let category = match self {
            Self::Domain(_) => "domain",
            Self::Ip(_) => "ip-address",
            Self::Hash(_) => "file",
        };
        let mut url =
            reqwest::Url::parse(&format!("https://www.virustotal.com/gui/{category}/")).unwrap();
        url.path_segments_mut()
            .unwrap()
            .pop_if_empty()
            .push(&self.value());
        url.to_string()
    }
}
fn private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.is_broadcast()
        }
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(|ip| private(IpAddr::V4(ip)))
            .unwrap_or_else(|| {
                ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_multicast()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
            }),
    }
}
#[derive(Clone, Debug)]
pub struct Query {
    pub capture: Capture,
    pub subjects: Vec<Subject>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Queued,
    Checking,
    Complete,
    Disabled,
    Excluded,
    MissingKey,
    InvalidKey,
    NotFound,
    QuotaExceeded,
    RateLimited,
    TimedOut,
    Failed,
    Cancelled,
    Stale,
    Busy,
    NoData,
}
impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Queued => "Waiting to check",
            Self::Checking => "Checking VirusTotal…",
            Self::Complete => "Report received",
            Self::Disabled => "Reputation checks are off",
            Self::Excluded => "Excluded from reputation checks",
            Self::MissingKey => "Choose an API key file",
            Self::InvalidKey => "The API key file or key is invalid",
            Self::NotFound => "No report was found",
            Self::QuotaExceeded => "This session's request budget is exhausted",
            Self::RateLimited => "VirusTotal request limit reached; try again later",
            Self::TimedOut => "Reputation check timed out",
            Self::Failed => "Could not read reputation information",
            Self::Cancelled => "Check cancelled",
            Self::Stale => "Settings changed; check again",
            Self::Busy => "Reputation checks are busy",
            Self::NoData => "No analysis data is available",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Unknown,
    NotFlagged,
    Warning,
}
#[derive(Clone, Debug, Default)]
pub struct Counts {
    pub malicious: u64,
    pub suspicious: u64,
    pub harmless: u64,
    pub undetected: u64,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub subject: Subject,
    pub status: Status,
    pub verdict: Verdict,
    pub counts: Counts,
    pub malicious_votes: Option<u64>,
    pub harmless_votes: Option<u64>,
    pub reputation: Option<i64>,
    pub score: Option<i64>,
    pub suggested_default: Option<crate::config::Decision>,
    pub details_url: String,
    /// Plain JSON excerpt, never interpreted HTML; key/credentials are absent.
    pub details: String,
    pub details_truncated: bool,
}
impl Report {
    pub(super) fn status(subject: Subject, status: Status) -> Self {
        let details_url = subject.details_url();
        Self {
            subject,
            status,
            verdict: Verdict::Unknown,
            counts: Counts::default(),
            malicious_votes: None,
            harmless_votes: None,
            reputation: None,
            score: None,
            suggested_default: None,
            details_url,
            details: String::new(),
            details_truncated: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ResultSet {
    pub capture: Capture,
    pub settings_revision: Option<String>,
    pub status: Status,
    pub reports: Vec<Report>,
}
pub fn path_for(database: &Path) -> PathBuf {
    let mut path = database.as_os_str().to_os_string();
    path.push(".reputation.json");
    PathBuf::from(path)
}

pub(super) fn parse(subject: Subject, bytes: &[u8], settings: &Settings) -> Report {
    fn number(value: &serde_json::Value, key: &str) -> Option<u64> {
        value.get(key)?.as_u64()
    }
    let mut report = Report::status(subject, Status::NoData);
    if bytes.len() > MAX_BODY {
        report.status = Status::Failed;
        return report;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        report.status = Status::Failed;
        return report;
    };
    let Some(attributes) = value.get("data").and_then(|data| data.get("attributes")) else {
        return report;
    };
    let Some(stats) = attributes.get("last_analysis_stats") else {
        return report;
    };
    let Some(malicious) = number(stats, "malicious") else {
        return report;
    };
    report.counts = Counts {
        malicious,
        suspicious: number(stats, "suspicious").unwrap_or(0),
        harmless: number(stats, "harmless").unwrap_or(0),
        undetected: number(stats, "undetected").unwrap_or(0),
    };
    report.malicious_votes = attributes
        .get("total_votes")
        .and_then(|v| number(v, "malicious"));
    report.harmless_votes = attributes
        .get("total_votes")
        .and_then(|v| number(v, "harmless"));
    report.reputation = attributes
        .get("reputation")
        .and_then(serde_json::Value::as_i64);
    let mut score = i128::from(malicious);
    if settings.use_suspicious {
        score += i128::from(report.counts.suspicious);
    }
    if settings.use_community_score {
        let (Some(bad), Some(good)) = (report.malicious_votes, report.harmless_votes) else {
            return report;
        };
        score += i128::from(bad) - i128::from(good);
    }
    if settings.use_reputation {
        let Some(reputation) = report.reputation else {
            return report;
        };
        score -= i128::from(reputation);
    }
    report.score = Some(score.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64);
    if score >= i128::from(settings.minimum_threshold) {
        report.verdict = Verdict::Warning;
        if settings.enabled && settings.warning_action == WarningAction::RecommendReject {
            report.suggested_default = Some(crate::config::Decision::Reject);
        }
    } else if [
        report.counts.malicious,
        report.counts.suspicious,
        report.counts.harmless,
        report.counts.undetected,
    ]
    .iter()
    .any(|n| *n > 0)
    {
        report.verdict = Verdict::NotFlagged;
    }
    report.status = if report.verdict == Verdict::Unknown {
        Status::NoData
    } else {
        Status::Complete
    };
    // Summaries are typed. Raw diagnostics are a bounded plain-text excerpt.
    let mut end = bytes.len().min(MAX_REPORT);
    while bytes
        .get(end)
        .is_some_and(|byte| byte & 0b1100_0000 == 0b1000_0000)
    {
        end -= 1;
    }
    report.details = String::from_utf8_lossy(&bytes[..end]).into_owned();
    report.details_truncated = end < bytes.len();
    report
}

#[cfg(test)]
mod tests;
