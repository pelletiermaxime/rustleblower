use super::*;
use serde_json::{Map, Value};
fn object(value: Value) -> Result<Map<String, Value>> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Expected configuration object"))
}
fn text(map: &mut Map<String, Value>, key: &str) -> Result<String> {
    map.remove(key)
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| anyhow::anyhow!("Missing configuration text field"))
}
pub fn import_source(json: &str) -> Result<Settings> {
    ensure!(json.len() <= MAX_CONFIG, "Import exceeds 64 KiB");
    let mut outer = object(serde_json::from_str(json)?)?;
    let mut actions = object(
        outer
            .remove("actions")
            .ok_or_else(|| anyhow::anyhow!("Missing actions"))?,
    )?;
    let mut downloader = object(
        actions
            .remove("downloader")
            .ok_or_else(|| anyhow::anyhow!("Missing downloader"))?,
    )?;
    downloader.remove("enabled");
    let config = downloader
        .remove("config")
        .and_then(|v| v.as_array().cloned())
        .ok_or_else(|| anyhow::anyhow!("Missing downloader groups"))?;
    ensure!(config.len() <= 16, "Too many groups");
    let mut groups = Vec::new();
    for value in config {
        let mut group = object(value)?;
        let name = text(&mut group, "name")?;
        let interval = match group.remove("interval") {
            Some(Value::String(s)) => s,
            Some(Value::Number(n)) => n.to_string(),
            _ => anyhow::bail!("Missing interval"),
        };
        let units = serde_json::from_value(
            group
                .remove("units")
                .ok_or_else(|| anyhow::anyhow!("Missing interval units"))?,
        )?;
        let values = group
            .remove("urls")
            .and_then(|v| v.as_array().cloned())
            .ok_or_else(|| anyhow::anyhow!("Missing URL array"))?;
        ensure!(values.len() <= MAX_RESOURCES, "Too many URLs");
        let mut urls = Vec::new();
        for (index, value) in values.into_iter().enumerate() {
            let mut item = object(value)?;
            let name = item
                .remove("name")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("Resource {}", index + 1));
            let enabled = item
                .remove("enabled")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| anyhow::anyhow!("Missing enabled flag"))?;
            let remote = text(&mut item, "remote")?;
            let localfile = PathBuf::from(text(&mut item, "localfile")?);
            urls.push(Resource {
                name,
                enabled,
                remote,
                localfile,
                extra: item.into_iter().collect(),
            });
        }
        let notify = group.remove("notify").unwrap_or(Value::Null);
        groups.push(Group {
            name,
            interval,
            units,
            urls,
            notify,
            extra: group.into_iter().collect(),
        });
    }
    let settings = Settings {
        enabled: false,
        groups,
        source: SourceMetadata {
            outer: outer.into_iter().collect(),
            actions: actions.into_iter().collect(),
            downloader: downloader.into_iter().collect(),
        },
    };
    settings.validate()?;
    Ok(settings)
}
pub fn export_source(settings: &Settings) -> Result<String> {
    settings.validate()?;
    let mut outer: Map<_, _> = settings.source.outer.clone().into_iter().collect();
    let mut actions: Map<_, _> = settings.source.actions.clone().into_iter().collect();
    let mut downloader: Map<_, _> = settings.source.downloader.clone().into_iter().collect();
    let groups = settings
        .groups
        .iter()
        .map(|group| {
            let mut item: Map<_, _> = group.extra.clone().into_iter().collect();
            item.insert("name".into(), Value::String(group.name.clone()));
            item.insert("interval".into(), Value::String(group.interval.clone()));
            item.insert("units".into(), Value::String(group.units.label().into()));
            item.insert("notify".into(), group.notify.clone());
            item.insert(
                "urls".into(),
                Value::Array(
                    group
                        .urls
                        .iter()
                        .map(|resource| {
                            let mut value: Map<_, _> = resource.extra.clone().into_iter().collect();
                            value.insert("name".into(), Value::String(resource.name.clone()));
                            value.insert("enabled".into(), Value::Bool(resource.enabled));
                            value.insert("remote".into(), Value::String(resource.remote.clone()));
                            value.insert(
                                "localfile".into(),
                                Value::String(resource.localfile.to_string_lossy().into()),
                            );
                            Value::Object(value)
                        })
                        .collect(),
                ),
            );
            Value::Object(item)
        })
        .collect();
    downloader.insert("enabled".into(), Value::Bool(settings.enabled));
    downloader.insert("config".into(), Value::Array(groups));
    actions.insert("downloader".into(), Value::Object(downloader));
    outer.insert("actions".into(), Value::Object(actions));
    let json = serde_json::to_string(&Value::Object(outer))?;
    ensure!(json.len() <= MAX_CONFIG, "Export exceeds 64 KiB");
    Ok(json)
}
