use anyhow::Result;
use hyper_util::rt::TokioIo;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    history,
    nodes::{self, Change},
    protocol::{self, ui_client::UiClient},
    rules,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tonic::transport::{Channel, Endpoint};
fn options(path: &Path) -> Config {
    Config {
        transport: Default::default(),
        socket: path.join("ui.sock"),
        database: path.join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
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
fn rule() -> protocol::Rule {
    protocol::Rule {
        name: "shared-rule".into(),
        action: "deny".into(),
        duration: "always".into(),
        enabled: true,
        operator: Some(protocol::Operator {
            r#type: "simple".into(),
            operand: "process.path".into(),
            data: "/bin/fixture".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn inventory(name: &str) -> protocol::ClientConfig {
    protocol::ClientConfig{name:name.into(),version:"fixture-kernel".into(),is_firewall_running:true,config:r#"{"DefaultAction":"deny","DefaultDuration":"once","Rules":{"Path":"/fixture/rules"},"Future":{"keep":true}}"#.into(),log_level:2,rules:vec![rule()],..Default::default()}
}
async fn subscribe(client: &mut UiClient<Channel>, name: &str, backend: &Backend) -> Result<u64> {
    let config = inventory(name);
    assert_eq!(client.subscribe(config.clone()).await?.into_inner(), config);
    backend.handle.history.flush().await?;
    Ok(backend.handle.node(name).unwrap().id)
}
async fn notifications(
    client: &mut UiClient<Channel>,
) -> Result<(
    tokio::sync::mpsc::Sender<protocol::NotificationReply>,
    tonic::Streaming<protocol::Notification>,
)> {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let stream = client
        .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await?
        .into_inner();
    Ok((tx, stream))
}
async fn ack(
    tx: &tokio::sync::mpsc::Sender<protocol::NotificationReply>,
    id: u64,
    ok: bool,
) -> Result<()> {
    tx.send(protocol::NotificationReply {
        id,
        code: if ok {
            protocol::NotificationReplyCode::Ok
        } else {
            protocol::NotificationReplyCode::Error
        } as i32,
        data: if ok {
            ""
        } else {
            "synthetic persistent configuration failure"
        }
        .into(),
    })
    .await?;
    Ok(())
}
async fn wait_prompts(backend: &Backend, count: usize) {
    let start = Instant::now();
    while backend.handle.prompts.snapshot().len() != count {
        assert!(start.elapsed() < Duration::from_secs(2));
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}
async fn cached_rule(backend: &Backend, name: &str) -> Result<protocol::Rule> {
    Ok(backend
        .handle
        .history
        .query_rules(rules::Query {
            node: name.into(),
            ..Default::default()
        })
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?
        .rules
        .remove(0))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_nodes_route_rules_prompts_reconnects_and_stale_acknowledgements() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut a = client(&config.socket).await?;
    let a_id = subscribe(&mut a, "alpha", &backend).await?;
    let (a_tx, mut a_stream) = notifications(&mut a).await?;
    let mut b = client(&config.socket).await?;
    let b_id = subscribe(&mut b, "beta", &backend).await?;
    let (b_tx, mut b_stream) = notifications(&mut b).await?;
    assert_eq!(backend.handle.status().sessions.len(), 2);
    assert!(backend.handle.history.health.catalogues.ready(a_id));
    assert!(backend.handle.history.health.catalogues.ready(b_id));
    let mut a_rule = rule();
    a_rule.action = "allow".into();
    let a_change = backend
        .handle
        .control
        .submit(a_id, protocol::Action::ChangeRule, a_rule)
        .map_err(anyhow::Error::msg)?;
    let a_command = a_stream.message().await?.unwrap();
    assert_eq!(a_command.client_name, "alpha");
    let mut b_rule = rule();
    b_rule.action = "reject".into();
    let b_change = backend
        .handle
        .control
        .submit(b_id, protocol::Action::ChangeRule, b_rule)
        .map_err(anyhow::Error::msg)?;
    let b_command = b_stream.message().await?.unwrap();
    assert_eq!(b_command.client_name, "beta");
    // A forged/crossed acknowledgement cannot consume the other node's pending ID.
    ack(&b_tx, a_command.id, true).await?;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(cached_rule(&backend, "alpha").await?.action, "deny");
    let mut asking_a = a.clone();
    let a_prompt = tokio::spawn(async move {
        asking_a
            .ask_rule(protocol::Connection {
                process_path: "/bin/alpha".into(),
                ..Default::default()
            })
            .await
    });
    let mut asking_b = b.clone();
    let b_prompt = tokio::spawn(async move {
        asking_b
            .ask_rule(protocol::Connection {
                process_path: "/bin/beta".into(),
                ..Default::default()
            })
            .await
    });
    wait_prompts(&backend, 2).await;
    let mut replacement = client(&config.socket).await?;
    let new_a = subscribe(&mut replacement, "alpha", &backend).await?;
    assert_ne!(a_id, new_a);
    assert_eq!(backend.handle.status().sessions.len(), 2);
    assert!(a_change.await?.unwrap_err().contains("reconnected"));
    assert_eq!(a_prompt.await??.into_inner().action, "deny");
    wait_prompts(&backend, 1).await;
    let remaining = backend.handle.prompts.snapshot()[0].clone();
    assert_eq!(remaining.session, b_id);
    assert!(backend.handle.decide(remaining.id, Decision::Allow));
    assert_eq!(b_prompt.await??.into_inner().action, "allow");
    ack(&a_tx, a_command.id, true).await?;
    ack(&b_tx, b_command.id, true).await?;
    b_change.await?.map_err(anyhow::Error::msg)?;
    assert_eq!(cached_rule(&backend, "alpha").await?.action, "deny");
    assert_eq!(cached_rule(&backend, "beta").await?.action, "reject");
    backend
        .handle
        .select_node(b_id)
        .map_err(anyhow::Error::msg)?;
    assert_eq!(backend.handle.status().session.unwrap().name, "beta");
    assert!(backend.handle.session(new_a).is_some());
    drop(a_tx);
    drop(a_stream);
    drop(a);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(backend.handle.session(new_a).is_some());
    assert!(backend.handle.session(b_id).is_some());
    backend.handle.history.flush().await?;
    let history = backend
        .handle
        .history
        .query(history::Query::default())
        .unwrap()
        .await?
        .map_err(anyhow::Error::msg)?;
    assert!(
        history
            .rows
            .iter()
            .any(|row| row.node == "alpha" && row.action == "deny")
    );
    assert!(
        history
            .rows
            .iter()
            .any(|row| row.node == "beta" && row.action == "allow")
    );
    drop(b_tx);
    drop(b_stream);
    drop(b);
    drop(replacement);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn node_controls_commit_only_after_matching_ack_and_preserve_failures_and_reopen()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut a = client(&config.socket).await?;
    let a_id = subscribe(&mut a, "alpha", &backend).await?;
    let (a_tx, mut stream) = notifications(&mut a).await?;
    let mut b = client(&config.socket).await?;
    let b_id = subscribe(&mut b, "beta", &backend).await?;
    let (b_tx, mut b_stream) = notifications(&mut b).await?;
    let pending = backend
        .handle
        .control
        .submit_node(a_id, Change::Interception(false))
        .unwrap();
    let command = stream.message().await?.unwrap();
    assert_eq!(command.r#type, protocol::Action::DisableInterception as i32);
    assert!(command.data.is_empty() && command.rules.is_empty());
    assert!(nodes::detail(&config.database, "alpha")?.row.running);
    assert!(
        backend
            .handle
            .control
            .submit_node(a_id, Change::Interception(true))
            .is_err()
    );
    assert!(
        backend
            .handle
            .control
            .submit(a_id, protocol::Action::ChangeRule, rule())
            .is_err()
    );
    ack(&a_tx, command.id, true).await?;
    pending.await?.map_err(anyhow::Error::msg)?;
    assert!(!nodes::detail(&config.database, "alpha")?.row.running);
    assert!(nodes::detail(&config.database, "beta")?.row.running);
    let old = nodes::detail(&config.database, "alpha")?.config;
    let mut changed: serde_json::Value = serde_json::from_str(&old)?;
    changed["DefaultAction"] = "allow".into();
    changed["DefaultDuration"] = "5m".into();
    let changed = serde_json::to_string(&changed)?;
    let pending = backend
        .handle
        .control
        .submit_node(a_id, Change::Configuration(changed.clone()))
        .unwrap();
    let command = stream.message().await?.unwrap();
    assert_eq!(command.data, changed);
    assert_eq!(command.r#type, protocol::Action::ChangeConfig as i32);
    assert_eq!(nodes::detail(&config.database, "alpha")?.config, old);
    ack(&a_tx, command.id, true).await?;
    pending.await?.map_err(anyhow::Error::msg)?;
    assert_eq!(nodes::detail(&config.database, "alpha")?.config, changed);
    let persisted: serde_json::Value =
        serde_json::from_str(&nodes::detail(&config.database, "alpha")?.config)?;
    assert_eq!(persisted["Future"]["keep"], true);
    let pending = backend
        .handle
        .control
        .submit_node(
            a_id,
            Change::Configuration("{\"DefaultAction\":\"reject\"}".into()),
        )
        .unwrap();
    let command = stream.message().await?.unwrap();
    ack(&a_tx, command.id, false).await?;
    assert!(
        pending
            .await?
            .unwrap_err()
            .contains("live state may have changed")
    );
    assert_eq!(nodes::detail(&config.database, "alpha")?.config, changed);
    assert!(backend.handle.history.health.catalogues.uncertain(a_id));
    assert!(!backend.handle.history.health.catalogues.uncertain(b_id));
    assert!(
        backend
            .handle
            .control
            .submit_node(a_id, Change::Interception(true))
            .is_err()
    );
    let pending = backend
        .handle
        .control
        .submit_node(b_id, Change::Interception(false))
        .unwrap();
    let command = b_stream.message().await?.unwrap();
    ack(&b_tx, command.id, true).await?;
    pending.await?.map_err(anyhow::Error::msg)?;
    assert!(
        backend
            .handle
            .control
            .submit_node(b_id, Change::Configuration("[]".into()))
            .is_err()
    );
    assert!(
        backend
            .handle
            .control
            .submit_node(
                b_id,
                Change::Configuration("x".repeat(nodes::CONFIG_BYTES + 1))
            )
            .is_err()
    );
    drop(a_tx);
    drop(b_tx);
    drop(stream);
    drop(b_stream);
    drop(a);
    drop(b);
    backend.join();
    let reopened = Backend::start(config.clone())?;
    assert!(reopened.handle.status().sessions.is_empty());
    assert_eq!(nodes::page(&config.database, "")?.len(), 2);
    assert_eq!(nodes::detail(&config.database, "alpha")?.config, changed);
    assert!(!nodes::detail(&config.database, "beta")?.row.running);
    reopened.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admission_and_cached_nodes_are_bounded_and_closed_transports_release_slots() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut clients = Vec::new();
    for index in 0..nodes::MAX_NODES {
        let mut node = client(&config.socket).await?;
        subscribe(&mut node, &format!("node-{index:02}"), &backend).await?;
        clients.push(node);
    }
    assert_eq!(backend.handle.status().sessions.len(), 32);
    let mut extra = client(&config.socket).await?;
    assert_eq!(
        extra
            .subscribe(inventory("extra"))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::ResourceExhausted
    );
    drop(clients.remove(0));
    let started = Instant::now();
    while backend.handle.status().sessions.len() == 32 {
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "Closed HTTP/2 transport kept its node slot"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    subscribe(&mut extra, "extra", &backend).await?;
    assert_eq!(backend.handle.status().sessions.len(), 32);
    let page = nodes::page(&config.database, "")?;
    assert_eq!(page.len(), 32);
    assert!(!page.iter().any(|row| row.name == "node-00"));
    assert_eq!(nodes::page(&config.database, "EXTRA")?.len(), 1);
    let mut invalid = client(&config.socket).await?;
    assert_eq!(
        invalid
            .subscribe(inventory(&"x".repeat(129)))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::InvalidArgument
    );
    drop(invalid);
    drop(extra);
    drop(clients);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_subscribe_on_one_transport_uses_a_fresh_generation_and_keeps_other_nodes()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = options(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut a = client(&config.socket).await?;
    let old = subscribe(&mut a, "alpha", &backend).await?;
    let (old_tx, old_stream) = notifications(&mut a).await?;
    let mut b = client(&config.socket).await?;
    let b_id = subscribe(&mut b, "beta", &backend).await?;
    // Two simultaneous reports retain one fair latest slot each, including flush.
    let pa = protocol::PingRequest {
        stats: Some(protocol::Statistics {
            by_executable: [("/bin/a".into(), 10)].into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let pb = protocol::PingRequest {
        stats: Some(protocol::Statistics {
            by_executable: [("/bin/b".into(), 20)].into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let (ra, rb) = tokio::join!(a.ping(pa), b.ping(pb));
    ra?;
    rb?;
    backend.handle.history.flush().await?;
    for (node, total) in [("alpha", 10), ("beta", 20)] {
        assert_eq!(
            rustleblower::statistics::capture(
                &config.database,
                rustleblower::statistics::Query {
                    node: node.into(),
                    ..Default::default()
                }
            )?
            .total,
            total
        );
    }
    let current = subscribe(&mut a, "alpha", &backend).await?;
    assert_ne!(current, old);
    assert!(backend.handle.session(old).is_none());
    assert!(backend.handle.session(b_id).is_some());
    drop(old_tx);
    drop(old_stream);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        backend.handle.session(current).is_some(),
        "Closing the old stream must not disconnect a new subscription on the same transport"
    );
    let (tx, mut stream) = notifications(&mut a).await?;
    let pending = backend
        .handle
        .control
        .submit_node(current, Change::Interception(false))
        .unwrap();
    let command = stream.message().await?.unwrap();
    ack(&tx, command.id, true).await?;
    pending.await?.map_err(anyhow::Error::msg)?;
    assert!(!nodes::detail(&config.database, "alpha")?.row.running);
    drop(tx);
    drop(stream);
    drop(a);
    drop(b);
    backend.join();
    Ok(())
}
