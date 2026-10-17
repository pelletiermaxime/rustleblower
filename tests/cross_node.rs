use anyhow::Result;
use hyper_util::rt::TokioIo;
use prost::Message;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    protocol::{self, Action, Operator, Rule, ui_client::UiClient},
    rules::{
        self,
        distribute::{self, Destination, Status},
        transfer::{self, ExportScope},
    },
};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tonic::transport::{Channel, Endpoint};
fn options(p: &Path) -> Config {
    Config {
        transport: Default::default(),
        socket: p.join("ui.sock"),
        database: p.join("history.sqlite3"),
        prompt_timeout: 10,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    }
}
fn rule(i: usize) -> Rule {
    Rule {
        name: format!("copy-{i:04}"),
        created: 123,
        description: format!("Source rule {i}"),
        enabled: !i.is_multiple_of(3),
        action: "deny".into(),
        duration: if i.is_multiple_of(2) { "always" } else { "5m" }.into(),
        precedence: true,
        nolog: true,
        operator: Some(Operator {
            r#type: "list".into(),
            operand: "list".into(),
            list: vec![
                Operator {
                    r#type: "simple".into(),
                    operand: "process.path".into(),
                    data: format!("/owned/app-{i}"),
                    sensitive: true,
                    ..Default::default()
                },
                Operator {
                    r#type: "simple".into(),
                    operand: "user.id".into(),
                    data: "1001".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
    }
}
async fn client(path: &Path) -> Result<UiClient<Channel>> {
    let path = path.to_owned();
    Ok(UiClient::new(
        Endpoint::try_from("http://localhost")?
            .connect_with_connector(tower::service_fn(move |_| {
                let path = path.clone();
                async move {
                    tokio::net::UnixStream::connect(path)
                        .await
                        .map(TokioIo::new)
                }
            }))
            .await?,
    ))
}
async fn subscribe(
    backend: &Backend,
    client: &mut UiClient<Channel>,
    name: &str,
    rules: Vec<Rule>,
) -> Result<Destination> {
    client
        .subscribe(protocol::ClientConfig {
            name: name.into(),
            is_firewall_running: true,
            config: "{\"DefaultAction\":\"deny\",\"Future\":true}".into(),
            rules,
            ..Default::default()
        })
        .await?;
    backend.handle.history.flush().await?;
    Ok(Destination {
        session: backend.handle.node(name).unwrap().id,
        name: name.into(),
    })
}
async fn replies(
    client: &mut UiClient<Channel>,
    hold: Arc<AtomicBool>,
    accept: bool,
    observed: Arc<Mutex<Vec<protocol::Notification>>>,
) -> Result<tokio::task::JoinHandle<()>> {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let mut stream = client
        .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await?
        .into_inner();
    Ok(tokio::spawn(async move {
        while let Ok(Some(command)) = stream.message().await {
            let id = command.id;
            {
                let mut events = observed.lock().unwrap();
                assert!(events.len() < 1024);
                events.push(command);
            }
            while hold.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            if tx
                .send(protocol::NotificationReply {
                    id,
                    code: if accept {
                        protocol::NotificationReplyCode::Ok
                    } else {
                        protocol::NotificationReplyCode::Error
                    } as i32,
                    data: if accept { "" } else { "owned target rejection" }.into(),
                })
                .await
                .is_err()
            {
                break;
            }
        }
    }))
}
fn scope() -> ExportScope {
    ExportScope {
        query: rules::Query {
            node: "source".into(),
            ..Default::default()
        },
        names: None,
    }
}
async fn wait(observed: &Mutex<Vec<protocol::Notification>>, count: usize) {
    let end = tokio::time::Instant::now() + Duration::from_secs(3);
    while observed.lock().unwrap().len() < count {
        assert!(tokio::time::Instant::now() < end);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
async fn run(
    backend: &Backend,
    review: Arc<distribute::Review>,
    progress: distribute::Progress,
    stop: Arc<AtomicBool>,
) -> distribute::Progress {
    let (tx, _rx) = tokio::sync::watch::channel(progress.clone());
    distribute::run(backend.handle.clone(), review, progress, stop, tx).await
}
fn saved(path: &Path, node: &str) -> Vec<Rule> {
    transfer::load_rules(
        path,
        &ExportScope {
            query: rules::Query {
                node: node.into(),
                ..Default::default()
            },
            names: None,
        },
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copies_all_pages_with_reviewed_conflicts_stop_resume_and_independent_permission()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut source = client(&config.socket).await?;
    let originals = (0..230).map(rule).collect::<Vec<_>>();
    subscribe(&backend, &mut source, "source", originals.clone()).await?;
    let mut beta = client(&config.socket).await?;
    let mut conflict = rule(0);
    conflict.action = "allow".into();
    let b = subscribe(&backend, &mut beta, "beta", vec![conflict.clone(), rule(1)]).await?;
    let mut gamma = client(&config.socket).await?;
    let g = subscribe(&backend, &mut gamma, "gamma", vec![conflict.clone()]).await?;
    let hold = Arc::new(AtomicBool::new(true));
    let bo = Arc::new(Mutex::new(Vec::new()));
    let go = Arc::new(Mutex::new(Vec::new()));
    let bt = replies(&mut beta, hold.clone(), true, bo.clone()).await?;
    let gt = replies(
        &mut gamma,
        Arc::new(AtomicBool::new(false)),
        true,
        go.clone(),
    )
    .await?;
    let db = rusqlite::Connection::open(&config.database)?;
    let pending = rule(2);
    db.execute(
        "INSERT INTO prompt_rules(node,name,wire,process,description,duration) VALUES(?,?,?,?,?,?)",
        rusqlite::params![
            "beta",
            pending.name,
            pending.encode_to_vec(),
            "/owned/app-2",
            pending.description,
            pending.duration
        ],
    )?;
    drop(db);
    let review = Arc::new(distribute::prepare(
        &backend.handle,
        &scope(),
        vec![b.clone(), g.clone()],
        false,
    )?);
    assert_eq!(
        (
            review.targets[0].copies,
            review.targets[0].existing,
            review.targets[0].identical,
            review.targets[0].awaiting
        ),
        (227, 1, 1, 1)
    );
    assert_eq!(review.targets[1].copies, 229);
    let mut marked = scope();
    marked.names = Some(std::collections::BTreeSet::from([
        rule(201).name,
        rule(229).name,
    ]));
    let selected = distribute::prepare(&backend.handle, &marked, vec![b.clone()], false)?;
    assert_eq!(selected.rules.len(), 2);
    drop(selected);
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, _rx) = tokio::sync::watch::channel(review.progress());
    let task = tokio::spawn(distribute::run(
        backend.handle.clone(),
        review.clone(),
        review.progress(),
        stop.clone(),
        tx,
    ));
    wait(&bo, 1).await;
    assert!(transfer::find_confirmed_rule(&config.database, "beta", &rule(3).name)?.is_none());
    assert!(
        backend
            .handle
            .control
            .submit(b.session, Action::ChangeRule, rule(999))
            .err()
            .unwrap()
            .contains("reviewed rule copy")
    );
    assert!(
        backend
            .handle
            .control
            .refresh(b.session)
            .unwrap_err()
            .contains("rule copy")
    );
    let mut asking = gamma.clone();
    let ask = tokio::spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/copy-permission".into(),
                ..Default::default()
            })
            .await
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while backend.handle.prompts.snapshot().is_empty() {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        backend
            .handle
            .prompts
            .decide(backend.handle.prompts.snapshot()[0].id, Decision::Allow)
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(500), ask)
            .await???
            .into_inner()
            .action,
        "allow"
    );
    let db = rusqlite::Connection::open(&config.database)?;
    db.busy_timeout(Duration::from_secs(1))?;
    // The owned history writer also checkpoints. A competing checkpoint may
    // report busy immediately; require an actual successful one while the RPC
    // is still held, rather than confusing writer contention with a pinned read.
    let checkpoint_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            r.get::<_, i64>(0)
        })? == 0
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < checkpoint_deadline,
            "the held ACK must not pin a SQLite snapshot"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(hold.load(Ordering::Acquire));
    assert_eq!(bo.lock().unwrap().len(), 1);
    drop(db);
    stop.store(true, Ordering::Release);
    hold.store(false, Ordering::Release);
    let partial = task.await?;
    assert_eq!(partial.confirmed(), 1);
    assert_eq!(partial.targets[0].status, Status::Stopped);
    assert!(go.lock().unwrap().is_empty());
    assert!(!backend.handle.control.node_busy(b.session));
    stop.store(false, Ordering::Release);
    let done = run(&backend, review, partial, stop).await;
    assert_eq!(done.confirmed(), 456);
    assert!(done.targets.iter().all(|p| p.status == Status::Complete));
    assert_eq!(bo.lock().unwrap().len(), 227);
    assert_eq!(go.lock().unwrap().len(), 229);
    assert_eq!(saved(&config.database, "source"), originals);
    assert_eq!(
        transfer::find_confirmed_rule(&config.database, "gamma", &rule(229).name)?,
        Some(rule(229))
    );
    assert_eq!(
        transfer::find_confirmed_rule(&config.database, "beta", &rule(0).name)?,
        Some(conflict)
    );
    bt.abort();
    gt.abort();
    drop(source);
    drop(beta);
    drop(gamma);
    backend.join();
    assert!(!config.socket.exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn target_errors_are_independent_and_fresh_review_skips_already_confirmed_copies()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut source = client(&config.socket).await?;
    let originals = (0..3).map(rule).collect::<Vec<_>>();
    subscribe(&backend, &mut source, "source", originals.clone()).await?;
    let mut beta = client(&config.socket).await?;
    let b = subscribe(&backend, &mut beta, "beta", vec![]).await?;
    let mut gamma = client(&config.socket).await?;
    let g = subscribe(&backend, &mut gamma, "gamma", vec![]).await?;
    let bo = Arc::new(Mutex::new(Vec::new()));
    let go = Arc::new(Mutex::new(Vec::new()));
    let bt = replies(
        &mut beta,
        Arc::new(AtomicBool::new(false)),
        false,
        bo.clone(),
    )
    .await?;
    let gt = replies(
        &mut gamma,
        Arc::new(AtomicBool::new(false)),
        true,
        go.clone(),
    )
    .await?;
    let review = Arc::new(distribute::prepare(
        &backend.handle,
        &scope(),
        vec![b, g.clone()],
        true,
    )?);
    let stop = Arc::new(AtomicBool::new(false));
    let result = run(&backend, review.clone(), review.progress(), stop.clone()).await;
    assert!(matches!(result.targets[0].status, Status::Failed(_)));
    assert_eq!(result.targets[1].status, Status::Complete);
    assert_eq!(result.confirmed(), 3);
    let result = run(&backend, review, result, stop.clone()).await;
    assert!(matches!(result.targets[0].status, Status::Failed(_)));
    assert_eq!(bo.lock().unwrap().len(), 1);
    assert_eq!(go.lock().unwrap().len(), 3);
    bt.abort();
    let b = subscribe(&backend, &mut beta, "beta", vec![]).await?;
    let bt = replies(
        &mut beta,
        Arc::new(AtomicBool::new(false)),
        true,
        bo.clone(),
    )
    .await?;
    let review = Arc::new(distribute::prepare(
        &backend.handle,
        &scope(),
        vec![b, g],
        true,
    )?);
    assert_eq!(review.targets[1].identical, 3);
    assert_eq!(review.targets[1].copies, 0);
    let result = run(&backend, review.clone(), review.progress(), stop).await;
    assert!(result.targets.iter().all(|p| p.status == Status::Complete));
    assert_eq!(bo.lock().unwrap().len(), 4);
    assert_eq!(go.lock().unwrap().len(), 3);
    assert_eq!(saved(&config.database, "source"), originals);
    bt.abort();
    gt.abort();
    drop(source);
    drop(beta);
    drop(gamma);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_catalogues_generations_capacity_and_range_guards_refuse_unreviewed_writes()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut source = client(&config.socket).await?;
    subscribe(&backend, &mut source, "source", vec![rule(0)]).await?;
    let mut beta = client(&config.socket).await?;
    let b = subscribe(&backend, &mut beta, "beta", vec![rule(0)]).await?;
    let bo = Arc::new(Mutex::new(Vec::new()));
    let bt = replies(
        &mut beta,
        Arc::new(AtomicBool::new(false)),
        true,
        bo.clone(),
    )
    .await?;
    let review = Arc::new(distribute::prepare(
        &backend.handle,
        &scope(),
        vec![b.clone()],
        true,
    )?);
    let mut changed = rule(0);
    changed.action = "reject".into();
    backend
        .handle
        .control
        .submit(b.session, Action::ChangeRule, changed.clone())
        .map_err(anyhow::Error::msg)?
        .await?
        .map_err(anyhow::Error::msg)?;
    let result = run(
        &backend,
        review.clone(),
        review.progress(),
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert!(
        matches!(&result.targets[0].status,Status::Failed(e)if e.contains("inventory changed"))
    );
    assert_eq!(bo.lock().unwrap().len(), 1);
    assert_eq!(
        transfer::find_confirmed_rule(&config.database, "beta", &rule(0).name)?,
        Some(changed)
    );
    let review = Arc::new(distribute::prepare(
        &backend.handle,
        &scope(),
        vec![b],
        true,
    )?);
    bt.abort();
    let _new = subscribe(&backend, &mut beta, "beta", vec![]).await?;
    let bt = replies(
        &mut beta,
        Arc::new(AtomicBool::new(false)),
        true,
        bo.clone(),
    )
    .await?;
    let result = run(
        &backend,
        review.clone(),
        review.progress(),
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert!(
        matches!(&result.targets[0].status,Status::Failed(e)if e.contains("connection changed"))
    );
    assert_eq!(bo.lock().unwrap().len(), 1);
    bt.abort();
    let mut range = rule(0);
    range.operator = Some(Operator {
        r#type: "range".into(),
        operand: "dest.port".into(),
        data: "8000-8080".into(),
        ..Default::default()
    });
    subscribe(&backend, &mut source, "source", vec![range]).await?;
    let b = subscribe(&backend, &mut beta, "beta", vec![]).await?;
    let bt = replies(
        &mut beta,
        Arc::new(AtomicBool::new(false)),
        true,
        bo.clone(),
    )
    .await?;
    let review = Arc::new(distribute::prepare(
        &backend.handle,
        &scope(),
        vec![b],
        true,
    )?);
    assert!(
        review.targets[0]
            .blocked
            .as_deref()
            .unwrap()
            .contains("Port-range")
    );
    let result = run(
        &backend,
        review.clone(),
        review.progress(),
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert!(matches!(result.targets[0].status, Status::Failed(_)));
    assert_eq!(bo.lock().unwrap().len(), 1);
    bt.abort();
    subscribe(&backend, &mut source, "source", vec![rule(10001)]).await?;
    let small = (0..10000)
        .map(|i| Rule {
            name: format!("full-{i:05}"),
            action: "deny".into(),
            duration: "always".into(),
            operator: Some(Operator {
                r#type: "simple".into(),
                operand: "true".into(),
                data: "true".into(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    let b = subscribe(&backend, &mut beta, "beta", small).await?;
    let bt = replies(
        &mut beta,
        Arc::new(AtomicBool::new(false)),
        true,
        bo.clone(),
    )
    .await?;
    let review = Arc::new(distribute::prepare(
        &backend.handle,
        &scope(),
        vec![b],
        false,
    )?);
    assert!(
        review.targets[0]
            .blocked
            .as_deref()
            .unwrap()
            .contains("cache limit")
    );
    let result = run(
        &backend,
        review.clone(),
        review.progress(),
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert!(matches!(result.targets[0].status, Status::Failed(_)));
    assert_eq!(bo.lock().unwrap().len(), 1);
    let db = rusqlite::Connection::open(&config.database)?;
    db.execute(
        "UPDATE application_rules SET wire=zeroblob(65537) WHERE node='source'",
        [],
    )?;
    let destination = Destination {
        session: backend.handle.node("beta").unwrap().id,
        name: "beta".into(),
    };
    assert!(
        distribute::prepare(&backend.handle, &scope(), vec![destination], false)
            .err()
            .unwrap()
            .to_string()
            .contains("bounded capacity")
    );
    drop(db);
    bt.abort();
    drop(source);
    drop(beta);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supplied_drafts_share_one_source_and_rename_resumes_only_the_original_removal()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut alpha = client(&config.socket).await?;
    let mut beta = client(&config.socket).await?;
    let a = subscribe(&backend, &mut alpha, "alpha", vec![rule(0)]).await?;
    let b = subscribe(&backend, &mut beta, "beta", vec![rule(0)]).await?;
    let hold = Arc::new(AtomicBool::new(true));
    let ao = Arc::new(Mutex::new(Vec::new()));
    let bo = Arc::new(Mutex::new(Vec::new()));
    let at = replies(&mut alpha, hold.clone(), true, ao.clone()).await?;
    let bt = replies(
        &mut beta,
        Arc::new(AtomicBool::new(false)),
        true,
        bo.clone(),
    )
    .await?;
    let mut renamed = rule(0);
    renamed.name = "renamed-direct-draft".into();
    renamed.action = "allow".into();
    let rules = Arc::new(vec![renamed.clone()]);
    let review = Arc::new(distribute::prepare_rules(
        &backend.handle,
        "Unsaved draft",
        rules.clone(),
        vec![a.clone(), b],
        false,
        Some(rule(0).name),
    )?);
    assert!(Arc::ptr_eq(&rules, &review.rules));
    assert!(review.targets.iter().all(|t| t.rename_removals == 1));
    assert!(transfer::find_confirmed_rule(&config.database, "alpha", &renamed.name)?.is_none());
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, _rx) = tokio::sync::watch::channel(review.progress());
    let task = tokio::spawn(distribute::run(
        backend.handle.clone(),
        review.clone(),
        review.progress(),
        stop.clone(),
        tx,
    ));
    wait(&ao, 1).await;
    assert_eq!(ao.lock().unwrap()[0].r#type, Action::ChangeRule as i32);
    stop.store(true, Ordering::Release);
    hold.store(false, Ordering::Release);
    let partial = task.await?;
    assert_eq!(partial.targets[0].status, Status::Stopped);
    assert_eq!(partial.targets[0].confirmed, 1);
    assert_eq!(partial.targets[0].removed, 0);
    assert_eq!(ao.lock().unwrap().len(), 1);
    assert!(bo.lock().unwrap().is_empty());
    assert_eq!(saved(&config.database, "alpha").len(), 2);
    stop.store(false, Ordering::Release);
    let result = run(&backend, review, partial, stop).await;
    assert!(result.targets.iter().all(|p| p.status == Status::Complete));
    assert_eq!(result.targets.iter().map(|p| p.removed).sum::<usize>(), 2);
    assert_eq!(
        ao.lock()
            .unwrap()
            .iter()
            .map(|n| n.r#type)
            .collect::<Vec<_>>(),
        vec![Action::ChangeRule as i32, Action::DeleteRule as i32]
    );
    assert!(ao.lock().unwrap()[1].rules[0].operator.is_none());
    assert_eq!(
        bo.lock()
            .unwrap()
            .iter()
            .map(|n| n.r#type)
            .collect::<Vec<_>>(),
        vec![Action::ChangeRule as i32, Action::DeleteRule as i32]
    );
    assert_eq!(saved(&config.database, "alpha"), vec![renamed.clone()]);
    assert_eq!(saved(&config.database, "beta"), vec![renamed]);
    at.abort();
    bt.abort();
    drop(alpha);
    drop(beta);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partial_rename_failure_retains_both_cached_names_and_rebuild_sends_only_delete()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut alpha = client(&config.socket).await?;
    let mut beta = client(&config.socket).await?;
    let a = subscribe(&backend, &mut alpha, "alpha", vec![rule(0)]).await?;
    let b = subscribe(&backend, &mut beta, "beta", vec![rule(0)]).await?;
    let ao = Arc::new(Mutex::new(Vec::new()));
    let bo = Arc::new(Mutex::new(Vec::new()));
    let at = replies(
        &mut alpha,
        Arc::new(AtomicBool::new(false)),
        true,
        ao.clone(),
    )
    .await?;
    let bt = replies(
        &mut beta,
        Arc::new(AtomicBool::new(false)),
        true,
        bo.clone(),
    )
    .await?;
    let db = rusqlite::Connection::open(&config.database)?;
    db.execute_batch("CREATE TRIGGER refuse_original_delete BEFORE DELETE ON application_rules WHEN OLD.node='alpha' AND OLD.name='copy-0000' BEGIN SELECT RAISE(ABORT,'owned original delete failure'); END;")?;
    let mut renamed = rule(0);
    renamed.name = "renamed-after-failure".into();
    let rules = Arc::new(vec![renamed.clone()]);
    let review = Arc::new(distribute::prepare_rules(
        &backend.handle,
        "Draft",
        rules.clone(),
        vec![a.clone(), b.clone()],
        false,
        Some(rule(0).name),
    )?);
    let result = run(
        &backend,
        review.clone(),
        review.progress(),
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert!(
        matches!(&result.targets[0].status,Status::Failed(e)if e.contains("original removal failed"))
    );
    assert_eq!(result.targets[0].confirmed, 1);
    assert_eq!(result.targets[0].removed, 0);
    assert_eq!(result.targets[1].removed, 1);
    assert_eq!(saved(&config.database, "alpha").len(), 2);
    assert_eq!(saved(&config.database, "beta"), vec![renamed.clone()]);
    assert!(
        backend
            .handle
            .history
            .health
            .catalogues
            .uncertain(a.session)
    );
    let _ = run(&backend, review, result, Arc::new(AtomicBool::new(false))).await;
    assert_eq!(ao.lock().unwrap().len(), 2);
    assert_eq!(bo.lock().unwrap().len(), 2);
    db.execute_batch("DROP TRIGGER refuse_original_delete;")?;
    drop(db);
    at.abort();
    let a = subscribe(
        &backend,
        &mut alpha,
        "alpha",
        vec![rule(0), renamed.clone()],
    )
    .await?;
    let at = replies(
        &mut alpha,
        Arc::new(AtomicBool::new(false)),
        true,
        ao.clone(),
    )
    .await?;
    let review = Arc::new(distribute::prepare_rules(
        &backend.handle,
        "Draft",
        rules,
        vec![a, b],
        false,
        Some(rule(0).name),
    )?);
    assert_eq!(review.targets[0].copies, 0);
    assert_eq!(review.targets[0].identical, 1);
    assert_eq!(review.targets[0].rename_removals, 1);
    let result = run(
        &backend,
        review.clone(),
        review.progress(),
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert!(result.targets.iter().all(|p| p.status == Status::Complete));
    assert_eq!(result.confirmed(), 0);
    assert_eq!(ao.lock().unwrap().len(), 3);
    assert_eq!(ao.lock().unwrap()[2].r#type, Action::DeleteRule as i32);
    assert_eq!(bo.lock().unwrap().len(), 2);
    assert_eq!(saved(&config.database, "alpha"), vec![renamed]);
    at.abort();
    bt.abort();
    drop(alpha);
    drop(beta);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_inline_rules_and_rename_collisions_require_explicit_replacement() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut alpha = client(&config.socket).await?;
    let mut beta = client(&config.socket).await?;
    let mut renamed = rule(0);
    renamed.name = "renamed-conflict".into();
    let mut conflict = renamed.clone();
    conflict.action = "reject".into();
    let a = subscribe(
        &backend,
        &mut alpha,
        "alpha",
        vec![rule(0), conflict.clone()],
    )
    .await?;
    let b = subscribe(&backend, &mut beta, "beta", vec![rule(0)]).await?;
    let ao = Arc::new(Mutex::new(Vec::new()));
    let bo = Arc::new(Mutex::new(Vec::new()));
    let at = replies(
        &mut alpha,
        Arc::new(AtomicBool::new(false)),
        true,
        ao.clone(),
    )
    .await?;
    let bt = replies(
        &mut beta,
        Arc::new(AtomicBool::new(false)),
        true,
        bo.clone(),
    )
    .await?;
    let rules = Arc::new(vec![renamed.clone()]);
    let review = Arc::new(distribute::prepare_rules(
        &backend.handle,
        "Draft",
        rules.clone(),
        vec![a.clone(), b],
        false,
        Some(rule(0).name),
    )?);
    assert!(
        review.targets[0]
            .blocked
            .as_ref()
            .unwrap()
            .contains("conflict")
    );
    let result = run(
        &backend,
        review.clone(),
        review.progress(),
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert!(matches!(result.targets[0].status, Status::Failed(_)));
    assert!(ao.lock().unwrap().is_empty());
    assert_eq!(saved(&config.database, "alpha").len(), 2);
    let review = Arc::new(distribute::prepare_rules(
        &backend.handle,
        "Draft",
        rules,
        vec![a.clone()],
        true,
        Some(rule(0).name),
    )?);
    let result = run(
        &backend,
        review.clone(),
        review.progress(),
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert_eq!(result.targets[0].removed, 1);
    assert_eq!(saved(&config.database, "alpha"), vec![renamed]);
    let text = serde_json::to_string(&vec![
        rules::document::rule_json(&rule(3)),
        rules::document::rule_json(&rule(4)),
    ])?;
    let imported = Arc::new(transfer::Import::from_text(&text)?.rules);
    let review = Arc::new(distribute::prepare_rules(
        &backend.handle,
        "Imported file",
        imported.clone(),
        vec![a],
        false,
        None,
    )?);
    assert!(Arc::ptr_eq(&review.rules, &imported));
    let result = run(
        &backend,
        review.clone(),
        review.progress(),
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    assert_eq!(result.confirmed(), 2);
    assert_eq!(saved(&config.database, "alpha").len(), 3);
    assert!(
        distribute::prepare_rules(
            &backend.handle,
            "Draft",
            Arc::new(vec![rule(3), rule(3)]),
            vec![],
            false,
            None
        )
        .is_err()
    );
    assert!(
        distribute::prepare_rules(
            &backend.handle,
            "Draft",
            Arc::new(vec![rule(3), rule(4)]),
            vec![],
            false,
            Some(rule(0).name)
        )
        .is_err()
    );
    at.abort();
    bt.abort();
    drop(alpha);
    drop(beta);
    backend.join();
    Ok(())
}
