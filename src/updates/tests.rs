use super::*;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn defaults_intervals_versions_and_distinct_paths_are_explicit() {
    let mut settings = Settings::default();
    assert!(!settings.enabled);
    assert!(settings.check_on_start);
    assert_eq!(settings.duration().unwrap(), Duration::from_secs(43200));
    settings.interval = 0;
    assert!(settings.duration().is_err());
    settings.interval = u32::MAX;
    settings.unit = Unit::Days;
    assert!(settings.duration().is_err());
    assert_eq!(version("v1.9.0"), Version::parse("1.9.0").ok());
    assert!(version("unknown").is_none());
    assert!(
        version("1.9.0+foo")
            .unwrap()
            .cmp_precedence(&version("1.9.0+bar").unwrap())
            .is_eq()
    );
    assert!(
        version("1.9.0-rc.1")
            .unwrap()
            .cmp_precedence(&version("1.9.0").unwrap())
            .is_lt()
    );
    assert_ne!(
        path_for(Path::new("history.db")),
        path_for(Path::new("history.sqlite3"))
    );
    assert_eq!(
        path_for(Path::new("history.sqlite3")),
        PathBuf::from("history.sqlite3.updates.json")
    );
    assert!(
        TargetVersion {
            node: "bad\nname".into(),
            version: "1.0.0".into()
        }
        .bounded()
        .is_none()
    );
}

#[test]
fn preferences_preserve_invalid_stale_and_nonregular_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("updates.json");
    let settings = Settings::default();
    let revision = save(&path, &None, &settings).unwrap();
    assert_eq!(read(&path).unwrap().0, settings);
    assert!(save(&path, &None, &settings).is_err());
    assert!(save(&path, &revision, &settings).is_ok());
    std::fs::write(&path, b"broken secret payload").unwrap();
    assert!(save(&path, &revision, &settings).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"broken secret payload");
    std::fs::write(&path, vec![b' '; SETTINGS_BYTES + 1]).unwrap();
    assert!(read(&path).is_err());
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(read(&link).is_err());
    assert!(read(directory.path()).is_err());
    let race = directory.path().join("first-create-race.json");
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let threads = [false, true].map(|enabled| {
        let (barrier, race) = (barrier.clone(), race.clone());
        std::thread::spawn(move || {
            barrier.wait();
            save(
                &race,
                &None,
                &Settings {
                    enabled,
                    ..Settings::default()
                },
            )
            .is_ok()
        })
    });
    assert_eq!(
        threads
            .into_iter()
            .filter_map(|thread| thread.join().ok())
            .filter(|saved| *saved)
            .count(),
        1
    );
    assert!(read(&race).is_ok());
}

struct Response {
    status: u16,
    body: String,
    delay: Duration,
    declared: Option<usize>,
}
impl Response {
    fn json(body: &str) -> Self {
        Self {
            status: 200,
            body: body.into(),
            delay: Duration::ZERO,
            declared: None,
        }
    }
}
struct Fixture {
    endpoint: String,
    responses: Arc<Mutex<VecDeque<Response>>>,
    hits: Arc<AtomicUsize>,
    paths: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn start() -> Self {
        // No fixture may listen on the host network. Match the repository's
        // mapped-user owned network harness contract, with only loopback active.
        require_fixture_namespace().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let responses = Arc::new(Mutex::new(VecDeque::<Response>::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let (queue, counter, captured) = (responses.clone(), hits.clone(), paths.clone());
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = vec![0; 8192];
                let count = socket.read(&mut request).await.unwrap_or(0);
                captured.lock().unwrap().push(
                    String::from_utf8_lossy(&request[..count])
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string(),
                );
                counter.fetch_add(1, Ordering::SeqCst);
                let response = queue
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected HTTP request");
                tokio::time::sleep(response.delay).await;
                let wire = format!(
                    "HTTP/1.1 {} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.status,
                    response.declared.unwrap_or(response.body.len()),
                    response.body
                );
                let _ = socket.write_all(wire.as_bytes()).await;
            }
        });
        Self {
            endpoint,
            responses,
            hits,
            paths,
            task,
        }
    }
    fn push(&self, response: Response) {
        self.responses.lock().unwrap().push_back(response);
    }
    fn worker(&self, path: PathBuf, notify: Option<Notify>) -> Worker {
        Worker::start_at(path, notify, Endpoints::Fixture(self.endpoint.clone()))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn command(handle: &Handle, command: Command) -> State {
    handle.request(command).await.unwrap().unwrap()
}
async fn wait(handle: &Handle, predicate: impl Fn(&State) -> bool) -> State {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let state = handle.state();
            if predicate(&state) {
                return state;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("update state deadline")
}
async fn check_once(handle: &Handle) -> State {
    let previous = handle.state().completed_checks;
    command(handle, Command::CheckNow).await;
    wait(handle, |s| !s.busy && s.completed_checks > previous).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_manual_optin_release_precedence_and_notice_deduplication() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let output = notices.clone();
    let worker = fixture.worker(
        directory.path().join("updates.json"),
        Some(Arc::new(move |notice| output.lock().unwrap().push(notice))),
    );
    let handle = worker.handle();
    command(&handle, Command::Reload).await;
    assert!(
        handle
            .request(Command::Save {
                expected_revision: Some("a".repeat(65536)),
                settings: Settings::default()
            })
            .await
            .unwrap()
            .is_err()
    );
    assert!(
        handle
            .request(Command::Save {
                expected_revision: Some("z".repeat(64)),
                settings: Settings::default()
            })
            .await
            .unwrap()
            .is_err()
    );
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        fixture.hits.load(Ordering::SeqCst),
        0,
        "disabled must never check automatically"
    );
    fixture.push(Response::json(
        r#"{"tag_name":"v0.3.0","html_url":"https://untrusted.invalid/"}"#,
    ));
    let state = check_once(&handle).await;
    assert_eq!(state.outcome, Outcome::Available);
    assert_eq!(state.current.unwrap().version, APP_VERSION);
    assert_eq!(state.next_due, None);
    assert_eq!(
        state.release_url.as_deref(),
        Some("https://github.com/pelletiermaxime/rustleblower/releases/tag/v0.3.0")
    );
    assert_eq!(notices.lock().unwrap().len(), 1);
    command(&handle, Command::Reload).await;
    fixture.push(Response::json(r#"{"tag_name":"v0.3.0"}"#));
    assert_eq!(check_once(&handle).await.outcome, Outcome::Available);
    assert_eq!(notices.lock().unwrap().len(), 1);
    assert!(
        fixture
            .paths
            .lock()
            .unwrap()
            .iter()
            .all(|p| p.starts_with("GET /releases/latest "))
    );
    worker.join();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_stable_absence_rate_limit_body_limit_and_invalid_release() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let worker = fixture.worker(directory.path().join("updates.json"), None);
    let handle = worker.handle();
    for (response, expected) in [
        (
            Response {
                status: 404,
                ..Response::json("{}")
            },
            Outcome::NoStableRelease,
        ),
        (
            Response {
                status: 429,
                ..Response::json("{}")
            },
            Outcome::RateLimited,
        ),
        (
            Response {
                declared: Some(RESPONSE_BYTES + 1),
                ..Response::json("{}")
            },
            Outcome::Failed("Could not read release information from the official service".into()),
        ),
        (
            Response::json(r#"{"tag_name":"not a semantic version"}"#),
            Outcome::NoComparableRelease,
        ),
        (
            Response::json(r#"{"tag_name":"0.2.3+different"}"#),
            Outcome::Current,
        ),
        (
            Response::json(r#"{"tag_name":"0.3.0-rc.1","prerelease":true}"#),
            Outcome::NoComparableRelease,
        ),
    ] {
        command(&handle, Command::Reload).await;
        fixture.push(response);
        assert_eq!(check_once(&handle).await.outcome, expected);
    }
    worker.join();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_prerelease_scope_unknown_daemon_and_cancelled_stale_context() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let worker = fixture.worker(directory.path().join("updates.json"), None);
    let handle = worker.handle();
    command(&handle, Command::Reload).await;
    let settings = Settings {
        target: Target::SelectedDaemon,
        include_prereleases: true,
        ..Settings::default()
    };
    let state = command(
        &handle,
        Command::Save {
            expected_revision: None,
            settings,
        },
    )
    .await;
    command(&handle, Command::CheckNow).await;
    wait(&handle, |s| s.outcome == Outcome::UnknownCurrentVersion).await;
    assert_eq!(fixture.hits.load(Ordering::SeqCst), 0);
    let daemon = TargetVersion {
        node: "203.0.113.1/daemon".into(),
        version: "1.8.0".into(),
    };
    handle.set_daemon(Some(daemon.clone()));
    wait(&handle, |s| s.current.as_ref() == Some(&daemon)).await;
    fixture.push(Response::json(r#"[{"tag_name":"v1.9.0-rc.1","prerelease":true},{"tag_name":"2.0.0","draft":true},{"tag_name":"1.8.0"}]"#));
    assert_eq!(
        check_once(&handle).await.latest.as_deref(),
        Some("1.9.0-rc.1")
    );
    assert!(
        fixture
            .paths
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .starts_with("GET /releases?per_page=30 ")
    );
    command(&handle, Command::Reload).await;
    fixture.push(Response {
        delay: Duration::from_millis(350),
        ..Response::json(r#"[{"tag_name":"9.0.0"}]"#)
    });
    command(&handle, Command::CheckNow).await;
    wait(&handle, |s| s.busy).await;
    let before = std::time::Instant::now();
    command(&handle, Command::Cancel).await;
    assert!(before.elapsed() < Duration::from_millis(200));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(handle.state().outcome, Outcome::Cancelled);
    assert_eq!(handle.state().latest, None);
    assert!(
        handle
            .request(Command::Save {
                expected_revision: None,
                settings: Settings::default()
            })
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(handle.state().revision, state.revision);
    worker.join();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_settings_or_changed_daemon_drop_the_inflight_response() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let worker = fixture.worker(directory.path().join("updates.json"), None);
    let handle = worker.handle();
    command(&handle, Command::Reload).await;
    let daemon = TargetVersion {
        node: "test".into(),
        version: "1.8.0".into(),
    };
    handle.set_daemon(Some(daemon.clone()));
    command(
        &handle,
        Command::Save {
            expected_revision: None,
            settings: Settings {
                target: Target::SelectedDaemon,
                ..Settings::default()
            },
        },
    )
    .await;
    fixture.push(Response {
        delay: Duration::from_millis(300),
        ..Response::json(r#"{"tag_name":"9.0.0"}"#)
    });
    command(&handle, Command::CheckNow).await;
    wait(&handle, |s| s.busy).await;
    handle.set_daemon(Some(daemon.clone()));
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        handle.state().busy,
        "identical maintenance metadata must not cancel HTTP"
    );
    handle.set_daemon(Some(TargetVersion {
        version: "1.9.0".into(),
        ..daemon
    }));
    wait(&handle, |s| !s.busy).await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(handle.state().latest, None);
    fixture.push(Response {
        delay: Duration::from_millis(300),
        ..Response::json(r#"{"tag_name":"9.0.0"}"#)
    });
    command(&handle, Command::CheckNow).await;
    wait(&handle, |s| s.busy).await;
    command(
        &handle,
        Command::Save {
            expected_revision: handle.state().revision,
            settings: Settings::default(),
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(handle.state().outcome, Outcome::Disabled);
    assert_eq!(handle.state().latest, None);
    worker.join();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_start_and_interval_schedule_do_not_catch_up_or_overlap() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("updates.json");
    let settings = Settings {
        enabled: true,
        interval: 1,
        unit: Unit::Seconds,
        ..Settings::default()
    };
    save(&path, &None, &settings).unwrap();
    fixture.push(Response {
        delay: Duration::from_millis(200),
        ..Response::json(r#"{"tag_name":"0.3.0"}"#)
    });
    fixture.push(Response::json(r#"{"tag_name":"0.3.0"}"#));
    let worker = fixture.worker(path, None);
    let handle = worker.handle();
    wait(&handle, |s| s.completed_checks == 1).await;
    assert_eq!(fixture.hits.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(fixture.hits.load(Ordering::SeqCst), 1);
    wait(&handle, |s| s.completed_checks == 2).await;
    worker.join();
    let path = directory.path().join("delayed.json");
    save(
        &path,
        &None,
        &Settings {
            check_on_start: false,
            ..settings
        },
    )
    .unwrap();
    fixture.push(Response::json(r#"{"tag_name":"0.3.0"}"#));
    let worker = fixture.worker(path, None);
    let handle = worker.handle();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        fixture.hits.load(Ordering::SeqCst),
        2,
        "check_on_start=false must wait for its interval"
    );
    wait(&handle, |s| s.completed_checks == 1).await;
    worker.join();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_total_timeout_and_release_count_bound_are_enforced() {
    let fixture = Fixture::start().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let settings = Settings {
        include_prereleases: true,
        ..Settings::default()
    };
    let target = TargetVersion {
        node: "app".into(),
        version: APP_VERSION.into(),
    };
    fixture.push(Response::json(&format!(
        "[{}]",
        std::iter::repeat_n(r#"{"tag_name":"1.0.0"}"#, RELEASES + 1)
            .collect::<Vec<_>>()
            .join(",")
    )));
    let checked = check(
        &client,
        settings.clone(),
        target.clone(),
        Endpoints::Fixture(fixture.endpoint.clone()),
    )
    .await;
    assert!(matches!(checked.outcome, Outcome::Failed(_)));
    fixture.push(Response {
        delay: TIMEOUT + Duration::from_secs(1),
        ..Response::json("[]")
    });
    let before = std::time::Instant::now();
    let checked = check(
        &client,
        settings,
        target,
        Endpoints::Fixture(fixture.endpoint.clone()),
    )
    .await;
    assert_eq!(
        checked.outcome,
        Outcome::Failed("Release check timed out".into())
    );
    assert!(before.elapsed() >= TIMEOUT && before.elapsed() < TIMEOUT + Duration::from_secs(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_http_shutdown_drops_held_request_without_waiting_for_total_timeout() {
    let fixture = Fixture::start().await;
    let directory = tempfile::tempdir().unwrap();
    let worker = fixture.worker(directory.path().join("updates.json"), None);
    let handle = worker.handle();
    wait(&handle, |s| s.ready).await;
    fixture.push(Response {
        delay: TIMEOUT + Duration::from_secs(1),
        ..Response::json(r#"{"tag_name":"9.0.0"}"#)
    });
    command(&handle, Command::CheckNow).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.hits.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let before = std::time::Instant::now();
    worker.join();
    assert!(before.elapsed() < Duration::from_millis(500));
    assert!(!handle.state().busy);
    assert_eq!(handle.state().outcome, Outcome::Cancelled);
}
