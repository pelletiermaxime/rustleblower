use anyhow::{Result, anyhow};
use rustleblower::subscriptions::{
    Handle, Reply, Request, Worker,
    bulk::Scope,
    model::*,
    transfer::{self, Document, Encoding, Review},
};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
fn evidence_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(())
}
async fn request(h: &Handle, r: Request) -> Result<Reply> {
    tokio::time::timeout(
        Duration::from_secs(4),
        h.request(r).map_err(anyhow::Error::msg)?,
    )
    .await??
    .map_err(anyhow::Error::msg)
}
async fn save(h: &Handle, previous: Option<(u64, u64)>, spec: Spec) -> Result<u64> {
    match request(h, Request::Save { previous, spec }).await? {
        Reply::Saved(id) => Ok(id),
        _ => Err(anyhow!("Unexpected save reply")),
    }
}
async fn get(h: &Handle, id: u64) -> Result<Row> {
    match request(h, Request::Get(id)).await? {
        Reply::Row(row) => Ok(*row),
        _ => Err(anyhow!("Unexpected row reply")),
    }
}
async fn review(h: &Handle, path: &Path, replace: bool, defaults: bool) -> Result<Box<Review>> {
    match request(
        h,
        Request::ReviewImport {
            path: path.into(),
            replace,
            defaults,
            catalogue: false,
        },
    )
    .await?
    {
        Reply::ImportReview(review) => Ok(review),
        _ => Err(anyhow!("Unexpected review reply")),
    }
}
fn spec(name: &str) -> Spec {
    Spec {
        name: name.into(),
        url: format!("http://127.0.0.1:9/{name}"),
        ..Default::default()
    }
}
fn native(subscriptions: Vec<Spec>) -> Document {
    Document {
        version: 1,
        settings: Settings::default(),
        subscriptions,
        opensnitch: json!({}),
    }
}
fn fixture() -> Value {
    json!({"name":"ownedAction","created":"2025-01-02T03:04:05+00:00","description":"Owned configuration","type":["global","main-dialog"],"x-envelope":{"keep":42},"actions":{"other_plugin":{"enabled":true,"config":{"never_execute":"owned marker"}},"list_subscriptions":{"enabled":false,"x-action":"retained","config":{"lists_dir":"/not/adopted/by/rustleblower","interval":2,"interval_units":"hours","timeout":2,"timeout_units":"minutes","max_size":3,"max_size_units":"MB","user_agent":"Rustleblower owned migration fixture","notify":{"success":{"desktop":"Owned success"},"error":{"desktop":"Owned error"}},"future_default":{"preserved":true},"subscriptions":[{"name":"Legacy hosts","url":"http://127.0.0.1:9/hosts","filename":"legacy-hosts.txt","format":"hosts","group":"Work, ALL","groups":["Other","WORK"],"interval":"1h 30m","timeout":2,"timeout_units":"minutes","max_size":8,"max_size_units":"KB","future_source":{"kept":7}},{"name":"IP fixture","url":"http://127.0.0.1:9/ips","filename":"ip-ips.txt","format":"ips","enabled":false}]}}}})
}
#[test]
fn pinned_units_legacy_groups_optional_inheritance_metadata_and_bounded_json() -> Result<()> {
    let source = fixture();
    let d =
        transfer::decode(&source.to_string(), &Settings::default()).map_err(anyhow::Error::msg)?;
    assert_eq!(
        (
            d.settings.interval,
            d.settings.timeout,
            d.settings.max_bytes
        ),
        (7200, 120, 3 * 1024 * 1024)
    );
    assert_eq!(d.subscriptions[0].groups, ["other", "work", "work-all"]);
    assert_eq!(
        effective(&d.subscriptions[0], &d.settings).map_err(anyhow::Error::msg)?,
        (5400, 120, 8192)
    );
    assert_eq!(
        (
            d.subscriptions[1].interval,
            d.subscriptions[1].timeout,
            d.subscriptions[1].max_bytes
        ),
        (None, None, None)
    );
    assert_eq!(d.opensnitch["x-envelope"], source["x-envelope"]);
    let config = &source["actions"]["list_subscriptions"]["config"];
    assert_eq!(
        transfer::decode(&config.to_string(), &Settings::default())
            .map_err(anyhow::Error::msg)?
            .subscriptions,
        d.subscriptions
    );
    let mut reference_cases = Vec::new();
    for (name, subscriptions) in [
        (
            "baseline",
            fixture()["actions"]["list_subscriptions"]["config"]["subscriptions"].clone(),
        ),
        (
            "unit overrides",
            json!([{"name":"Units","url":"http://127.0.0.1:9/units","interval_units":"minutes","timeout_units":"s","max_size_units":"KB"}]),
        ),
        (
            "composite and aliases",
            json!([{"name":"Compact","url":"http://127.0.0.1:9/list","interval":"1w2d3h4m5s","timeout":"1m 2s","max_size":2,"max_size_units":"mb"}]),
        ),
        (
            "groups csv",
            json!([{"name":"Groups","url":"http://127.0.0.1:9/list","groups":"Work, Home, all, Work"}]),
        ),
        (
            "legacy groups",
            json!([{"name":"Legacy","url":"http://127.0.0.1:9/list","group":"Work, ALL"}]),
        ),
        (
            "url filename",
            json!([{"url":"http://127.0.0.1:9/encoded%2Etxt","format":"nets"}]),
        ),
        (
            "hidden filename",
            json!([{"name":"Hidden","url":"http://127.0.0.1:9/","filename":".hidden","format":"md5"}]),
        ),
        (
            "duplicate filename",
            json!([{"name":"First","url":"http://127.0.0.1:9/a","filename":"list.txt"},{"name":"Second","url":"http://127.0.0.1:9/b","filename":"list.txt"}]),
        ),
        (
            "name slug",
            json!([{"name":"Long application list with spaces","url":"http://127.0.0.1:9/"},{"name":"all","url":"http://127.0.0.1:9/"}]),
        ),
    ] {
        let mut source = fixture();
        source["actions"]["list_subscriptions"]["config"]["subscriptions"] = subscriptions;
        let actual = transfer::decode(&source.to_string(), &Settings::default())
            .map_err(anyhow::Error::msg)?;
        reference_cases.push(json!({"name":name,"source":source,"actual":actual}));
    }
    if let Some(path) = std::env::var_os("RUSTLEBLOWER_INTERCHANGE_EVIDENCE") {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path)?;
        evidence_write(
            &path.join("reference-cases.json"),
            &serde_json::to_vec_pretty(&reference_cases)?,
        )?;
    }
    let encoded = serde_json::to_string(&d)?;
    assert_eq!(
        transfer::decode(&encoded, &Settings::default()).map_err(anyhow::Error::msg)?,
        d
    );
    for bad in [
        r#"{"version":1,"version":1}"#.to_owned(),
        "[".repeat(18) + &"]".repeat(18),
        json!({"array":vec![0;257]}).to_string(),
        json!({"text":"a".repeat(8193)}).to_string(),
        " ".repeat(transfer::MAX_DOCUMENT + 1),
    ] {
        assert!(transfer::guard(&bad).is_err());
    }
    for bad in [
        json!({"actions":true}),
        json!({"actions":{"list_subscriptions":{"config":true}}}),
        json!({"version":1,"settings":Settings::default(),"subscriptions":[],"opensnitch":{"actions":true}}),
    ] {
        assert!(transfer::decode(&bad.to_string(), &Settings::default()).is_err());
    }
    let mut bad = source.clone();
    bad["actions"]["list_subscriptions"]["config"]["interval_units"] = json!("centuries");
    assert!(transfer::decode(&bad.to_string(), &Settings::default()).is_err());
    let mut bad = d.clone();
    bad.subscriptions.push(d.subscriptions[0].clone());
    assert!(bad.validate().is_err());
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reviewed_migration_keeps_unknown_metadata_defaults_and_owned_paths_across_reopen()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let input = dir.path().join("source.json");
    let raw = serde_json::to_vec_pretty(&fixture())?;
    std::fs::write(&input, &raw)?;
    let plan = review(&h, &input, false, true).await?;
    assert_eq!(
        (plan.entries.len(), plan.changed(), plan.conflicts()),
        (2, 2, 0)
    );
    let empty = match request(&h, Request::Page(Query::default())).await? {
        Reply::Page(p) => p.total,
        _ => panic!(),
    };
    assert_eq!(empty, 0, "Preview does not mutate the catalogue");
    assert!(matches!(
        request(&h, Request::ApplyImport(plan.clone())).await?,
        Reply::Imported(2)
    ));
    assert_eq!(
        std::fs::read(&input)?,
        raw,
        "Never auto-save normalized config into the source file"
    );
    let first = get(&h, 1).await?;
    assert!(first.file.is_none());
    assert_eq!((first.checked, first.bytes), (0, 0));
    assert_eq!(first.spec, plan.document.subscriptions[0]);
    let Reply::Settings(_, settings) = request(&h, Request::Settings).await? else {
        panic!()
    };
    assert_eq!(settings, plan.document.settings);
    assert!(!settings.automatic);
    let exported = dir.path().join("action.json");
    assert!(matches!(
        request(
            &h,
            Request::Export {
                path: exported.clone(),
                scope: Scope::Matching(String::new()),
                encoding: Encoding::OpenSnitch
            }
        )
        .await?,
        Reply::Exported(2)
    ));
    let out: Value = serde_json::from_slice(&std::fs::read(&exported)?)?;
    assert_eq!(out["x-envelope"]["keep"], 42);
    assert_eq!(
        out["actions"]["other_plugin"],
        fixture()["actions"]["other_plugin"]
    );
    assert_eq!(
        out["actions"]["list_subscriptions"]["config"]["notify"],
        fixture()["actions"]["list_subscriptions"]["config"]["notify"]
    );
    assert_eq!(
        out["actions"]["list_subscriptions"]["config"]["lists_dir"],
        "/not/adopted/by/rustleblower"
    );
    assert!(
        out["actions"]["list_subscriptions"]["config"]["subscriptions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["future_source"]["kept"] == 7)
    );
    assert!(!Path::new("/not/adopted/by/rustleblower").exists());
    if let Some(evidence) = std::env::var_os("RUSTLEBLOWER_INTERCHANGE_EVIDENCE") {
        let p = std::path::PathBuf::from(evidence);
        std::fs::create_dir_all(&p)?;
        evidence_write(&p.join("source-action.json"), &raw)?;
        evidence_write(&p.join("exported-action.json"), &std::fs::read(&exported)?)?;
        evidence_write(
            &p.join("native-imported.json"),
            &serde_json::to_vec_pretty(&plan.document)?,
        )?;
    }
    worker.join();
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let target = dir.path().join("native.json");
    request(
        &h,
        Request::Export {
            path: target.clone(),
            scope: Scope::Selected(vec![1]),
            encoding: Encoding::Native,
        },
    )
    .await?;
    let saved = transfer::read(&target, &Settings::default()).map_err(anyhow::Error::msg)?;
    assert_eq!(saved.subscriptions, [first.spec]);
    assert_eq!(
        saved.opensnitch["actions"]["list_subscriptions"]["config"]["subscriptions"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "Selected export must not leak unselected source templates"
    );
    let bytes = std::fs::read(&target)?;
    assert!(
        request(
            &h,
            Request::Export {
                path: target.clone(),
                scope: Scope::Matching(String::new()),
                encoding: Encoding::Native
            }
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&target)?, bytes);
    worker.join();
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn import_conflicts_restore_stale_catalogue_and_late_failure_are_atomic() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let a = save(&h, None, spec("A")).await?;
    let z = save(&h, None, spec("Z")).await?;
    let mut changed = spec("Z");
    changed.groups = vec!["changed".into()];
    let input = dir.path().join("input.json");
    let mut document = native(vec![spec("New"), changed.clone()]);
    document.settings.timeout = 123;
    document.opensnitch = json!({"description":"new template"});
    std::fs::write(&input, serde_json::to_vec(&document)?)?;
    let conflicts = review(&h, &input, false, true).await?;
    assert_eq!(conflicts.conflicts(), 1);
    assert!(request(&h, Request::ApplyImport(conflicts)).await.is_err());
    let plan = review(&h, &input, true, true).await?;
    let row = get(&h, a).await?;
    let mut renamed = row.spec;
    renamed.name = "Concurrent".into();
    save(&h, Some((a, row.revision)), renamed).await?;
    assert!(
        request(&h, Request::ApplyImport(plan))
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("changed since review")
    );
    assert_eq!(get(&h, z).await?.spec.groups.len(), 0);
    let plan = review(&h, &input, true, true).await?;
    let db = rusqlite::Connection::open(dir.path().join("subscriptions.sqlite3"))?;
    db.execute_batch(&format!("CREATE TRIGGER owned_import_failure BEFORE UPDATE ON subscriptions WHEN OLD.id={z} BEGIN SELECT RAISE(ABORT,'owned late import failure'); END;"))?;
    assert!(
        request(&h, Request::ApplyImport(plan.clone()))
            .await
            .is_err()
    );
    let count: i64 = db.query_row("SELECT count(*) FROM subscriptions", [], |r| r.get(0))?;
    assert_eq!(
        count, 2,
        "New rows preceding the late failing UPDATE must roll back"
    );
    let revision: u64 = db.query_row(
        "SELECT revision FROM subscription_interchange WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(revision, plan.template_revision);
    let Reply::Settings(_, settings) = request(&h, Request::Settings).await? else {
        panic!()
    };
    assert_ne!(settings.timeout, 123);
    db.execute_batch("DROP TRIGGER owned_import_failure;")?;
    std::fs::write(&input, b"not JSON now")?;
    assert!(
        matches!(
            request(&h, Request::ApplyImport(plan)).await?,
            Reply::Imported(2)
        ),
        "Applying uses the reviewed bytes, not a changed file"
    );
    assert_eq!(get(&h, z).await?.spec, changed);
    assert_eq!(get(&h, 3).await?.spec.name, "New");
    let row = get(&h, z).await?;
    request(
        &h,
        Request::Remove {
            id: z,
            revision: row.revision,
        },
    )
    .await?;
    let path = dir.path().join("restore.json");
    std::fs::write(&path, serde_json::to_vec(&native(vec![changed.clone()]))?)?;
    assert_eq!(review(&h, &path, false, false).await?.conflicts(), 1);
    let plan = review(&h, &path, true, false).await?;
    assert!(plan.entries[0].before.as_ref().unwrap().removed);
    request(&h, Request::ApplyImport(plan)).await?;
    assert!(!get(&h, z).await?.removed);
    worker.join();
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_matching_export_caps_duplicates_and_file_guards_do_not_damage_sources() -> Result<()>
{
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    for i in 0..70 {
        save(&h, None, spec(&format!("Export {i:03}"))).await?;
    }
    save(&h, None, spec("Unrelated")).await?;
    let path = dir.path().join("full.json");
    assert!(matches!(
        request(
            &h,
            Request::Export {
                path: path.clone(),
                scope: Scope::Matching("Export".into()),
                encoding: Encoding::Native
            }
        )
        .await?,
        Reply::Exported(70)
    ));
    let document = transfer::read(&path, &Settings::default()).map_err(anyhow::Error::msg)?;
    assert_eq!(document.subscriptions.len(), 70);
    assert_eq!(document.subscriptions.last().unwrap().name, "Export 069");
    assert_eq!(
        std::fs::metadata(&path)?.permissions().mode() & 0o777,
        0o600
    );
    let link = dir.path().join("link.json");
    symlink(&path, &link)?;
    assert!(review(&h, &link, false, false).await.is_err());
    assert!(review(&h, dir.path(), false, false).await.is_err());
    let previous = std::fs::read(&path)?;
    assert!(
        request(
            &h,
            Request::Export {
                path: link.clone(),
                scope: Scope::Selected(vec![1]),
                encoding: Encoding::Native
            }
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read(&path)?, previous);
    let db = dir.path().join("subscriptions.sqlite3");
    assert!(
        request(
            &h,
            Request::Export {
                path: db.clone(),
                scope: Scope::Selected(vec![1]),
                encoding: Encoding::Native
            }
        )
        .await
        .is_err()
    );
    assert_eq!(get(&h, 1).await?.spec.name, "Export 000");
    let too_many = native((0..129).map(|i| spec(&format!("Over {i}"))).collect());
    std::fs::write(&path, serde_json::to_vec(&too_many)?)?;
    assert!(review(&h, &path, false, false).await.is_err());
    let mut duplicate = spec("Export 000");
    duplicate.url = "http://127.0.0.1:9/second-identity".into();
    duplicate.filename = "other.txt".into();
    save(&h, None, duplicate).await?;
    let mut ambiguous = spec("Export 000");
    ambiguous.url = "http://127.0.0.1:9/new-identity".into();
    ambiguous.filename = "fresh.txt".into();
    std::fs::write(&path, serde_json::to_vec(&native(vec![ambiguous]))?)?;
    assert!(
        review(&h, &path, true, false)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("Ambiguous")
    );
    let mut malicious = native(vec![]);
    malicious.opensnitch = json!({"actions":true});
    std::fs::write(&path, serde_json::to_vec(&malicious)?)?;
    assert!(review(&h, &path, false, false).await.is_err());
    assert!(
        get(&h, 1).await.is_ok(),
        "Malformed metadata must not stop the worker"
    );
    worker.join();
    Ok(())
}

async fn review_catalogue(h: &Handle, path: &Path) -> Result<Box<Review>> {
    match request(
        h,
        Request::ReviewImport {
            path: path.into(),
            replace: false,
            defaults: false,
            catalogue: true,
        },
    )
    .await?
    {
        Reply::ImportReview(plan) => Ok(plan),
        _ => Err(anyhow!("Unexpected catalogue review")),
    }
}
fn tagged(specs: Vec<Spec>, tags: &[Value]) -> Document {
    let mut doc = native(specs);
    doc.opensnitch = transfer::bind_metadata(
        &json!({}),
        &doc.subscriptions,
        &tags.iter().cloned().map(Some).collect::<Vec<_>>(),
    )
    .unwrap();
    doc
}
async fn plan_for_budget(h: &Handle, path: &Path) -> Result<Box<Review>> {
    review(h, path, false, false).await
}
async fn export_native(h: &Handle, path: &Path, scope: Scope) -> Result<Document> {
    request(
        h,
        Request::Export {
            path: path.into(),
            scope,
            encoding: Encoding::Native,
        },
    )
    .await?;
    transfer::read(path, &Settings::default()).map_err(anyhow::Error::msg)
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opaque_fields_follow_internal_ids_through_rename_duplicate_name_restore_and_reopen()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let path = dir.path().join("import.json");
    let original = spec("Original");
    let doc = tagged(
        vec![original.clone()],
        &[json!({"future":{"owner":"original"}})],
    );
    std::fs::write(&path, serde_json::to_vec(&doc)?)?;
    let plan = review(&h, &path, false, false).await?;
    request(&h, Request::ApplyImport(plan)).await?;
    let row = get(&h, 1).await?;
    let mut renamed = row.spec;
    renamed.name = "Shared name".into();
    save(&h, Some((1, row.revision)), renamed.clone()).await?;
    let mut clone = renamed.clone();
    clone.url = "http://127.0.0.1:9/clone".into();
    let clone_id = save(&h, None, clone.clone()).await?;
    let output = export_native(
        &h,
        &dir.path().join("duplicates.json"),
        Scope::Matching(String::new()),
    )
    .await?;
    assert_eq!(
        output
            .subscriptions
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Shared name", "Shared name"]
    );
    let records = transfer::metadata_records(&output).map_err(anyhow::Error::msg)?;
    assert_eq!(records[0].as_ref().unwrap()["future"]["owner"], "original");
    assert_eq!(records[1], Some(json!({})));
    let row = get(&h, clone_id).await?;
    clone.name = "Independent clone".into();
    save(&h, Some((clone_id, row.revision)), clone).await?;
    let selected = export_native(
        &h,
        &dir.path().join("selected.json"),
        Scope::Selected(vec![1]),
    )
    .await?;
    assert_eq!(
        transfer::metadata_records(&selected).map_err(anyhow::Error::msg)?[0]
            .as_ref()
            .unwrap()["future"]["owner"],
        "original"
    );
    let row = get(&h, 1).await?;
    request(
        &h,
        Request::Remove {
            id: 1,
            revision: row.revision,
        },
    )
    .await?;
    std::fs::write(&path, serde_json::to_vec(&native(vec![renamed]))?)?;
    let plan = review(&h, &path, true, false).await?;
    assert!(plan.entries[0].before.as_ref().unwrap().removed);
    request(&h, Request::ApplyImport(plan)).await?;
    worker.join();
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let selected = export_native(
        &h,
        &dir.path().join("reopened.json"),
        Scope::Selected(vec![1]),
    )
    .await?;
    assert_eq!(
        transfer::metadata_records(&selected).map_err(anyhow::Error::msg)?[0]
            .as_ref()
            .unwrap()["future"]["owner"],
        "original"
    );
    let clone = export_native(
        &h,
        &dir.path().join("clone.json"),
        Scope::Selected(vec![clone_id]),
    )
    .await?;
    assert_eq!(
        transfer::metadata_records(&clone).map_err(anyhow::Error::msg)?[0],
        Some(json!({})),
        "Renaming another row cannot steal retained fields"
    );
    worker.join();
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_names_and_filenames_roundtrip_without_binding_metadata_by_name() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let mut a = spec("Shared");
    a.url = "http://127.0.0.1:9/first".into();
    let mut b = a.clone();
    b.url = a.url.clone();
    let document = tagged(
        vec![a.clone(), b.clone()],
        &[json!({"tag":"first"}), json!({"tag":"second"})],
    );
    let input = dir.path().join("input.json");
    std::fs::write(&input, serde_json::to_vec(&document)?)?;
    let plan = review(&h, &input, false, false).await?;
    assert_eq!(plan.changed(), 2);
    request(&h, Request::ApplyImport(plan)).await?;
    let backup = dir.path().join("native.json");
    let saved = export_native(&h, &backup, Scope::Matching(String::new())).await?;
    assert_eq!(saved.subscriptions, [a, b]);
    let plan = review(&h, &backup, false, false).await?;
    assert_eq!((plan.changed(), plan.conflicts()), (0, 0));
    assert_eq!(
        plan.entries
            .iter()
            .map(|e| e.before.as_ref().unwrap().id)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    request(&h, Request::ApplyImport(plan)).await?;
    let mut oversized = (*plan_for_budget(&h, &backup).await?).clone();
    let fields = (0..128)
        .map(|i| (format!("field-{i}"), json!("x".repeat(8192))))
        .collect::<serde_json::Map<_, _>>();
    let entry = oversized.entries[0].clone();
    oversized.document.opensnitch = json!({});
    oversized.document.subscriptions = vec![entry.after.clone(); 7];
    oversized.entries = (0..7)
        .map(|_| {
            let mut e = entry.clone();
            e.metadata = Some(Value::Object(fields.clone()));
            e
        })
        .collect();
    assert!(
        h.request(Request::ApplyImport(Box::new(oversized)))
            .err()
            .unwrap()
            .contains("6 MiB"),
        "Reject a malicious aggregate review before it enters the bounded queue"
    );
    let action = dir.path().join("action.json");
    request(
        &h,
        Request::Export {
            path: action.clone(),
            scope: Scope::Matching(String::new()),
            encoding: Encoding::OpenSnitch,
        },
    )
    .await?;
    let parsed = transfer::read(&action, &Settings::default()).map_err(anyhow::Error::msg)?;
    assert_eq!(parsed.subscriptions[0].name, "Shared");
    assert_eq!(parsed.subscriptions[1].name, "Shared");
    assert_ne!(
        parsed.subscriptions[0].filename,
        parsed.subscriptions[1].filename
    );
    let records = transfer::metadata_records(&parsed).map_err(anyhow::Error::msg)?;
    assert_eq!(records[0].as_ref().unwrap()["tag"], "first");
    assert_eq!(records[1].as_ref().unwrap()["tag"], "second");
    let plan = review(&h, &action, true, false).await?;
    assert_eq!(
        plan.entries
            .iter()
            .map(|e| e.before.as_ref().unwrap().id)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    request(&h, Request::ApplyImport(plan)).await?;
    assert_eq!(get(&h, 1).await?.spec.url, "http://127.0.0.1:9/first");
    if let Some(path) = std::env::var_os("RUSTLEBLOWER_INTERCHANGE_EVIDENCE") {
        let p = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&p)?;
        evidence_write(
            &p.join("duplicate-native.json"),
            &serde_json::to_vec_pretty(&saved)?,
        )?;
        evidence_write(&p.join("duplicate-action.json"), &std::fs::read(&action)?)?;
    }
    worker.join();
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn complete_catalogue_review_late_rollback_and_restore_keep_data_and_collection_ownership()
-> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/list", listener.local_addr()?);
    let http = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut bytes = vec![0; 8192];
        let _ = stream.read(&mut bytes).await?;
        let body = "0.0.0.0 retained.example\n";
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(reply.as_bytes()).await?;
        Ok::<_, std::io::Error>(())
    });
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let mut a = spec("Retained");
    a.url = url;
    a.groups = vec!["old-group".into()];
    let input = dir.path().join("source.json");
    std::fs::write(
        &input,
        serde_json::to_vec(&tagged(vec![a.clone()], &[json!({"tag":"old"})]))?,
    )?;
    let plan = review(&h, &input, false, false).await?;
    request(&h, Request::ApplyImport(plan)).await?;
    let b = save(&h, None, spec("Absent")).await?;
    request(&h, Request::Refresh { id: 1, force: true }).await?;
    let old = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = get(&h, 1).await?;
            if row.status == "updated" {
                break Ok::<_, anyhow::Error>(row);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    http.await??;
    let file = old.file.as_ref().unwrap();
    let bytes = std::fs::read(file)?;
    let pointer = dir.path().join("collections/current");
    let original_generation = std::fs::read_link(&pointer)?;
    let mut one = spec("Shared");
    one.url = "http://127.0.0.1:9/new-one".into();
    let mut two = one.clone();
    two.url = "http://127.0.0.1:9/new-two".into();
    let mut document = tagged(vec![one, two], &[json!({"tag":1}), json!({"tag":2})]);
    document.settings.timeout = 123;
    let input = dir.path().join("replacement.json");
    std::fs::write(&input, serde_json::to_vec(&document)?)?;
    let plan = review_catalogue(&h, &input).await?;
    assert_eq!(
        (plan.entries.len(), plan.removals.len(), plan.changed()),
        (2, 2, 4)
    );
    assert!(!get(&h, 1).await?.removed);
    assert_eq!(std::fs::read(file)?, bytes);
    let row = get(&h, b).await?;
    let mut concurrent = row.spec;
    concurrent.name = "Concurrent absent".into();
    save(&h, Some((b, row.revision)), concurrent).await?;
    assert!(request(&h, Request::ApplyImport(plan)).await.is_err());
    assert!(!get(&h, 1).await?.removed);
    let plan = match request(
        &h,
        Request::ReviewImport {
            path: input.clone(),
            replace: false,
            defaults: true,
            catalogue: true,
        },
    )
    .await?
    {
        Reply::ImportReview(plan) => plan,
        _ => panic!(),
    };
    let db = rusqlite::Connection::open(dir.path().join("subscriptions.sqlite3"))?;
    db.execute_batch(&format!("CREATE TRIGGER owned_replacement_failure BEFORE UPDATE ON subscriptions WHEN OLD.id={b} AND NEW.removed=1 BEGIN SELECT RAISE(ABORT,'owned late removal'); END;"))?;
    assert!(
        request(&h, Request::ApplyImport(plan.clone()))
            .await
            .is_err()
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM subscriptions", [], |r| r
            .get::<_, u64>(0))?,
        2
    );
    assert!(!get(&h, 1).await?.removed);
    assert_eq!(std::fs::read_link(&pointer)?, original_generation);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM subscription_source_metadata",
            [],
            |r| r.get::<_, u64>(0)
        )?,
        1
    );
    let Reply::Settings(_, settings) = request(&h, Request::Settings).await? else {
        panic!()
    };
    assert_ne!(settings.timeout, 123);
    db.execute_batch("DROP TRIGGER owned_replacement_failure;")?;
    assert!(matches!(
        request(&h, Request::ApplyImport(plan)).await?,
        Reply::Imported(4)
    ));
    assert!(get(&h, 1).await?.removed && get(&h, b).await?.removed);
    assert_eq!(std::fs::read(file)?, bytes);
    assert_ne!(std::fs::read_link(&pointer)?, original_generation);
    let Reply::Collections(collections) =
        request(&h, Request::Collections(Default::default())).await?
    else {
        panic!()
    };
    assert_eq!(collections.total, 5);
    assert!(collections.rows.iter().all(|c| c.members == 0));
    let restore = dir.path().join("restore.json");
    std::fs::write(&restore, serde_json::to_vec(&native(vec![a]))?)?;
    let plan = review(&h, &restore, true, false).await?;
    assert_eq!(plan.entries[0].before.as_ref().unwrap().id, 1);
    request(&h, Request::ApplyImport(plan)).await?;
    assert_eq!(get(&h, 1).await?.file.as_ref(), Some(file));
    let restored = export_native(
        &h,
        &dir.path().join("restored.json"),
        Scope::Selected(vec![1]),
    )
    .await?;
    assert_eq!(
        transfer::metadata_records(&restored).map_err(anyhow::Error::msg)?[0]
            .as_ref()
            .unwrap()["tag"],
        "old"
    );
    worker.join();
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_name_metadata_migrates_once_and_archives_ambiguous_records_until_reimport()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let original = spec("Original");
    let mut duplicate = spec("Ambiguous");
    duplicate.url = "http://127.0.0.1:9/duplicate".into();
    let ghost = spec("Ghost");
    let a = save(&h, None, original.clone()).await?;
    save(&h, None, duplicate.clone()).await?;
    save(&h, None, duplicate.clone()).await?;
    let row = get(&h, a).await?;
    let mut renamed = row.spec;
    renamed.name = "Renamed".into();
    save(&h, Some((a, row.revision)), renamed).await?;
    worker.join();
    let legacy = transfer::action(&tagged(
        vec![original, duplicate, ghost],
        &[
            json!({"tag":"first"}),
            json!({"tag":"ambiguous"}),
            json!({"tag":"ghost"}),
        ],
    ))
    .map_err(anyhow::Error::msg)?;
    let db = rusqlite::Connection::open(dir.path().join("subscriptions.sqlite3"))?;
    db.execute_batch("DROP TABLE subscription_source_metadata;DROP TABLE subscription_metadata_archive;DROP TABLE subscription_metadata_schema;")?;
    db.execute(
        "UPDATE subscription_interchange SET template=? WHERE id=1",
        [legacy.to_string()],
    )?;
    drop(db);
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    get(&h, a).await?;
    assert_eq!(h.state().archived_metadata, 2);
    let selected = export_native(
        &h,
        &dir.path().join("migrated.json"),
        Scope::Selected(vec![a]),
    )
    .await?;
    assert_eq!(selected.subscriptions[0].name, "Renamed");
    assert_eq!(
        transfer::metadata_records(&selected).map_err(anyhow::Error::msg)?[0]
            .as_ref()
            .unwrap()["tag"],
        "first"
    );
    let source = dir.path().join("legacy-action.json");
    std::fs::write(&source, serde_json::to_vec(&legacy)?)?;
    let plan = review(&h, &source, true, false).await?;
    assert_eq!(plan.archived_remove.len(), 2);
    request(&h, Request::ApplyImport(plan)).await?;
    assert_eq!(h.state().archived_metadata, 0);
    worker.join();
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let saved = export_native(
        &h,
        &dir.path().join("reopened.json"),
        Scope::Matching(String::new()),
    )
    .await?;
    let extras = transfer::metadata_records(&saved).map_err(anyhow::Error::msg)?;
    assert_eq!(
        extras
            .iter()
            .filter(|v| v.as_ref().is_some_and(|v| v["tag"] == "ambiguous"))
            .count(),
        1
    );
    assert_eq!(h.state().archived_metadata, 0);
    worker.join();
    let db = rusqlite::Connection::open(dir.path().join("subscriptions.sqlite3"))?;
    db.execute(
        "UPDATE subscription_source_metadata SET record=? WHERE subscription=?",
        rusqlite::params![
            json!({"oversized":"x".repeat(transfer::MAX_DOCUMENT+1)}).to_string(),
            1
        ],
    )?;
    drop(db);
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    assert!(request(&h, Request::Settings).await.is_err());
    // Closing the request channel can wake us before startup publishes its
    // error on the independent state channel. Join before inspecting it.
    worker.join();
    assert!(
        h.state()
            .error
            .as_deref()
            .unwrap_or("")
            .contains("exceeds 2 MiB")
    );
    Ok(())
}
