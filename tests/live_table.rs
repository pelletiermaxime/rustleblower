use anyhow::Result;
use rustleblower::live::{
    model,
    presentation::{self, Field, Settings, Sort},
    table::{self, Capture, Export, Format, Selection},
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
fn sockets(count: usize) -> Arc<model::Sockets> {
    let rows=(0..count).map(|i|serde_json::json!({"Socket":{"ID":{"Source":"127.0.0.1","Destination":format!("10.0.{}.{}",i/250,i%250+1),"SourcePort":i%65535,"DestinationPort":(count-i)%65535},"UID":i%100,"Family":2,"State":1},"PID":i+1,"Iface":"owned0","Proto":6})).collect::<Vec<_>>();
    Arc::new(model::sockets(serde_json::json!({"Table":rows}).to_string().into()).unwrap())
}
#[test]
fn global_numeric_sort_and_all_thirteen_columns_use_typed_snapshot_values() -> Result<()> {
    let data = sockets(650);
    let order = table::matching(
        &data,
        "",
        Sort {
            field: Field::DestinationPort,
            ascending: true,
        },
    );
    assert_eq!(order.len(), 650);
    let ports = order
        .iter()
        .map(|i| data.rows[*i].values.destination_port)
        .collect::<Vec<_>>();
    assert_eq!(ports, (1..=650).collect::<Vec<_>>());
    let descending = table::matching(
        &data,
        "",
        Sort {
            field: Field::Pid,
            ascending: false,
        },
    );
    assert_eq!(data.rows[descending[0]].pid, Some(650));
    for field in presentation::FIELDS {
        let sorted = table::matching(
            &data,
            "",
            Sort {
                field,
                ascending: true,
            },
        );
        assert_eq!(sorted.len(), 650);
        assert!(!field.key().is_empty());
        let _ = field.text(&data.rows[sorted[0]]);
    }
    assert_eq!(
        table::matching(&data, "owned0", Settings::default().sort).len(),
        650
    );
    assert!(
        table::matching(&data, "unavailable-owned-search", Settings::default().sort).is_empty()
    );
    Ok(())
}
#[test]
fn captured_selection_crosses_pages_ranges_and_refuses_new_revision_atomically() -> Result<()> {
    let data = sockets(650);
    let capture = Capture {
        session: 7,
        revision: 3,
    };
    let order = table::matching(&data, "", Settings::default().sort);
    let mut selection = Selection::default();
    selection.reset(capture);
    selection.toggle(capture, &order, 199, false)?;
    selection.toggle(capture, &order, 402, true)?;
    assert_eq!(selection.len(), 204);
    assert!(selection.contains(order[400]));
    let before = selection.len();
    assert!(
        selection
            .toggle(
                Capture {
                    revision: 4,
                    ..capture
                },
                &order,
                0,
                false
            )
            .is_err()
    );
    assert_eq!(selection.len(), before);
    selection.all(capture, &order)?;
    assert_eq!(selection.len(), 650);
    let export = Export::capture(
        capture,
        data.clone(),
        &order,
        &[Field::Pid],
        Some(&selection),
    )?;
    assert_eq!(export.order.len(), 650);
    let reverse = order.iter().copied().rev().collect::<Vec<_>>();
    assert_eq!(
        Export::capture(capture, data, &reverse, &[Field::Pid], Some(&selection))?.order[0],
        order[649]
    );
    selection.reset(Capture {
        revision: 4,
        ..capture
    });
    assert!(selection.is_empty());
    Ok(())
}
#[test]
fn frozen_selected_full_csv_json_export_preserves_destination_and_protected_files() -> Result<()> {
    let data = sockets(650);
    let capture = Capture {
        session: 7,
        revision: 3,
    };
    let order = table::matching(
        &data,
        "",
        Sort {
            field: Field::DestinationPort,
            ascending: true,
        },
    );
    let mut selection = Selection::default();
    selection.reset(capture);
    selection.toggle(capture, &order, 199, false)?;
    selection.toggle(capture, &order, 201, true)?;
    let export = Export::capture(
        capture,
        data.clone(),
        &order,
        &[Field::Pid, Field::DestinationPort],
        Some(&selection),
    )?;
    let stop = AtomicBool::new(false);
    let csv = export.copy(Format::Csv, &stop)?;
    assert_eq!(csv.lines().count(), 4);
    assert!(csv.starts_with("\"PID\",\"Destination port\""));
    let json: serde_json::Value = serde_json::from_str(&export.copy(Format::Json, &stop)?)?;
    assert_eq!(json.as_array().unwrap().len(), 3);
    let changed = sockets(2);
    assert_eq!(changed.rows.len(), 2);
    assert_eq!(export.order.len(), 3);
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("history.sqlite3");
    std::fs::write(&database, b"owned database fixture")?;
    let target = directory.path().join("sockets.csv");
    export.save(&database, &target, false, Format::Csv, &stop)?;
    assert_eq!(std::fs::read_to_string(&target)?, csv);
    assert!(
        export
            .save(&database, &target, false, Format::Csv, &stop)
            .is_err()
    );
    let cancelled = AtomicBool::new(true);
    assert!(
        export
            .save(&database, &target, true, Format::Csv, &cancelled)
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&target)?, csv);
    assert!(
        export
            .save(
                &database,
                &presentation::path_for(&database),
                false,
                Format::Csv,
                &stop
            )
            .is_err()
    );
    let oversized = vec![0; model::MAX_ROWS + 1];
    assert!(Export::capture(capture, data.clone(), &oversized, &[Field::Pid], None).is_err());
    let full = Export::capture(capture, data, &order, &[Field::Pid], None)?;
    assert_eq!(full.copy(Format::Csv, &stop)?.lines().count(), 651);
    Ok(())
}
#[tokio::test]
async fn view_preferences_restore_stopped_settings_and_preserve_stale_invalid_sources() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let path = presentation::path_for(&directory.path().join("history.sqlite3"));
    assert!(path.ends_with("history.sqlite3.live-view.json"));
    let worker = presentation::Worker::start(path.clone());
    let handle = worker.handle();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !handle.snapshot().ready {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let settings = Settings {
        protocol: 17,
        family: 10,
        state: 7,
        search: "owned".into(),
        columns: vec![presentation::Column {
            field: Field::Pid,
            width: 160,
        }],
        ..Default::default()
    };
    let saved = handle
        .request(presentation::Command::Save {
            expected_revision: None,
            settings: settings.clone(),
        })
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(saved.settings, settings);
    assert!(
        handle
            .request(presentation::Command::Save {
                expected_revision: None,
                settings: Settings::default()
            })
            .await?
            .is_err()
    );
    worker.join();
    let worker = presentation::Worker::start(path.clone());
    let handle = worker.handle();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !handle.snapshot().ready {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert_eq!(handle.snapshot().settings, settings);
    std::fs::write(&path, b"external invalid preferences")?;
    let snapshot = handle
        .request(presentation::Command::Reload)
        .await?
        .map_err(anyhow::Error::msg)?;
    assert!(snapshot.warning.is_some());
    assert_eq!(snapshot.settings, settings);
    assert!(
        handle
            .request(presentation::Command::Save {
                expected_revision: saved.revision,
                settings: settings.clone()
            })
            .await?
            .is_err()
    );
    assert_eq!(std::fs::read(&path)?, b"external invalid preferences");
    let mut invalid = settings;
    invalid.columns.push(invalid.columns[0]);
    assert!(
        handle
            .request(presentation::Command::Save {
                expected_revision: None,
                settings: invalid
            })
            .await?
            .is_err()
    );
    worker.join();
    Ok(())
}
