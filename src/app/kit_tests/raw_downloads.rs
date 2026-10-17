use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::raw_downloads::{Snapshot, Status};
    use gpui::InputEvent as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn reveal(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) {
        for _ in 0..32 {
            let direction = cx
                .update_window(w, |_, w, cx| {
                    w.render_frame(cx);
                    let body = w.find("raw-downloads-scroll").bounds();
                    let target = w.find(id).bounds();
                    if target.center().y < body.top() + px(8.) {
                        Some(120.)
                    } else if target.center().y > body.bottom() - px(8.) {
                        Some(-120.)
                    } else {
                        None
                    }
                })
                .unwrap();
            let Some(direction) = direction else { return };
            cx.update_window(w, |_, w, cx| {
                let position = w.find("raw-downloads-scroll").bounds().center();
                w.dispatch_event(
                    gpui::MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                w.render_frame(cx);
                w.dispatch_event(
                    gpui::ScrollWheelEvent {
                        position,
                        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(direction))),
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                w.render_frame(cx);
            })
            .unwrap();
            flush(cx);
        }
        panic!("Raw control {id} must be reachable by actual wheel input");
    }
    fn control(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) {
        let footer = cx
            .update_window(w, |_, w, _| {
                w.find(id).path().contains(&"settings-footer".into())
            })
            .unwrap();
        if !footer {
            reveal(cx, w, id);
        }
        click(cx, w, id);
        flush(cx);
    }

    fn input(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str, value: &str) {
        reveal(cx, w, id);
        fill(cx, w, id, value);
    }
    fn state(
        cx: &mut TestAppContext,
        h: &crate::raw_downloads::Handle,
        p: impl Fn(&Snapshot) -> bool,
    ) -> Snapshot {
        for _ in 0..80 {
            flush(cx);
            let s = h.snapshot();
            if p(&s) {
                flush(cx);
                flush(cx);
                return s;
            }
        }
        panic!("Expected raw worker state, got {:?}", h.snapshot());
    }
    fn opened(cx: &mut TestAppContext, main: AnyWindowHandle) -> AnyWindowHandle {
        let before = cx.update(|cx| cx.windows());
        click(cx, main, "raw-download-preferences");
        flush(cx);
        cx.update(|cx| {
            cx.windows()
                .into_iter()
                .find(|w| !before.contains(w))
                .unwrap()
        })
    }
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let mapping = std::fs::read_to_string("/proc/self/uid_map").unwrap();
    let fields = mapping.split_whitespace().collect::<Vec<_>>();
    assert!(
        fields.len() == 3 && fields[0] == "0" && fields[1] != "0" && fields[2] == "1",
        "Run raw GPU through owned_net.py"
    );
    let links = std::process::Command::new("ip")
        .args(["-j", "link", "show"])
        .output()
        .unwrap();
    let links: Vec<serde_json::Value> = serde_json::from_slice(&links.stdout).unwrap();
    assert!(links.len() == 1 && links[0]["ifname"] == "lo");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    type Response = (u16, Vec<u8>, bool);
    let responses = Arc::new(Mutex::new(std::collections::VecDeque::<Response>::new()));
    let hits = Arc::new(AtomicUsize::new(0));
    let hold = Arc::new(AtomicBool::new(false));
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let url = format!("http://{}/raw", listener.local_addr().unwrap());
    let (queue, count, held) = (responses.clone(), hits.clone(), hold.clone());
    let http = runtime.spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (queue, count, held) = (queue.clone(), count.clone(), held.clone());
            tokio::spawn(async move {
                let mut bytes = [0; 4096];
                let n = socket.read(&mut bytes).await.unwrap_or(0);
                assert!(bytes[..n].starts_with(b"GET /raw "));
                count.fetch_add(1, Ordering::SeqCst);
                let (code, body, wait) = queue
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("Unexpected raw HTTP");
                while wait && held.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let header = format!(
                    "HTTP/1.1 {code} owned\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            });
        }
    });
    let backend = Backend::start(config.clone()).unwrap();
    let raw = backend.handle.raw_downloads.clone();
    let mut notices = backend.handle.desktop_notices.take_receiver().unwrap();
    let mut daemon = runtime.block_on(async {
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
                name: "raw-download-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        daemon
    });
    let destination = dir.path().join("owned-resource.bin");
    let profile = dir.path().join("owned-profile.json");
    let document = serde_json::json!({"unknown_outer":{"keep":1},"actions":{"future_action":true,"downloader":{"enabled":true,"unknown_downloader":"retained","config":[{"name":"Owned raw group","interval":"7","units":"hours","unknown_group":99,"notify":{"success":{"desktop":"Owned raw published"},"error":{"desktop":"Owned raw failed"}},"urls":[{"name":"Owned resource","enabled":true,"remote":url,"localfile":destination,"unknown_resource":[1,2]}]}]}}});
    std::fs::write(&profile, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    let mut cx = context(test_cx);
    let (main, view) = cx.update(|cx| {
        gpui_kit::open_window(crate::app::dashboard_options(), cx, |w, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx))
        })
        .unwrap()
    });
    let (tray, _tray_actions, _availability) = crate::app::tray::Client::fixture();
    let (notifier, actions) = crate::app::desktop_notifications::Client::fixture();
    cx.update_entity(&view, |v, _| {
        v.tray = Some(tray);
        v.desktop_notifications = Some(notifier)
    });
    state(&mut cx, &raw, |s| s.ready);
    let mut w = opened(&mut cx, main);
    assert!(!raw.snapshot().settings.enabled);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    checkpoint(&mut cx, w, &evidence.join("01-raw-default-disabled.json"));
    input(&mut cx, w, "raw-profile-path", profile.to_str().unwrap());
    control(&mut cx, w, "raw-import");
    for _ in 0..60 {
        flush(&mut cx);
        if cx
            .update_window(w, |_, w, _| w.try_find("raw-interval-0").is_some())
            .unwrap()
        {
            break;
        }
    }
    assert!(raw.snapshot().settings.groups.is_empty());
    assert!(!crate::raw_downloads::path_for(&config.database).exists());
    assert!(!destination.exists());
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    control(&mut cx, w, "raw-save");
    checkpoint(
        &mut cx,
        w,
        &evidence.join("02-import-off-review-exact-paths.json"),
    );
    control(&mut cx, w, "raw-dismiss-review");
    assert!(!crate::raw_downloads::path_for(&config.database).exists());
    input(&mut cx, w, "raw-interval-0", "0");
    control(&mut cx, w, "raw-save");
    assert!(
        cx.update_window(w, |_, w, _| w.try_find("raw-confirm").is_none())
            .unwrap()
    );
    checkpoint(
        &mut cx,
        w,
        &evidence.join("03-invalid-raw-interval-refused.json"),
    );
    input(&mut cx, w, "raw-interval-0", "300");
    reveal(&mut cx, w, "raw-unit-0");
    click(&mut cx, w, "raw-unit-0");
    flush(&mut cx);
    cx.update_window(w, |_, w, cx| {
        w.press("up", cx);
        w.press("enter", cx)
    })
    .unwrap();
    flush(&mut cx);
    control(&mut cx, w, "raw-enabled");
    control(&mut cx, w, "raw-save");
    checkpoint(&mut cx, w, &evidence.join("04-reviewed-raw-settings.json"));
    control(&mut cx, w, "raw-confirm");
    state(&mut cx, &raw, |s| {
        s.settings.enabled && s.settings.groups.len() == 1
    });
    assert_eq!(raw.snapshot().settings.groups[0].interval, "300");
    assert_eq!(
        raw.snapshot().settings.groups[0].units,
        crate::raw_downloads::Unit::Minutes
    );
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "First interval is not immediate"
    );
    control(&mut cx, w, "raw-close");
    w = opened(&mut cx, main);
    checkpoint(&mut cx, w, &evidence.join("05-raw-saved-reopened.json"));
    let saved_profile = evidence.join("exported-profile.json");
    input(
        &mut cx,
        w,
        "raw-profile-path",
        saved_profile.to_str().unwrap(),
    );
    control(&mut cx, w, "raw-export");
    wait_for(&mut cx, |_cx| saved_profile.exists());
    let exported: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&saved_profile).unwrap()).unwrap();
    assert_eq!(exported["unknown_outer"], document["unknown_outer"]);
    assert_eq!(exported["actions"]["future_action"], true);
    let group = &exported["actions"]["downloader"]["config"][0];
    assert_eq!(group["unknown_group"], 99);
    assert_eq!(
        group["urls"][0]["unknown_resource"],
        serde_json::json!([1, 2])
    );
    assert_eq!(exported["actions"]["downloader"]["enabled"], true);
    let exported_bytes = std::fs::read(&saved_profile).unwrap();
    control(&mut cx, w, "raw-export");
    assert_eq!(std::fs::read(&saved_profile).unwrap(), exported_bytes);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("06-raw-export-preserves-unknown-and-existing.json"),
    );
    control(&mut cx, w, "raw-run-0");
    control(&mut cx, w, "raw-dismiss-review");
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    const BODY: &[u8] = b"owned\0binary\n127.0.0.1 stays raw\xff";
    responses
        .lock()
        .unwrap()
        .push_back((200, BODY.to_vec(), false));
    control(&mut cx, w, "raw-run-0");
    control(&mut cx, w, "raw-confirm");
    state(&mut cx, &raw, |s| !s.busy && s.completed_runs == 1);
    assert_eq!(std::fs::read(&destination).unwrap(), BODY);
    std::fs::copy(&destination, evidence.join("published-owned-resource.bin")).unwrap();
    assert_eq!(raw.snapshot().reports[0].status, Status::Published);
    let notice = notices.try_recv().unwrap();
    assert_eq!(
        notice.target,
        crate::desktop_notifications::Target::RawDownloads
    );
    assert_eq!(notice.summary, "Owned raw published");
    checkpoint(
        &mut cx,
        w,
        &evidence.join("07-raw-exact-binary-publication-notice.json"),
    );
    responses
        .lock()
        .unwrap()
        .push_back((500, b"owned failure".to_vec(), false));
    control(&mut cx, w, "raw-run-0");
    control(&mut cx, w, "raw-confirm");
    state(&mut cx, &raw, |s| !s.busy && s.completed_runs == 2);
    assert_eq!(std::fs::read(&destination).unwrap(), BODY);
    assert_eq!(raw.snapshot().reports[0].status, Status::Failed);
    assert_eq!(notices.try_recv().unwrap().summary, "Owned raw failed");
    checkpoint(
        &mut cx,
        w,
        &evidence.join("08-raw-error-preserves-last-good.json"),
    );
    hold.store(true, Ordering::Release);
    responses
        .lock()
        .unwrap()
        .push_back((200, b"cancelled replacement".to_vec(), true));
    control(&mut cx, w, "raw-run-0");
    control(&mut cx, w, "raw-confirm");
    state(&mut cx, &raw, |s| s.busy);
    wait_for(&mut cx, |_cx| hits.load(Ordering::SeqCst) == 3);
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    let ask = runtime.spawn(async move {
        daemon
            .ask_rule(protocol::Connection {
                process_path: "/owned/raw-held-permission".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    let mut popup = None;
    wait_for(&mut cx, |cx| {
        popup = cx.read_entity(&view, |v, _| v.prompt_window);
        popup.is_some()
    });
    let popup = popup.unwrap();
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(ask).unwrap().action, "deny");
    assert!(raw.snapshot().busy);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("09-permission-during-held-raw-get.json"),
    );
    control(&mut cx, w, "raw-cancel");
    state(&mut cx, &raw, |s| !s.busy && s.completed_runs == 3);
    assert_eq!(std::fs::read(&destination).unwrap(), BODY);
    assert_eq!(raw.snapshot().reports[0].status, Status::Cancelled);
    assert!(notices.try_recv().is_err());
    hold.store(false, Ordering::Release);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("10-cancel-raw-keeps-last-good.json"),
    );
    control(&mut cx, w, "raw-run-0");
    let path = crate::raw_downloads::path_for(&config.database);
    let mut external: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    external["settings"]["groups"][0]["interval"] = "5".into();
    let external = serde_json::to_vec_pretty(&external).unwrap();
    std::fs::write(&path, &external).unwrap();
    control(&mut cx, w, "raw-confirm");
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    assert_eq!(std::fs::read(&path).unwrap(), external);
    assert_eq!(std::fs::read(&destination).unwrap(), BODY);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("11-frozen-run-revision-refused.json"),
    );
    control(&mut cx, w, "raw-reload");
    state(&mut cx, &raw, |s| s.settings.groups[0].interval == "5");
    control(&mut cx, w, "raw-close");
    cx.update_window(main, |_, w, cx| {
        assert!(view.update(cx, |v, _| v.should_close_dashboard(w)));
        w.remove_window()
    })
    .unwrap();
    flush(&mut cx);
    let before = cx.update(|cx| cx.windows());
    actions
        .try_send(crate::desktop_notifications::Target::RawDownloads)
        .unwrap();
    flush(&mut cx);
    let candidates = cx.update(|cx| cx.windows());
    w = candidates
        .into_iter()
        .find(|candidate| {
            !before.contains(candidate)
                && cx
                    .update_window(*candidate, |_, w, _| w.try_find("raw-save").is_some())
                    .unwrap_or(false)
        })
        .unwrap();
    let count = cx.update(|cx| cx.windows().len());
    actions
        .try_send(crate::desktop_notifications::Target::RawDownloads)
        .unwrap();
    wait_for(&mut cx, |cx| (cx.windows().len()) == (count));

    assert_eq!(cx.update(|cx| cx.windows().len()), count);
    assert!(cx.read_entity(&view, |v, _| v.main_window.is_none()));
    checkpoint(
        &mut cx,
        w,
        &evidence.join("12-raw-notice-opens-from-hidden-singleton.json"),
    );
    control(&mut cx, w, "raw-enabled");
    control(&mut cx, w, "raw-save");
    control(&mut cx, w, "raw-confirm");
    state(&mut cx, &raw, |s| !s.settings.enabled);
    assert_eq!(std::fs::read(&destination).unwrap(), BODY);
    control(&mut cx, w, "raw-close");
    drop(view);
    shutdown(cx);
    drop(raw);
    http.abort();
    backend.join();
    drop(runtime);
    assert!(!config.socket.exists());
    let checkpoints = (1..=12)
        .map(|n| {
            std::fs::read_dir(&evidence)
                .unwrap()
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().to_string())
                .find(|f| f.starts_with(&format!("{n:02}-")) && f.ends_with(".json"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":checkpoints,"proof_boundary":"Actual native profile stage OFF/review/cancel/invalid/save/reopen, selected count/unit, exact binary publication/error/cancel last-good preservation, unknown-preserving saved-profile export and existing-file refusal, frozen RunNow revision, actual permission during held owned GET, real worker mailbox notices plus synthetic hidden-dashboard RawDownloads callback/singleton. Owned namespace HTTP and temporary files only, no system daemon/policy, external endpoint, native compositor or package claim."})).unwrap()).unwrap();
}
