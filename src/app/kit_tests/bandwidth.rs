use super::*;
use crate::bandwidth::{Sort, Status};
use std::{io::Write, os::unix::net::UnixStream};

fn frame(started: u64, sample: u64, scale: u64, count: u32) -> Vec<u8> {
    let uid = unsafe { libc::getuid() };
    let rows = (1..=count).map(|pid| serde_json::json!({
        "pid":pid,"uid":uid,"start_ns":pid as u64*1000,"inode":pid,"device":30,"mount_id":33,
        "comm_hex":hex::encode("fixture"),"executable_hex":hex::encode(format!("/usr/bin/app-{pid:03}")),
        "sent":scale*100*pid as u64,"received":scale*200*pid as u64,
    })).collect::<Vec<_>>();
    let mut bytes = serde_json::to_vec(&serde_json::json!({"version":1,"started_ms":started,
        "sample_ms":sample,"rows":rows,"lost_operations":0,"truncated":false}))
    .unwrap();
    bytes.push(b'\n');
    bytes
}

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let daemon = runtime.block_on(async {
        use hyper_util::rt::TokioIo;
        let path = config.socket.clone();
        let channel = tonic::transport::Endpoint::try_from("http://localhost")
            .unwrap()
            .connect_with_connector(tower::service_fn(move |_| {
                let path = path.clone();
                async move {
                    tokio::net::UnixStream::connect(path)
                        .await
                        .map(TokioIo::new)
                }
            }))
            .await
            .unwrap();
        let mut daemon = protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(protocol::ClientConfig {
                name: "bandwidth-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    let mut cx = context(test_cx);
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(crate::app::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    // Replace only the external authorization program. Start still creates and
    // owns a real child; input and the production receiver stay unchanged.
    cx.update_entity(&view, |view, _| {
        view.bandwidth.launch_fixture = Some(crate::bandwidth::launcher::Launch {
            prepared: None,
            program: "/usr/bin/python3".into(),
            arguments: vec!["-c".into(), "import time; time.sleep(600)".into()],
        });
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-8", cx))
        .unwrap();
    flush(&mut cx);
    assert!(cx.read_entity(&view, |v, _| v.bandwidth.snapshot.started_ms.is_none()));
    checkpoint(&mut cx, window, &evidence.join("01-not-monitoring.json"));
    click(&mut cx, window, "bandwidth-start");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.bandwidth.snapshot.status == Status::Authorizing
        })
    });
    let endpoint = cx.read_entity(&view, |v, _| v.bandwidth.snapshot.endpoint.clone().unwrap());
    click(&mut cx, window, "bandwidth-address");
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        endpoint.to_string_lossy()
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("02-waiting-for-collector.json"),
    );
    let mut stream = UnixStream::connect(&endpoint).unwrap();
    stream.write_all(&frame(1000, 1000, 1, 225)).unwrap();
    stream.write_all(&frame(1000, 2000, 2, 225)).unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.bandwidth.snapshot.received == 10_170_000)
    });
    cx.read_entity(&view, |v, _| {
        let snapshot = &v.bandwidth.snapshot;
        assert_eq!(snapshot.matches, 225);
        assert_eq!(snapshot.rows.len(), 100);
        assert_eq!(snapshot.rows[0].name, "app-225");
        assert_eq!(snapshot.rows[0].download, Some(45000.));
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("03-live-application-rates.json"),
    );
    click(&mut cx, window, "bandwidth-next");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.bandwidth.snapshot.query.offset == 100)
    });
    assert_eq!(
        cx.read_entity(&view, |v, _| v.bandwidth.snapshot.rows[0].name.clone()),
        "app-125"
    );
    checkpoint(&mut cx, window, &evidence.join("04-global-page-two.json"));
    fill(&mut cx, window, "bandwidth-search", "app-00");
    assert_eq!(
        cx.read_entity(&view, |v, _| v.bandwidth.snapshot.matches),
        9
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("05-filtered-applications.json"),
    );
    fill(&mut cx, window, "bandwidth-search", "");
    for _ in 0..2 {
        cx.update_window(window, |_, window, cx| {
            window.within("table").click(("icon-sort", 4usize), cx)
        })
        .unwrap();
        flush(&mut cx);
    }
    cx.read_entity(&view, |v, _| {
        assert_eq!(v.bandwidth.query.sort, Sort::Sent);
        assert!(v.bandwidth.query.ascending);
        assert_eq!(v.bandwidth.snapshot.rows[0].sent, 200);
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("06-numeric-upload-sort.json"),
    );
    // Collection continues while a different feature is displayed.
    cx.update_window(window, |_, window, cx| window.press("ctrl-1", cx))
        .unwrap();
    stream.write_all(&frame(1000, 3000, 3, 225)).unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.bandwidth.snapshot.received == 15_255_000)
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-8", cx))
        .unwrap();
    flush(&mut cx);
    // Hold a real partial collector frame while answering an unrelated RPC.
    let next = frame(1000, 4000, 4, 225);
    stream.write_all(&next[..next.len() / 2]).unwrap();
    let mut permission = daemon.clone();
    let ask = runtime.spawn(async move {
        permission
            .ask_rule(protocol::Connection {
                process_path: "/owned/bandwidth-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    let popup = permission_window(&mut cx, &view);
    checkpoint(
        &mut cx,
        popup,
        &evidence.join("07-independent-permission.json"),
    );
    click(&mut cx, popup, "deny-once");
    let reply = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), ask)
            .await
            .unwrap()
            .unwrap()
    });
    assert_eq!(reply.action, "deny");
    stream.write_all(&next[next.len() / 2..]).unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.bandwidth.snapshot.received == 20_340_000)
    });
    stream.write_all(b"{\"broken\":true}\n").unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            matches!(v.bandwidth.snapshot.status, Status::Unavailable(_))
        })
    });
    cx.read_entity(&view, |v, _| {
        assert_eq!(v.bandwidth.snapshot.received, 20_340_000);
        assert!(
            v.bandwidth
                .snapshot
                .rows
                .iter()
                .all(|row| row.download.is_none())
        );
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("08-unavailable-retains-totals.json"),
    );
    drop(stream);
    let mut stream = UnixStream::connect(&endpoint).unwrap();
    stream.write_all(&frame(9000, 9000, 1, 2)).unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.bandwidth.snapshot.started_ms == Some(9000))
    });
    cx.read_entity(&view, |v, _| {
        assert_eq!(v.bandwidth.snapshot.received, 600);
        assert!(v.bandwidth.snapshot.rows[0].download.is_none());
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("09-new-monitoring-session.json"),
    );
    let mut usage: serde_json::Value = serde_json::from_slice(&frame(9000, 10000, 1, 2)).unwrap();
    for index in 1..=3u64 {
        usage["rows"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "pid":10000+index,"uid":unsafe { libc::getuid() },"start_ns":index*1000,
                "inode":9999,"device":30,"mount_id":33,"exec_id":1,
                "comm_hex":hex::encode("curl"),"executable_hex":hex::encode("/usr/bin/curl"),
                "sent":index*100,"received":index*1000,
            }));
    }
    let mut combined = serde_json::to_vec(&usage).unwrap();
    combined.push(b'\n');
    stream.write_all(&combined).unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| v.bandwidth.snapshot.received == 6600)
    });
    cx.read_entity(&view, |v, _| {
        assert_eq!(v.bandwidth.snapshot.matches, 3);
        let curls = v
            .bandwidth
            .snapshot
            .rows
            .iter()
            .filter(|row| row.name == "curl")
            .collect::<Vec<_>>();
        assert_eq!(curls.len(), 1);
        assert_eq!(curls[0].path.as_deref(), Some("/usr/bin/curl"));
        assert_eq!(curls[0].received, 6000);
        assert_eq!(curls[0].sent, 600);
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("09b-short-lived-app-totals.json"),
    );
    click(&mut cx, window, "bandwidth-stop");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.bandwidth.client.as_ref().unwrap().finished()
        })
    });
    assert!(!endpoint.exists());
    assert_eq!(
        cx.read_entity(&view, |v, _| v.bandwidth.snapshot.status.clone()),
        Status::Stopped
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("10-stopped-owned-cleanup.json"),
    );
    drop(stream);
    cx.update_entity(&view, |view, _| {
        view.bandwidth.launch_fixture = Some(crate::bandwidth::launcher::Launch {
            prepared: None,
            program: "/usr/bin/python3".into(),
            arguments: vec!["-c".into(), "import sys; sys.exit(126)".into()],
        });
    });
    click(&mut cx, window, "bandwidth-start");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.bandwidth.snapshot.status == Status::Cancelled
                && v.bandwidth.client.as_ref().unwrap().finished()
        })
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("11-authorization-cancelled.json"),
    );
    cx.update_entity(&view, |view, _| {
        view.bandwidth.launch_fixture = Some(crate::bandwidth::launcher::Launch {
            prepared: None,
            program: "/usr/bin/python3".into(),
            arguments: vec!["-c".into(), "import time; time.sleep(600)".into()],
        });
    });
    click(&mut cx, window, "bandwidth-start");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.bandwidth.snapshot.status == Status::Authorizing
        })
    });
    let pending_endpoint =
        cx.read_entity(&view, |v, _| v.bandwidth.snapshot.endpoint.clone().unwrap());
    click(&mut cx, window, "bandwidth-stop");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            v.bandwidth.client.as_ref().unwrap().finished()
        })
    });
    assert!(!pending_endpoint.exists());
    checkpoint(
        &mut cx,
        window,
        &evidence.join("12-stop-pending-authorization.json"),
    );
    backend.handle.shutdown();
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    shutdown(cx);
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let checkpoints = [
        "01-not-monitoring.json",
        "02-waiting-for-collector.json",
        "03-live-application-rates.json",
        "04-global-page-two.json",
        "05-filtered-applications.json",
        "06-numeric-upload-sort.json",
        "07-independent-permission.json",
        "08-unavailable-retains-totals.json",
        "09-new-monitoring-session.json",
        "09b-short-lived-app-totals.json",
        "10-stopped-owned-cleanup.json",
        "11-authorization-cancelled.json",
        "12-stop-pending-authorization.json",
    ];
    std::fs::write(evidence.join("headless.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,
        "checkpoints":checkpoints,"proof_boundary":"Actual Kit navigation/start/copy/search/header sort/page/stop input; framed Unix-stream fixture, rates and session totals, malformed-frame retention, new session, hidden-page ingestion, real independent AskRule and owned cleanup. Kernel accounting, fonts and compositor proof are separate."
    })).unwrap()).unwrap();
}
