//! Bounded file-configured cell/row styles. Native column keys are explicit;
//! OpenSnitch numeric columns are imported only through a caller-supplied map.
mod store;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
pub use store::{Command, Handle, Snapshot, Worker};
pub const MAX_BYTES: usize = 64 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    History,
    Rules,
    Nodes,
    Applications,
    Hosts,
    Addresses,
    Ports,
    Users,
    Protocols,
    Alerts,
    Firewall,
    LiveSockets,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Alignment {
    Center,
    Hcenter,
    Vcenter,
}
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Style {
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub bgcolor: Option<String>,
    #[serde(default)]
    pub alignment: Vec<Alignment>,
    #[serde(default)]
    pub margins: Option<[u8; 2]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub cols: Vec<String>,
    pub text: Vec<String>,
    #[serde(flatten)]
    pub style: Style,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub view: View,
    pub enabled: bool,
    pub cells: Vec<Rule>,
    pub rows: Vec<Rule>,
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub profiles: Vec<Profile>,
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Applied {
    pub color: Option<[u8; 4]>,
    pub bgcolor: Option<[u8; 4]>,
    pub horizontal_center: bool,
    pub vertical_center: bool,
    pub margins: Option<[u8; 2]>,
}
/// Row substring matching is independent of the cell being painted. Prepare it
/// once for a visible row using the same immutable Snapshot as its cells.
#[derive(Clone, Copy, Debug)]
pub struct PreparedRow {
    view: View,
    eligible: bool,
    row_style: Option<Applied>,
}
#[derive(Clone, Debug)]
struct CompiledRule {
    cols: Vec<String>,
    text: Vec<String>,
    style: Applied,
}
#[derive(Clone, Debug)]
pub struct CompiledProfile {
    pub view: View,
    pub enabled: bool,
    cells: Vec<CompiledRule>,
    rows: Vec<CompiledRule>,
}
fn key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub fn color(value: &str) -> Result<[u8; 4]> {
    ensure!(
        !value.is_empty() && value.len() <= 64,
        "Color must be 1–64 bytes"
    );
    let value = value.to_ascii_lowercase();
    // QColor's eight-digit hexadecimal syntax is alpha first, unlike CSS.
    if let Some(hex) = value.strip_prefix('#')
        && matches!(hex.len(), 8 | 9 | 12)
    {
        ensure!(
            hex.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid hexadecimal color"
        );
        if hex.len() == 8 {
            let n = u32::from_str_radix(hex, 16)?;
            return Ok([(n >> 16) as u8, (n >> 8) as u8, n as u8, (n >> 24) as u8]);
        }
        let width = hex.len() / 3;
        let maximum = (1u32 << (width * 4)) - 1;
        let mut result = [0, 0, 0, 255];
        for (index, channel) in result[..3].iter_mut().enumerate() {
            let n = u32::from_str_radix(&hex[index * width..(index + 1) * width], 16)?;
            *channel = ((n * 255 + maximum / 2) / maximum) as u8;
        }
        return Ok(result);
    }
    let parsed: svgtypes::Color = value
        .parse()
        .map_err(|_| anyhow::anyhow!("Unknown color"))?;
    Ok([parsed.red, parsed.green, parsed.blue, parsed.alpha])
}
impl Style {
    fn compile(&self) -> Result<Applied> {
        ensure!(
            self.alignment.len() <= 3 && self.margins.is_none_or(|m| m.iter().all(|v| *v <= 32)),
            "Alignment/margins exceed limits"
        );
        Ok(Applied {
            color: self
                .color
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(color)
                .transpose()?,
            bgcolor: self
                .bgcolor
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(color)
                .transpose()?,
            horizontal_center: self
                .alignment
                .iter()
                .any(|a| matches!(a, Alignment::Center | Alignment::Hcenter)),
            vertical_center: self
                .alignment
                .iter()
                .any(|a| matches!(a, Alignment::Center | Alignment::Vcenter)),
            margins: self.margins,
        })
    }
}
impl Rule {
    fn compile(&self) -> Result<CompiledRule> {
        ensure!(
            !self.cols.is_empty() && self.cols.len() <= 16 && self.cols.iter().all(|v| key(v)),
            "Use 1–16 explicit native column keys"
        );
        ensure!(
            !self.text.is_empty()
                && self.text.len() <= 16
                && self
                    .text
                    .iter()
                    .all(|v| !v.is_empty() && v.len() <= 128 && !v.chars().any(char::is_control)),
            "Use 1–16 patterns of 1–128 bytes"
        );
        Ok(CompiledRule {
            cols: self.cols.clone(),
            text: self.text.clone(),
            style: self.style.compile()?,
        })
    }
}
impl Settings {
    pub fn compile(&self) -> Result<Vec<CompiledProfile>> {
        ensure!(self.profiles.len() <= 16, "Use at most 16 view profiles");
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_BYTES,
            "Highlight settings exceed 64 KiB"
        );
        let mut views = BTreeSet::new();
        let mut profiles = Vec::new();
        for profile in &self.profiles {
            ensure!(views.insert(profile.view), "Duplicate highlight view");
            ensure!(
                profile.cells.len() + profile.rows.len() <= 64,
                "Use at most 64 rules per view"
            );
            profiles.push(CompiledProfile {
                view: profile.view,
                enabled: profile.enabled,
                cells: profile
                    .cells
                    .iter()
                    .map(Rule::compile)
                    .collect::<Result<_>>()?,
                rows: profile
                    .rows
                    .iter()
                    .map(Rule::compile)
                    .collect::<Result<_>>()?,
            });
        }
        Ok(profiles)
    }
}
impl CompiledProfile {
    /// Supply the bounded displayed row (at most64 cells,16KiB/cell). Selection
    /// colors stay native. Matching never queries storage or another worker.
    pub fn prepare_row(&self, row: &[(&str, &str)], selected: bool) -> PreparedRow {
        let eligible = self.enabled
            && !selected
            && row.len() <= 64
            && row.iter().all(|(_, value)| value.len() <= 16 * 1024);
        let mut row_style = None;
        if eligible {
            for rule in &self.rows {
                if row.iter().any(|(column, value)| {
                    rule.cols.iter().any(|c| c == column)
                        && rule.text.iter().any(|text| value.contains(text))
                }) {
                    merge(row_style.get_or_insert_with(Applied::default), rule.style);
                }
            }
        }
        PreparedRow {
            view: self.view,
            eligible,
            row_style,
        }
    }
    pub fn style_prepared(&self, column: &str, value: &str, row: PreparedRow) -> Option<Applied> {
        if !self.enabled || !row.eligible || row.view != self.view || value.len() > 16 * 1024 {
            return None;
        }
        let mut result = None;
        for rule in &self.cells {
            if rule.cols.iter().any(|c| c == column) && rule.text.iter().any(|text| text == value) {
                merge(result.get_or_insert_with(Applied::default), rule.style);
            }
        }
        if let Some(style) = row.row_style {
            merge(result.get_or_insert_with(Applied::default), style);
        }
        result
    }
    /// Compatibility path for one cell; a multi-column renderer should prepare
    /// the row once instead of repeating its substring scans for every cell.
    pub fn style(
        &self,
        column: &str,
        value: &str,
        row: &[(&str, &str)],
        selected: bool,
    ) -> Option<Applied> {
        self.style_prepared(column, value, self.prepare_row(row, selected))
    }
}

fn merge(result: &mut Applied, style: Applied) {
    if style.color.is_some() {
        result.color = style.color;
    }
    if style.bgcolor.is_some() {
        result.bgcolor = style.bgcolor;
    }
    if style.horizontal_center || style.vertical_center {
        result.horizontal_center = style.horizontal_center;
        result.vertical_center = style.vertical_center;
    }
    if style.margins.is_some() {
        result.margins = style.margins;
    }
}
/// Source arrays use Qt numeric columns. An unmapped index is refused; never
/// silently shift a rule to a different native column.
pub fn import_source(json: &str, view: View, columns: &[&str]) -> Result<Profile> {
    ensure!(
        json.len() <= MAX_BYTES && columns.len() <= 64 && columns.iter().all(|v| key(v)),
        "Source profile or column map exceeds bounds"
    );
    let document: serde_json::Value = serde_json::from_str(json)?;
    let config = document
        .get("actions")
        .and_then(|v| v.get("highlight"))
        .unwrap_or(&document);
    let parse = |kind: &str| -> Result<Vec<Rule>> {
        let Some(value) = config.get(kind) else {
            return Ok(Vec::new());
        };
        let array = value
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Expected rule array"))?;
        ensure!(array.len() <= 64, "Too many source rules");
        let mut rules = Vec::new();
        for value in array {
            let indices = value
                .get("cols")
                .and_then(|v| v.as_array())
                .ok_or_else(|| anyhow::anyhow!("Missing columns"))?;
            ensure!(indices.len() <= 16, "Too many columns");
            let cols = indices
                .iter()
                .map(|v| {
                    v.as_u64()
                        .and_then(|i| usize::try_from(i).ok())
                        .and_then(|i| columns.get(i))
                        .map(|s| s.to_string())
                        .ok_or_else(|| anyhow::anyhow!("Unmapped source column"))
                })
                .collect::<Result<_>>()?;
            let text = serde_json::from_value(
                value
                    .get("text")
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Missing text"))?,
            )?;
            let style = Style {
                color: serde_json::from_value(value.get("color").cloned().unwrap_or_default())?,
                bgcolor: serde_json::from_value(value.get("bgcolor").cloned().unwrap_or_default())?,
                alignment: serde_json::from_value(
                    value
                        .get("alignment")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!([])),
                )?,
                margins: serde_json::from_value(value.get("margins").cloned().unwrap_or_default())?,
            };
            rules.push(Rule { cols, text, style });
        }
        Ok(rules)
    };
    let profile = Profile {
        view,
        enabled: config
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        cells: parse("cells")?,
        rows: parse("rows")?,
    };
    Settings {
        profiles: vec![profile.clone()],
    }
    .compile()?;
    Ok(profile)
}
pub fn path_for(database: &Path) -> PathBuf {
    let mut value = database.as_os_str().to_os_string();
    value.push(".highlighting.json");
    PathBuf::from(value)
}
#[cfg(test)]
mod tests;
