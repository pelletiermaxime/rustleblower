use super::*;
fn profile() -> Profile {
    import_source(r##"{"actions":{"highlight":{"enabled":true,"cells":[{"cols":[0],"text":["allow"],"color":"green","alignment":["hcenter"]}],"rows":[{"cols":[1],"text":["blocked"],"color":"white","bgcolor":"darkred","margins":[2,1]}]}}}"##,View::Rules,&["action","name"]).unwrap()
}
#[test]
fn exact_cells_substring_rows_order_and_native_selection_are_preserved() {
    let mut profile = profile();
    profile.rows.push(Rule {
        cols: vec!["name".into()],
        text: vec!["blocked".into()],
        style: Style {
            color: Some("yellow".into()),
            alignment: vec![Alignment::Vcenter],
            ..Default::default()
        },
    });
    let profiles = Settings {
        profiles: vec![profile],
    }
    .compile()
    .unwrap();
    let p = &profiles[0];
    let row = [("action", "allow"), ("name", "allow-blocked-service")];
    let style = p.style("action", "allow", &row, false).unwrap();
    assert_eq!(style.color, Some([255, 255, 0, 255]));
    assert_eq!(style.bgcolor, Some([139, 0, 0, 255]));
    assert_eq!(style.margins, Some([2, 1]));
    assert!(!style.horizontal_center);
    assert!(style.vertical_center);
    let prepared = p.prepare_row(&row, false);
    let other_cell = p
        .style_prepared("name", "allow-blocked-service", prepared)
        .unwrap();
    assert_eq!(other_cell.color, Some([255, 255, 0, 255]));
    assert_eq!(other_cell.bgcolor, Some([139, 0, 0, 255]));
    assert!(!other_cell.horizontal_center && other_cell.vertical_center);
    assert!(
        p.style_prepared("action", "allow", p.prepare_row(&row, true))
            .is_none()
    );
    assert!(p.style("action", "allow", &row, true).is_none());
    assert!(
        p.style("action", "allow-extra", &[("name", "ordinary")], false)
            .is_none()
    );
}
#[test]
fn source_column_mapping_colors_and_limits_are_explicit() {
    assert_eq!(color("#80ff0000").unwrap(), [255, 0, 0, 128]);
    assert_eq!(color("#ffff00000000").unwrap(), [255, 0, 0, 255]);
    assert_eq!(color("lightgoldenrodyellow").unwrap(), [250, 250, 210, 255]);
    assert_eq!(color("transparent").unwrap(), [0, 0, 0, 0]);
    assert!(color("invalid-color").is_err());
    assert!(
        import_source(
            r#"{"enabled":true,"cells":[{"cols":[3],"text":["x"]}],"rows":[]}"#,
            View::Rules,
            &["action"]
        )
        .is_err()
    );
    let mut p = profile();
    p.rows[0].style.margins = Some([33, 0]);
    assert!(Settings { profiles: vec![p] }.compile().is_err());
    assert!(import_source(&" ".repeat(MAX_BYTES + 1), View::Rules, &[]).is_err());
}
#[test]
fn atomic_stale_invalid_and_symlink_files_preserve_last_good() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("highlighting.json");
    let settings = Settings {
        profiles: vec![profile()],
    };
    let first = store::test_save(&path, &None, &settings).unwrap();
    assert!(store::test_save(&path, &None, &settings).is_err());
    std::fs::write(&path, b"foreign-invalid").unwrap();
    assert!(store::test_save(&path, &first.revision, &settings).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign-invalid");
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(store::test_save(&link, &None, &settings).is_err());
}
#[tokio::test]
async fn worker_reload_retains_last_good_and_shutdown_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("highlighting.json");
    let worker = Worker::start(path.clone());
    let handle = worker.handle();
    let state = handle.request(Command::Reload).await.unwrap().unwrap();
    assert!(state.ready);
    let saved = handle
        .request(Command::Save {
            expected_revision: state.revision,
            settings: Settings {
                profiles: vec![profile()],
            },
        })
        .await
        .unwrap()
        .unwrap();
    let first_snapshot = handle.snapshot();
    let same_snapshot = handle.snapshot();
    assert!(std::sync::Arc::ptr_eq(&first_snapshot, &same_snapshot));
    std::fs::write(&path, b"invalid").unwrap();
    let state = handle.request(Command::Reload).await.unwrap().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&first_snapshot, &handle.snapshot()));
    assert_eq!(state.revision, saved.revision);
    assert!(state.warning.is_some());
    assert!(
        state
            .style(View::Rules, "action", "allow", &[], false)
            .is_some()
    );
    let time = std::time::Instant::now();
    worker.join();
    assert!(time.elapsed() < std::time::Duration::from_millis(250));
}

#[test]
#[ignore = "focused render-cost measurement, invoked explicitly"]
fn row_preparation_cost_measurement() {
    use std::{hint::black_box, time::Instant};
    let mut profile = Profile {
        view: View::History,
        enabled: true,
        cells: vec![],
        rows: vec![],
    };
    for index in 0..32 {
        profile.rows.push(Rule {
            cols: (0..8).map(|column| format!("col{column}")).collect(),
            text: (0..4)
                .map(|text| format!("absent-pattern-{index}-{text}"))
                .collect(),
            style: Style {
                color: Some("red".into()),
                ..Default::default()
            },
        });
    }
    let profiles = Settings {
        profiles: vec![profile],
    }
    .compile()
    .unwrap();
    let profile = &profiles[0];
    let owned = (0..24)
        .map(|index| (format!("col{index}"), "ordinary-value ".repeat(64)))
        .collect::<Vec<_>>();
    let row = owned
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let before = Instant::now();
    for _ in 0..12 {
        for (key, value) in &row {
            assert!(black_box(profile.style(key, value, black_box(&row), false)).is_none());
        }
    }
    let compatibility = before.elapsed();
    let before = Instant::now();
    for _ in 0..12 {
        let prepared = profile.prepare_row(black_box(&row), false);
        for (key, value) in &row {
            assert!(black_box(profile.style_prepared(key, value, prepared)).is_none());
        }
    }
    let prepared = before.elapsed();
    println!(
        "row-cost compatibility_us={} prepared_us={} rows=12 cells_per_row=24 rules=32 row_columns=24",
        compatibility.as_micros(),
        prepared.as_micros()
    );
}
