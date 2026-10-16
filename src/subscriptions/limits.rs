//! Independent count/unit inheritance; old numeric overrides remain absolute.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    #[serde(rename = "seconds")]
    Seconds,
    #[serde(rename = "minutes")]
    Minutes,
    #[serde(rename = "hours")]
    Hours,
    #[serde(rename = "days")]
    Days,
    #[serde(rename = "weeks")]
    Weeks,
    #[serde(rename = "bytes")]
    Bytes,
    #[serde(rename = "KB")]
    Kib,
    #[serde(rename = "MB")]
    Mib,
    #[serde(rename = "GB")]
    Gib,
}
impl Unit {
    pub const TIME: &[Self] = &[
        Self::Seconds,
        Self::Minutes,
        Self::Hours,
        Self::Days,
        Self::Weeks,
    ];
    pub const SIZE: &[Self] = &[Self::Bytes, Self::Kib, Self::Mib, Self::Gib];
    pub fn label(self) -> &'static str {
        match self {
            Self::Seconds => "seconds",
            Self::Minutes => "minutes",
            Self::Hours => "hours",
            Self::Days => "days",
            Self::Weeks => "weeks",
            Self::Bytes => "bytes",
            Self::Kib => "KB",
            Self::Mib => "MB",
            Self::Gib => "GB",
        }
    }
    pub fn factor(self) -> u64 {
        match self {
            Self::Seconds | Self::Bytes => 1,
            Self::Minutes => 60,
            Self::Hours => 3600,
            Self::Days => 86400,
            Self::Weeks => 604800,
            Self::Kib => 1024,
            Self::Mib => 1024 * 1024,
            Self::Gib => 1024 * 1024 * 1024,
        }
    }
    pub fn valid(self, size: bool) -> bool {
        if size {
            Self::SIZE.contains(&self)
        } else {
            Self::TIME.contains(&self)
        }
    }
    pub fn parse(value: &str, size: bool) -> Result<Self, String> {
        let value = value.to_ascii_lowercase();
        let unit = match value.as_str() {
            "seconds" | "s" | "composite" => Self::Seconds,
            "minutes" | "m" => Self::Minutes,
            "hours" | "h" => Self::Hours,
            "days" | "d" => Self::Days,
            "weeks" | "w" => Self::Weeks,
            "bytes" => Self::Bytes,
            "kb" => Self::Kib,
            "mb" => Self::Mib,
            "gb" => Self::Gib,
            _ => return Err(format!("Unsupported unit: {value}")),
        };
        if !unit.valid(size) {
            return Err("Unit does not match the limit type".into());
        }
        Ok(unit)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Units {
    pub interval: Unit,
    pub timeout: Unit,
    pub size: Unit,
}
impl Default for Units {
    fn default() -> Self {
        Self {
            interval: Unit::Hours,
            timeout: Unit::Seconds,
            size: Unit::Mib,
        }
    }
}
impl Units {
    pub fn legacy() -> Self {
        Self {
            interval: Unit::Seconds,
            timeout: Unit::Seconds,
            size: Unit::Bytes,
        }
    }
    pub fn validate(self, values: (u64, u64, u64)) -> Result<(), String> {
        for (unit, value, size) in [
            (self.interval, values.0, false),
            (self.timeout, values.1, false),
            (self.size, values.2, true),
        ] {
            if !unit.valid(size) || !value.is_multiple_of(unit.factor()) {
                return Err("Default values must be whole counts in their selected units".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parts {
    pub count: Option<u64>,
    pub unit: Option<Unit>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Override {
    Absolute(u64),
    Parts(Parts),
}
impl Override {
    pub fn parts(count: Option<u64>, unit: Option<Unit>) -> Option<Self> {
        if count.is_none() && unit.is_none() {
            None
        } else {
            Some(Self::Parts(Parts { count, unit }))
        }
    }
    pub fn validate(self, size: bool) -> Result<(), String> {
        if let Self::Parts(parts) = self {
            if parts.count == Some(0) || parts.unit.is_some_and(|u| !u.valid(size)) {
                return Err(
                    "Choose a positive count and a compatible unit, or inherit them".into(),
                );
            }
            if let (Some(count), Some(unit)) = (parts.count, parts.unit) {
                count
                    .checked_mul(unit.factor())
                    .ok_or("Limit conversion overflow")?;
            }
        }
        Ok(())
    }
    pub fn resolve(self, default: u64, unit: Unit, size: bool) -> Result<u64, String> {
        self.validate(size)?;
        match self {
            Self::Absolute(value) => Ok(value),
            Self::Parts(parts) => {
                if !default.is_multiple_of(unit.factor()) {
                    return Err("Invalid default count/unit pair".into());
                }
                parts
                    .count
                    .unwrap_or(default / unit.factor())
                    .checked_mul(parts.unit.unwrap_or(unit).factor())
                    .ok_or_else(|| "Limit conversion overflow".into())
            }
        }
    }
    pub fn display(self, size: bool) -> (String, Option<Unit>) {
        match self {
            Self::Absolute(value) if size && value.is_multiple_of(1024) => {
                ((value / 1024).to_string(), Some(Unit::Kib))
            }
            Self::Absolute(value) => (
                value.to_string(),
                Some(if size { Unit::Bytes } else { Unit::Seconds }),
            ),
            Self::Parts(parts) => (
                parts.count.map(|v| v.to_string()).unwrap_or_default(),
                parts.unit,
            ),
        }
    }
}
