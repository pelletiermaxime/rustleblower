use anyhow::Result;
use rustleblower::history::{
    query::{Comparison, Field, Filter},
    settings::{Options, Presentation, Store},
};
fn sample() -> Presentation {
    Presentation {
        search: "literal_%' café".into(),
        action: "reject".into(),
        filters: vec![
            Filter {
                field: Field::User,
                comparison: Comparison::Equals,
                value: "1001".into(),
            },
            Filter {
                field: Field::Command,
                comparison: Comparison::Contains,
                value: "--session blue".into(),
            },
        ],
        sort: Some((Field::Time, true)),
        widths: vec![(Field::Time, 319.5), (Field::Process, 420.)],
    }
}
#[test]
fn legacy_migration_and_independent_writers_preserve_user_choices() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("db");
    let path = dir.path().join("db.history.json");
    for version in [1, 2] {
        let mut old = serde_json::to_value(Options::default())?;
        old["version"] = version.into();
        old.as_object_mut().unwrap().remove("presentation");
        if version == 1 {
            old.as_object_mut().unwrap().remove("refresh_seconds");
        }
        std::fs::write(&path, serde_json::to_vec(&old)?)?;
        let store = Store::open(&db)?;
        assert_eq!(store.snapshot().version, 3);
        assert_eq!(store.snapshot().presentation, Presentation::default());
        let mut columns = store.snapshot();
        columns.columns = vec![Field::Time, Field::Process];
        columns.table = true;
        store.save_presentation(sample())?;
        store.save_refresh(0)?;
        store.save_columns(columns)?;
        let current = Store::open(&db)?.snapshot();
        assert_eq!(current.presentation, sample());
        assert_eq!(current.refresh_seconds, 0);
        assert!(current.table);
        assert_eq!(current.columns, vec![Field::Time, Field::Process]);
        let query = current.presentation.query();
        assert!(query.before.is_none() && query.cursor.is_none() && query.ceiling.is_none());
        assert_eq!(query.search, current.presentation.search);
        assert_eq!(query.filters, current.presentation.filters);
    }
    Ok(())
}
#[test]
fn bounded_conditions_widths_and_unicode_survive_without_paging_state() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("db");
    let store = Store::open(&db)?;
    let mut value = sample();
    value.search = "é".repeat(128);
    value.filters = vec![
        Filter {
            field: Field::Command,
            comparison: Comparison::Contains,
            value: "ß".repeat(128)
        };
        8
    ];
    value.widths = rustleblower::history::query::FIELDS
        .iter()
        .enumerate()
        .map(|(i, f)| (*f, 40. + i as f32 * 70.25))
        .collect();
    for action in ["", "allow", "deny", "reject"] {
        for sort in [
            None,
            Some((Field::Port, true)),
            Some((Field::Process, false)),
        ] {
            value.action = action.into();
            value.sort = sort;
            store.save_presentation(value.clone())?;
            assert_eq!(Store::open(&db)?.snapshot().presentation, value);
        }
    }
    assert!(std::fs::metadata(dir.path().join("db.history.json"))?.len() <= 16384);
    let valid = value.clone();
    value.search.push('x');
    assert!(store.save_presentation(value.clone()).is_err());
    value = valid.clone();
    value.filters.push(value.filters[0].clone());
    assert!(store.save_presentation(value.clone()).is_err());
    value = valid.clone();
    value.action = "garbage".into();
    assert!(store.save_presentation(value.clone()).is_err());
    value = valid.clone();
    for width in [f32::NAN, f32::INFINITY, 39.9, 2000.1] {
        value.widths[0].1 = width;
        assert!(store.save_presentation(value.clone()).is_err());
    }
    value = valid.clone();
    value.widths.push(value.widths[0]);
    assert!(store.save_presentation(value).is_err());
    assert_eq!(store.snapshot().presentation, valid);
    Ok(())
}
#[test]
fn stale_invalid_oversize_and_symlink_settings_are_preserved() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db = dir.path().join("db");
    let path = dir.path().join("db.history.json");
    let store = Store::open(&db)?;
    store.save_presentation(sample())?;
    let original = std::fs::read(&path)?;
    for bytes in [
        b"{bad".to_vec(),
        vec![b'x'; 16385],
        original
            .iter()
            .copied()
            .chain(b" ".iter().copied())
            .collect(),
    ] {
        std::fs::write(&path, &bytes)?;
        assert!(store.save_presentation(Presentation::default()).is_err());
        assert!(store.save_refresh(2).is_err());
        assert_eq!(std::fs::read(&path)?, bytes);
        assert_eq!(store.snapshot().presentation, sample());
    }
    std::fs::remove_file(&path)?;
    let target = dir.path().join("keep");
    std::fs::write(&target, &original)?;
    std::os::unix::fs::symlink(&target, &path)?;
    assert!(Store::open(&db).is_err());
    assert!(store.save_presentation(Presentation::default()).is_err());
    assert_eq!(std::fs::read(&target)?, original);
    Ok(())
}

#[tokio::test]
async fn damaged_presentation_never_prevents_rpc_startup_or_permission_replies() -> Result<()> {
    use rustleblower::{
        backend::Backend,
        config::{Config, Decision},
        protocol::{self, ui_client::UiClient},
    };
    use std::time::Duration;
    for bad in [
        b"{bad".to_vec(),
        vec![b'x'; 16385],
        b"{\"version\":99}".to_vec(),
    ] {
        let dir = tempfile::tempdir()?;
        let database = dir.path().join("history.sqlite3");
        let path = dir.path().join("history.sqlite3.history.json");
        std::fs::write(&path, &bad)?;
        let config = Config {
            database: database.clone(),
            socket: dir.path().join("ui.sock"),
            prompt_timeout: 30,
            timeout_action: Decision::Deny,
            history_days: 7,
            history_mib: 16,
            headless: true,
        };
        let backend = Backend::start(config.clone())?;
        assert_eq!(
            backend.handle.history.settings.snapshot(),
            Options::default()
        );
        assert!(backend.handle.history.settings.load_error().is_some());
        assert!(
            backend
                .handle
                .history
                .settings
                .save_presentation(sample())
                .is_err()
        );
        let socket = config.socket.clone();
        let channel = tonic::transport::Endpoint::try_from("http://localhost")?
            .connect_with_connector(tower::service_fn(move |_| {
                let socket = socket.clone();
                async move {
                    tokio::net::UnixStream::connect(socket)
                        .await
                        .map(hyper_util::rt::TokioIo::new)
                }
            }))
            .await?;
        let mut client = UiClient::new(channel);
        client
            .subscribe(protocol::ClientConfig {
                name: "damaged-history-view-fixture".into(),
                ..Default::default()
            })
            .await?;
        tokio::time::timeout(
            Duration::from_millis(500),
            client.ping(protocol::PingRequest {
                id: 7,
                ..Default::default()
            }),
        )
        .await??;
        let asking = tokio::spawn(async move {
            client
                .ask_rule(protocol::Connection {
                    process_path: "/owned/damaged-history-view".into(),
                    ..Default::default()
                })
                .await
        });
        tokio::time::timeout(Duration::from_millis(500), async {
            while backend.handle.prompts.snapshot().is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await?;
        assert!(
            backend
                .handle
                .decide(backend.handle.prompts.snapshot()[0].id, Decision::Deny)
        );
        let rule = tokio::time::timeout(Duration::from_millis(500), asking)
            .await???
            .into_inner();
        assert_eq!(rule.action, "deny");
        assert_eq!(rule.duration, "once");
        backend.join();
        assert!(!config.socket.exists());
        assert_eq!(std::fs::read(&path)?, bad);
    }
    Ok(())
}
