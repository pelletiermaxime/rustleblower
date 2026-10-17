//! Source-compatible subscription result messages. Delivery is best effort.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_MESSAGE: usize = 512;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub success_message: String,
    pub error_message: String,
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        for text in [&self.success_message, &self.error_message] {
            if text.len() > MAX_MESSAGE || text.chars().any(char::is_control) {
                return Err("Notification messages must be plain text of at most 512 bytes".into());
            }
        }
        Ok(())
    }
    pub fn from_template(template: &Value) -> Result<Self, String> {
        let notify = template.pointer("/actions/list_subscriptions/config/notify");
        let message = |key| {
            notify
                .and_then(|v| v.get(key))
                .and_then(|v| v.get("desktop"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let value = Self {
            enabled: notify.is_some_and(Value::is_object),
            success_message: message("success"),
            error_message: message("error"),
        };
        value.validate()?;
        Ok(value)
    }
    /// Only edit the known desktop fields; preserve other plugins and unknown keys.
    pub fn merge(&self, template: &mut Value) -> Result<(), String> {
        self.validate()?;
        let mut target = template;
        for key in ["actions", "list_subscriptions", "config"] {
            let object = target
                .as_object_mut()
                .ok_or("Source configuration is not an object")?;
            target = object.entry(key).or_insert_with(|| json!({}));
        }
        let config = target
            .as_object_mut()
            .ok_or("Subscription configuration is not an object")?;
        // The pinned plugin treats every object as enabled, even an empty one.
        // A non-object disables it; disabled messages therefore have no wire value.
        if !self.enabled {
            config.insert("notify".into(), Value::Null);
            return Ok(());
        }
        let notify = config.entry("notify").or_insert_with(|| json!({}));
        if !notify.is_object() {
            *notify = json!({});
        }
        for (key, text) in [
            ("success", &self.success_message),
            ("error", &self.error_message),
        ] {
            let entry = notify
                .as_object_mut()
                .unwrap()
                .entry(key)
                .or_insert_with(|| json!({}));
            if !entry.is_object() {
                *entry = json!({});
            }
            entry
                .as_object_mut()
                .unwrap()
                .insert("desktop".into(), json!(text));
        }
        Ok(())
    }
    pub fn message(&self, name: &str, status: &str, error: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let success = matches!(status, "updated" | "unchanged");
        if !success && status != "error" {
            return None;
        }
        let custom = if success {
            &self.success_message
        } else {
            &self.error_message
        };
        let text = if custom.is_empty() {
            if success {
                format!("{name}: {status}")
            } else {
                format!("{name} failed: {error}")
            }
        } else {
            custom.clone()
        };
        Some(crate::history::bounded(&text, MAX_MESSAGE))
    }
}
