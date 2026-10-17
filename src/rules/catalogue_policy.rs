//! Local catalogue display/retention preferences. This module never changes a
//! protocol Rule or sends a daemon command; derived expiry is cached UI status.
mod store;
use crate::protocol::Rule;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub use store::{Command, Handle, Snapshot, Worker};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    #[serde(rename = "once")]
    Once,
    #[serde(rename = "30s")]
    ThirtySeconds,
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "15m")]
    FifteenMinutes,
    #[serde(rename = "30m")]
    ThirtyMinutes,
    #[serde(rename = "1h")]
    OneHour,
    #[serde(rename = "12h")]
    TwelveHours,
    #[serde(rename = "until restart")]
    UntilRestart,
}
pub const PRESETS: [Preset; 8] = [
    Preset::Once,
    Preset::ThirtySeconds,
    Preset::FiveMinutes,
    Preset::FifteenMinutes,
    Preset::ThirtyMinutes,
    Preset::OneHour,
    Preset::TwelveHours,
    Preset::UntilRestart,
];
impl Preset {
    pub fn duration(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::ThirtySeconds => "30s",
            Self::FiveMinutes => "5m",
            Self::FifteenMinutes => "15m",
            Self::ThirtyMinutes => "30m",
            Self::OneHour => "1h",
            Self::TwelveHours => "12h",
            Self::UntilRestart => "until restart",
        }
    }
    pub fn index(self) -> usize {
        PRESETS.iter().position(|value| *value == self).unwrap()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub ignore_temporary: bool,
    pub threshold: Preset,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            ignore_temporary: false,
            threshold: Preset::ThirtySeconds,
        }
    }
}
impl Settings {
    pub fn includes(self, duration: &str) -> bool {
        !self.ignore_temporary
            || !PRESETS
                .iter()
                .position(|value| value.duration() == duration)
                .is_some_and(|index| index < self.threshold.index())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Unknown,
    Restored,
    ObservedThisUiRun { received_unix: i64 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    DaemonDisabled,
    Active,
    ExpiredByAge,
    ExpiredAtUiRestart,
    Unknown,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::DaemonDisabled => "Disabled (daemon report)",
            Self::Active => "Active in UI cache",
            Self::ExpiredByAge => "Expired in UI cache",
            Self::ExpiredAtUiRestart => "Previous UI session",
            Self::Unknown => "Expiry unavailable",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Display {
    pub included: bool,
    pub daemon_enabled: bool,
    pub status: Status,
}
/// A bounded positive Go-style duration, plus source d/w extensions using real
/// 24-hour days/7-day weeks. Precision is nanoseconds, capped at Go int64 range.
pub fn duration_nanos(value: &str) -> Option<u64> {
    if value.is_empty() || value.len() > 64 {
        return None;
    }
    let mut remaining = value;
    let mut total = 0u128;
    let mut segments = 0;
    while !remaining.is_empty() {
        segments += 1;
        if segments > 16 {
            return None;
        }
        let end = remaining
            .bytes()
            .take_while(|b| b.is_ascii_digit() || *b == b'.')
            .count();
        if end == 0 {
            return None;
        }
        let number = &remaining[..end];
        remaining = &remaining[end..];
        let (unit, multiplier) = [
            ("ns", 1u128),
            ("us", 1_000),
            ("µs", 1_000),
            ("μs", 1_000),
            ("ms", 1_000_000),
            ("s", 1_000_000_000),
            ("m", 60_000_000_000),
            ("h", 3_600_000_000_000),
            ("d", 86_400_000_000_000),
            ("w", 604_800_000_000_000),
        ]
        .into_iter()
        .find(|(unit, _)| remaining.starts_with(unit))?;
        remaining = &remaining[unit.len()..];
        let mut parts = number.split('.');
        let integer = parts.next()?;
        let fraction = parts.next();
        if parts.next().is_some() || (integer.is_empty() && fraction.is_none_or(str::is_empty)) {
            return None;
        }
        let integer = if integer.is_empty() {
            0
        } else {
            integer.parse::<u128>().ok()?
        };
        let mut amount = integer.checked_mul(multiplier)?;
        if let Some(fraction) = fraction {
            if fraction.is_empty() || fraction.len() > 9 {
                return None;
            }
            let numerator = fraction.parse::<u128>().ok()?;
            amount = amount.checked_add(
                numerator.checked_mul(multiplier)? / 10u128.pow(fraction.len() as u32),
            )?;
        }
        total = total.checked_add(amount)?;
        if total > i64::MAX as u128 {
            return None;
        }
    }
    (total > 0).then_some(total as u64)
}
pub fn derive(settings: Settings, rule: &Rule, origin: Origin, now_unix: i64) -> Display {
    let status = if !rule.enabled {
        Status::DaemonDisabled
    } else if rule.duration == "always" {
        Status::Active
    } else if now_unix <= 0 || rule.created <= 0 || rule.created > now_unix {
        Status::Unknown
    } else {
        match origin {
            Origin::Unknown => Status::Unknown,
            Origin::ObservedThisUiRun { received_unix }
                if received_unix <= 0 || received_unix > now_unix =>
            {
                Status::Unknown
            }
            Origin::Restored if rule.duration == "until restart" => Status::ExpiredAtUiRestart,
            Origin::ObservedThisUiRun { .. } if rule.duration == "until restart" => Status::Active,
            Origin::ObservedThisUiRun { received_unix } if rule.duration == "once" => {
                match received_unix.checked_add(1) {
                    Some(deadline) => {
                        if now_unix >= deadline {
                            Status::ExpiredByAge
                        } else {
                            Status::Active
                        }
                    }
                    None => Status::Unknown,
                }
            }
            Origin::Restored if rule.duration == "once" => {
                if now_unix > rule.created {
                    Status::ExpiredByAge
                } else {
                    Status::Active
                }
            }
            _ => match duration_nanos(&rule.duration) {
                Some(nanos) => {
                    let seconds = nanos.div_ceil(1_000_000_000);
                    if i64::try_from(seconds)
                        .ok()
                        .and_then(|seconds| rule.created.checked_add(seconds))
                        .is_none()
                    {
                        Status::Unknown
                    } else if i128::from(now_unix) * 1_000_000_000
                        > i128::from(rule.created) * 1_000_000_000 + i128::from(nanos)
                    {
                        Status::ExpiredByAge
                    } else {
                        Status::Active
                    }
                }
                None => Status::Unknown,
            },
        }
    };
    Display {
        included: settings.includes(&rule.duration),
        daemon_enabled: rule.enabled,
        status,
    }
}
/// Random process identity, unrelated to node/session names or wall-clock time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunId(pub [u8; 16]);
pub fn process_run_id() -> Option<RunId> {
    static RUN: std::sync::OnceLock<Option<RunId>> = std::sync::OnceLock::new();
    *RUN.get_or_init(|| {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).ok()?;
        Some(RunId(bytes))
    })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    pub settings: Settings,
    pub revision: Option<[u8; 32]>,
    pub now_unix: i64,
    pub run: Option<RunId>,
}
impl Context {
    pub fn capture(snapshot: &Snapshot, run: Option<RunId>, now_unix: i64) -> Option<Self> {
        if !snapshot.ready {
            return None;
        }
        let revision = match snapshot.revision.as_deref() {
            None => None,
            Some(value) => {
                if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return None;
                }
                let mut bytes = [0; 32];
                for (index, byte) in bytes.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
                }
                Some(bytes)
            }
        };
        Some(Self {
            settings: snapshot.settings,
            revision,
            now_unix,
            run,
        })
    }
    pub fn origin(
        self,
        stored_run: Option<&[u8]>,
        received_unix: Option<i64>,
        pending: bool,
    ) -> Origin {
        if pending || self.run.is_none() {
            return Origin::Unknown;
        }
        match stored_run {
            None => Origin::Restored,
            Some(value) if value.len() != 16 => Origin::Unknown,
            Some(value) if self.run.is_some_and(|current| value == current.0) => {
                Origin::ObservedThisUiRun {
                    received_unix: received_unix.unwrap_or(0),
                }
            }
            Some(_) => Origin::Restored,
        }
    }
}
pub fn path_for(database: &Path) -> PathBuf {
    let mut value = database.as_os_str().to_os_string();
    value.push(".catalogue-policy.json");
    PathBuf::from(value)
}
#[cfg(test)]
mod tests;
