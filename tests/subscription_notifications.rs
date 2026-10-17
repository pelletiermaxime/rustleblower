use anyhow::{Result, anyhow};
use rustleblower::{
    desktop_notifications::{CAPACITY, Mailbox, Target},
    subscriptions::{
        Handle, Reply, Request, Worker,
        bulk::Scope,
        model::{Format, Row, Settings, Spec},
        notifications::{Config, MAX_MESSAGE},
        transfer::{self, Encoding},
    },
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[allow(dead_code)]
mod probe_http {
    include!("support/probe_http.rs");
}

fn template(notify: Value) -> Value {
    json!({"actions":{"list_subscriptions":{"enabled":false,"config":{
        "notify":notify,"subscriptions":[],"unknown_config":{"preserve":true}
    }},"other_plugin":{"keep":17}},"unknown_action":"preserve"})
}
fn enabled() -> Config {
    Config {
        enabled: true,
        success_message: String::new(),
        error_message: String::new(),
    }
}
async fn req(h: &Handle, request: Request) -> Result<Reply> {
    tokio::time::timeout(
        Duration::from_secs(4),
        h.request(request).map_err(anyhow::Error::msg)?,
    )
    .await??
    .map_err(anyhow::Error::msg)
}
async fn config(h: &Handle) -> Result<(u64, Config)> {
    match req(h, Request::NotificationSettings).await? {
        Reply::NotificationSettings(revision, config) => Ok((revision, config)),
        _ => Err(anyhow!("Expected notification configuration")),
    }
}
async fn set_config(h: &Handle, value: Config) -> Result<()> {
    let (revision, _) = config(h).await?;
    assert!(matches!(
        req(
            h,
            Request::SaveNotificationSettings {
                revision,
                config: value
            }
        )
        .await?,
        Reply::Done
    ));
    Ok(())
}
async fn get(h: &Handle, id: u64) -> Result<Row> {
    match req(h, Request::Get(id)).await? {
        Reply::Row(row) => Ok(*row),
        _ => Err(anyhow!("Expected row")),
    }
}
async fn save(h: &Handle, previous: Option<(u64, u64)>, name: &str, url: String) -> Result<u64> {
    let spec = Spec {
        name: name.into(),
        url,
        ..Default::default()
    };
    match req(h, Request::Save { previous, spec }).await? {
        Reply::Saved(id) => Ok(id),
        _ => Err(anyhow!("Expected saved row")),
    }
}
async fn idle(h: &Handle) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(4), async {
        while h.state().active.is_some() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}
async fn refresh(h: &Handle, id: u64, force: bool, status: &str) -> Result<Row> {
    let before = h.state().revision;
    req(h, Request::Refresh { id, force }).await?;
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let row = get(h, id).await?;
            if h.state().active.is_none()
                && h.state().revision >= before + 3
                && row.status == status
            {
                break Ok::<_, anyhow::Error>(row);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?
}
async fn import_review(h: &Handle, path: &std::path::Path) -> Result<Box<transfer::Review>> {
    match req(
        h,
        Request::ReviewImport {
            path: path.to_owned(),
            replace: true,
            defaults: true,
            catalogue: false,
        },
    )
    .await?
    {
        Reply::ImportReview(review) => Ok(review),
        _ => Err(anyhow!("Expected import review")),
    }
}

#[test]
fn source_notify_object_semantics_custom_fallback_and_bounds() -> Result<()> {
    for value in [
        Value::Null,
        json!(false),
        json!(true),
        json!(1),
        json!("enabled"),
        json!([]),
    ] {
        assert!(
            !Config::from_template(&template(value))
                .map_err(anyhow::Error::msg)?
                .enabled
        );
    }
    assert!(
        !Config::from_template(&json!({}))
            .map_err(anyhow::Error::msg)?
            .enabled
    );
    let fallback = Config::from_template(&template(json!({}))).map_err(anyhow::Error::msg)?;
    assert!(fallback.enabled);
    assert_eq!(
        fallback.message("Owned", "updated", ""),
        Some("Owned: updated".into())
    );
    assert_eq!(
        fallback.message("Owned", "unchanged", ""),
        Some("Owned: unchanged".into())
    );
    assert_eq!(
        fallback.message("Owned", "error", "HTTP 404"),
        Some("Owned failed: HTTP 404".into())
    );
    for status in ["cancelled", "downloading", "paused", ""] {
        assert!(fallback.message("Owned", status, "").is_none());
    }
    assert!(Config::default().message("Owned", "updated", "").is_none());
    let custom = Config::from_template(&template(
        json!({"success":{"desktop":"Custom success"},"error":{"desktop":"Custom failure"}}),
    ))
    .map_err(anyhow::Error::msg)?;
    assert_eq!(
        custom.message("Owned", "updated", ""),
        Some("Custom success".into())
    );
    assert_eq!(
        custom.message("Owned", "unchanged", ""),
        Some("Custom success".into())
    );
    assert_eq!(
        custom.message("Owned", "error", "ignored"),
        Some("Custom failure".into())
    );
    let mut bounded = enabled();
    bounded.success_message = "x".repeat(MAX_MESSAGE);
    bounded.validate().map_err(anyhow::Error::msg)?;
    bounded.success_message.push('x');
    assert!(bounded.validate().is_err());
    for invalid in ["bad\nmessage", "bad\0message", "bad\tmessage"] {
        assert!(
            Config {
                error_message: invalid.into(),
                ..enabled()
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        Config::from_template(&template(
            json!({"success":{"desktop":"x".repeat(MAX_MESSAGE+1)}})
        ))
        .is_err()
    );
    let fallback = fallback
        .message(&"é".repeat(500), "error", &"failure".repeat(500))
        .unwrap();
    assert!(fallback.len() <= MAX_MESSAGE && !fallback.chars().any(char::is_control));
    Ok(())
}

#[tokio::test]
async fn metadata_revision_import_export_unknown_fields_and_reopen_are_atomic() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let (initial_revision, initial) = config(&h).await?;
    assert!(!initial.enabled);
    let source = template(
        json!({"success":{"desktop":"Imported success","other_channel":"preserve"},"error":{"desktop":"Imported error"},"unknown_notify":23}),
    );
    let path = dir.path().join("source.json");
    std::fs::write(&path, serde_json::to_vec(&source)?)?;
    let old_review = import_review(&h, &path).await?;
    set_config(&h, enabled()).await?;
    assert!(
        req(&h, Request::ApplyImport(old_review)).await.is_err(),
        "Metadata edit invalidates a captured import review"
    );
    assert!(
        req(
            &h,
            Request::SaveNotificationSettings {
                revision: initial_revision,
                config: Config::default()
            }
        )
        .await
        .is_err()
    );
    let review = import_review(&h, &path).await?;
    req(&h, Request::ApplyImport(review)).await?;
    let imported = config(&h).await?;
    assert_eq!(imported.1.success_message, "Imported success");
    let changed = Config {
        success_message: "Changed success".into(),
        error_message: "Changed error".into(),
        ..enabled()
    };
    set_config(&h, changed.clone()).await?;
    assert!(
        req(
            &h,
            Request::SaveNotificationSettings {
                revision: imported.0,
                config: Config::default()
            }
        )
        .await
        .is_err()
    );
    for encoding in [Encoding::Native, Encoding::OpenSnitch] {
        let path = dir.path().join(if matches!(encoding, Encoding::Native) {
            "native.json"
        } else {
            "action.json"
        });
        req(
            &h,
            Request::Export {
                path: path.clone(),
                scope: Scope::Matching(String::new()),
                encoding,
            },
        )
        .await?;
        let document = transfer::decode(&std::fs::read_to_string(path)?, &Settings::default())
            .map_err(anyhow::Error::msg)?;
        assert_eq!(
            Config::from_template(&document.opensnitch).map_err(anyhow::Error::msg)?,
            changed
        );
        assert_eq!(
            document.opensnitch.pointer("/actions/other_plugin/keep"),
            Some(&json!(17))
        );
        assert_eq!(
            document.opensnitch.pointer("/unknown_action"),
            Some(&json!("preserve"))
        );
        assert_eq!(
            document
                .opensnitch
                .pointer("/actions/list_subscriptions/config/unknown_config/preserve"),
            Some(&json!(true))
        );
        assert_eq!(
            document
                .opensnitch
                .pointer("/actions/list_subscriptions/config/notify/unknown_notify"),
            Some(&json!(23))
        );
        assert_eq!(
            document
                .opensnitch
                .pointer("/actions/list_subscriptions/config/notify/success/other_channel"),
            Some(&json!("preserve"))
        );
    }
    let db = rusqlite::Connection::open(dir.path().join("subscriptions.sqlite3"))?;
    db.execute_batch("CREATE TRIGGER owned_refusal BEFORE UPDATE ON subscription_interchange BEGIN SELECT RAISE(ABORT,'owned notification refusal'); END;")?;
    assert!(set_config(&h, Config::default()).await.is_err());
    assert_eq!(config(&h).await?.1, changed);
    db.execute_batch("DROP TRIGGER owned_refusal")?;
    worker.join();
    let reopened = Worker::start(dir.path());
    assert_eq!(config(&reopened.handle()).await?.1, changed);
    reopened.join();
    Ok(())
}

/// A disposable HTTP server with one bounded request buffer and one connection
/// at a time; ETag allows a real production conditional-GET/304 path.
struct ConditionalHttp {
    url: String,
    stop: tokio_util::sync::CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for ConditionalHttp {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}
impl ConditionalHttp {
    async fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let stop = tokio_util::sync::CancellationToken::new();
        let token = stop.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = tokio::select! {_=token.cancelled()=>break,result=listener.accept()=>match result{Ok(v)=>v,Err(_)=>break}};
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                loop {
                    let n = match tokio::time::timeout(
                        Duration::from_secs(1),
                        socket.read(&mut buffer),
                    )
                    .await
                    {
                        Ok(Ok(n)) if n > 0 => n,
                        _ => break,
                    };
                    request.extend_from_slice(&buffer[..n]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") || request.len() > 8192 {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                let error = request.starts_with("GET /error ");
                let unchanged = request
                    .to_ascii_lowercase()
                    .contains("if-none-match: \"owned-v1\"");
                let (status, body) = if error {
                    ("404 Missing", "")
                } else if unchanged {
                    ("304 Not Modified", "")
                } else {
                    ("200 OK", "0.0.0.0 notification.example\n")
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nETag: \"owned-v1\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        Ok(Self { url, stop, task })
    }
}

#[tokio::test]
async fn real_download_updated_304_error_delivery_is_bounded_and_best_effort() -> Result<()> {
    let http = ConditionalHttp::start().await?;
    let dir = tempfile::tempdir()?;
    let mailbox = Arc::new(Mailbox::default());
    let mut receiver = mailbox.take_receiver().unwrap();
    let worker = Worker::start_with_notices(dir.path(), mailbox.clone());
    let h = worker.handle();
    set_config(&h, enabled()).await?;
    let id = save(&h, None, "Owned notices", format!("{}/ok", http.url)).await?;
    for (force, status) in [(true, "updated"), (false, "unchanged")] {
        let row = refresh(&h, id, force, status).await?;
        assert!(row.file.is_some());
        let notice = receiver.try_recv()?;
        assert_eq!(notice.target, Target::Subscriptions);
        assert_eq!(notice.node, "Owned notices");
        assert_eq!(notice.summary, format!("Owned notices: {status}"));
        assert_eq!(notice.title(), "Rustleblower — List subscriptions");
        assert_eq!(notice.target.action(), "open-subscriptions");
        assert_eq!(notice.urgency(), 1);
    }
    let bad = save(&h, None, "Owned failure", format!("{}/error", http.url)).await?;
    let row = refresh(&h, bad, true, "error").await?;
    assert!(row.file.is_none());
    let error = receiver.try_recv()?;
    assert_eq!(error.target, Target::Subscriptions);
    assert!(error.summary.starts_with("Owned failure failed:"));
    assert!(error.summary.len() <= MAX_MESSAGE);
    set_config(
        &h,
        Config {
            success_message: "<owned & custom>".into(),
            ..enabled()
        },
    )
    .await?;
    refresh(&h, id, true, "updated").await?;
    let custom = receiver.try_recv()?;
    assert_eq!(custom.summary, "<owned & custom>");
    assert!(custom.body().contains("&lt;owned &amp; custom&gt;"));
    for _ in 0..CAPACITY + 4 {
        refresh(&h, id, true, "updated").await?;
    }
    assert_eq!(mailbox.skipped.load(Ordering::Relaxed), 4);
    assert_eq!(
        (0..CAPACITY)
            .filter_map(|_| receiver.try_recv().ok())
            .count(),
        CAPACITY
    );
    assert!(receiver.try_recv().is_err());
    assert!(get(&h, id).await?.file.is_some());
    assert!(h.state().error.is_none());
    worker.join();
    Ok(())
}

#[tokio::test]
async fn cancellation_stale_edit_probe_and_disabled_config_produce_no_notice() -> Result<()> {
    let http = probe_http::Server::start().await;
    let dir = tempfile::tempdir()?;
    let mailbox = Arc::new(Mailbox::default());
    let mut receiver = mailbox.take_receiver().unwrap();
    let worker = Worker::start_with_notices(dir.path(), mailbox);
    let h = worker.handle();
    set_config(&h, enabled()).await?;
    let id = save(&h, None, "Owned slow", format!("{}/slow", http.url)).await?;
    req(&h, Request::Refresh { id, force: true }).await?;
    tokio::time::timeout(Duration::from_secs(3), async {
        while !http.held.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(5)).await
        }
    })
    .await?;
    req(&h, Request::Cancel).await?;
    idle(&h).await?;
    assert_eq!(get(&h, id).await?.status, "cancelled");
    assert!(receiver.try_recv().is_err());
    http.held.store(false, Ordering::Release);
    req(&h, Request::Refresh { id, force: true }).await?;
    tokio::time::timeout(Duration::from_secs(3), async {
        while !http.held.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(5)).await
        }
    })
    .await?;
    let old = get(&h, id).await?;
    save(
        &h,
        Some((id, old.revision)),
        "Owned renamed",
        format!("{}/hosts", http.url),
    )
    .await?;
    idle(&h).await?;
    assert!(
        receiver.try_recv().is_err(),
        "Edited in-flight response is not a normal download outcome"
    );
    let mut probe = h
        .test_url(format!("{}/hosts", http.url), Format::Hosts)
        .map_err(anyhow::Error::msg)?;
    let report = tokio::time::timeout(Duration::from_secs(3), probe.result())
        .await?
        .map_err(anyhow::Error::msg)?;
    assert_eq!(report.status, 200);
    assert!(receiver.try_recv().is_err());
    set_config(&h, Config::default()).await?;
    refresh(&h, id, true, "updated").await?;
    assert!(receiver.try_recv().is_err());
    http.release();
    worker.join();
    Ok(())
}
