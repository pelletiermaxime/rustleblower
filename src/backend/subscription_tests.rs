//! Hold the real subscription writer at a known point without a CPU-speed race.
use super::*;
use crate::subscriptions::{Reply, Request, Worker, bulk, model::Spec};
use std::sync::atomic::AtomicBool;

async fn reply(handle: &crate::subscriptions::Handle, request: Request) -> Result<Reply> {
    tokio::time::timeout(
        Duration::from_secs(3),
        handle.request(request).map_err(anyhow::Error::msg)?,
    )
    .await??
    .map_err(anyhow::Error::msg)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permissions_reply_while_a_real_bulk_sql_transaction_is_busy() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = Config {
        transport: Default::default(),
        socket: dir.path().join("ui.sock"),
        database: dir.path().join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    };
    let armed = Arc::new(AtomicBool::new(false));
    let hook_armed = armed.clone();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    // Dropping the sender also releases the hook if an assertion unwinds.
    let mut resume = Some(resume_tx);
    let backend = Backend::start_with_subscription_worker(
        config.clone(),
        Default::default(),
        move |path, notices| {
            Worker::start_with_connection_setup(path, notices, move |db| {
                let mut entered = Some(entered_tx);
                db.update_hook(Some(move |action, _: &str, table: &str, _: i64| {
                    if action == rusqlite::hooks::Action::SQLITE_UPDATE
                        && table == "subscriptions"
                        && hook_armed.swap(false, Ordering::SeqCst)
                    {
                        let _ = entered.take().unwrap().send(());
                        let _ = resume_rx.recv_timeout(Duration::from_secs(10));
                    }
                }))
                .expect("Install the owned subscription UPDATE gate");
            })
        },
    )?;
    let result = async {
        let handle = &backend.handle.subscriptions;
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
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "bulk-permission-fixture".into(),
                ..Default::default()
            })
            .await?;
        let mut ids = Vec::new();
        for name in ["Busy bulk A", "Busy bulk B"] {
            let Reply::Saved(id) = reply(
                handle,
                Request::Save {
                    previous: None,
                    spec: Spec {
                        name: name.into(),
                        url: "http://127.0.0.1:1/unused".into(),
                        ..Default::default()
                    },
                },
            )
            .await?
            else {
                anyhow::bail!("Unexpected save reply");
            };
            ids.push(id);
        }
        let Reply::BulkReview(review) = reply(
            handle,
            Request::ReviewBulk {
                scope: bulk::Scope::Selected(ids.clone()),
                patch: bulk::Patch {
                    enabled: Some(false),
                    ..Default::default()
                },
            },
        )
        .await?
        else {
            anyhow::bail!("Unexpected review reply");
        };
        let db = rusqlite::Connection::open(handle.root.join("subscriptions.sqlite3"))?;
        db.busy_timeout(Duration::ZERO)?;
        let busy = || {
            assert_eq!(
                db.execute_batch("BEGIN IMMEDIATE;")
                    .unwrap_err()
                    .sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy)
            );
        };
        armed.store(true, Ordering::SeqCst);
        let mut applying = handle
            .request(Request::ApplyBulk(review))
            .map_err(anyhow::Error::msg)?;
        tokio::time::timeout(Duration::from_secs(3), entered_rx)
            .await
            .context("The actual subscription UPDATE did not reach its gate")??;
        busy();
        tokio::time::timeout(
            Duration::from_millis(500),
            daemon.ping(protocol::PingRequest::default()),
        )
        .await??;
        let mut asking = daemon.clone();
        let ask = tokio::spawn(async move {
            asking
                .ask_rule(protocol::Connection {
                    process_path: "/usr/bin/bulk-isolation".into(),
                    ..Default::default()
                })
                .await
        });
        tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                if let Some(prompt) = backend.handle.prompts.snapshot().first() {
                    backend.handle.decide(prompt.id, Decision::Allow);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await?;
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(500), ask)
                .await???
                .into_inner()
                .action,
            "allow"
        );
        busy();
        assert!(matches!(
            applying.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        resume.take().unwrap().send(())?;
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(3), applying)
                .await??
                .map_err(anyhow::Error::msg)?,
            Reply::BulkApplied(2)
        ));
        for id in ids {
            let Reply::Row(row) = reply(handle, Request::Get(id)).await? else {
                anyhow::bail!("Unexpected row reply");
            };
            assert!(!row.spec.enabled);
        }
        Ok(())
    }
    .await;
    drop(resume);
    backend.join();
    assert!(!config.socket.exists());
    result
}
