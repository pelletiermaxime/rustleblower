use super::limits::{Override, Units};
use serde::{Deserialize, Serialize};
pub const MAX_SUBSCRIPTIONS: usize = 128;
pub const MAX_RECORDS: usize = 256;
pub const PAGE: usize = 64;
pub const MAX_FILE: u64 = 100 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Hosts,
    Domains,
    Ips,
    Nets,
    Regexp,
    Md5,
}
impl Format {
    pub fn directory(&self) -> &'static str {
        match self {
            Self::Hosts | Self::Domains => "domains",
            Self::Ips => "ips",
            Self::Nets => "nets",
            Self::Regexp => "regexp",
            Self::Md5 => "md5",
        }
    }
    pub fn operand(&self) -> &'static str {
        match self {
            Self::Hosts | Self::Domains => "lists.domains",
            Self::Ips => "lists.ips",
            Self::Nets => "lists.nets",
            Self::Regexp => "lists.domains_regexp",
            Self::Md5 => "lists.hash.md5",
        }
    }
    pub fn key(&self) -> &'static str {
        match self {
            Self::Hosts => "hosts",
            Self::Domains => "domains",
            Self::Ips => "ips",
            Self::Nets => "nets",
            Self::Regexp => "regexp",
            Self::Md5 => "md5",
        }
    }
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "hosts" => Ok(Self::Hosts),
            "domains" => Ok(Self::Domains),
            "ips" => Ok(Self::Ips),
            "nets" => Ok(Self::Nets),
            "regexp" => Ok(Self::Regexp),
            "md5" => Ok(Self::Md5),
            _ => Err("Choose a supported list format".into()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Spec {
    pub name: String,
    pub url: String,
    pub filename: String,
    pub format: Format,
    pub enabled: bool,
    pub groups: Vec<String>,
    pub interval: Option<Override>,
    pub timeout: Option<Override>,
    pub max_bytes: Option<Override>,
}
impl Default for Spec {
    fn default() -> Self {
        Self {
            name: String::new(),
            url: String::new(),
            filename: "subscription.txt".into(),
            format: Format::Hosts,
            enabled: true,
            groups: Vec::new(),
            interval: None,
            timeout: None,
            max_bytes: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<Units>,
    pub automatic: bool,
    pub interval: u32,
    pub timeout: u32,
    pub max_bytes: u64,
    pub disk_bytes: u64,
    pub user_agent: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            units: Some(Units::default()),
            automatic: false,
            interval: 86400,
            timeout: 60,
            max_bytes: 20 * 1024 * 1024,
            disk_bytes: 512 * 1024 * 1024,
            user_agent: format!("Rustleblower/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}
fn text(s: &str, max: usize) -> bool {
    s.len() <= max && !s.chars().any(char::is_control)
}
impl Spec {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || !text(&self.name, 128) {
            return Err("Name must contain 1–128 bytes without control characters".into());
        }
        if !text(&self.url, 2048) {
            return Err("URL exceeds 2048 bytes or contains control characters".into());
        }
        let url = reqwest::Url::parse(&self.url).map_err(|_| "Enter a valid HTTP or HTTPS URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err("Use an HTTP(S) URL without embedded credentials or a fragment".into());
        }
        if self.filename.is_empty()
            || self.filename.len() > 128
            || self.filename == "."
            || self.filename == ".."
            || !self
                .filename
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(
                "Filename must be a simple ASCII name (letters, digits, dot, dash, underscore)"
                    .into(),
            );
        }
        if self.groups.len() > 8 || self.groups.iter().any(|g| g.is_empty() || !text(g, 64)) {
            return Err("Use at most eight group labels of 1–64 bytes".into());
        }
        for (value, size, min, max) in [
            (self.interval, false, 60, 31 * 86400),
            (self.timeout, false, 1, 300),
            (self.max_bytes, true, 1024, MAX_FILE),
        ] {
            if let Some(value) = value {
                value.validate(size)?;
                if let Override::Absolute(value) = value
                    && !(min..=max).contains(&value)
                {
                    return Err("Absolute subscription limit is outside supported bounds".into());
                }
            }
        }
        Ok(())
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        self.units().validate((
            u64::from(self.interval),
            u64::from(self.timeout),
            self.max_bytes,
        ))?;
        effective(&Spec::default(), self)?;
        if !(20 * 1024 * 1024..=2 * 1024 * 1024 * 1024).contains(&self.disk_bytes)
            || self.disk_bytes < self.max_bytes
        {
            return Err("Disk budget must be 20–2048 MiB and cover one maximum list".into());
        }
        if self.user_agent.is_empty()
            || !text(&self.user_agent, 256)
            || reqwest::header::HeaderValue::from_str(&self.user_agent).is_err()
        {
            return Err("User agent must contain 1–256 bytes without controls".into());
        }
        Ok(())
    }
    pub fn units(&self) -> Units {
        self.units.unwrap_or_else(Units::legacy)
    }
    pub fn count_units_label(&self) -> String {
        let u = self.units();
        format!(
            "{} {} / {} {} / {} {}",
            u64::from(self.interval) / u.interval.factor(),
            u.interval.label(),
            u64::from(self.timeout) / u.timeout.factor(),
            u.timeout.label(),
            self.max_bytes / u.size.factor(),
            u.size.label()
        )
    }
}
pub fn effective(spec: &Spec, settings: &Settings) -> Result<(u32, u32, u64), String> {
    let units = settings.units();
    let resolve = |value: Option<Override>, default, unit, size| {
        value
            .map(|v| v.resolve(default, unit, size))
            .unwrap_or(Ok(default))
    };
    let interval = resolve(
        spec.interval,
        u64::from(settings.interval),
        units.interval,
        false,
    )?;
    let timeout = resolve(
        spec.timeout,
        u64::from(settings.timeout),
        units.timeout,
        false,
    )?;
    let max = resolve(spec.max_bytes, settings.max_bytes, units.size, true)?;
    if !(60..=31 * 86400).contains(&interval) {
        return Err("Refresh interval must be 60 seconds to 31 days".into());
    }
    if !(1..=300).contains(&timeout) {
        return Err("Timeout must be 1–300 seconds".into());
    }
    if !(1024..=MAX_FILE).contains(&max) {
        return Err("Maximum list size must be 1 KiB to 100 MiB".into());
    }
    Ok((interval as u32, timeout as u32, max))
}
#[derive(Clone, Debug)]
pub struct Row {
    pub id: u64,
    pub revision: u64,
    pub spec: Spec,
    pub removed: bool,
    pub checked: i64,
    pub updated: i64,
    pub next_due: i64,
    pub failures: u32,
    pub bytes: u64,
    pub status: String,
    pub error: String,
    pub etag: String,
    pub modified: String,
    pub file: Option<String>,
    pub publication: Option<Publication>,
}
#[derive(Clone, Debug)]
pub struct Publication {
    pub file: String,
    pub bytes: u64,
    pub format: Format,
    pub url: String,
}
#[derive(Clone, Copy, Default)]
pub enum Sort {
    #[default]
    Name,
    Updated,
    Size,
    Status,
}
#[derive(Clone, Default)]
pub struct Query {
    pub search: String,
    pub offset: usize,
    pub descending: bool,
    pub sort: Sort,
    pub removed: bool,
}
pub struct Page {
    pub settings: (u64, Settings),
    pub rows: Vec<Row>,
    pub total: usize,
    pub offset: usize,
}
