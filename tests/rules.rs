use anyhow::Result;
use hyper_util::rt::TokioIo;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    protocol::{self, Action, Operator, Rule, ui_client::UiClient},
    rules::Query,
};
use std::{path::Path, time::Duration};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Channel, Endpoint};

fn options(path: &Path) -> Config {
    Config {
        transport: Default::default(),
        socket: path.join("ui.sock"),
        database: path.join("history.sqlite3"),
        prompt_timeout: 1,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    }
}
async fn client(path: &Path, rules: Vec<Rule>) -> Result<UiClient<Channel>> {
    let path = path.to_owned();
    let channel = Endpoint::try_from("http://localhost")?
        .connect_with_connector(tower::service_fn(move |_| {
            let path = path.clone();
            async move {
                tokio::net::UnixStream::connect(path)
                    .await
                    .map(TokioIo::new)
            }
        }))
        .await?;
    let mut client = UiClient::new(channel);
    client
        .subscribe(protocol::ClientConfig {
            name: "rule-daemon".into(),
            rules,
            ..Default::default()
        })
        .await?;
    Ok(client)
}
fn rule(i: usize) -> Rule {
    Rule {
        name: format!("fixture.{i:03}"),
        created: 123,
        description: format!("Application {i}"),
        enabled: true,
        precedence: true,
        nolog: true,
        action: "deny".into(),
        duration: if i.is_multiple_of(2) { "always" } else { "5m" }.into(),
        operator: Some(Operator {
            r#type: "list".into(),
            operand: "list".into(),
            sensitive: true,
            list: vec![
                Operator {
                    r#type: "simple".into(),
                    operand: "process.path".into(),
                    data: format!("/usr/bin/fixture-{i}"),
                    sensitive: true,
                    ..Default::default()
                },
                Operator {
                    r#type: "regexp".into(),
                    operand: "dest.host".into(),
                    data: ".*\\.example\\.test".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
    }
}
async fn page(backend: &Backend, mut query: Query) -> Result<rustleblower::rules::Page> {
    query.node = "rule-daemon".into();
    backend.handle.history.flush().await?;
    backend
        .handle
        .history
        .query_rules(query)
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inventory_is_paged_searchable_filtered_and_durable() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let client = client(&config.socket, (0..260).map(rule).collect()).await?;
    let first = page(&backend, Query::default()).await?;
    assert_eq!(first.rules.len(), 200);
    assert!(first.has_more);
    let next = page(
        &backend,
        Query {
            after: first.rules.last().unwrap().name.clone(),
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(next.rules.len(), 60);
    assert!(!next.has_more);
    let filtered = page(
        &backend,
        Query {
            search: "/usr/bin/fixture-205".into(),
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(filtered.rules, vec![rule(205)]);
    assert_eq!(
        page(
            &backend,
            Query {
                temporary: Some(false),
                ..Default::default()
            }
        )
        .await?
        .rules
        .len(),
        130
    );
    assert_eq!(
        page(
            &backend,
            Query {
                temporary: Some(true),
                ..Default::default()
            }
        )
        .await?
        .rules
        .len(),
        130
    );
    drop(client);
    backend.join();
    let db = rusqlite::Connection::open(config.database)?;
    assert_eq!(
        db.query_row("SELECT count(*) FROM application_rules", [], |r| r
            .get::<_, i64>(0))?,
        260
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rule_changes_wait_for_the_daemon_and_preserve_full_operators() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut client = client(&config.socket, vec![rule(0)]).await?;
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let mut stream = client
        .notifications(ReceiverStream::new(rx))
        .await?
        .into_inner();
    let session = backend.handle.status().session.unwrap().id;
    backend.handle.history.flush().await?;
    let mut updated = rule(0);
    updated.action = "allow".into();
    let result = backend
        .handle
        .control
        .submit(session, Action::ChangeRule, updated.clone())
        .map_err(anyhow::Error::msg)?;
    let command = tokio::time::timeout(Duration::from_secs(1), stream.message())
        .await??
        .unwrap();
    assert_eq!(command.rules, vec![updated.clone()]);
    assert_eq!(
        page(&backend, Query::default()).await?.rules,
        vec![rule(0)],
        "no optimistic policy update"
    );
    tx.send(protocol::NotificationReply {
        id: command.id,
        code: protocol::NotificationReplyCode::Error as i32,
        data: "read-only rule directory".into(),
    })
    .await?;
    assert!(result.await?.unwrap_err().contains("read-only"));
    assert!(backend.handle.history.health.catalogues.uncertain(session));
    assert_eq!(page(&backend, Query::default()).await?.rules, vec![rule(0)]);
    let result = backend
        .handle
        .control
        .submit(session, Action::ChangeRule, updated.clone())
        .map_err(anyhow::Error::msg)?;
    let command = stream.message().await?.unwrap();
    tx.send(protocol::NotificationReply {
        id: command.id,
        code: protocol::NotificationReplyCode::Ok as i32,
        ..Default::default()
    })
    .await?;
    result.await?.map_err(anyhow::Error::msg)?;
    assert_eq!(
        page(&backend, Query::default()).await?.rules,
        vec![updated.clone()]
    );
    for action in [Action::DisableRule, Action::EnableRule, Action::DeleteRule] {
        let result = backend
            .handle
            .control
            .submit(session, action, updated.clone())
            .map_err(anyhow::Error::msg)?;
        let command = stream.message().await?.unwrap();
        assert_eq!(command.r#type, action as i32);
        tx.send(protocol::NotificationReply {
            id: command.id,
            ..Default::default()
        })
        .await?;
        result.await?.map_err(anyhow::Error::msg)?;
        let records = page(&backend, Query::default()).await?.rules;
        if action == Action::DeleteRule {
            assert!(records.is_empty());
        } else {
            assert_eq!(records[0].enabled, action == Action::EnableRule);
            assert_eq!(records[0].operator, updated.operator);
        }
    }
    drop(tx);
    drop(stream);
    drop(client);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_cancels_unconfirmed_edits_and_ignores_old_replies() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut old = client(&config.socket, vec![rule(0)]).await?;
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let mut stream = old
        .notifications(ReceiverStream::new(rx))
        .await?
        .into_inner();
    let session = backend.handle.status().session.unwrap().id;
    backend.handle.history.flush().await?;
    let mut changed = rule(0);
    changed.action = "allow".into();
    let result = backend
        .handle
        .control
        .submit(session, Action::ChangeRule, changed.clone())
        .map_err(anyhow::Error::msg)?;
    let command = stream.message().await?.unwrap();
    let replacement = client(&config.socket, vec![rule(0)]).await?;
    assert!(result.await?.unwrap_err().contains("reconnected"));
    assert!(
        backend
            .handle
            .control
            .submit(session, Action::ChangeRule, changed)
            .is_err()
    );
    tx.send(protocol::NotificationReply {
        id: command.id,
        ..Default::default()
    })
    .await?;
    assert_eq!(page(&backend, Query::default()).await?.rules, vec![rule(0)]);
    drop(tx);
    drop(stream);
    drop(old);
    drop(replacement);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_refresh_preserves_socket_and_refuses_pending_decisions_or_commands() -> Result<()>
{
    use std::{os::unix::fs::MetadataExt, time::Instant};
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut old = client(&config.socket, vec![rule(0)]).await?;
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let mut stream = old
        .notifications(ReceiverStream::new(rx))
        .await?
        .into_inner();
    backend.handle.history.flush().await?;
    let session = backend.handle.status().session.unwrap().id;
    let inode = std::fs::metadata(&config.socket)?.ino();
    let (prompt, mut reply) = backend
        .handle
        .prompts
        .insert(
            session,
            protocol::Connection::default(),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
    assert!(
        backend
            .handle
            .refresh_rules(session)
            .unwrap_err()
            .contains("permission")
    );
    assert!(reply.try_recv().is_err());
    backend.handle.prompts.remove(prompt.id);
    let result = backend
        .handle
        .control
        .submit(session, Action::DisableRule, rule(0))
        .map_err(anyhow::Error::msg)?;
    let command = stream.message().await?.unwrap();
    assert!(
        backend
            .handle
            .refresh_rules(session)
            .unwrap_err()
            .contains("outstanding")
    );
    tx.send(protocol::NotificationReply {
        id: command.id,
        ..Default::default()
    })
    .await?;
    result.await?.map_err(anyhow::Error::msg)?;
    backend
        .handle
        .refresh_rules(session)
        .map_err(anyhow::Error::msg)?;
    assert_eq!(
        stream.message().await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
    assert_eq!(std::fs::metadata(&config.socket)?.ino(), inode);
    assert!(!backend.handle.history.health.catalogues.ready(session));
    let replacement = client(&config.socket, vec![rule(1)]).await?;
    backend.handle.history.flush().await?;
    assert_eq!(page(&backend, Query::default()).await?.rules, vec![rule(1)]);
    drop(tx);
    drop(stream);
    drop(old);
    drop(replacement);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acknowledged_change_with_a_failed_cache_write_requires_a_fresh_inventory() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut original = client(&config.socket, vec![rule(0)]).await?;
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let mut stream = original
        .notifications(ReceiverStream::new(rx))
        .await?
        .into_inner();
    backend.handle.history.flush().await?;
    let session = backend.handle.status().session.unwrap().id;
    let db = rusqlite::Connection::open(&config.database)?;
    db.execute_batch("CREATE TRIGGER refuse_rule_changes BEFORE INSERT ON application_rules BEGIN SELECT RAISE(ABORT, 'cache write failed'); END;")?;
    let mut changed = rule(0);
    changed.action = "allow".into();
    let result = backend
        .handle
        .control
        .submit(session, Action::ChangeRule, changed.clone())
        .map_err(anyhow::Error::msg)?;
    let command = stream.message().await?.unwrap();
    tx.send(protocol::NotificationReply {
        id: command.id,
        ..Default::default()
    })
    .await?;
    let error = result.await?.unwrap_err();
    assert!(error.contains("Daemon confirmed") && error.contains("cache write failed"));
    assert_eq!(page(&backend, Query::default()).await?.rules, vec![rule(0)]);
    assert!(
        backend
            .handle
            .control
            .submit(session, Action::ChangeRule, changed.clone())
            .unwrap_err()
            .contains("inventory")
    );
    db.execute_batch("DROP TRIGGER refuse_rule_changes")?;
    let replacement = client(&config.socket, vec![changed.clone()]).await?;
    assert_eq!(page(&backend, Query::default()).await?.rules, vec![changed]);
    assert!(
        backend
            .handle
            .history
            .health
            .catalogues
            .ready(backend.handle.status().session.unwrap().id)
    );
    drop(db);
    drop(tx);
    drop(stream);
    drop(original);
    drop(replacement);
    backend.join();
    Ok(())
}
