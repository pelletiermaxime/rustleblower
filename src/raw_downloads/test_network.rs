use super::*;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct Response {
    status: u16,
    body: Vec<u8>,
    delay: Duration,
    length: Option<u64>,
}
impl Response {
    fn body(body: &[u8]) -> Self {
        Self {
            status: 200,
            body: body.into(),
            delay: Duration::ZERO,
            length: None,
        }
    }
}
struct Fixture {
    origin: String,
    responses: Arc<Mutex<VecDeque<Response>>>,
    hits: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn start() -> Self {
        let map = std::fs::read_to_string("/proc/self/uid_map").unwrap();
        let fields: Vec<_> = map.split_whitespace().collect();
        assert!(
            fields.len() == 3 && fields[0] == "0" && fields[1] != "0" && fields[2] == "1",
            "Use scripts/owned_net.py"
        );
        let output = std::process::Command::new("ip")
            .args(["-j", "link", "show"])
            .output()
            .unwrap();
        let links: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).unwrap();
        assert!(output.status.success() && links.len() == 1 && links[0]["ifname"] == "lo");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/raw", listener.local_addr().unwrap());
        let responses = Arc::new(Mutex::new(VecDeque::<Response>::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let (queue, count) = (responses.clone(), hits.clone());
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut bytes = [0; 4096];
                let n = socket.read(&mut bytes).await.unwrap_or(0);
                assert!(bytes[..n].starts_with(b"GET /raw "));
                count.fetch_add(1, Ordering::SeqCst);
                let response = queue
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected owned HTTP request");
                tokio::time::sleep(response.delay).await;
                let header = format!(
                    "HTTP/1.1 {} fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.status,
                    response.length.unwrap_or(response.body.len() as u64)
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&response.body).await;
            }
        });
        Self {
            origin,
            responses,
            hits,
            task,
        }
    }
    fn push(&self, response: Response) {
        self.responses.lock().unwrap().push_back(response);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn ready(handle: &Handle) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !handle.snapshot().ready {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
async fn request(handle: &Handle, command: Command) -> Reply {
    handle.request(command).await.unwrap().unwrap()
}
async fn finished(handle: &Handle, previous: u64) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let state = handle.snapshot();
            if !state.busy && state.completed_runs > previous {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("raw batch completion deadline")
}
async fn run(handle: &Handle) -> Snapshot {
    let previous = handle.snapshot().completed_runs;
    request(
        handle,
        Command::RunNow {
            group: "owned resources".into(),
            expected_revision: handle.snapshot().revision,
        },
    )
    .await;
    finished(handle, previous).await
}
async fn start(
    directory: &Path,
    settings: &Settings,
    notify: Option<Arc<dyn Fn(Notice) + Send + Sync>>,
) -> Worker {
    let database = directory.join("history.db");
    store::save(&path_for(&database), &None, settings).unwrap();
    let worker = Worker::start(database, directory.join("ui.sock"), notify);
    ready(&worker.handle()).await;
    worker
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_raw_binary_last_good_atomic_errors_and_notices() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let group = group(directory.path(), &fixture.origin);
    let target = group.urls[0].localfile.clone();
    let settings = settings(group);
    let notices = Arc::new(Mutex::new(Vec::new()));
    let output = notices.clone();
    let worker = start(
        directory.path(),
        &settings,
        Some(Arc::new(move |notice| output.lock().unwrap().push(notice))),
    )
    .await;
    let handle = worker.handle();
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        fixture.hits.load(Ordering::SeqCst),
        0,
        "defaultOFF performs no automatic request"
    );
    assert!(
        handle
            .request(Command::RunNow {
                group: "owned resources".into(),
                expected_revision: None
            })
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(
        fixture.hits.load(Ordering::SeqCst),
        0,
        "stale review cannot write or contact HTTP"
    );
    fixture.push(Response::body(&[0, 1, 255, 2, 10]));
    assert_eq!(run(&handle).await.reports[0].status, Status::Published);
    assert_eq!(std::fs::read(&target).unwrap(), [0, 1, 255, 2, 10]);
    assert!(notices.lock().unwrap()[0].success);
    fixture.push(Response {
        status: 500,
        ..Response::body(b"error")
    });
    assert_eq!(run(&handle).await.reports[0].status, Status::Failed);
    assert_eq!(std::fs::read(&target).unwrap(), [0, 1, 255, 2, 10]);
    assert!(!notices.lock().unwrap()[1].success);
    assert_eq!(notices.lock().unwrap()[1].failed_names, ["resource"]);
    fixture.push(Response {
        length: Some(MAX_FILE + 1),
        ..Response::body(b"oversized")
    });
    assert_eq!(
        run(&handle).await.reports[0].resources[0].status,
        Status::TooLarge
    );
    assert_eq!(std::fs::read(&target).unwrap(), [0, 1, 255, 2, 10]);
    fixture.push(Response {
        length: Some(1000),
        ..Response::body(b"truncated")
    });
    assert_eq!(
        run(&handle).await.reports[0].resources[0].status,
        Status::Failed
    );
    assert_eq!(std::fs::read(&target).unwrap(), [0, 1, 255, 2, 10]);
    worker.join();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_cancel_and_external_destination_change_keep_last_good() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let group = group(directory.path(), &fixture.origin);
    let target = group.urls[0].localfile.clone();
    std::fs::write(&target, b"last-good").unwrap();
    let worker = start(directory.path(), &settings(group), None).await;
    let handle = worker.handle();
    fixture.push(Response {
        delay: Duration::from_millis(350),
        ..Response::body(b"remote")
    });
    let previous = handle.snapshot().completed_runs;
    request(
        &handle,
        Command::RunNow {
            group: "owned resources".into(),
            expected_revision: handle.snapshot().revision,
        },
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.hits.load(Ordering::SeqCst) < 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let before = std::time::Instant::now();
    request(&handle, Command::Cancel).await;
    assert_eq!(
        finished(&handle, previous).await.reports[0].status,
        Status::Cancelled
    );
    assert!(before.elapsed() < Duration::from_millis(250));
    assert_eq!(std::fs::read(&target).unwrap(), b"last-good");
    tokio::time::sleep(Duration::from_millis(400)).await;
    fixture.push(Response {
        delay: Duration::from_millis(250),
        ..Response::body(b"remote replacement")
    });
    let previous = handle.snapshot().completed_runs;
    request(
        &handle,
        Command::RunNow {
            group: "owned resources".into(),
            expected_revision: handle.snapshot().revision,
        },
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.hits.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    std::fs::write(&target, b"external-owner-data").unwrap();
    assert_eq!(
        finished(&handle, previous).await.reports[0].resources[0].status,
        Status::Changed
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"external-owner-data");
    worker.join();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_schedule_waits_first_interval_and_never_overlaps() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let mut group = group(directory.path(), &fixture.origin);
    group.interval = "1".into();
    let mut settings = settings(group);
    settings.enabled = true;
    fixture.push(Response {
        delay: Duration::from_millis(200),
        ..Response::body(b"scheduled")
    });
    let worker = start(directory.path(), &settings, None).await;
    let handle = worker.handle();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.hits.load(Ordering::SeqCst), 0);
    assert_eq!(
        finished(&handle, 0).await.reports[0].status,
        Status::Published
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.hits.load(Ordering::SeqCst), 1);
    worker.join();
}
