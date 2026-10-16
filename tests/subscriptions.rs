use anyhow::{Result, anyhow};
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    protocol::{self, ui_client::UiClient},
    subscriptions::{self, Handle, Reply, Request, Worker, model::*},
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
async fn request(h: &Handle, r: Request) -> Result<Reply> {
    tokio::time::timeout(
        Duration::from_secs(3),
        h.request(r).map_err(anyhow::Error::msg)?,
    )
    .await??
    .map_err(anyhow::Error::msg)
}
async fn save(h: &Handle, previous: Option<(u64, u64)>, spec: Spec) -> Result<u64> {
    match request(h, Request::Save { previous, spec }).await? {
        Reply::Saved(id) => Ok(id),
        _ => Err(anyhow!("Unexpected save reply")),
    }
}
async fn get(h: &Handle, id: u64) -> Result<Row> {
    match request(h, Request::Get(id)).await? {
        Reply::Row(r) => Ok(*r),
        _ => Err(anyhow!("Unexpected item reply")),
    }
}
async fn page(h: &Handle, q: Query) -> Result<subscriptions::model::Page> {
    match request(h, Request::Page(q)).await? {
        Reply::Page(p) => Ok(p),
        _ => Err(anyhow!("Unexpected page reply")),
    }
}
async fn bulk_review(
    h: &Handle,
    scope: subscriptions::bulk::Scope,
    patch: subscriptions::bulk::Patch,
) -> Result<subscriptions::bulk::Review> {
    match request(h, Request::ReviewBulk { scope, patch }).await? {
        Reply::BulkReview(review) => Ok(review),
        _ => Err(anyhow!("Unexpected bulk review reply")),
    }
}
async fn status(h: &Handle, id: u64, value: &str) -> Result<Row> {
    status_with_timeout(h, id, value, Duration::from_secs(5)).await
}
async fn status_with_timeout(h: &Handle, id: u64, value: &str, timeout: Duration) -> Result<Row> {
    tokio::time::timeout(timeout, async {
        loop {
            let r = get(h, id).await?;
            if r.status == value {
                return Ok(r);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}
fn spec(url: &str, name: &str) -> Spec {
    Spec {
        name: name.into(),
        url: url.into(),
        interval: Some(rustleblower::subscriptions::limits::Override::Absolute(60)),
        timeout: Some(rustleblower::subscriptions::limits::Override::Absolute(1)),
        max_bytes: Some(rustleblower::subscriptions::limits::Override::Absolute(
            1024,
        )),
        ..Default::default()
    }
}
struct Http {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Http {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Http {
    async fn start() -> Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let task = tokio::spawn(async move {
            let mut jobs = tokio::task::JoinSet::new();
            loop {
                tokio::select! {result=listener.accept()=>{let Ok((mut stream,_))=result else{break;};let log=log.clone();jobs.spawn(async move{
                    let mut request=Vec::new();let mut buffer=[0;1024];while request.len()<8192&&!request.windows(4).any(|v|v==b"\r\n\r\n"){let n=stream.read(&mut buffer).await.unwrap_or(0);if n==0{return;}request.extend_from_slice(&buffer[..n]);}
                    let text=String::from_utf8_lossy(&request).into_owned();let path=text.split_whitespace().nth(1).unwrap_or("/").to_owned();{let mut log=log.lock().unwrap();if log.len()<64{log.push(text.clone());}}
                    if path=="/slow"{let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n0.0.0.0 pending.example\n").await;tokio::time::sleep(Duration::from_secs(10)).await;return;}
                    if path=="/budget" {
                        let size=20*1024*1024;
                        let prefix=b"0.0.0.0 budget.example\n";
                        let header=format!("HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n");
                        if stream.write_all(header.as_bytes()).await.is_err() || stream.write_all(prefix).await.is_err(){return;}
                        let chunk=format!("0.0.0.0 {}\n", "a".repeat(55)).repeat(64);
                        let mut remaining=size-prefix.len();
                        while remaining>=chunk.len(){if stream.write_all(chunk.as_bytes()).await.is_err(){return;}remaining-=chunk.len();}
                        while remaining>=64{let line=format!("0.0.0.0 {}\n", "a".repeat(55));if stream.write_all(line.as_bytes()).await.is_err(){return;}remaining-=64;}
                        if remaining>0{let tail=format!("0.0.0.0 {}\n", "a".repeat(remaining-9));let _=stream.write_all(tail.as_bytes()).await;}
                        return;
                    }
                    if path=="/timeout"{tokio::time::sleep(Duration::from_secs(2)).await;return;}
                    let response=if path=="/large"{"HTTP/1.1 200 OK\r\nContent-Length: 999999\r\nConnection: close\r\n\r\n".into()}
                    else if path=="/chunked"{let body="x".repeat(2048);format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",body.len(),body)}
                    else if path=="/bad"{let body="<html>bad gateway</html>\n";format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len())}
                    else if path=="/error"{"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()}
                    else if text.to_lowercase().contains("if-none-match: \"fixture-v1\""){"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n".into()}
                    else{let body=match path.as_str(){
                        "/domains"=>"# comment\nExact.example\n*.wild.example\n.leading.example\napi[0-9].glob.example\n",
                        "/ips"|"/nets"=>"# entries\n192.0.2.1\n198.51.100.0/24\n2001:db8::/32\n",
                        "/md5"=>"ABCDEF0123456789ABCDEF0123456789 /file\n",
                        "/regexp"=>"# regex\n^regex[0-9]+\\.example$\n",
                        "/aliases"=>"127.0.0.1 localhost\n0.0.0.0 first.example alias.example # note\n::1 localhost\n",
                        _=>"0.0.0.0 blocked.example\n127.0.0.1 another.example\n"};format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"fixture-v1\"\r\nLast-Modified: Wed, 01 Jan 2025 00:00:00 GMT\r\nConnection: close\r\n\r\n{body}",body.len())};let _=stream.write_all(response.as_bytes()).await;
                });},_ = jobs.join_next(),if !jobs.is_empty()=>{}}
            }
        });
        Ok(Self {
            url,
            requests,
            task,
        })
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalogue_defaults_paging_full_sort_stale_revisions_removal_restore_and_locking()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    assert_eq!(page(&h, Query::default()).await?.total, 0);
    let settings = request(&h, Request::Settings).await?;
    let Reply::Settings(revision, mut settings) = settings else {
        panic!()
    };
    assert!(!settings.automatic);
    settings.interval = 60;
    settings.units.as_mut().unwrap().interval = rustleblower::subscriptions::limits::Unit::Seconds;
    request(
        &h,
        Request::SaveSettings {
            revision,
            settings: settings.clone(),
        },
    )
    .await?;
    assert!(
        request(&h, Request::SaveSettings { revision, settings })
            .await
            .is_err()
    );
    for i in 0..70 {
        save(
            &h,
            None,
            spec("http://127.0.0.1:1/list", &format!("List {i:03}")),
        )
        .await?;
    }
    let first = page(&h, Query::default()).await?;
    assert_eq!(first.rows.len(), 64);
    assert_eq!(first.rows[0].spec.name, "List 000");
    let second = page(
        &h,
        Query {
            offset: 64,
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(second.rows.len(), 6);
    let descending = page(
        &h,
        Query {
            descending: true,
            ..Default::default()
        },
    )
    .await?;
    assert_eq!(descending.rows[0].spec.name, "List 069");
    assert_eq!(
        page(
            &h,
            Query {
                search: "List 06".into(),
                ..Default::default()
            }
        )
        .await?
        .total,
        10
    );
    let row = first.rows[0].clone();
    request(
        &h,
        Request::Remove {
            id: row.id,
            revision: row.revision,
        },
    )
    .await?;
    assert_eq!(
        page(
            &h,
            Query {
                removed: true,
                ..Default::default()
            }
        )
        .await?
        .total,
        1
    );
    let removed = get(&h, row.id).await?;
    save(&h, Some((removed.id, removed.revision)), removed.spec).await?;
    assert!(
        save(&h, Some((row.id, row.revision)), row.spec)
            .await
            .is_err()
    );
    let other = Worker::start(dir.path());
    let other_handle = other.handle();
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert!(other_handle.state().error.is_some());
    assert!(
        request(&other_handle, Request::Page(Query::default()))
            .await
            .is_err()
    );
    other.join();
    for i in 70..128 {
        save(
            &h,
            None,
            spec("http://127.0.0.1:1/list", &format!("List {i:03}")),
        )
        .await?;
    }
    assert!(
        save(&h, None, spec("http://127.0.0.1:1/list", "overflow"))
            .await
            .is_err()
    );
    let mut invalid = spec("file:///etc/passwd", "invalid");
    assert!(
        h.request(Request::Save {
            previous: None,
            spec: invalid.clone()
        })
        .is_err()
    );
    invalid.url = "http://user:password@example.org/list".into();
    assert!(
        h.request(Request::Save {
            previous: None,
            spec: invalid
        })
        .is_err()
    );
    worker.join();
    let reopened = Worker::start(dir.path());
    assert_eq!(page(&reopened.handle(), Query::default()).await?.total, 128);
    reopened.join();
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streamed_download_validators_force_bounds_failures_backoff_and_last_good_data()
-> Result<()> {
    let http = Http::start().await?;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let id = save(&h, None, spec(&format!("{}/ok", http.url), "Owned list")).await?;
    request(&h, Request::Refresh { id, force: false }).await?;
    let row = status(&h, id, "updated").await?;
    let file = row.file.clone().unwrap();
    let good = std::fs::read(&file)?;
    assert!(row.bytes > 0 && row.bytes <= 1024);
    assert_eq!(row.etag, "\"fixture-v1\"");
    assert!(row.next_due - row.checked >= 60);
    request(&h, Request::Refresh { id, force: false }).await?;
    let unchanged = status(&h, id, "unchanged").await?;
    assert_eq!(unchanged.updated, row.updated);
    assert_eq!(std::fs::read(&file)?, good);
    assert!(
        http.requests.lock().unwrap()[1]
            .to_lowercase()
            .contains("if-none-match")
    );
    request(&h, Request::Refresh { id, force: true }).await?;
    status(&h, id, "updated").await?;
    assert!(
        !http.requests.lock().unwrap()[2]
            .to_lowercase()
            .contains("if-none-match")
    );
    for path in ["bad", "large", "chunked", "error", "timeout"] {
        let old = get(&h, id).await?;
        let mut config = old.spec;
        config.url = format!("{}/{path}", http.url);
        save(&h, Some((id, old.revision)), config).await?;
        request(&h, Request::Refresh { id, force: false }).await?;
        let failed = status(&h, id, "error").await?;
        assert!(!failed.error.is_empty());
        assert!(failed.next_due - failed.checked >= 119);
        assert_eq!(std::fs::read(&file)?, good, "preserve data on {path}");
        assert!(std::fs::read_dir(dir.path().join("tmp"))?.next().is_none());
    }
    let Reply::Logs(log) = request(&h, Request::Logs(id)).await? else {
        panic!()
    };
    assert!(log.len() <= 100 && log.iter().any(|s| s.contains("exceeds")));
    let Reply::Preview(text) = request(&h, Request::Preview(id)).await? else {
        panic!()
    };
    assert_eq!(text.as_bytes(), good);
    worker.join();
    let reopened = Worker::start(dir.path());
    assert_eq!(
        get(&reopened.handle(), id).await?.file.as_deref(),
        Some(file.as_str())
    );
    reopened.join();
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_request_cancel_edit_remove_and_shutdown_do_not_publish_obsolete_data() -> Result<()> {
    let http = Http::start().await?;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let mut config = spec(&format!("{}/slow", http.url), "Slow list");
    config.timeout = Some(rustleblower::subscriptions::limits::Override::Absolute(30));
    let id = save(&h, None, config).await?;
    for mode in ["cancel", "edit", "remove"] {
        let old = get(&h, id).await?;
        if old.removed {
            save(&h, Some((id, old.revision)), old.spec).await?;
        }
        request(&h, Request::Refresh { id, force: true }).await?;
        let waiting = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let state = h.state();
                if state.active == Some(id)
                    && state.downloaded.load(std::sync::atomic::Ordering::Relaxed) > 0
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        if let Err(error) = waiting {
            let row = get(&h, id).await?;
            let state = h.state();
            return Err(anyhow!(
                "{mode} waiting for streamed bytes: {error}; active {:?}, status {}, error {:?}, requests {:?}",
                state.active,
                row.status,
                state.error,
                http.requests.lock().unwrap()
            ));
        }
        match mode {
            "cancel" => {
                request(&h, Request::Cancel).await?;
            }
            "edit" => {
                let row = get(&h, id).await?;
                let mut spec = row.spec;
                spec.name = "Edited during request".into();
                save(&h, Some((id, row.revision)), spec).await?;
            }
            _ => {
                let row = get(&h, id).await?;
                request(
                    &h,
                    Request::Remove {
                        id,
                        revision: row.revision,
                    },
                )
                .await?;
            }
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while h.state().active.is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|e| anyhow!("{mode} cancellation did not finish: {e}"))?;
        assert!(get(&h, id).await?.file.is_none());
        assert!(std::fs::read_dir(dir.path().join("tmp"))?.next().is_none());
    }
    let row = get(&h, id).await?;
    save(&h, Some((id, row.revision)), row.spec).await?;
    request(&h, Request::Refresh { id, force: true }).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let start = Instant::now();
    worker.join();
    assert!(start.elapsed() < Duration::from_secs(2));
    Ok(())
}
async fn daemon(path: &Path) -> Result<UiClient<tonic::transport::Channel>> {
    let path = path.to_owned();
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
    Ok(UiClient::new(channel))
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_updates_wait_for_local_node_and_permissions_remain_independent() -> Result<()> {
    let http = Http::start().await?;
    let dir = tempfile::tempdir()?;
    let options = Config {
        socket: dir.path().join("ui.sock"),
        database: dir.path().join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    };
    let backend = Backend::start(options.clone())?;
    let h = &backend.handle.subscriptions;
    let mut config = spec(&format!("{}/slow", http.url), "Automatic local fixture");
    config.timeout = Some(rustleblower::subscriptions::limits::Override::Absolute(30));
    let id = save(h, None, config).await?;
    request(h, Request::Runtime(true)).await?;
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(http.requests.lock().unwrap().is_empty());
    let mut daemon = daemon(&options.socket).await?;
    daemon
        .subscribe(protocol::ClientConfig {
            name: "subscription-node".into(),
            ..Default::default()
        })
        .await?;
    let (tx, rx) = tokio::sync::mpsc::channel(2);
    let commands = daemon
        .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
        .await?
        .into_inner();
    tokio::time::timeout(Duration::from_secs(3), async {
        while h.state().active != Some(id)
            || h.state()
                .downloaded
                .load(std::sync::atomic::Ordering::Relaxed)
                == 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    tokio::time::timeout(
        Duration::from_millis(500),
        daemon.ping(protocol::PingRequest::default()),
    )
    .await??;
    let mut asking = daemon.clone();
    let ask = tokio::spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/subscription-prompt".into(),
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
    request(h, Request::Runtime(false)).await?;
    status(h, id, "cancelled").await?;
    let Reply::Settings(_, settings) = request(h, Request::Settings).await? else {
        panic!()
    };
    assert!(!settings.automatic);
    drop(commands);
    drop(tx);
    drop(daemon);
    backend.join();
    assert!(!options.socket.exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unavailable_subscription_storage_does_not_prevent_grpc_or_permissions() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("subscriptions"),
        "owned unavailable fixture",
    )?;
    let options = Config {
        socket: dir.path().join("ui.sock"),
        database: dir.path().join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    };
    let backend = Backend::start(options.clone())?;
    let mut daemon = daemon(&options.socket).await?;
    daemon
        .subscribe(protocol::ClientConfig {
            name: "unavailable-subscriptions".into(),
            ..Default::default()
        })
        .await?;
    tokio::time::timeout(Duration::from_secs(1), async {
        while backend.handle.subscriptions.state().error.is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    tokio::time::timeout(
        Duration::from_millis(500),
        daemon.ping(protocol::PingRequest::default()),
    )
    .await??;
    let mut asking = daemon.clone();
    let ask = tokio::spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/isolated-failure-prompt".into(),
                ..Default::default()
            })
            .await
    });
    tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            if let Some(prompt) = backend.handle.prompts.snapshot().first() {
                backend.handle.decide(prompt.id, Decision::Deny);
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
        "deny"
    );
    drop(daemon);
    backend.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn published_disk_budget_rejects_candidate_and_preserves_last_good_file() -> Result<()> {
    let http = Http::start().await?;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let Reply::Settings(revision, mut settings) = request(&h, Request::Settings).await? else {
        panic!()
    };
    settings.disk_bytes = 20 * 1024 * 1024;
    request(&h, Request::SaveSettings { revision, settings }).await?;
    let mut ids = Vec::new();
    for name in ["Other retained list", "Budget replacement"] {
        let id = save(&h, None, spec(&format!("{}/ok", http.url), name)).await?;
        request(&h, Request::Refresh { id, force: true }).await?;
        status(&h, id, "updated").await?;
        ids.push(id);
    }
    let id = ids[1];
    let old = get(&h, id).await?;
    let file = old.file.clone().unwrap();
    let good = std::fs::read(&file)?;
    let mut config = old.spec;
    config.url = format!("{}/budget", http.url);
    config.max_bytes = Some(rustleblower::subscriptions::limits::Override::Absolute(
        20 * 1024 * 1024,
    ));
    config.timeout = Some(rustleblower::subscriptions::limits::Override::Absolute(30));
    save(&h, Some((id, old.revision)), config).await?;
    request(&h, Request::Refresh { id, force: true }).await?;
    // This case streams and normalizes 20 MiB before rejecting publication.
    // Its completion allowance is separate from the strict prompt/Ping limits.
    let failed = status_with_timeout(&h, id, "error", Duration::from_secs(60)).await?;
    assert!(failed.error.contains("disk budget"), "{}", failed.error);
    assert_eq!(failed.bytes, good.len() as u64);
    assert_eq!(std::fs::read(file)?, good);
    assert!(std::fs::read_dir(dir.path().join("tmp"))?.next().is_none());
    assert!(h.state().error.is_none());
    worker.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stored_preview_paths_are_bounded_owned_and_regular_files() -> Result<()> {
    let http = Http::start().await?;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let id = save(&h, None, spec(&format!("{}/ok", http.url), "Path fixture")).await?;
    request(&h, Request::Refresh { id, force: true }).await?;
    let row = status(&h, id, "updated").await?;
    let file = row.file.unwrap();
    let external = dir.path().join("unrelated.txt");
    std::fs::write(&external, "unrelated data must never appear in preview")?;
    let db = rusqlite::Connection::open(dir.path().join("subscriptions.sqlite3"))?;
    for path in [external.to_string_lossy().to_string(), "x".repeat(4097)] {
        db.execute(
            "UPDATE subscriptions SET file=? WHERE id=?",
            rusqlite::params![path, id],
        )?;
        assert!(request(&h, Request::Preview(id)).await.is_err());
        assert!(page(&h, Query::default()).await.is_err());
    }
    db.execute(
        "UPDATE subscriptions SET file=? WHERE id=?",
        rusqlite::params![file, id],
    )?;
    std::fs::remove_file(&file)?;
    std::os::unix::fs::symlink(&external, &file)?;
    assert!(request(&h, Request::Preview(id)).await.is_err());
    assert_eq!(
        std::fs::read_to_string(&external)?,
        "unrelated data must never appear in preview"
    );
    worker.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_publications_keep_stable_format_directories_and_all_retained_bytes() -> Result<()>
{
    use rustleblower::subscriptions::attachment;
    let http = Http::start().await?;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let mut config = spec(&format!("{}/domains", http.url), "Canonical fixture");
    config.format = Format::Domains;
    config.filename = "without-extension".into();
    let id = save(&h, None, config).await?;
    let mut publications = Vec::new();
    for (format, path, expected) in [
        (
            Format::Domains,
            "domains",
            "0.0.0.0 exact.example\n0.0.0.0 *.wild.example\n0.0.0.0 .leading.example\n0.0.0.0 api[0-9].glob.example\n",
        ),
        (
            Format::Ips,
            "ips",
            "192.0.2.1\n198.51.100.0/24\n2001:db8::/32\n",
        ),
        (
            Format::Nets,
            "nets",
            "192.0.2.1\n198.51.100.0/24\n2001:db8::/32\n",
        ),
        (Format::Md5, "md5", "abcdef0123456789abcdef0123456789\n"),
        (Format::Regexp, "regexp", "^regex[0-9]+\\.example$\n"),
        (
            Format::Hosts,
            "aliases",
            "0.0.0.0 first.example\n0.0.0.0 alias.example\n",
        ),
    ] {
        let before = get(&h, id).await?;
        let mut config = before.spec;
        config.format = format.clone();
        config.url = format!("{}/{path}", http.url);
        save(&h, Some((id, before.revision)), config).await?;
        assert!(
            get(&h, id).await?.publication.is_none(),
            "Changed format/URL must require a compatible publication"
        );
        request(&h, Request::Refresh { id, force: true }).await?;
        let row = status(&h, id, "updated").await?;
        let publication = row.publication.as_ref().unwrap();
        assert_eq!(std::fs::read_to_string(&publication.file)?, expected);
        assert_eq!(publication.bytes, expected.len() as u64);
        assert!(Path::new(&publication.file).extension().is_some());
        assert_eq!(
            Path::new(&publication.file)
                .parent()
                .unwrap()
                .file_name()
                .unwrap(),
            format.directory()
        );
        let rule = attachment::draft(&row).map_err(anyhow::Error::msg)?;
        if let Some(output) = std::env::var_os("RUSTLEBLOWER_LIST_FIXTURE_EVIDENCE") {
            let output = std::path::PathBuf::from(output);
            std::fs::create_dir_all(&output)?;
            let destination = output.join(format!("{}.txt", format.key()));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)?;
            std::io::Write::write_all(&mut file, std::fs::read(&publication.file)?.as_slice())?;
        }

        let root = rule.operator.as_ref().unwrap();
        assert_eq!(root.r#type, "list");
        assert_eq!(root.operand, "list");
        assert_eq!(root.list.len(), 2);
        assert_eq!(
            root.list[0],
            protocol::Operator {
                r#type: "simple".into(),
                operand: "user.id".into(),
                data: rustix::process::getuid().as_raw().to_string(),
                sensitive: true,
                ..Default::default()
            }
        );
        assert_eq!(root.list[1].operand, format.operand());
        assert_eq!(root.list[1].r#type, "lists");
        assert_eq!(
            root.list[1].data,
            Path::new(&publication.file)
                .parent()
                .unwrap()
                .to_string_lossy()
        );
        assert!(attachment::references(&rule, publication));
        assert_eq!(rule.action, "deny");
        assert_eq!(rule.duration, "always");
        if format != Format::Hosts {
            publications.push(publication.clone());
        }
    }
    // Hosts and domain names share the daemon's domain reader/directory; the
    // other four typed publications retain their data for existing rules.
    for publication in &publications[1..] {
        assert!(Path::new(&publication.file).is_file());
    }
    let db = rusqlite::Connection::open(dir.path().join("subscriptions.sqlite3"))?;
    assert_eq!(
        db.query_row("SELECT count(*) FROM subscription_publications", [], |r| {
            r.get::<_, u64>(0)
        })?,
        5
    );
    let retained = db.query_row(
        "SELECT sum(bytes) FROM subscription_publications",
        [],
        |r| r.get::<_, u64>(0),
    )?;
    assert!(retained > get(&h, id).await?.bytes);
    worker.join();
    let reopened = Worker::start(dir.path());
    assert!(get(&reopened.handle(), id).await?.publication.is_some());
    reopened.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attached_rule_inspection_scans_full_confirmed_catalogue_and_preserves_nested_operators()
-> Result<()> {
    use rustleblower::{
        protocol::{Operator, Rule},
        subscriptions::{attachment, model::Publication},
    };
    let dir = tempfile::tempdir()?;
    let options = Config {
        socket: dir.path().join("ui.sock"),
        database: dir.path().join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    };
    let backend = Backend::start(options.clone())?;
    let publication = Publication {
        file: dir
            .path()
            .join("sources/1/domains/download-data.txt")
            .to_string_lossy()
            .into_owned(),
        bytes: 12,
        format: Format::Hosts,
        url: "http://127.0.0.1:9/fixture".into(),
    };
    let op = Operator {
        r#type: "lists".into(),
        operand: publication.format.operand().into(),
        data: attachment::directory(&publication)
            .map_err(anyhow::Error::msg)?
            .to_string_lossy()
            .into_owned(),
        sensitive: false,
        ..Default::default()
    };
    let mut inventory = Vec::new();
    for index in 0..201 {
        inventory.push(Rule {
            name: format!("aaa-unrelated-{index:03}"),
            enabled: true,
            action: "allow".into(),
            duration: "always".into(),
            operator: Some(Operator {
                r#type: "simple".into(),
                operand: "dest.host".into(),
                data: "other.example".into(),
                ..Default::default()
            }),
            ..Default::default()
        });
    }
    for index in 0..20 {
        let operator = if index % 2 == 0 {
            Operator {
                r#type: "list".into(),
                operand: "list".into(),
                list: vec![
                    op.clone(),
                    Operator {
                        r#type: "simple".into(),
                        operand: "user.id".into(),
                        data: "1000".into(),
                        sensitive: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }
        } else {
            op.clone()
        };
        inventory.push(Rule {
            name: format!("zzz-attached-{index:03}"),
            enabled: index % 2 == 0,
            action: "deny".into(),
            duration: "always".into(),
            operator: Some(operator),
            ..Default::default()
        });
    }
    let mut node = daemon(&options.socket).await?;
    node.subscribe(protocol::ClientConfig {
        name: "attached-node".into(),
        rules: inventory.clone(),
        ..Default::default()
    })
    .await?;
    backend.handle.history.flush().await?;
    let first = attachment::page(&options.database, "attached-node", &publication, 0)?;
    assert_eq!(first.total, 20);
    assert_eq!(first.rules.len(), 16);
    assert_eq!(first.rules[0], inventory[201]);
    let second = attachment::page(&options.database, "attached-node", &publication, 16)?;
    assert_eq!(second.total, 20);
    assert_eq!(second.rules.len(), 4);
    assert_eq!(second.rules[0], inventory[217]);
    assert_eq!(
        attachment::page(&options.database, "other-node", &publication, 0)?.total,
        0
    );
    let mut changed = publication.clone();
    changed.format = Format::Ips;
    assert_eq!(
        attachment::page(&options.database, "attached-node", &changed, 0)?.total,
        0
    );
    changed = publication.clone();
    changed.file = dir
        .path()
        .join("sources/2/domains/download-data.txt")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        attachment::page(&options.database, "attached-node", &changed, 0)?.total,
        0
    );
    let proposal = Rule {
        name: "popup-only-proposal".into(),
        operator: Some(op),
        ..Default::default()
    };
    let db = rusqlite::Connection::open(&options.database)?;
    use prost::Message;
    db.execute(
        "INSERT INTO prompt_rules VALUES(?,?,?,?,?,?)",
        rusqlite::params![
            "attached-node",
            proposal.name,
            proposal.encode_to_vec(),
            "",
            "",
            "always"
        ],
    )?;
    assert_eq!(
        attachment::page(&options.database, "attached-node", &publication, 0)?.total,
        20,
        "Unobserved popup proposals must be excluded"
    );
    backend.join();
    assert!(!options.socket.exists());
    Ok(())
}

async fn collections(
    h: &Handle,
    search: &str,
    offset: usize,
) -> Result<rustleblower::subscriptions::collections::Page> {
    match request(
        h,
        Request::Collections(rustleblower::subscriptions::collections::Query {
            search: search.into(),
            offset,
            ..Default::default()
        }),
    )
    .await?
    {
        Reply::Collections(page) => Ok(page),
        _ => Err(anyhow!("Unexpected collection page")),
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_collections_normalize_groups_switch_generations_and_retain_source_data() -> Result<()>
{
    use rustleblower::subscriptions::collections as c;
    let http = Http::start().await?;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    assert_eq!(collections(&h, "", 0).await?.total, 5);
    let mut ids = Vec::new();
    for (name, path, format, groups) in [
        (
            "One",
            "ok",
            Format::Hosts,
            vec!["Work", "Work", "ALL", ".."],
        ),
        ("Two", "aliases", Format::Hosts, vec!["Work", "Other"]),
        ("IPs", "ips", Format::Ips, vec!["Work"]),
    ] {
        let mut config = spec(&format!("{}/{path}", http.url), name);
        config.format = format;
        config.groups = groups.into_iter().map(str::to_owned).collect();
        let id = save(&h, None, config).await?;
        request(&h, Request::Refresh { id, force: true }).await?;
        status(&h, id, "updated").await?;
        ids.push(id);
    }
    let catalogue = collections(&h, "", 0).await?;
    assert_eq!(catalogue.total, 8);
    let find = |group: Option<&str>, kind: &str| {
        catalogue
            .rows
            .iter()
            .find(|r| r.group.as_deref() == group && r.format.directory() == kind)
            .unwrap()
            .clone()
    };
    let all = find(None, "domains");
    let work = find(Some("work"), "domains");
    assert_eq!(all.members, 2);
    assert_eq!(work.members, 2);
    assert_eq!(find(Some("work"), "ips").members, 1);
    assert_eq!(
        std::fs::read_dir(&work.directory)?.count(),
        3,
        "Two sources and one stable refresh marker"
    );
    assert_eq!(
        std::fs::read(work.directory.join(format!("{}.txt", ids[0])))?,
        std::fs::read(get(&h, ids[0]).await?.file.unwrap())?
    );
    let rule = c::draft(&work).map_err(anyhow::Error::msg)?;
    let op = rule.operator.unwrap();
    assert_eq!(op.list[0].operand, "user.id");
    assert_eq!(
        op.list[0].data,
        rustix::process::getuid().as_raw().to_string()
    );
    assert_eq!(op.list[1].data, work.directory.to_string_lossy());
    assert_eq!(op.list[1].operand, "lists.domains");
    let pointer = dir.path().join("collections/current");
    let before = std::fs::read_link(&pointer)?;
    let row = get(&h, ids[0]).await?;
    let mut config = row.spec;
    config.name = "Name only".into();
    save(&h, Some((row.id, row.revision)), config).await?;
    assert_eq!(
        std::fs::read_link(&pointer)?,
        before,
        "Metadata-only rename does not reload unchanged collections"
    );
    let previous_marker = std::fs::metadata(work.directory.join(c::REFRESH_MARKER))?.modified()?;
    let row = get(&h, ids[0]).await?;
    let old_file = row.file.clone().unwrap();
    let mut config = row.spec;
    config.enabled = false;
    save(&h, Some((row.id, row.revision)), config).await?;
    assert_ne!(std::fs::read_link(&pointer)?, before);
    assert!(!before.exists());
    assert!(Path::new(&old_file).is_file());
    assert!(!work.directory.join(format!("{}.txt", ids[0])).exists());
    assert!(work.directory.join(format!("{}.txt", ids[1])).exists());
    assert!(
        std::fs::metadata(work.directory.join(c::REFRESH_MARKER))?.modified()? > previous_marker
    );
    let row = get(&h, ids[1]).await?;
    let mut config = row.spec;
    config.url = format!("{}/bad", http.url);
    save(&h, Some((row.id, row.revision)), config).await?;
    assert!(
        work.directory.join(format!("{}.txt", ids[1])).exists(),
        "Changed URL keeps last valid collection data"
    );
    let row = get(&h, ids[1]).await?;
    request(
        &h,
        Request::Remove {
            id: row.id,
            revision: row.revision,
        },
    )
    .await?;
    assert!(!work.directory.exists());
    assert!(all.directory.exists());
    assert_eq!(std::fs::read_dir(&all.directory)?.count(), 1);
    assert_eq!(
        collections(&h, "work", 0).await?.rows.len(),
        1,
        "Only the IP collection remains active"
    );
    worker.join();
    let reopened = Worker::start(dir.path());
    assert_eq!(
        collections(&reopened.handle(), "work", 0).await?.rows[0].format,
        Format::Ips
    );
    reopened.join();
    for (raw, expected) in [
        (" __Work Group.. ", Some("work-group")),
        ("../a/b", Some("a-b")),
        ("ALL", None),
        ("...", None),
    ] {
        assert_eq!(c::normalize_group(raw).as_deref(), expected);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_collection_pointer_is_preserved_and_does_not_stop_grpc() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("subscriptions");
    std::fs::create_dir_all(root.join("collections"))?;
    let pointer = root.join("collections/current");
    std::fs::write(&pointer, "unrelated file")?;
    let options = Config {
        socket: dir.path().join("ui.sock"),
        database: dir.path().join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    };
    let backend = Backend::start(options.clone())?;
    tokio::time::timeout(Duration::from_secs(2), async {
        while backend.handle.subscriptions.state().error.is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert_eq!(std::fs::read_to_string(&pointer)?, "unrelated file");
    let mut node = daemon(&options.socket).await?;
    node.subscribe(protocol::ClientConfig {
        name: "collection-failure".into(),
        ..Default::default()
    })
    .await?;
    tokio::time::timeout(
        Duration::from_millis(500),
        node.ping(protocol::PingRequest::default()),
    )
    .await??;
    backend.join();
    assert!(!options.socket.exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn collection_search_and_pages_cover_all_normalized_groups() -> Result<()> {
    let http = Http::start().await?;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    for index in 0..10 {
        let mut config = spec(&format!("{}/ok", http.url), &format!("Source {index}"));
        config.groups = (index * 8..index * 8 + 8)
            .map(|g| format!("group-{g:03}"))
            .collect();
        let id = save(&h, None, config).await?;
        request(&h, Request::Refresh { id, force: true }).await?;
        status(&h, id, "updated").await?;
    }
    let first = collections(&h, "", 0).await?;
    assert_eq!(first.total, 85);
    assert_eq!(first.rows.len(), 64);
    let Reply::Collections(sorted) = request(
        &h,
        Request::Collections(rustleblower::subscriptions::collections::Query {
            sort: rustleblower::subscriptions::collections::Sort::Members,
            descending: true,
            ..Default::default()
        }),
    )
    .await?
    else {
        panic!()
    };
    assert_eq!(sorted.total, 85);
    assert_eq!(sorted.rows[0].members, 10);
    assert!(sorted.rows[0].group.is_none());
    let Reply::Collection(target) = request(
        &h,
        Request::Collection {
            group: Some("group-079".into()),
            format: Format::Hosts,
        },
    )
    .await?
    else {
        panic!()
    };
    assert_eq!(target.members, 1);
    assert_eq!(target.group.as_deref(), Some("group-079"));
    assert_eq!(target.format, Format::Domains);
    assert!(
        request(
            &h,
            Request::Collection {
                group: Some("missing-group".into()),
                format: Format::Domains
            }
        )
        .await
        .is_err()
    );
    let second = collections(&h, "", 64).await?;
    assert_eq!(second.total, 85);
    assert_eq!(second.rows.len(), 21);
    assert_eq!(second.rows[0].group.as_deref(), Some("group-059"));
    let search = collections(&h, "group-079", 0).await?;
    assert_eq!(search.total, 1);
    assert_eq!(search.rows[0].group.as_deref(), Some("group-079"));
    assert_eq!(search.rows[0].members, 1);
    assert!(collections(&h, &"x".repeat(257), 0).await.is_err());
    worker.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bulk_frozen_global_scope_stale_guard_and_sql_rollback_preserve_all_metadata() -> Result<()>
{
    use subscriptions::bulk::{Groups, Patch, Scope};
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let mut ids = Vec::new();
    for i in 0..70 {
        let mut value = spec("http://127.0.0.1:1/no-request", &format!("Bulk {i:03}"));
        value.filename = format!("list-{i}.txt");
        value.groups = vec!["Work".into(), "work".into(), "Other".into(), "WORK".into()];
        ids.push(save(&h, None, value).await?);
    }
    let unrelated = save(
        &h,
        None,
        spec("http://127.0.0.1:1/unused", "Unrelated source"),
    )
    .await?;
    let patch = Patch {
        enabled: Some(false),
        groups: Groups::Add(vec!["New Group".into()]),
        interval: Some(None),
        timeout: Some(Some(123)),
        max_bytes: Some(Some(4096)),
        ..Default::default()
    };
    let review = bulk_review(&h, Scope::Matching("Bulk".into()), patch.clone()).await?;
    assert_eq!((review.scanned, review.entries.len()), (70, 70));
    let last = save(&h, None, spec("http://127.0.0.1:1/new", "Bulk later")).await?;
    let row = get(&h, ids[69]).await?;
    let mut changed = row.spec.clone();
    changed.filename = "concurrent.txt".into();
    save(&h, Some((row.id, row.revision)), changed.clone()).await?;
    let error = request(&h, Request::ApplyBulk(review))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("changed since review"), "{error}");
    assert!(get(&h, ids[0]).await?.spec.enabled);
    assert_eq!(get(&h, ids[69]).await?.spec, changed);
    let review = bulk_review(&h, Scope::Selected(ids.clone()), patch).await?;
    let db = rusqlite::Connection::open(dir.path().join("subscriptions.sqlite3"))?;
    db.execute_batch(&format!("CREATE TRIGGER owned_bulk_failure BEFORE UPDATE ON subscriptions WHEN OLD.id={} BEGIN SELECT RAISE(ABORT,'owned late failure'); END;",ids[69]))?;
    let pointer = std::fs::read_link(dir.path().join("collections/current"))?;
    assert!(
        request(&h, Request::ApplyBulk(review.clone()))
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("owned late failure")
    );
    assert_eq!(
        get(&h, ids[0]).await?.revision,
        1,
        "An error on the last row rolls back earlier writes"
    );
    assert_eq!(
        std::fs::read_link(dir.path().join("collections/current"))?,
        pointer
    );
    let count: i64 = db.query_row(
        "SELECT count(*) FROM logs WHERE message='Updated by a reviewed bulk subscription change'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(count, 0);
    db.execute_batch("DROP TRIGGER owned_bulk_failure;")?;
    assert!(matches!(
        request(&h, Request::ApplyBulk(review.clone())).await?,
        Reply::BulkApplied(70)
    ));
    for entry in &review.entries {
        let row = get(&h, entry.id).await?;
        assert_eq!(row.spec, entry.after);
        assert_eq!(row.revision, entry.revision + 1);
        assert_eq!((row.checked, row.bytes, row.file), (0, 0, None));
        assert_eq!(row.spec.groups, vec!["work", "other", "new-group"]);
        assert_eq!(
            row.spec.filename,
            if row.id == ids[69] {
                "concurrent.txt".into()
            } else {
                format!("list-{}.txt", row.id - 1)
            }
        );
    }
    assert!(
        get(&h, last).await?.spec.enabled,
        "A new matching row is outside the frozen reviewed scope"
    );
    assert!(
        get(&h, unrelated).await?.spec.enabled,
        "Matching scope excludes unrelated names"
    );
    let noop = bulk_review(&h, Scope::Selected(ids.clone()), review.patch.clone()).await?;
    assert_eq!((noop.scanned, noop.entries.len()), (70, 0));
    assert!(
        request(
            &h,
            Request::ReviewBulk {
                scope: Scope::Selected(vec![ids[0], ids[0]]),
                patch: Default::default()
            }
        )
        .await
        .is_err()
    );
    let mut tampered = review;
    tampered.entries[0].after.url = "http://localhost/tamper".into();
    assert!(h.request(Request::ApplyBulk(tampered)).is_err());
    worker.join();
    let reopened = Worker::start(dir.path());
    assert!(!get(&reopened.handle(), ids[0]).await?.spec.enabled);
    reopened.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bulk_group_limits_fail_before_writes_and_format_refresh_is_explicit() -> Result<()> {
    use subscriptions::bulk::{Groups, Patch, Scope};
    let http = Http::start().await?;
    let dir = tempfile::tempdir()?;
    let worker = Worker::start(dir.path());
    let h = worker.handle();
    let mut one = spec(&format!("{}/ok", http.url), "First");
    one.groups = vec!["Work".into()];
    let a = save(&h, None, one).await?;
    let mut two = spec(&format!("{}/ok", http.url), "Last");
    two.groups = (0..8).map(|i| format!("group-{i}")).collect();
    let b = save(&h, None, two).await?;
    for id in [a, b] {
        request(&h, Request::Refresh { id, force: true }).await?;
        status(&h, id, "updated").await?;
    }
    let before = get(&h, a).await?;
    let bytes = std::fs::read(before.file.as_ref().unwrap())?;
    let pointer = dir.path().join("collections/current");
    let old = std::fs::read_link(&pointer)?;
    assert!(
        bulk_review(
            &h,
            Scope::Selected(vec![a, b]),
            Patch {
                groups: Groups::Add(vec!["extra".into()]),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(get(&h, a).await?.spec, before.spec);
    assert_eq!(std::fs::read_link(&pointer)?, old);
    let review = bulk_review(
        &h,
        Scope::Selected(vec![a, b]),
        Patch {
            enabled: Some(false),
            ..Default::default()
        },
    )
    .await?;
    request(&h, Request::ApplyBulk(review)).await?;
    assert_ne!(std::fs::read_link(&pointer)?, old);
    assert!(
        collections(&h, "", 0)
            .await?
            .rows
            .iter()
            .all(|c| c.members == 0)
    );
    assert_eq!(std::fs::read(before.file.as_ref().unwrap())?, bytes);
    assert_eq!(
        get(&h, a).await?.etag,
        before.etag,
        "Membership edits keep validators"
    );
    let review = bulk_review(
        &h,
        Scope::Selected(vec![a, b]),
        Patch {
            format: Some(Format::Domains),
            groups: Groups::Remove(vec!["WORK".into()]),
            ..Default::default()
        },
    )
    .await?;
    request(&h, Request::ApplyBulk(review)).await?;
    assert!(get(&h, a).await?.etag.is_empty());
    assert!(get(&h, a).await?.modified.is_empty());
    assert!(get(&h, a).await?.spec.groups.is_empty());
    assert_eq!(get(&h, b).await?.spec.groups.len(), 8);
    let review = bulk_review(
        &h,
        Scope::Selected(vec![a]),
        Patch {
            groups: Groups::Replace(vec!["Test".into(), "test".into()]),
            ..Default::default()
        },
    )
    .await?;
    request(&h, Request::ApplyBulk(review)).await?;
    assert_eq!(get(&h, a).await?.spec.groups, vec!["test"]);
    let review = bulk_review(
        &h,
        Scope::Selected(vec![a]),
        Patch {
            groups: Groups::Clear,
            ..Default::default()
        },
    )
    .await?;
    request(&h, Request::ApplyBulk(review)).await?;
    assert!(get(&h, a).await?.spec.groups.is_empty());
    assert_eq!(
        http.requests.lock().unwrap().len(),
        2,
        "Bulk edits perform no implicit download"
    );
    worker.join();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permissions_reply_while_a_real_bulk_sql_transaction_is_busy() -> Result<()> {
    use subscriptions::bulk::{Patch, Scope};
    let dir = tempfile::tempdir()?;
    let options = Config {
        socket: dir.path().join("ui.sock"),
        database: dir.path().join("history.sqlite3"),
        prompt_timeout: 30,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    };
    let backend = Backend::start(options.clone())?;
    let h = &backend.handle.subscriptions;
    let mut daemon = daemon(&options.socket).await?;
    daemon
        .subscribe(protocol::ClientConfig {
            name: "bulk-permission-fixture".into(),
            ..Default::default()
        })
        .await?;
    let a = save(h, None, spec("http://127.0.0.1:1/unused", "Busy bulk A")).await?;
    let b = save(h, None, spec("http://127.0.0.1:1/unused", "Busy bulk B")).await?;
    let review = bulk_review(
        h,
        Scope::Selected(vec![a, b]),
        Patch {
            enabled: Some(false),
            ..Default::default()
        },
    )
    .await?;
    let db = rusqlite::Connection::open(h.root.join("subscriptions.sqlite3"))?;
    db.busy_timeout(Duration::ZERO)?;
    // Only this owned DB is delayed. Constant-memory SQL holds the real worker's
    // actual write transaction while production RPCs and permission decisions run.
    db.execute_batch(&format!("CREATE TRIGGER owned_bulk_delay BEFORE UPDATE ON subscriptions WHEN OLD.id={a} BEGIN SELECT (WITH RECURSIVE steps(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM steps WHERE x<20000000) SELECT max(x) FROM steps); END;"))?;
    let mut applying = h
        .request(Request::ApplyBulk(review))
        .map_err(anyhow::Error::msg)?;
    let busy = || -> Result<bool> {
        match db.execute_batch("BEGIN IMMEDIATE;") {
            Ok(()) => {
                db.execute_batch("ROLLBACK;")?;
                Ok(false)
            }
            Err(e) if e.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy) => Ok(true),
            Err(e) => Err(e.into()),
        }
    };
    // Give the worker the first write opportunity. Repeated BEGIN probes before
    // it upgrades its read snapshot can themselves cause SQLITE_BUSY_SNAPSHOT.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        busy()?,
        "The owned trigger must be holding the actual write lock"
    );
    assert!(matches!(
        applying.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
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
    let answer = tokio::time::timeout(Duration::from_millis(500), ask)
        .await???
        .into_inner();
    assert_eq!(answer.action, "allow");
    assert!(
        busy()?,
        "The actual subscription SQL transaction must still overlap the permission reply"
    );
    assert!(matches!(
        applying.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(30), applying)
            .await??
            .map_err(anyhow::Error::msg)?,
        Reply::BulkApplied(2)
    ));
    assert!(!get(h, a).await?.spec.enabled);
    assert!(!get(h, b).await?.spec.enabled);
    db.execute_batch("DROP TRIGGER owned_bulk_delay;")?;
    drop(db);
    drop(daemon);
    backend.join();
    assert!(!options.socket.exists());
    Ok(())
}
