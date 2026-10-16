use super::{COLUMNS, Store, now, row};
use crate::subscriptions::{bulk::Scope, model::*, transfer as data};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;
fn decode_json(text: &str) -> Result<Value, String> {
    data::guard(text)?;
    serde_json::from_str(text).map_err(|e| e.to_string())
}
fn all_rows(db: &Connection) -> Result<Vec<data::Existing>, String> {
    let mut statement = db
        .prepare(&format!(
            "SELECT {COLUMNS} FROM subscriptions ORDER BY id LIMIT 257"
        ))
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if rows.len() > MAX_RECORDS {
        return Err("Catalogue exceeds 256 retained records".into());
    }
    Ok(rows
        .into_iter()
        .map(|r| data::Existing {
            id: r.id,
            revision: r.revision,
            spec: r.spec,
            removed: r.removed,
        })
        .collect())
}
fn same_source(a: &Spec, b: &Spec) -> bool {
    a.url == b.url
        && a.format == b.format
        && (a.filename == b.filename
            || data::typed_filename(&a.filename, &a.format)
                == data::typed_filename(&b.filename, &b.format))
}
fn legacy_record(template: &Value, record: &Value, base: &Settings) -> Option<(Spec, Value)> {
    let mut config = template
        .pointer("/actions/list_subscriptions/config")?
        .clone();
    config["subscriptions"] = json!([record]);
    let decoded = data::decode(&serde_json::to_string(&config).ok()?, base).ok()?;
    Some((
        decoded.subscriptions[0].clone(),
        decoded
            .opensnitch
            .pointer("/actions/list_subscriptions/config/subscriptions/0")?
            .clone(),
    ))
}
fn metadata_usage(db: &Connection) -> Result<i64, String> {
    let (bytes,bound,archived):(i64,usize,usize)=db.query_row("SELECT (SELECT coalesce(sum(length(CAST(record AS BLOB))),0) FROM subscription_source_metadata)+(SELECT coalesce(sum(length(CAST(record AS BLOB))),0) FROM subscription_metadata_archive)+(SELECT length(CAST(template AS BLOB)) FROM subscription_interchange WHERE id=1),(SELECT count(*) FROM subscription_source_metadata),(SELECT count(*) FROM subscription_metadata_archive)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    if bytes > data::MAX_DOCUMENT as i64 || bound > MAX_RECORDS || archived > MAX_RECORDS {
        return Err("Retained source metadata exceeds 2 MiB or 256 records per table".into());
    }
    Ok(bytes)
}
pub(super) fn initialize(db: &Connection) -> Result<(), String> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS subscription_interchange(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL,template TEXT NOT NULL);INSERT OR IGNORE INTO subscription_interchange VALUES(1,1,'{}');CREATE TABLE IF NOT EXISTS subscription_source_metadata(subscription INTEGER PRIMARY KEY,record TEXT NOT NULL);CREATE TABLE IF NOT EXISTS subscription_metadata_archive(id INTEGER PRIMARY KEY,record TEXT NOT NULL);CREATE TABLE IF NOT EXISTS subscription_metadata_schema(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL);").map_err(|e|e.to_string())?;
    let version: Option<u64> = db
        .query_row(
            "SELECT version FROM subscription_metadata_schema WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(version) = version {
        if version != 2 {
            return Err("Unsupported subscription metadata schema".into());
        }
        metadata_usage(db)?;
        return Ok(());
    }
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let text: String = tx
        .query_row(
            "SELECT substr(template,1,2097153) FROM subscription_interchange WHERE id=1",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let template = decode_json(&text)?;
    let catalogue = all_rows(db)?;
    let settings: String = tx
        .query_row("SELECT config FROM settings WHERE id=1", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let settings: Settings = serde_json::from_str(&settings).map_err(|e| e.to_string())?;
    let normalized_all = data::decode(
        &serde_json::to_string(&template).map_err(|e| e.to_string())?,
        &settings,
    )
    .ok();
    let mut used = BTreeSet::new();
    if let Some(records) = template
        .pointer("/actions/list_subscriptions/config/subscriptions")
        .and_then(Value::as_array)
    {
        for (index, record) in records.iter().enumerate() {
            let decoded = normalized_all
                .as_ref()
                .and_then(|d| {
                    d.subscriptions
                        .get(index)
                        .zip(
                            d.opensnitch
                                .pointer("/actions/list_subscriptions/config/subscriptions")
                                .and_then(Value::as_array)?
                                .get(index),
                        )
                        .map(|(s, r)| (s.clone(), r.clone()))
                })
                .or_else(|| legacy_record(&template, record, &settings));
            let normalized = decoded.as_ref().map(|(_, r)| r).unwrap_or(record);
            let exact = decoded
                .as_ref()
                .map(|(spec, _)| {
                    catalogue
                        .iter()
                        .filter(|r| !used.contains(&r.id) && same_source(&r.spec, spec))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let by_name = catalogue
                .iter()
                .filter(|r| {
                    !used.contains(&r.id)
                        && record.get("name").and_then(Value::as_str) == Some(r.spec.name.as_str())
                })
                .collect::<Vec<_>>();
            let matched = if exact.len() == 1 {
                exact.first().copied()
            } else if exact.is_empty() && by_name.len() == 1 {
                by_name.first().copied()
            } else {
                None
            };
            if let Some(row) = matched {
                used.insert(row.id);
                let opaque = data::opaque_record(normalized)?;
                if opaque.as_object().is_some_and(|m| !m.is_empty()) {
                    tx.execute(
                        "INSERT INTO subscription_source_metadata(subscription,record) VALUES(?,?)",
                        params![
                            row.id,
                            serde_json::to_string(&opaque).map_err(|e| e.to_string())?
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                }
            } else {
                tx.execute(
                    "INSERT INTO subscription_metadata_archive(record) VALUES(?)",
                    [serde_json::to_string(normalized).map_err(|e| e.to_string())?],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }
    let globals = data::global_template(&template)?;
    tx.execute(
        "UPDATE subscription_interchange SET template=?,revision=revision+1 WHERE id=1",
        [serde_json::to_string(&globals).map_err(|e| e.to_string())?],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO subscription_metadata_schema VALUES(1,2)", [])
        .map_err(|e| e.to_string())?;
    metadata_usage(db)?;
    tx.commit().map_err(|e| e.to_string())
}
impl Store {
    fn interchange_template(&self) -> Result<(u64, Value), String> {
        let (revision,text):(u64,String)=self.db.query_row("SELECT revision,substr(template,1,2097153) FROM subscription_interchange WHERE id=1",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        Ok((revision, decode_json(&text)?))
    }
    fn source_metadata(&self, id: u64) -> Result<Value, String> {
        let text:Option<String>=self.db.query_row("SELECT substr(record,1,2097153) FROM subscription_source_metadata WHERE subscription=?",[id],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        let value = text
            .as_deref()
            .map(decode_json)
            .transpose()?
            .unwrap_or_else(|| json!({}));
        data::validate_opaque(&value)?;
        Ok(value)
    }
    fn archived_metadata(&self) -> Result<Vec<(u64, Value)>, String> {
        let mut statement=self.db.prepare("SELECT id,substr(record,1,2097153) FROM subscription_metadata_archive ORDER BY id LIMIT 257").map_err(|e|e.to_string())?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if rows.len() > MAX_RECORDS {
            return Err("Legacy metadata archive exceeds 256 records".into());
        }
        rows.into_iter()
            .map(|(id, text)| Ok((id, decode_json(&text)?)))
            .collect()
    }
    pub fn archived_count(&self) -> Result<usize, String> {
        self.db
            .query_row(
                "SELECT count(*) FROM subscription_metadata_archive",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }
    pub fn import_review(
        &self,
        document: data::Document,
        replace: bool,
        defaults: bool,
        replace_catalogue: bool,
    ) -> Result<data::Review, String> {
        document.validate()?;
        metadata_usage(&self.db)?;
        let catalogue = all_rows(&self.db)?;
        let settings_before = self.settings()?;
        let (template_revision, template) = self.interchange_template()?;
        let template_after = data::merge_template(&template, &document.opensnitch)?;
        let incoming = data::metadata_records(&document)?;
        let archive = self.archived_metadata()?;
        let mut metadata = std::collections::BTreeMap::new();
        for row in &catalogue {
            metadata.insert(row.id, self.source_metadata(row.id)?);
        }
        let mut review = data::Review {
            document,
            replace,
            defaults,
            replace_catalogue,
            removals: Vec::new(),
            archived_remove: Vec::new(),
            entries: Vec::new(),
            catalogue: catalogue.iter().map(|r| (r.id, r.revision)).collect(),
            settings_before,
            template_revision,
            template_after,
        };
        let mut used = BTreeSet::new();
        for (spec, extra) in review.document.subscriptions.iter().zip(incoming) {
            let available = catalogue
                .iter()
                .filter(|r| !used.contains(&r.id))
                .collect::<Vec<_>>();
            let mut candidates = available
                .iter()
                .copied()
                .filter(|r| same_source(&r.spec, spec))
                .collect::<Vec<_>>();
            let mut reason = "Source URL, reader and filename";
            if candidates.is_empty() {
                candidates = available
                    .iter()
                    .copied()
                    .filter(|r| r.spec.url == spec.url && r.spec.format == spec.format)
                    .collect();
                reason = "Unique URL and reader";
            }
            if candidates.is_empty() {
                candidates = available
                    .iter()
                    .copied()
                    .filter(|r| r.spec.name == spec.name)
                    .collect();
                reason = "Unique name";
            }
            if candidates.len() > 1 {
                let equal = candidates
                    .iter()
                    .copied()
                    .filter(|r| r.spec == *spec)
                    .collect::<Vec<_>>();
                if !equal.is_empty() {
                    candidates = equal;
                    reason = "Exact saved configuration";
                }
            }
            if candidates.len() > 1
                && let Some(extra) = &extra
            {
                let equal = candidates
                    .iter()
                    .copied()
                    .filter(|r| metadata.get(&r.id) == Some(extra))
                    .collect::<Vec<_>>();
                if equal.len() == 1 {
                    candidates = equal;
                    reason = "Exact retained source metadata";
                }
            }
            if candidates.len() > 1 {
                let named = candidates
                    .iter()
                    .copied()
                    .filter(|r| r.spec.name == spec.name)
                    .collect::<Vec<_>>();
                if named.len() == 1 {
                    candidates = named;
                    reason = "Source identity and exact name";
                }
            }
            if candidates.len() > 1 {
                let indistinguishable = candidates.iter().all(|r| {
                    r.spec == candidates[0].spec
                        && metadata.get(&r.id) == metadata.get(&candidates[0].id)
                });
                if indistinguishable {
                    candidates.truncate(1);
                    reason = "First unused identical identity (stable ID order)";
                } else if replace_catalogue {
                    candidates.clear();
                    reason = "New identity; ambiguous old identities retained separately";
                } else {
                    return Err(format!(
                        "Ambiguous existing identities for {}. Use complete catalogue replacement to review retained removals",
                        spec.name
                    ));
                }
            }
            let before = candidates.first().map(|r| (*r).clone());
            if let Some(row) = &before {
                used.insert(row.id);
            }
            let previous = before.as_ref().and_then(|r| metadata.get(&r.id));
            let metadata_changed = extra.as_ref().is_some_and(|v| {
                previous.map_or(!v.as_object().is_some_and(|m| m.is_empty()), |p| p != v)
            });
            let conflict = !replace
                && !replace_catalogue
                && before
                    .as_ref()
                    .is_some_and(|b| b.removed || b.spec != *spec || metadata_changed);
            let resolution = if before.is_none()
                && reason != "New identity; ambiguous old identities retained separately"
            {
                "New identity"
            } else {
                reason
            };
            review.entries.push(data::Entry {
                before,
                after: spec.clone(),
                conflict,
                metadata: extra,
                metadata_changed,
                resolution: resolution.into(),
            });
        }
        if replace_catalogue {
            review.removals = catalogue
                .iter()
                .filter(|r| !r.removed && !used.contains(&r.id))
                .cloned()
                .collect();
        }
        let additions = review
            .entries
            .iter()
            .filter(|e| !e.conflict && e.before.as_ref().is_none_or(|r| r.removed))
            .count();
        let active =
            catalogue.iter().filter(|r| !r.removed).count() + additions - review.removals.len();
        let total = catalogue.len()
            + review
                .entries
                .iter()
                .filter(|e| !e.conflict && e.before.is_none())
                .count();
        if active > MAX_SUBSCRIPTIONS || total > MAX_RECORDS {
            return Err("Import would exceed 128 active / 256 retained subscriptions".into());
        }
        let source_records = review
            .document
            .opensnitch
            .pointer("/actions/list_subscriptions/config/subscriptions")
            .and_then(Value::as_array);
        if let Some(records) = source_records {
            for record in records {
                if let Some((id, _)) = archive
                    .iter()
                    .find(|(id, old)| !review.archived_remove.contains(id) && old == record)
                {
                    review.archived_remove.push(*id);
                }
            }
        }
        let used_bytes:i64=self.db.query_row("SELECT (SELECT coalesce(sum(length(CAST(record AS BLOB))),0) FROM subscription_source_metadata)+(SELECT coalesce(sum(length(CAST(record AS BLOB))),0) FROM subscription_metadata_archive)",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        let mut projected = used_bytes
            + serde_json::to_vec(&review.template_after)
                .map_err(|e| e.to_string())?
                .len() as i64;
        for entry in &review.entries {
            if let Some(extra) = &entry.metadata {
                let old = entry.before.as_ref().and_then(|b| metadata.get(&b.id));
                projected += if extra.as_object().is_some_and(|m| m.is_empty()) {
                    0
                } else {
                    serde_json::to_vec(extra).map_err(|e| e.to_string())?.len() as i64
                } - old
                    .map(|v| {
                        if v.as_object().is_some_and(|m| m.is_empty()) {
                            0
                        } else {
                            serde_json::to_vec(v).unwrap().len() as i64
                        }
                    })
                    .unwrap_or(0);
            }
        }
        for (id, record) in &archive {
            if review.archived_remove.contains(id) {
                projected -= serde_json::to_vec(record).map_err(|e| e.to_string())?.len() as i64;
            }
        }
        if projected > data::MAX_DOCUMENT as i64 {
            return Err("Combined retained source metadata exceeds 2 MiB".into());
        }
        if defaults && review.document.settings.disk_bytes < self.used()? {
            return Err("Imported disk budget is below retained downloaded data".into());
        }
        review.validate()?;
        let settings = if defaults {
            &review.document.settings
        } else {
            &review.settings_before.1
        };
        for row in &catalogue {
            let after = review
                .entries
                .iter()
                .find(|e| !e.conflict && e.before.as_ref().is_some_and(|b| b.id == row.id))
                .map(|e| &e.after)
                .unwrap_or(&row.spec);
            effective(after, settings).map_err(|e| {
                format!(
                    "{}: {e}; review the defaults and inherited units",
                    after.name
                )
            })?;
        }
        for entry in &review.entries {
            if !entry.conflict {
                effective(&entry.after, settings).map_err(|e| {
                    format!(
                        "{}: {e}; review the defaults and inherited units",
                        entry.after.name
                    )
                })?;
            }
        }
        Ok(review)
    }
    pub fn apply_import(&self, review: &data::Review) -> Result<(Vec<u64>, usize), String> {
        review.validate()?;
        if review.conflicts() > 0 {
            return Err("Resolve the reviewed configuration conflicts before importing".into());
        }
        let tx = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        let current = self.import_review(
            review.document.clone(),
            review.replace,
            review.defaults,
            review.replace_catalogue,
        )?;
        if &current != review {
            return Err(
                "Subscription configuration changed since review; rebuild before importing".into(),
            );
        }
        let mut changed = Vec::new();
        for entry in &review.entries {
            let config = serde_json::to_string(&entry.after).map_err(|e| e.to_string())?;
            let id = if let Some(before) = &entry.before {
                if before.removed || before.spec != entry.after || entry.metadata_changed {
                    let invalidate = before.spec.url != entry.after.url
                        || before.spec.format != entry.after.format;
                    tx.execute("UPDATE subscriptions SET revision=revision+1,config=?1,enabled=?2,removed=0,next_due=0,status='changed',etag=CASE WHEN ?3 THEN '' ELSE etag END,modified=CASE WHEN ?3 THEN '' ELSE modified END,error='' WHERE id=?4 AND revision=?5",params![config,entry.after.enabled,invalidate,before.id,before.revision]).map_err(|e|e.to_string())?;
                    changed.push(before.id);
                }
                before.id
            } else {
                tx.execute(
                    "INSERT INTO subscriptions(revision,config,enabled) VALUES(1,?,?)",
                    params![config, entry.after.enabled],
                )
                .map_err(|e| e.to_string())?;
                let id = tx.last_insert_rowid() as u64;
                changed.push(id);
                id
            };
            if let Some(extra) = &entry.metadata {
                if extra.as_object().is_some_and(|m| m.is_empty()) {
                    tx.execute(
                        "DELETE FROM subscription_source_metadata WHERE subscription=?",
                        [id],
                    )
                    .map_err(|e| e.to_string())?;
                } else {
                    tx.execute("INSERT INTO subscription_source_metadata(subscription,record) VALUES(?,?) ON CONFLICT(subscription) DO UPDATE SET record=excluded.record",params![id,serde_json::to_string(extra).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
                }
            }
        }
        for row in &review.removals {
            tx.execute("UPDATE subscriptions SET removed=1,enabled=0,revision=revision+1,status='removed' WHERE id=? AND revision=?",params![row.id,row.revision]).map_err(|e|e.to_string())?;
            changed.push(row.id);
        }
        if review.defaults && review.settings_before.1 != review.document.settings {
            tx.execute(
                "UPDATE settings SET revision=revision+1,config=? WHERE id=1 AND revision=?",
                params![
                    serde_json::to_string(&review.document.settings).map_err(|e| e.to_string())?,
                    review.settings_before.0
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute("UPDATE subscription_interchange SET revision=revision+1,template=? WHERE id=1 AND revision=?",params![serde_json::to_string(&review.template_after).map_err(|e|e.to_string())?,review.template_revision]).map_err(|e|e.to_string())?;
        for id in &review.archived_remove {
            tx.execute("DELETE FROM subscription_metadata_archive WHERE id=?", [id])
                .map_err(|e| e.to_string())?;
        }
        for id in &changed {
            tx.execute(
                "INSERT INTO logs(subscription,time,message) VALUES(?,?,?)",
                params![id, now(), "Imported after configuration identity review"],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "DELETE FROM logs WHERE id NOT IN(SELECT id FROM logs ORDER BY id DESC LIMIT 1024)",
            [],
        )
        .map_err(|e| e.to_string())?;
        let schedule = super::validate_limits(&self.db, &self.settings()?.1)?;
        if review.defaults {
            super::apply_schedule(&self.db, &schedule)?;
        }
        let archived = self.archived_count()?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok((changed, archived))
    }
    pub fn export_document(&self, scope: &Scope) -> Result<data::Document, String> {
        scope.validate()?;
        metadata_usage(&self.db)?;
        let tx = self.db.unchecked_transaction().map_err(|e| e.to_string())?;
        let mut rows = Vec::new();
        match scope {
            Scope::Selected(ids) => {
                for id in ids {
                    let row = self.get(*id)?;
                    if row.removed {
                        return Err("Restore removed subscriptions before exporting".into());
                    }
                    rows.push(row);
                }
            }
            Scope::Matching(search) => {
                let mut q = Query {
                    search: search.clone(),
                    ..Default::default()
                };
                loop {
                    let page = self.page(&q)?;
                    rows.extend(page.rows);
                    if page.offset + PAGE >= page.total {
                        break;
                    }
                    q.offset += PAGE;
                }
            }
        }
        rows.sort_by(|a, b| a.spec.name.cmp(&b.spec.name).then(a.id.cmp(&b.id)));
        let specs = rows.iter().map(|r| r.spec.clone()).collect::<Vec<_>>();
        let metadata = rows
            .iter()
            .map(|r| self.source_metadata(r.id).map(Some))
            .collect::<Result<Vec<_>, _>>()?;
        let document = data::Document {
            version: 2,
            settings: self.settings()?.1,
            opensnitch: data::bind_metadata(&self.interchange_template()?.1, &specs, &metadata)?,
            subscriptions: specs,
        };
        document.validate()?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(document)
    }
}
