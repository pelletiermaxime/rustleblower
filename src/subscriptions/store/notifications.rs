use super::Store;
use crate::subscriptions::{notifications::Config, transfer};
use rusqlite::params;
use serde_json::Value;

impl Store {
    pub fn notification_settings(&self) -> Result<(u64, Config), String> {
        let (revision, template) = self.notification_template()?;
        Ok((revision, Config::from_template(&template)?))
    }
    fn notification_template(&self) -> Result<(u64, Value), String> {
        let (revision, text): (u64, String) = self.db.query_row(
            "SELECT revision,substr(template,1,2097153) FROM subscription_interchange WHERE id=1",
            [], |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(|e| e.to_string())?;
        transfer::guard(&text)?;
        Ok((
            revision,
            serde_json::from_str(&text).map_err(|e| e.to_string())?,
        ))
    }
    pub fn save_notification_settings(&self, revision: u64, config: &Config) -> Result<(), String> {
        config.validate()?;
        let tx = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        let (current, mut template) = self.notification_template()?;
        if current != revision {
            return Err("Subscription source configuration changed; reload notification settings before saving".into());
        }
        config.merge(&mut template)?;
        let text = serde_json::to_string(&template).map_err(|e| e.to_string())?;
        transfer::guard(&text)?;
        let changed = tx.execute(
            "UPDATE subscription_interchange SET revision=revision+1,template=? WHERE id=1 AND revision=?",
            params![text, revision],
        ).map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("Subscription notification settings changed; reload before saving".into());
        }
        super::transfer::metadata_usage(&tx)?;
        tx.commit().map_err(|e| e.to_string())
    }
}
