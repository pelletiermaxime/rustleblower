use anyhow::{Result, anyhow};
use rustleblower::subscriptions::{
    self, Handle, Reply, Request, Worker,
    limits::{Override, Unit, Units},
    model::*,
    transfer,
};
use serde_json::{Value, json};
use std::time::Duration;
#[allow(dead_code)]
mod http {
    include!("support/probe_http.rs");
}
fn source() -> Value {
    let entries=[json!({"name":"all-inherit"}),json!({"name":"unit-only","interval_units":"minutes","timeout_units":"seconds","max_size_units":"KB"}),json!({"name":"count-only","interval":3,"timeout":1,"max_size":2}),json!({"name":"explicit","interval":1,"interval_units":"hours","timeout":3,"timeout_units":"seconds","max_size":4,"max_size_units":"KB"}),json!({"name":"compact","interval":"1h 30m","timeout":"1m"})].into_iter().map(|mut value|{let name=value["name"].as_str().unwrap().to_owned();value["url"]=json!(format!("http://127.0.0.1:9/{name}"));value["filename"]=json!(format!("{name}-domains.txt"));value["format"]=json!("domains");value}).collect::<Vec<_>>();
    json!({"actions":{"list_subscriptions":{"enabled":false,"config":{"interval":2,"interval_units":"hours","timeout":2,"timeout_units":"minutes","max_size":3,"max_size_units":"MB","subscriptions":entries}}}})
}
async fn req(h: &Handle, r: Request) -> Result<Reply> {
    tokio::time::timeout(
        Duration::from_secs(4),
        h.request(r).map_err(anyhow::Error::msg)?,
    )
    .await??
    .map_err(anyhow::Error::msg)
}
async fn settings(h: &Handle) -> Result<(u64, Settings)> {
    match req(h, Request::Settings).await? {
        Reply::Settings(r, s) => Ok((r, s)),
        _ => Err(anyhow!("Expected settings")),
    }
}
async fn get(h: &Handle, id: u64) -> Result<Row> {
    match req(h, Request::Get(id)).await? {
        Reply::Row(row) => Ok(*row),
        _ => Err(anyhow!("Expected row")),
    }
}
async fn save(h: &Handle, spec: Spec) -> Result<u64> {
    match req(
        h,
        Request::Save {
            previous: None,
            spec,
        },
    )
    .await?
    {
        Reply::Saved(id) => Ok(id),
        _ => Err(anyhow!("Expected saved row")),
    }
}
#[test]
fn independent_counts_units_and_legacy_absolute_roundtrip() -> Result<()> {
    let mut d = transfer::decode(&source().to_string(), &Settings::default())
        .map_err(anyhow::Error::msg)?;
    assert_eq!(d.version, 2);
    assert_eq!(
        d.settings.units,
        Some(Units {
            interval: Unit::Hours,
            timeout: Unit::Minutes,
            size: Unit::Mib
        })
    );
    let expected = [
        (7200, 120, 3145728),
        (120, 2, 3072),
        (10800, 60, 2097152),
        (3600, 3, 4096),
        (5400, 60, 3145728),
    ];
    for (spec, expected) in d.subscriptions.iter().zip(expected) {
        assert_eq!(
            effective(spec, &d.settings).map_err(anyhow::Error::msg)?,
            expected
        );
    }
    let mut cases = Vec::new();
    for step in 0..3 {
        if step == 1 {
            d.settings.interval = 10800;
            d.settings.timeout = 240;
            d.settings.max_bytes = 5 * 1024 * 1024;
        }
        if step == 2 {
            d.settings.interval = 240;
            d.settings.timeout = 10;
            d.settings.max_bytes = 10240;
            d.settings.units = Some(Units {
                interval: Unit::Minutes,
                timeout: Unit::Seconds,
                size: Unit::Kib,
            });
        }
        d.validate().map_err(anyhow::Error::msg)?;
        let action = transfer::action(&d).map_err(anyhow::Error::msg)?;
        let reread = transfer::decode(&action.to_string(), &Settings::default())
            .map_err(anyhow::Error::msg)?;
        for (a, b) in d.subscriptions.iter().zip(&reread.subscriptions) {
            assert_eq!(
                effective(a, &d.settings).map_err(anyhow::Error::msg)?,
                effective(b, &reread.settings).map_err(anyhow::Error::msg)?
            );
        }
        assert!(
            action
                .pointer("/actions/list_subscriptions/config/subscriptions/1/interval")
                .unwrap()
                .is_null()
        );
        assert_eq!(
            action
                .pointer("/actions/list_subscriptions/config/subscriptions/1/interval_units")
                .unwrap(),
            "minutes"
        );
        let values = d
            .subscriptions
            .iter()
            .map(|s| {
                let (i, t, m) = effective(s, &d.settings).unwrap();
                json!({"name":s.name,"interval":i,"timeout":t,"max_bytes":m})
            })
            .collect::<Vec<_>>();
        cases.push(json!({"action":action,"values":values}));
    }
    assert_eq!(
        effective(&d.subscriptions[1], &d.settings).map_err(anyhow::Error::msg)?,
        (240, 10, 10240)
    );
    assert_eq!(
        effective(&d.subscriptions[2], &d.settings).map_err(anyhow::Error::msg)?,
        (180, 1, 2048)
    );
    let legacy = json!({"version":1,"settings":{"automatic":false,"interval":86400,"timeout":60,"max_bytes":20971520,"disk_bytes":536870912,"user_agent":"Owned legacy"},"subscriptions":[{"name":"legacy","url":"http://127.0.0.1:9/legacy","interval":60,"timeout":2,"max_bytes":1024}]});
    let legacy =
        transfer::decode(&legacy.to_string(), &Settings::default()).map_err(anyhow::Error::msg)?;
    assert!(legacy.settings.units.is_none());
    assert_eq!(
        legacy.subscriptions[0].interval,
        Some(Override::Absolute(60))
    );
    assert_eq!(
        effective(&legacy.subscriptions[0], &d.settings).map_err(anyhow::Error::msg)?,
        (60, 2, 1024)
    );
    for bad in [
        Override::parts(Some(0), Some(Unit::Hours)),
        Override::parts(Some(u64::MAX), Some(Unit::Hours)),
        Override::parts(None, Some(Unit::Bytes)),
    ]
    .into_iter()
    .flatten()
    {
        assert!(bad.validate(false).is_err());
    }
    if let Some(path) = std::env::var_os("RUSTLEBLOWER_INHERITANCE_EVIDENCE") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path)?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join("rust-inheritance-cases.json"))?;
        f.write_all(&serde_json::to_vec_pretty(&cases)?)?;
    }
    Ok(())
}
#[tokio::test]
async fn defaults_validate_all_dependents_reschedule_and_rollback_atomically() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let (r, mut initial) = settings(&h).await?;
    initial.interval = 7200;
    initial.timeout = 120;
    initial.units = Some(Units {
        interval: Unit::Hours,
        timeout: Unit::Minutes,
        size: Unit::Mib,
    });
    initial.max_bytes = 3 * 1024 * 1024;
    req(
        &h,
        Request::SaveSettings {
            revision: r,
            settings: initial,
        },
    )
    .await?;
    let d = transfer::decode(&source().to_string(), &Settings::default())
        .map_err(anyhow::Error::msg)?;
    let id = save(&h, d.subscriptions[1].clone()).await?;
    let db = rusqlite::Connection::open(dir.path().join("subscriptions.sqlite3"))?;
    db.execute(
        "UPDATE subscriptions SET checked=100,next_due=999999 WHERE id=?",
        [id],
    )?;
    let (r, mut s) = settings(&h).await?;
    s.interval = 10800;
    req(
        &h,
        Request::SaveSettings {
            revision: r,
            settings: s,
        },
    )
    .await?;
    assert_eq!(get(&h, id).await?.next_due, 280); // inherited count 3 × minutes + checked 100
    let before = settings(&h).await?;
    let mut invalid = before.1.clone();
    invalid.units = None;
    invalid.interval = 120;
    invalid.timeout = 2;
    invalid.max_bytes = 200 * 1024;
    assert!(
        req(
            &h,
            Request::SaveSettings {
                revision: before.0,
                settings: invalid
            }
        )
        .await
        .is_err()
    );
    assert_eq!(settings(&h).await?, before);
    assert_eq!(get(&h, id).await?.next_due, 280);
    db.execute_batch("CREATE TRIGGER refuse_schedule BEFORE UPDATE OF next_due ON subscriptions BEGIN SELECT RAISE(ABORT,'owned late schedule refusal'); END;")?;
    let mut candidate = before.1.clone();
    candidate.interval = 14400;
    assert!(
        req(
            &h,
            Request::SaveSettings {
                revision: before.0,
                settings: candidate.clone()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(settings(&h).await?, before);
    assert_eq!(get(&h, id).await?.next_due, 280);
    db.execute_batch("DROP TRIGGER refuse_schedule")?;
    req(
        &h,
        Request::SaveSettings {
            revision: before.0,
            settings: candidate,
        },
    )
    .await?;
    let row = get(&h, id).await?;
    assert_eq!(row.next_due, 340);
    let mut edited = row.spec.clone();
    edited.name = "stale editor must not save".into();
    assert!(
        req(
            &h,
            Request::SaveChecked {
                previous: Some((id, row.revision)),
                spec: edited,
                settings_revision: before.0
            }
        )
        .await
        .is_err()
    );
    assert_eq!(get(&h, id).await?.spec, row.spec);
    db.execute_batch("CREATE TRIGGER refuse_log BEFORE INSERT ON logs BEGIN SELECT RAISE(ABORT,'owned log refusal'); END;")?;
    let current = settings(&h).await?;
    let mut candidate = row.spec.clone();
    candidate.name = "late failure must roll back".into();
    assert!(
        req(
            &h,
            Request::SaveChecked {
                previous: Some((id, row.revision)),
                spec: candidate,
                settings_revision: current.0
            }
        )
        .await
        .is_err()
    );
    assert_eq!(get(&h, id).await?.spec, row.spec);
    db.execute_batch("DROP TRIGGER refuse_log")?;
    worker.join();
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let s = settings(&h).await?.1;
    assert_eq!(
        effective(&get(&h, id).await?.spec, &s).map_err(anyhow::Error::msg)?,
        (240, 2, 3072)
    );
    worker.join();
    Ok(())
}
#[tokio::test]
async fn import_review_and_export_keep_independent_parts_and_refuse_incompatible_defaults()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let (r, mut s) = settings(&h).await?;
    s.units = None;
    req(
        &h,
        Request::SaveSettings {
            revision: r,
            settings: s,
        },
    )
    .await?;
    let path = dir.path().join("source.json");
    std::fs::write(&path, serde_json::to_vec(&source())?)?;
    assert!(
        req(
            &h,
            Request::ReviewImport {
                path: path.clone(),
                replace: false,
                defaults: false,
                catalogue: false
            }
        )
        .await
        .is_err()
    );
    let review = match req(
        &h,
        Request::ReviewImport {
            path: path.clone(),
            replace: false,
            defaults: true,
            catalogue: false,
        },
    )
    .await?
    {
        Reply::ImportReview(r) => r,
        _ => return Err(anyhow!("Expected review")),
    };
    req(&h, Request::ApplyImport(review)).await?;
    let output = dir.path().join("native.json");
    req(
        &h,
        Request::Export {
            path: output.clone(),
            scope: subscriptions::bulk::Scope::Matching("".into()),
            encoding: transfer::Encoding::Native,
        },
    )
    .await?;
    let document = transfer::read(&output, &Settings::default()).map_err(anyhow::Error::msg)?;
    assert_eq!(document.version, 2);
    let target = document
        .subscriptions
        .iter()
        .find(|s| s.name == "unit-only")
        .unwrap();
    assert_eq!(target.interval, Override::parts(None, Some(Unit::Minutes)));
    let action = transfer::action(&document).map_err(anyhow::Error::msg)?;
    let roundtrip =
        transfer::decode(&action.to_string(), &Settings::default()).map_err(anyhow::Error::msg)?;
    for spec in roundtrip.subscriptions {
        let original = document
            .subscriptions
            .iter()
            .find(|s| s.name == spec.name)
            .unwrap();
        assert_eq!(
            effective(&spec, &roundtrip.settings).map_err(anyhow::Error::msg)?,
            effective(original, &document.settings).map_err(anyhow::Error::msg)?
        );
    }
    worker.join();
    Ok(())
}
#[tokio::test]
async fn a_new_default_count_changes_the_actual_download_byte_limit() -> Result<()> {
    let http = http::Server::start().await;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let (r, mut s) = settings(&h).await?;
    s.max_bytes = 1024 * 1024;
    req(
        &h,
        Request::SaveSettings {
            revision: r,
            settings: s,
        },
    )
    .await?;
    let id = save(
        &h,
        Spec {
            name: "unit-only byte limit".into(),
            url: format!("{}/many", http.url),
            max_bytes: Override::parts(None, Some(Unit::Kib)),
            ..Default::default()
        },
    )
    .await?;
    req(&h, Request::Refresh { id, force: true }).await?;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if get(&h, id).await.unwrap().status == "error" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert!(get(&h, id).await?.publication.is_none());
    let (r, mut s) = settings(&h).await?;
    s.max_bytes = 32 * 1024 * 1024;
    req(
        &h,
        Request::SaveSettings {
            revision: r,
            settings: s,
        },
    )
    .await?;
    req(&h, Request::Refresh { id, force: true }).await?;
    let row = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let row = get(&h, id).await.unwrap();
            if row.publication.is_some() {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert!(row.bytes > 1024 && row.bytes <= 32768);
    assert_eq!(
        effective(&row.spec, &settings(&h).await?.1)
            .map_err(anyhow::Error::msg)?
            .2,
        32768
    );
    http.release();
    worker.join();
    Ok(())
}
