use anyhow::{Result, anyhow};
use rustleblower::subscriptions::{
    self, Reply, Request, Worker,
    model::*,
    probe::{self, Report},
};
use std::{sync::atomic::Ordering, time::Duration};
mod fixture {
    include!("support/probe_http.rs");
}
async fn check(h: &subscriptions::Handle, url: String, format: Format) -> Result<Report> {
    let mut ticket = h.test_url(url, format).map_err(anyhow::Error::msg)?;
    tokio::time::timeout(Duration::from_secs(6), ticket.result())
        .await?
        .map_err(anyhow::Error::msg)
}
async fn ready(h: &subscriptions::Handle) -> Result<()> {
    match tokio::time::timeout(
        Duration::from_secs(2),
        h.request(Request::Settings).map_err(anyhow::Error::msg)?,
    )
    .await??
    .map_err(anyhow::Error::msg)?
    {
        Reply::Settings(..) => Ok(()),
        _ => Err(anyhow!("Unexpected reply")),
    }
}
#[test]
fn pinned_hosts_heuristic_and_url_bounds() -> Result<()> {
    let cases = vec![
        Vec::new(),
        vec!["<html>short</html>".into(); 10],
        vec!["<html>invalid</html>".into(); 11],
        vec!["# comment".into(); 200],
        vec!["example.test".into(); 20],
        vec!["127.0.0.1 localhost".into(); 20],
        vec![":: bad/host.example".into(); 20],
        (0..11)
            .map(|i| {
                if i < 7 {
                    "0.0.0.0 good.example"
                } else {
                    "invalid"
                }
                .into()
            })
            .collect(),
        (0..11)
            .map(|i| {
                if i < 6 {
                    "0.0.0.0 good.example"
                } else {
                    "invalid"
                }
                .into()
            })
            .collect(),
    ];
    let expected = [true, true, false, true, true, false, false, true, false];
    let mut evidence = Vec::new();
    for (lines, expected) in cases.iter().zip(expected) {
        let (like, short) = probe::hosts_like(lines);
        assert_eq!(like, expected);
        evidence.push(serde_json::json!({"lines":lines,"hosts_like":like,"short_sample":short}));
    }
    for bad in [
        "",
        "file:///tmp/list",
        "ftp://localhost/list",
        "http://user:password@localhost/list",
        "http://localhost/list#fragment",
        "http://localhost/\n",
    ] {
        assert!(probe::validate_url(bad).is_err());
    }
    assert!(probe::validate_url(&format!("http://localhost/{}", "x".repeat(2048))).is_err());
    assert!(probe::validate_url("https://example.test/path?q=value").is_ok());
    if let Some(path) = std::env::var_os("RUSTLEBLOWER_URL_TEST_EVIDENCE") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path)?;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join("rust-hosts-samples.json"))?;
        f.write_all(&serde_json::to_vec_pretty(&evidence)?)?;
    }
    Ok(())
}
#[tokio::test]
async fn head_fallback_redirect_samples_and_format_scope_never_publish() -> Result<()> {
    let server = fixture::Server::start().await;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    ready(&h).await?;
    for path in [
        "/hosts",
        "/reject-head",
        "/forbid-head",
        "/redirect",
        "/split",
    ] {
        let report = check(&h, format!("{}{path}", server.url), Format::Hosts).await?;
        assert_eq!(report.hosts_like, Some(true));
        assert!(!report.short_sample);
        assert_eq!(report.status, 200);
        assert_eq!(
            report.get_fallback,
            matches!(path, "/reject-head" | "/forbid-head")
        );
        assert_eq!(report.redirected, path == "/redirect");
    }
    let invalid = check(&h, format!("{}/bad", server.url), Format::Hosts).await?;
    assert_eq!(invalid.hosts_like, Some(false));
    let short = check(&h, format!("{}/short", server.url), Format::Hosts).await?;
    assert_eq!(short.hosts_like, Some(true));
    assert!(short.short_sample && short.message().contains("not conclusive"));
    for path in ["/long", "/many"] {
        let report = check(&h, format!("{}{path}", server.url), Format::Hosts).await?;
        assert!(report.sample_capped);
        assert!(
            report.sample_bytes <= probe::MAX_SAMPLE_BYTES
                && report.sample_lines <= probe::MAX_SAMPLE_LINES
        );
    }
    for format in [
        Format::Domains,
        Format::Ips,
        Format::Nets,
        Format::Regexp,
        Format::Md5,
    ] {
        let report = check(&h, format!("{}/domains", server.url), format).await?;
        assert!(report.hosts_like.is_none() && report.sample_bytes == 0);
    }
    let requests = server.requests.lock().unwrap().clone();
    assert!(
        requests.iter().any(|v| v == "HEAD /reject-head")
            && requests.iter().any(|v| v == "GET /reject-head")
    );
    assert!(!requests.iter().any(|v| v == "GET /domains"));
    let db = rusqlite::Connection::open(dir.path().join("subscriptions.sqlite3"))?;
    assert_eq!(
        db.query_row("SELECT count(*) FROM subscriptions", [], |r| r
            .get::<_, u64>(0))?,
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM logs", [], |r| r.get::<_, u64>(0))?,
        0
    );
    assert!(h.state().active.is_none());
    worker.join();
    Ok(())
}
#[tokio::test]
async fn cancel_drop_busy_and_worker_requests_remain_independent() -> Result<()> {
    let server = fixture::Server::start().await;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    ready(&h).await?;
    let mut ticket = h
        .test_url(format!("{}/slow", server.url), Format::Hosts)
        .map_err(anyhow::Error::msg)?;
    tokio::time::timeout(Duration::from_secs(2), async {
        while !server.held.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    assert!(
        check(&h, format!("{}/hosts", server.url), Format::Hosts)
            .await
            .unwrap_err()
            .to_string()
            .contains("already running")
    );
    ready(&h).await?;
    let rows = tokio::time::timeout(
        Duration::from_millis(500),
        h.request(Request::Page(Query::default()))
            .map_err(anyhow::Error::msg)?,
    )
    .await??
    .map_err(anyhow::Error::msg)?;
    assert!(matches!(rows,Reply::Page(p) if p.total==0));
    ticket.cancel();
    assert!(
        tokio::time::timeout(Duration::from_millis(500), ticket.result())
            .await?
            .unwrap_err()
            .contains("cancelled")
    );
    let ticket = h
        .test_url(format!("{}/slow", server.url), Format::Hosts)
        .map_err(anyhow::Error::msg)?;
    drop(ticket);
    assert_eq!(
        check(&h, format!("{}/hosts", server.url), Format::Hosts)
            .await?
            .hosts_like,
        Some(true)
    );
    server.release();
    worker.join();
    Ok(())
}
#[tokio::test]
async fn failures_timeouts_and_shutdown_release_the_probe_slot() -> Result<()> {
    let server = fixture::Server::start().await;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    ready(&h).await?;
    for path in ["/gone", "/loop", "/long-redirect", "/broken"] {
        assert!(
            check(&h, format!("{}{path}", server.url), Format::Hosts)
                .await
                .is_err(),
            "{path}"
        );
    }
    assert!(
        check(
            &h,
            format!("{}/hosts", server.url.replace("http://", "https://")),
            Format::Hosts
        )
        .await
        .is_err()
    );
    assert!(
        check(&h, format!("{}/slow-head", server.url), Format::Hosts)
            .await
            .is_err()
    );
    let mut ticket = h
        .test_url(format!("{}/slow", server.url), Format::Hosts)
        .map_err(anyhow::Error::msg)?;
    tokio::time::sleep(Duration::from_millis(20)).await;
    worker.join();
    assert!(
        tokio::time::timeout(Duration::from_millis(500), ticket.result())
            .await?
            .is_err()
    );
    Ok(())
}
#[tokio::test]
async fn permission_rpc_remains_independent_of_a_held_url_probe() -> Result<()> {
    use rustleblower::{
        backend::Backend,
        config::{Config, Decision},
        protocol::{self, ui_client::UiClient},
    };
    let server = fixture::Server::start().await;
    let dir = tempfile::tempdir()?;
    let cfg = Config {
        transport: Default::default(),
        database: dir.path().join("history.sqlite3"),
        socket: dir.path().join("ui.sock"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    };
    let backend = Backend::start(cfg.clone())?;
    let h = &backend.handle.subscriptions;
    ready(h).await?;
    let mut ticket = h
        .test_url(format!("{}/slow", server.url), Format::Hosts)
        .map_err(anyhow::Error::msg)?;
    tokio::time::timeout(Duration::from_secs(2), async {
        while !server.held.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let path = cfg.socket.clone();
    let channel = tonic::transport::Endpoint::try_from("http://localhost")?
        .connect_with_connector(tower::service_fn(move |_| {
            let path = path.clone();
            async move {
                tokio::net::UnixStream::connect(path)
                    .await
                    .map(hyper_util::rt::TokioIo::new)
            }
        }))
        .await?;
    let mut client = UiClient::new(channel);
    client
        .subscribe(protocol::ClientConfig {
            name: "url-probe-permission".into(),
            ..Default::default()
        })
        .await?;
    tokio::time::timeout(
        Duration::from_millis(500),
        client.ping(protocol::PingRequest {
            id: 1,
            ..Default::default()
        }),
    )
    .await??;
    let ask = tokio::spawn(async move {
        client
            .ask_rule(protocol::Connection {
                process_path: "/owned/url-probe".into(),
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
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(500), ask)
            .await???
            .into_inner()
            .action,
        "deny"
    );
    assert!(ticket.try_result().is_none());
    ticket.cancel();
    assert!(ticket.result().await.is_err());
    backend.join();
    assert!(!cfg.socket.exists());
    Ok(())
}

#[tokio::test]
async fn url_probe_and_cancellation_do_not_stop_an_existing_download() -> Result<()> {
    let server = fixture::Server::start().await;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    ready(&h).await?;
    let spec = Spec {
        name: "Owned direct list".into(),
        url: format!("{}/hosts", server.url),
        ..Default::default()
    };
    let id = match h
        .request(Request::Save {
            previous: None,
            spec,
        })
        .map_err(anyhow::Error::msg)?
        .await?
        .map_err(anyhow::Error::msg)?
    {
        Reply::Saved(id) => id,
        _ => return Err(anyhow!("Unexpected save reply")),
    };
    h.request(Request::Refresh { id, force: true })
        .map_err(anyhow::Error::msg)?
        .await?
        .map_err(anyhow::Error::msg)?;
    let get = |h: subscriptions::Handle| async move {
        match h
            .request(Request::Get(id))
            .map_err(anyhow::Error::msg)?
            .await?
            .map_err(anyhow::Error::msg)?
        {
            Reply::Row(row) => Ok::<_, anyhow::Error>(*row),
            _ => Err(anyhow!("Unexpected row reply")),
        }
    };
    let row = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let row = get(h.clone()).await?;
            if row.publication.is_some() {
                break Ok::<_, anyhow::Error>(row);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    let data = std::fs::read(row.file.as_ref().unwrap())?;
    assert!(!data.is_empty());
    let mut changed = row.spec.clone();
    changed.url = format!("{}/slow", server.url);
    h.request(Request::Save {
        previous: Some((id, row.revision)),
        spec: changed,
    })
    .map_err(anyhow::Error::msg)?
    .await?
    .map_err(anyhow::Error::msg)?;
    h.request(Request::Refresh { id, force: true })
        .map_err(anyhow::Error::msg)?
        .await?
        .map_err(anyhow::Error::msg)?;
    tokio::time::timeout(Duration::from_secs(2), async {
        while !server.held.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    let before = get(h.clone()).await?;
    assert_eq!(
        check(&h, format!("{}/hosts", server.url), Format::Hosts)
            .await?
            .hosts_like,
        Some(true)
    );
    let mut ticket = h
        .test_url(format!("{}/slow", server.url), Format::Hosts)
        .map_err(anyhow::Error::msg)?;
    tokio::time::sleep(Duration::from_millis(20)).await;
    ticket.cancel();
    assert!(ticket.result().await.is_err());
    assert_eq!(h.state().active, Some(id));
    let after = get(h.clone()).await?;
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.spec, before.spec);
    assert_eq!(std::fs::read(after.file.as_ref().unwrap())?, data);
    server.release();
    worker.join();
    Ok(())
}
