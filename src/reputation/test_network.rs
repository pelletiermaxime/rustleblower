use super::*;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct Response {
    status: u16,
    body: String,
    delay: Duration,
    length: Option<usize>,
}
impl Response {
    fn report(mal: u64) -> Self {
        Self {
            status: 200,
            body: payload(mal),
            delay: Duration::ZERO,
            length: None,
        }
    }
}
struct Fixture {
    origin: String,
    responses: Arc<Mutex<VecDeque<Response>>>,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn start() -> Self {
        worker::fixture_guard("http://127.0.0.1:1").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let responses = Arc::new(Mutex::new(VecDeque::<Response>::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (queue, count, captured) = (responses.clone(), hits.clone(), requests.clone());
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = vec![0; 4096];
                let len = socket.read(&mut request).await.unwrap_or(0);
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request[..len]).into_owned());
                count.fetch_add(1, Ordering::SeqCst);
                let response = queue
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected owned fixture request");
                tokio::time::sleep(response.delay).await;
                let bytes = format!(
                    "HTTP/1.1 {} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.status,
                    response.length.unwrap_or(response.body.len()),
                    response.body
                );
                let _ = socket.write_all(bytes.as_bytes()).await;
            }
        });
        Self {
            origin,
            responses,
            hits,
            requests,
            task,
        }
    }
    fn push(&self, response: Response) {
        self.responses.lock().unwrap().push_back(response);
    }
    async fn configured(&self, settings: &Settings, directory: &Path) -> Worker {
        let path = directory.join("reputation.json");
        store::save(&path, &None, settings).unwrap();
        let worker = Worker::fixture(path, &self.origin).unwrap();
        let handle = worker.handle();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !handle.snapshot().ready {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        worker
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn done(ticket: &Ticket) -> ResultSet {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let result = ticket.snapshot();
            if !matches!(result.status, Status::Queued | Status::Checking) {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("reputation result deadline")
}
async fn started(fixture: &Fixture, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.hits.load(Ordering::SeqCst) < count {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_get_only_capture_cache_and_redacted_results() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let settings = settings(directory.path());
    let worker = fixture.configured(&settings, directory.path()).await;
    let handle = worker.handle();
    fixture.push(Response::report(2));
    let ticket = handle.lookup(query(1));
    let report = done(&ticket).await;
    assert_eq!(report.capture, query(1).capture);
    assert_eq!(report.status, Status::Complete);
    assert_eq!(report.reports[0].verdict, Verdict::Warning);
    assert!(!format!("{report:?}").contains(KEY));
    assert!(report.reports[0].details.contains('"'));
    assert!(
        report.reports[0]
            .details_url
            .starts_with("https://www.virustotal.com/gui/domain/")
    );
    let cached = handle.lookup(query(1));
    assert_eq!(done(&cached).await.reports[0].verdict, Verdict::Warning);
    assert_eq!(fixture.hits.load(Ordering::SeqCst), 1);
    assert_eq!(handle.snapshot().dispatched, 1);
    fixture.push(Response::report(0));
    let hash = handle.lookup(Query {
        capture: query(2).capture,
        subjects: vec![Subject::hash(&"a".repeat(64)).unwrap()],
    });
    assert_eq!(done(&hash).await.reports[0].verdict, Verdict::NotFlagged);
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.starts_with("GET /api/v3/") && r.to_ascii_lowercase().contains("x-apikey:"))
    );
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("/files/")
    );
    worker.join();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_unknown_errors_quota_and_disabled_never_become_clear() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let settings = settings(directory.path());
    let worker = fixture.configured(&settings, directory.path()).await;
    let handle = worker.handle();
    for (index, code, expected) in [
        (1, 404, Status::NotFound),
        (2, 401, Status::InvalidKey),
        (3, 500, Status::Failed),
    ] {
        fixture.push(Response {
            status: code,
            ..Response::report(0)
        });
        let result = done(&handle.lookup(query(index))).await;
        assert_eq!(result.status, expected);
        assert_eq!(result.reports[0].verdict, Verdict::Unknown);
    }
    fixture.push(Response {
        status: 429,
        ..Response::report(0)
    });
    assert_eq!(
        done(&handle.lookup(query(4))).await.status,
        Status::RateLimited
    );
    assert_eq!(
        done(&handle.lookup(query(5))).await.status,
        Status::RateLimited
    );
    assert_eq!(fixture.hits.load(Ordering::SeqCst), 4);
    worker.join();
    let other = directory.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let mut settings = settings.clone();
    settings.request_budget = 1;
    let worker = fixture.configured(&settings, &other).await;
    let handle = worker.handle();
    fixture.push(Response::report(0));
    assert_eq!(
        done(&handle.lookup(query(6))).await.status,
        Status::Complete
    );
    let result = done(&handle.lookup(query(7))).await;
    assert_eq!(result.status, Status::QuotaExceeded);
    assert_eq!(result.reports[0].verdict, Verdict::Unknown);
    worker.join();
    let disabled = directory.path().join("disabled");
    std::fs::create_dir(&disabled).unwrap();
    settings.enabled = false;
    let worker = fixture.configured(&settings, &disabled).await;
    assert_eq!(
        done(&worker.handle().lookup(query(8))).await.status,
        Status::Disabled
    );
    worker.join();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_queue_bound_reload_stale_cancel_and_quit_are_independent() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let settings = settings(directory.path());
    let worker = fixture.configured(&settings, directory.path()).await;
    let handle = worker.handle();
    fixture.push(Response {
        delay: Duration::from_secs(2),
        ..Response::report(1)
    });
    let first = handle.lookup(query(1));
    started(&fixture, 1).await;
    let queued = (2..=9)
        .map(|id| handle.lookup(query(id)))
        .collect::<Vec<_>>();
    assert_eq!(handle.lookup(query(10)).snapshot().status, Status::Busy);
    let before = std::time::Instant::now();
    handle.request(Command::Reload).await.unwrap().unwrap();
    assert!(before.elapsed() < Duration::from_millis(250));
    assert_eq!(done(&first).await.status, Status::Stale);
    for ticket in &queued {
        assert_eq!(done(ticket).await.status, Status::Stale);
    }
    assert_eq!(fixture.hits.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_secs(2)).await;
    fixture.push(Response {
        delay: Duration::from_secs(2),
        ..Response::report(1)
    });
    let cancel = handle.lookup(query(11));
    started(&fixture, 2).await;
    cancel.cancel();
    assert_eq!(done(&cancel).await.status, Status::Cancelled);
    let before = std::time::Instant::now();
    worker.join();
    assert!(before.elapsed() < Duration::from_millis(500));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_connection_capture_exclusions_and_bounded_body_timeout() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let settings = settings(directory.path());
    let worker = fixture.configured(&settings, directory.path()).await;
    let handle = worker.handle();
    let mut connection = crate::protocol::Connection {
        dst_host: "internal.lan".into(),
        dst_ip: "192.168.1.1".into(),
        process_checksums: std::collections::HashMap::from([(
            "process.hash.md5".into(),
            "b".repeat(32),
        )]),
        ..Default::default()
    };
    assert_eq!(
        handle
            .lookup_connection(query(1).capture, &connection)
            .snapshot()
            .status,
        Status::Excluded
    );
    assert_eq!(fixture.hits.load(Ordering::SeqCst), 0);
    connection.dst_ip = "203.0.113.1".into();
    connection.dst_host = "www.virustotal.com.".into();
    assert_eq!(
        handle
            .lookup_connection(query(1).capture, &connection)
            .snapshot()
            .status,
        Status::Excluded
    );
    fixture.push(Response {
        length: Some(MAX_BODY + 1),
        ..Response::report(0)
    });
    assert_eq!(done(&handle.lookup(query(2))).await.status, Status::Failed);
    fixture.push(Response {
        delay: Duration::from_millis(1200),
        ..Response::report(0)
    });
    assert_eq!(
        done(&handle.lookup(query(3))).await.status,
        Status::TimedOut
    );
    worker.join();
}
