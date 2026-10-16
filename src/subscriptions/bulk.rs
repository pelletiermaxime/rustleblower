//! Bounded immutable before/after plans for local subscription configuration.
use super::{collections::normalize_group, model::*};
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Groups {
    #[default]
    Keep,
    Add(Vec<String>),
    Remove(Vec<String>),
    Replace(Vec<String>),
    Clear,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Patch {
    pub enabled: Option<bool>,
    pub groups: Groups,
    pub format: Option<Format>,
    /// None keeps the override; Some(None) explicitly inherits global defaults.
    pub interval: Option<Option<u32>>,
    pub timeout: Option<Option<u32>>,
    pub max_bytes: Option<Option<u64>>,
}
#[derive(Clone)]
pub enum Scope {
    Selected(Vec<u64>),
    Matching(String),
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: u64,
    pub revision: u64,
    pub before: Spec,
    pub after: Spec,
}
#[derive(Clone, Debug)]
pub struct Review {
    pub patch: Patch,
    pub entries: Vec<Entry>,
    pub scanned: usize,
}
pub const MAX_REVIEW_BYTES: usize = 2 * 1024 * 1024;
impl Scope {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Selected(ids)
                if ids.is_empty()
                    || ids.len() > MAX_SUBSCRIPTIONS
                    || ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len() =>
            {
                Err("Select 1–128 distinct active subscriptions".into())
            }
            Self::Matching(search) if search.len() > 256 => {
                Err("Bulk filter exceeds 256 bytes".into())
            }
            _ => Ok(()),
        }
    }
}
impl Patch {
    pub fn validate(&self) -> Result<(), String> {
        self.apply(&Spec {
            name: "Bulk validation".into(),
            url: "http://localhost/".into(),
            ..Default::default()
        })
        .map(|_| ())
    }
    pub fn apply(&self, before: &Spec) -> Result<Spec, String> {
        let normalize = |values: &[String]| -> Result<Vec<String>, String> {
            if values.len() > 8 || values.iter().any(|v| v.len() > 64) {
                return Err("Use at most eight group labels of at most 64 bytes".into());
            }
            let mut groups = Vec::new();
            for value in values {
                let group = normalize_group(value)
                    .ok_or("Choose a nonempty group other than reserved all")?;
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
            Ok(groups)
        };
        let mut after = before.clone();
        if let Some(enabled) = self.enabled {
            after.enabled = enabled;
        }
        if let Some(format) = &self.format {
            after.format = format.clone();
        }
        if let Some(value) = self.interval {
            after.interval = value.map(|v| super::limits::Override::Absolute(u64::from(v)));
        }
        if let Some(value) = self.timeout {
            after.timeout = value.map(|v| super::limits::Override::Absolute(u64::from(v)));
        }
        if let Some(value) = self.max_bytes {
            after.max_bytes = value.map(super::limits::Override::Absolute);
        }
        match &self.groups {
            Groups::Keep => {}
            Groups::Clear => after.groups.clear(),
            Groups::Replace(values) => after.groups = normalize(values)?,
            Groups::Add(values) | Groups::Remove(values) => {
                if values.is_empty() {
                    return Err("Enter at least one group".into());
                }
                let values = normalize(values)?;
                let mut groups = before
                    .groups
                    .iter()
                    .filter_map(|s| normalize_group(s))
                    .collect::<Vec<_>>();
                let mut seen = std::collections::BTreeSet::new();
                groups.retain(|g| seen.insert(g.clone()));
                if matches!(self.groups, Groups::Add(_)) {
                    for value in values {
                        if !groups.contains(&value) {
                            groups.push(value);
                        }
                    }
                } else {
                    groups.retain(|g| !values.contains(g));
                }
                after.groups = groups;
            }
        }
        after.validate()?;
        Ok(after)
    }
}
impl Review {
    pub fn validate(&self) -> Result<(), String> {
        self.patch.validate()?;
        if self.entries.len() > MAX_SUBSCRIPTIONS
            || self.scanned > MAX_SUBSCRIPTIONS
            || self.entries.len() > self.scanned
        {
            return Err("Bulk review exceeds 128 targets".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut bytes = 0;
        for entry in &self.entries {
            entry.before.validate()?;
            entry.after.validate()?;
            if !ids.insert(entry.id)
                || entry.before == entry.after
                || self.patch.apply(&entry.before)? != entry.after
            {
                return Err("Bulk review was modified; rebuild it before applying".into());
            }
            bytes += serde_json::to_vec(&entry.before)
                .map_err(|e| e.to_string())?
                .len()
                + serde_json::to_vec(&entry.after)
                    .map_err(|e| e.to_string())?
                    .len();
            if bytes > MAX_REVIEW_BYTES {
                return Err("Bulk review exceeds 2 MiB; select fewer subscriptions".into());
            }
        }
        Ok(())
    }
}
