use super::*;
use serde_json::{Value, json};
use std::{
    io::Write,
    os::unix::{fs::PermissionsExt, net::UnixStream},
    time::{Duration, Instant},
};

fn row(pid: u32, inode: u64, path: Option<&str>, sent: u64, received: u64) -> Value {
    json!({"pid":pid,"uid":1000,"start_ns":pid as u64 * 1000,"inode":inode,"device":30,
        "mount_id":33,"comm_hex":hex::encode("fixture"),"executable_hex":path.map(hex::encode),"sent":sent,"received":received})
}
fn frame(sample: u64, rows: Vec<Value>) -> Value {
    json!({"version":1,"started_ms":1000,"sample_ms":sample,"rows":rows,"lost_operations":0,"truncated":false})
}
fn ingest(engine: &mut Engine, frame: &Value) -> Result<()> {
    engine.ingest(&serde_json::to_vec(frame).unwrap())
}
fn view(engine: &Engine) -> Arc<View> {
    engine.view(&Query::default(), Status::Live, None, 1)
}

#[test]
fn rates_group_processes_and_preserve_paths_after_exit() {
    let mut engine = Engine::new(1000);
    ingest(
        &mut engine,
        &frame(1000, vec![row(1, 10, Some("/usr/bin/firefox"), 100, 500)]),
    )
    .unwrap();
    assert_eq!(view(&engine).rows[0].download, None);
    ingest(
        &mut engine,
        &frame(
            2000,
            vec![
                row(1, 10, None, 200, 1000),
                row(2, 10, Some("/usr/bin/firefox"), 300, 500),
            ],
        ),
    )
    .unwrap();
    let snapshot = view(&engine);
    assert_eq!(snapshot.rows.len(), 1);
    assert_eq!(snapshot.rows[0].download, Some(1000.));
    assert_eq!(snapshot.rows[0].upload, Some(400.));
    assert_eq!(snapshot.received, 1500);
    assert_eq!(snapshot.rows[0].path.as_deref(), Some("/usr/bin/firefox"));
    for status in [
        Status::Stale,
        Status::Disconnected,
        Status::Stopped,
        Status::Unavailable("fixture".into()),
    ] {
        let snapshot = engine.view(&Query::default(), status, None, 2);
        assert_eq!(snapshot.received, 1500);
        assert!(snapshot.rows[0].download.is_none());
    }
}

#[test]
fn reused_pid_with_a_new_lifetime_does_not_reset_existing_bytes() {
    let mut engine = Engine::new(1000);
    ingest(
        &mut engine,
        &frame(1000, vec![row(1, 10, Some("/usr/bin/a"), 100, 500)]),
    )
    .unwrap();
    let mut reused = row(1, 10, Some("/usr/bin/a"), 1, 2);
    reused["start_ns"] = json!(9000);
    ingest(
        &mut engine,
        &frame(2000, vec![row(1, 10, None, 100, 500), reused]),
    )
    .unwrap();
    assert_eq!(view(&engine).sent, 101);
    assert_eq!(view(&engine).rows[0].upload, Some(1.));
}

#[test]
fn inode_reuse_or_unknown_paths_never_inherit_another_apps_label() {
    let mut engine = Engine::new(1000);
    ingest(
        &mut engine,
        &frame(1000, vec![row(1, 10, Some("/usr/bin/old-app"), 100, 500)]),
    )
    .unwrap();
    ingest(
        &mut engine,
        &frame(
            2000,
            vec![
                row(1, 10, None, 100, 500),
                row(2, 10, Some("/usr/bin/new-app"), 1, 2),
                row(3, 10, None, 3, 4),
            ],
        ),
    )
    .unwrap();
    let snapshot = view(&engine);
    assert_eq!(snapshot.rows.len(), 3);
    assert_eq!(
        snapshot
            .rows
            .iter()
            .find(|r| r.name == "new-app")
            .unwrap()
            .sent,
        1
    );
    assert_eq!(
        snapshot
            .rows
            .iter()
            .find(|r| r.path.is_none())
            .unwrap()
            .sent,
        3
    );
}

#[test]
fn short_lived_curl_processes_group_and_exec_replacement_stays_separate() {
    let mut engine = Engine::new(1000);
    ingest(&mut engine, &frame(1000, vec![])).unwrap();
    let rows: Vec<_> = (1..=3)
        .map(|pid| {
            let mut record = row(
                pid,
                10,
                Some("/usr/bin/curl"),
                pid as u64 * 100,
                pid as u64 * 1000,
            );
            record["exec_id"] = json!(1);
            record
        })
        .collect();
    // First observations already have the paths captured before the curl exits.
    ingest(&mut engine, &frame(2000, rows.clone())).unwrap();
    let snapshot = view(&engine);
    assert_eq!(snapshot.matches, 1);
    assert_eq!(snapshot.rows[0].path.as_deref(), Some("/usr/bin/curl"));
    assert_eq!(snapshot.rows[0].received, 6000);
    assert_eq!(snapshot.rows[0].sent, 600);
    assert_eq!(snapshot.rows[0].download, Some(6000.));
    // Even the same PID/lifetime/inode/comm cannot reuse metadata after exec.
    let mut replacement = row(1, 10, Some("/usr/bin/other-tool"), 1, 99);
    replacement["exec_id"] = json!(2);
    let mut next = rows;
    next.push(replacement);
    ingest(&mut engine, &frame(3000, next)).unwrap();
    let snapshot = view(&engine);
    assert_eq!(snapshot.matches, 2);
    assert_eq!(
        snapshot
            .rows
            .iter()
            .find(|r| r.name == "curl")
            .unwrap()
            .received,
        6000
    );
    assert_eq!(
        snapshot
            .rows
            .iter()
            .find(|r| r.name == "other-tool")
            .unwrap()
            .received,
        99
    );
    assert_eq!(snapshot.received, 6099);
}

#[test]
fn bad_frames_are_rejected_atomically() {
    let initial = frame(1000, vec![row(1, 10, Some("/usr/bin/a"), 100, 500)]);
    let valid = frame(2000, vec![row(1, 10, None, 200, 600)]);
    let mut failures = Vec::new();
    let mut bad = valid.clone();
    bad["rows"][0]["sent"] = json!(99);
    failures.push(bad);
    let mut bad = valid.clone();
    bad["rows"][0]["uid"] = json!(1001);
    failures.push(bad);
    let mut bad = valid.clone();
    bad["rows"][0]["executable_hex"] = json!(hex::encode("relative"));
    failures.push(bad);
    let mut bad = valid.clone();
    bad["rows"][0]["executable_hex"] = json!("aa".repeat(513));
    failures.push(bad);
    let mut bad = valid.clone();
    bad["rows"][0]["comm_hex"] = json!("xx");
    failures.push(bad);
    let mut bad = valid.clone();
    bad["started_ms"] = json!(999);
    failures.push(bad);
    let mut bad = valid.clone();
    bad["sample_ms"] = json!(1000);
    failures.push(bad);
    let mut bad = valid.clone();
    bad["rows"] = json!([]);
    failures.push(bad);
    let mut bad = valid.clone();
    bad["rows"] = json!([bad["rows"][0].clone(), bad["rows"][0].clone()]);
    failures.push(bad);
    for bad in failures {
        let mut engine = Engine::new(1000);
        ingest(&mut engine, &initial).unwrap();
        assert!(ingest(&mut engine, &bad).is_err(), "{bad}");
        assert_eq!(view(&engine).sent, 100);
        assert_eq!(engine.sample, Some(1000));
    }
}

#[test]
fn incomplete_snapshots_keep_totals_and_warn() {
    let mut engine = Engine::new(1000);
    ingest(&mut engine, &frame(1000, vec![row(1, 10, None, 100, 500)])).unwrap();
    let mut partial = frame(2000, vec![]);
    partial["truncated"] = json!(true);
    partial["lost_operations"] = json!(2);
    ingest(&mut engine, &partial).unwrap();
    assert_eq!(view(&engine).sent, 100);
    assert!(view(&engine).rows[0].download.is_none());
    assert!(
        view(&engine)
            .warning
            .as_ref()
            .unwrap()
            .contains("2 lost operations")
    );
    let mut backwards = partial.clone();
    backwards["sample_ms"] = json!(3000);
    backwards["lost_operations"] = json!(1);
    assert!(ingest(&mut engine, &backwards).is_err());
    let mut recovered = frame(3000, vec![row(1, 10, None, 200, 700)]);
    recovered["lost_operations"] = json!(2);
    ingest(&mut engine, &recovered).unwrap();
    assert!(view(&engine).rows[0].download.is_none());
    recovered["sample_ms"] = json!(4000);
    recovered["rows"][0]["received"] = json!(800);
    ingest(&mut engine, &recovered).unwrap();
    assert_eq!(view(&engine).rows[0].download, Some(100.));
}

#[test]
fn numeric_sort_search_and_paging_cover_the_whole_snapshot() {
    let mut engine = Engine::new(1000);
    ingest(
        &mut engine,
        &frame(
            1000,
            (1..=225)
                .map(|i| {
                    row(
                        i,
                        i as u64,
                        Some(&format!("/bin/app-{i:03}")),
                        i as u64 * 100,
                        i as u64 * 200,
                    )
                })
                .collect(),
        ),
    )
    .unwrap();
    let mut query = Query {
        sort: Sort::Sent,
        ..Default::default()
    };
    let snapshot = engine.view(&query, Status::Live, None, 1);
    assert_eq!(snapshot.matches, 225);
    assert_eq!(snapshot.rows.len(), PAGE);
    assert_eq!(snapshot.rows[0].sent, 22500);
    query.offset = 200;
    assert_eq!(engine.view(&query, Status::Live, None, 2).rows.len(), 25);
    query.search = "app-00".into();
    query.offset = 0;
    assert_eq!(engine.view(&query, Status::Live, None, 3).matches, 9);
    assert_eq!(snapshot.sent, (1..=225u128).sum::<u128>() * 100);
}

#[test]
fn record_and_payload_caps_are_enforced() {
    let mut engine = Engine::new(1000);
    assert!(engine.ingest(&vec![b' '; MAX_FRAME + 1]).is_err());
    let maximum = frame(
        1000,
        (1..=MAX_RECORDS as u32)
            .map(|i| row(i, i as u64, None, 1, 1))
            .collect(),
    );
    ingest(&mut engine, &maximum).unwrap();
    let mut too_many = maximum.clone();
    too_many["sample_ms"] = json!(2000);
    too_many["rows"]
        .as_array_mut()
        .unwrap()
        .push(row(9999, 9999, None, 1, 1));
    assert!(ingest(&mut engine, &too_many).is_err());
    assert_eq!(view(&engine).sent, MAX_RECORDS as u128);
}

fn wait(client: &mut Client, predicate: impl Fn(&View) -> bool) -> Arc<View> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = None;
    while Instant::now() < deadline {
        if let Some(view) = client.snapshot() {
            if predicate(&view) {
                return view;
            }
            last = Some(view);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("usage worker did not reach expected state: {last:?}");
}
fn own_frame(sample: u64, sent: u64, received: u64) -> Value {
    let mut data = frame(
        sample,
        vec![row(1, 10, Some("/usr/bin/fixture"), sent, received)],
    );
    data["rows"][0]["uid"] = json!(unsafe { libc::getuid() });
    data
}
fn send(stream: &mut UnixStream, frame: &Value) {
    let mut bytes = serde_json::to_vec(frame).unwrap();
    bytes.push(b'\n');
    stream.write_all(&bytes).unwrap();
}

#[test]
fn worker_fragments_reconnects_and_cleans_only_owned_socket() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::start_fixture(dir.path().to_owned(), unsafe { libc::getuid() }, None);
    let waiting = wait(&mut client, |v| v.status == Status::Waiting);
    let path = waiting.endpoint.clone().unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let mut stream = UnixStream::connect(&path).unwrap();
    let data = serde_json::to_vec(&own_frame(1000, 100, 500)).unwrap();
    stream.write_all(&data[..data.len() / 2]).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    if let Some(view) = client.snapshot() {
        assert!(view.started_ms.is_none());
    }
    stream.write_all(&data[data.len() / 2..]).unwrap();
    stream.write_all(b"\n").unwrap();
    wait(&mut client, |v| v.received == 500);
    send(&mut stream, &own_frame(2000, 200, 1500));
    let live = wait(&mut client, |v| v.received == 1500);
    assert_eq!(live.rows[0].download, Some(1000.));
    drop(stream);
    let disconnected = wait(&mut client, |v| v.status == Status::Disconnected);
    assert_eq!(disconnected.received, 1500);
    assert!(disconnected.rows[0].download.is_none());
    let mut stream = UnixStream::connect(&path).unwrap();
    let mut next = own_frame(5000, 3, 7);
    next["started_ms"] = json!(5000);
    send(&mut stream, &next);
    let restarted = wait(&mut client, |v| v.started_ms == Some(5000));
    assert_eq!(restarted.received, 7);
    assert!(restarted.rows[0].download.is_none());
    // Never remove a different file installed at the old endpoint name.
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, "foreign").unwrap();
    client.stop();
    wait(&mut client, |v| v.status == Status::Stopped);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !client.finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(client.finished());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "foreign");
}

#[test]
fn worker_rejects_bad_frames_keeps_last_totals_and_reports_staleness() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::start_fixture(dir.path().to_owned(), unsafe { libc::getuid() }, None);
    let path = wait(&mut client, |v| v.status == Status::Waiting)
        .endpoint
        .clone()
        .unwrap();
    let mut stream = UnixStream::connect(&path).unwrap();
    send(&mut stream, &own_frame(1000, 1, 20));
    wait(&mut client, |v| v.received == 20);
    let stale = wait(&mut client, |v| v.status == Status::Stale);
    assert_eq!(stale.received, 20);
    assert!(stale.rows[0].download.is_none());
    send(&mut stream, &own_frame(2000, 0, 21));
    let invalid = wait(&mut client, |v| matches!(v.status, Status::Unavailable(_)));
    assert_eq!(invalid.received, 20);
    client.stop();
    wait(&mut client, |v| v.status == Status::Stopped);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !client.finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!path.exists());
}

fn launch_script(code: &str, marker: &std::path::Path) -> super::launcher::Launch {
    super::launcher::Launch {
        prepared: None,
        program: "/usr/bin/python3".into(),
        arguments: vec!["-c".into(), code.into(), marker.as_os_str().to_owned()],
    }
}
fn finish(client: &Client) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !client.finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(client.finished(), "collector worker failed to finish");
}

#[test]
fn packaged_collector_copy_is_private_bounded_and_rejects_unsafe_files() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let source = tempfile::tempdir().unwrap();
    let uid = unsafe { libc::getuid() };
    for name in ["collector", "accounting.bpf.o"] {
        std::fs::write(source.path().join(name), name).unwrap();
        std::fs::set_permissions(
            source.path().join(name),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
    }
    std::fs::create_dir(source.path().join("lib")).unwrap();
    std::fs::write(source.path().join("lib/libbpf.so.1"), "library").unwrap();
    let prepared = super::launcher::prepare(source.path(), uid).unwrap();
    let copied = prepared.path().to_owned();
    assert_eq!(std::fs::metadata(&copied).unwrap().mode() & 0o777, 0o700);
    assert_eq!(
        std::fs::metadata(copied.join("collector")).unwrap().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::read(copied.join("lib/libbpf.so.1")).unwrap(),
        b"library"
    );
    std::fs::write(source.path().join("collector"), "changed source").unwrap();
    assert_eq!(
        std::fs::read(copied.join("collector")).unwrap(),
        b"collector"
    );
    drop(prepared);
    assert!(!copied.exists());
    std::fs::set_permissions(
        source.path().join("collector"),
        std::fs::Permissions::from_mode(0o666),
    )
    .unwrap();
    assert!(super::launcher::prepare(source.path(), uid).is_err());
    std::fs::remove_file(source.path().join("collector")).unwrap();
    symlink("accounting.bpf.o", source.path().join("collector")).unwrap();
    assert!(super::launcher::prepare(source.path(), uid).is_err());
    std::fs::remove_file(source.path().join("collector")).unwrap();
    std::fs::File::create(source.path().join("collector"))
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    assert!(super::launcher::prepare(source.path(), uid).is_err());
}

#[test]
fn managed_authorization_exit_codes_and_missing_launcher_are_visible_and_clean() {
    for code in [0, 126, 127, 1] {
        let dir = tempfile::tempdir().unwrap();
        let launch = launch_script(
            &format!("import sys; sys.stderr.write('x'*100000); sys.exit({code})"),
            &dir.path().join("unused"),
        );
        let mut client = Client::start_fixture(
            dir.path().to_owned(),
            unsafe { libc::getuid() },
            Some(launch),
        );
        let snapshot = wait(&mut client, |v| {
            matches!(
                v.status,
                Status::Cancelled | Status::Ended | Status::Unavailable(_)
            )
        });
        match code {
            0 => assert_eq!(snapshot.status, Status::Ended),
            126 => assert_eq!(snapshot.status, Status::Cancelled),
            127 => assert!(snapshot.status.label().contains("authentication agent")),
            _ => assert!(snapshot.status.label().len() <= 4200),
        }
        assert!(snapshot.started_ms.is_none());
        finish(&client);
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }
    let dir = tempfile::tempdir().unwrap();
    let launch = super::launcher::Launch {
        prepared: None,
        program: dir.path().join("missing-pkexec"),
        arguments: vec![],
    };
    let mut client = Client::start_fixture(
        dir.path().to_owned(),
        unsafe { libc::getuid() },
        Some(launch),
    );
    let snapshot = wait(&mut client, |v| matches!(v.status, Status::Unavailable(_)));
    assert!(snapshot.status.label().contains("Install pkexec"));
    finish(&client);
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
}

#[test]
fn stopping_pending_authorization_reaps_the_owned_child() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("child.pid");
    let launch = launch_script(
        "import os,sys,time; open(sys.argv[1],'w').write(str(os.getpid())); time.sleep(600)",
        &marker,
    );
    let mut client = Client::start_fixture(
        dir.path().to_owned(),
        unsafe { libc::getuid() },
        Some(launch),
    );
    let snapshot = wait(&mut client, |v| v.status == Status::Authorizing);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let pid: i32 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
    client.stop();
    wait(&mut client, |v| v.status == Status::Stopped);
    finish(&client);
    assert!(!snapshot.endpoint.as_ref().unwrap().exists());
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[test]
fn managed_child_connects_and_delivers_measurements_before_owned_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("child.pid");
    let code = r#"
import json,os,socket,sys,time
open(sys.argv[1],'w').write(str(os.getpid()))
s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
s.connect(sys.argv[-2])
uid=int(sys.argv[-1])
for sample in (1000,2000):
    row=dict(pid=os.getpid(),uid=uid,start_ns=123,inode=456,device=30,mount_id=33,comm_hex='66697874757265',executable_hex='/usr/bin/fixture'.encode().hex(),sent=sample,received=sample*2)
    s.sendall((json.dumps(dict(version=1,started_ms=1000,sample_ms=sample,rows=[row],lost_operations=0,truncated=False))+'\n').encode())
    time.sleep(.05)
s.recv(1)
"#;
    let mut client = Client::start_fixture(
        dir.path().to_owned(),
        unsafe { libc::getuid() },
        Some(launch_script(code, &marker)),
    );
    let snapshot = wait(&mut client, |v| v.received == 4000);
    assert_eq!(snapshot.status, Status::Live);
    assert_eq!(snapshot.rows[0].download, Some(2000.));
    let pid: i32 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
    client.stop();
    wait(&mut client, |v| v.status == Status::Stopped);
    finish(&client);
    assert!(!snapshot.endpoint.as_ref().unwrap().exists());
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
