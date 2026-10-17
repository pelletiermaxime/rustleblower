use anyhow::Result;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    protocol::{self, ui_client::UiClient},
    updates::{self, Command, Target, TargetVersion},
};
use std::{path::Path, time::Duration};
use tonic::transport::{Channel, Endpoint};

async fn client(socket: &Path, name: &str) -> Result<UiClient<Channel>> {
    let socket = socket.to_owned();
    let channel = Endpoint::try_from("http://localhost")?
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
            name: name.into(),
            // This is the reported kernel version, not the daemon version.
            version: "kernel-must-not-be-compared".into(),
            ..Default::default()
        })
        .await?;
    Ok(client)
}

async fn report(client: &mut UiClient<Channel>, version: &str) -> Result<()> {
    let reply = client
        .ping(protocol::PingRequest {
            id: 91,
            stats: Some(protocol::Statistics {
                daemon_version: version.into(),
                ..Default::default()
            }),
        })
        .await?
        .into_inner();
    assert_eq!(reply.id, 91);
    Ok(())
}

async fn current(handle: &updates::Handle, expected: Option<TargetVersion>) {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let state = handle.state();
            if state.ready && state.current == expected {
                assert!(!state.settings.enabled);
                assert_eq!(state.completed_checks, 0);
                assert_eq!(state.next_due, None);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("update target must follow the selected live daemon without a window");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_daemon_uses_actual_ping_metadata_without_a_window_or_automatic_network()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = Config {
        socket: dir.path().join("ui.sock"),
        database: dir.path().join("history.sqlite3"),
        transport: Default::default(),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    };
    let backend = Backend::start(config.clone())?;
    current(
        &backend.handle.updates,
        Some(TargetVersion {
            node: "Rustleblower".into(),
            version: updates::APP_VERSION.into(),
        }),
    )
    .await;
    let mut alpha = client(&config.socket, "alpha").await?;
    let mut beta = client(&config.socket, "beta").await?;
    report(&mut alpha, "1.9.0").await?;
    report(&mut beta, "1.8.1").await?;
    let alpha_id = backend.handle.node("alpha").unwrap().id;
    let beta_id = backend.handle.node("beta").unwrap().id;
    backend
        .handle
        .select_node(alpha_id)
        .map_err(anyhow::Error::msg)?;
    let state = backend.handle.updates.state();
    backend
        .handle
        .updates
        .request(Command::Save {
            expected_revision: state.revision,
            settings: updates::Settings {
                target: Target::SelectedDaemon,
                ..Default::default()
            },
        })
        .await?
        .map_err(anyhow::Error::msg)?;
    current(
        &backend.handle.updates,
        Some(TargetVersion {
            node: "alpha".into(),
            version: "1.9.0".into(),
        }),
    )
    .await;
    backend
        .handle
        .select_node(beta_id)
        .map_err(anyhow::Error::msg)?;
    current(
        &backend.handle.updates,
        Some(TargetVersion {
            node: "beta".into(),
            version: "1.8.1".into(),
        }),
    )
    .await;
    // A later report replaces metadata; no stale app/kernel version is substituted.
    report(&mut beta, "").await?;
    current(&backend.handle.updates, None).await;
    report(&mut beta, "vendor-build-without-semver").await?;
    current(
        &backend.handle.updates,
        Some(TargetVersion {
            node: "beta".into(),
            version: "vendor-build-without-semver".into(),
        }),
    )
    .await;
    let mut notices = backend.handle.desktop_notices.take_receiver().unwrap();
    assert!(notices.try_recv().is_err());
    drop(alpha);
    drop(beta);
    backend.handle.shutdown();
    backend.join();
    assert!(!config.socket.exists());
    Ok(())
}
