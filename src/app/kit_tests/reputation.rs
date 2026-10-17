use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::reputation::{Status, Worker};
    use gpui::InputEvent as _;
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    type Client = protocol::ui_client::UiClient<tonic::transport::Channel>;
    fn prompt_report(cx: &mut TestAppContext, prompt: AnyWindowHandle) {
        let mut trace = Vec::new();
        for attempt in 0..30 {
            let (direction,sample) = cx
                .update_window(prompt, |_, w, cx| {
                    w.render_frame(cx);
                    let body = w.find("prompt-content").bounds();
                    let target = w.find("prompt-reputation-details").bounds();
                    let direction = if target.top() < body.top() {
                        Some(120.)
                    } else if target.bottom() > body.bottom() {
                        Some(-120.)
                    } else {
                        None
                    };
                    (direction,serde_json::json!({"attempt":attempt,"body":format!("{body:?}"),"target":format!("{target:?}"),"target_visible":w.find("prompt-reputation-details").visible(),"target_disabled":w.find("prompt-reputation-details").disabled(),"viewport":format!("{:?}",w.viewport_size()),"direction":direction}))
                })
                .unwrap();
            trace.push(sample);
            let Some(direction) = direction else {
                click(cx, prompt, "prompt-reputation-details");
                flush(cx);
                return;
            };
            cx.update_window(prompt, |_, w, cx| {
                let position = w.find("prompt-content").bounds().center();
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
        let evidence = evidence();
        checkpoint(cx, prompt, &evidence.join("unreachable-prompt-report.json"));
        std::fs::write(
            evidence.join("prompt-report-scroll-trace.json"),
            serde_json::to_vec_pretty(&trace).unwrap(),
        )
        .unwrap();
        panic!(
            "Captured report button must be reachable inside actual prompt scroll viewport; retained bounds trace"
        );
    }

    fn ready(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) {
        for _ in 0..60 {
            flush(cx);
            if cx
                .update_window(w, |_, w, _| {
                    w.try_find(id)
                        .is_some_and(|e| e.visible() && e.disabled() != Some(true))
                })
                .unwrap()
            {
                return;
            }
        }
        panic!("Reputation control {id} never ready")
    }
    fn result(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        status: Status,
    ) -> crate::reputation::ResultSet {
        for _ in 0..60 {
            flush(cx);
            if let Some(r) = cx
                .update_window(w, |_, w, cx| {
                    crate::app::reputation_view::test_result(w, cx)
                })
                .unwrap()
                && r.status == status
            {
                return r;
            }
        }
        panic!("Expected reputation result {status:?}")
    }
    fn probe(cx: &mut impl AppContext, w: AnyWindowHandle) -> (bool, bool) {
        cx.update_window(w, |_, w, cx| {
            gpui_kit::base::Root::read(w, cx)
                .view()
                .clone()
                .downcast::<PromptWindow>()
                .unwrap()
                .read(cx)
                .reputation_status()
        })
        .unwrap()
    }
    fn warning(cx: &mut TestAppContext, w: AnyWindowHandle, emphasized: bool) {
        wait_for(cx, |cx| probe(cx, w) == (true, emphasized));
    }
    fn asking(
        runtime: &tokio::runtime::Runtime,
        daemon: &Client,
        name: &str,
    ) -> tokio::task::JoinHandle<protocol::Rule> {
        let mut daemon = daemon.clone();
        let name = name.to_string();
        runtime.spawn(async move {
            daemon
                .ask_rule(protocol::Connection {
                    process_path: format!("/owned/{name}"),
                    dst_host: format!("{name}.test"),
                    protocol: "tcp".into(),
                    ..Default::default()
                })
                .await
                .unwrap()
                .into_inner()
        })
    }
    fn popup(cx: &mut TestAppContext, v: &gpui::Entity<Dashboard>) -> AnyWindowHandle {
        for _ in 0..60 {
            flush(cx);
            if let Some(w) = cx.read_entity(v, |v, _| v.prompt_window) {
                return w;
            }
        }
        panic!("Expected real permission popup")
    }
    fn manual(cx: &mut TestAppContext, w: AnyWindowHandle, value: &str) {
        click(cx, w, "rep-tab-results");
        flush(cx);
        reveal(cx, w, "rep-query".into());
        fill(cx, w, "rep-query", value);
        reveal(cx, w, "rep-check".into());
        click(cx, w, "rep-check")
    }
    fn reveal(cx: &mut TestAppContext, window: AnyWindowHandle, id: gpui::ElementId) {
        for _ in 0..32 {
            let direction = cx
                .update_window(window, |_, w, cx| {
                    w.render_frame(cx);
                    let body = w.find("rep-scroll").bounds();
                    let target = w.find(id.clone()).bounds();
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
            cx.update_window(window, |_, w, cx| {
                let body = w.find("rep-scroll").bounds();
                w.dispatch_event(
                    gpui::MouseMoveEvent {
                        position: body.center(),
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                w.render_frame(cx);
                w.dispatch_event(
                    gpui::ScrollWheelEvent {
                        position: body.center(),
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
        panic!("Reputation control {id:?} must be reachable by wheel input");
    }

    const KEY: &str = "owned-reputation-gpu-key-0123456789abcdef";
    fn payload(warning: bool) -> String {
        serde_json::json!({"data":{"attributes":{"last_analysis_stats":{"malicious":if warning{10}else{0},"suspicious":0,"harmless":60,"undetected":3},"total_votes":{"malicious":0,"harmless":0},"reputation":0,"echo":KEY}}}).to_string()
    }
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let keyfile = dir.path().join("dummy-api-key");
    std::fs::write(&keyfile, KEY).unwrap();
    std::fs::set_permissions(&keyfile, std::fs::Permissions::from_mode(0o600)).unwrap();
    type Response = (u16, String, bool);
    let responses = Arc::new(Mutex::new(std::collections::VecDeque::<Response>::new()));
    let hits = Arc::new(AtomicUsize::new(0));
    let hold = Arc::new(AtomicBool::new(false));
    let paths = Arc::new(Mutex::new(Vec::<String>::new()));
    let(base,http)=runtime.block_on(async{let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let base=format!("http://{}/",listener.local_addr().unwrap());let(queue,hits,hold,paths)=(responses.clone(),hits.clone(),hold.clone(),paths.clone());let task=tokio::spawn(async move{while let Ok((mut socket,_))=listener.accept().await{let mut bytes=[0u8;8192];let count=socket.read(&mut bytes).await.unwrap_or(0);{let mut paths=paths.lock().unwrap();assert!(paths.len()<32);paths.push(String::from_utf8_lossy(&bytes[..count]).lines().next().unwrap_or("").to_string());}hits.fetch_add(1,Ordering::SeqCst);let(status,body,held)=queue.lock().unwrap().pop_front().expect("Unexpected owned reputation HTTP request");while held&&hold.load(Ordering::Acquire){tokio::time::sleep(Duration::from_millis(5)).await;}let wire=format!("HTTP/1.1 {status} owned\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());let _=socket.write_all(wire.as_bytes()).await;}});(base,task)});
    let worker =
        Worker::start_fixture(crate::reputation::path_for(&config.database), &base).unwrap();
    let reputation = worker.handle();
    let mut handle = backend.handle.clone();
    handle.reputation = reputation.clone();
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
        let mut c = Client::new(channel);
        c.subscribe(protocol::ClientConfig {
            name: "reputation-fixture".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        c
    });
    let mut cx = context(test_cx);
    let (main, view) = cx.update(|cx| {
        gpui_kit::open_window(crate::app::dashboard_options(), cx, |w, cx| {
            cx.new(|cx| Dashboard::new(handle.clone(), config.clone(), w, cx))
        })
        .unwrap()
    });
    flush(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, main, "reputation-preferences");
    flush(&mut cx);
    let w = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    });
    ready(&mut cx, w, "rep-save");
    assert!(!reputation.snapshot().settings.enabled);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("01-reputation-default-disabled.json"),
    );
    reveal(&mut cx, w, "rep-timeout".into());
    fill(&mut cx, w, "rep-timeout", "0");
    click(&mut cx, w, "rep-save");
    wait_for(&mut cx, |_cx| {
        !crate::reputation::path_for(&config.database).exists()
    });

    assert!(!crate::reputation::path_for(&config.database).exists());
    checkpoint(
        &mut cx,
        w,
        &evidence.join("02-invalid-reputation-settings.json"),
    );
    fill(&mut cx, w, "rep-timeout", "10");
    fill(&mut cx, w, "rep-budget", "30");
    fill(&mut cx, w, "rep-threshold", "3");
    reveal(&mut cx, w, "rep-key-file".into());
    fill(&mut cx, w, "rep-key-file", keyfile.to_str().unwrap());
    for id in [
        "rep-enabled",
        "rep-ips",
        "rep-hashes",
        "rep-suspicious",
        "rep-community",
        "rep-reputation",
    ] {
        reveal(&mut cx, w, id.into());
        click(&mut cx, w, id)
    }
    reveal(&mut cx, w, "rep-excluded-domains-add".into());
    click(&mut cx, w, "rep-excluded-domains-add");
    flush(&mut cx);
    reveal(&mut cx, w, "rep-excluded-domains-0".into());
    fill(&mut cx, w, "rep-excluded-domains-0", "skip.test");
    reveal(&mut cx, w, "rep-excluded-ips-add".into());
    click(&mut cx, w, "rep-excluded-ips-add");
    flush(&mut cx);
    reveal(&mut cx, w, "rep-excluded-ips-0".into());
    fill(&mut cx, w, "rep-excluded-ips-0", "10.");
    click(&mut cx, w, "rep-save");
    ready(&mut cx, w, "rep-save");
    let saved = reputation.snapshot().settings;
    assert!(
        saved.enabled
            && saved.domains
            && !saved.ips
            && !saved.hashes
            && saved.use_suspicious
            && saved.use_community_score
            && saved.use_reputation
    );
    assert_eq!(saved.timeout_seconds, 10);
    assert_eq!(saved.request_budget, 30);
    assert_eq!(saved.minimum_threshold, 3);
    assert_eq!(saved.excluded_domains, vec!["skip.test"]);
    assert_eq!(saved.excluded_ips, vec!["10."]);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    let path = crate::reputation::path_for(&config.database);
    let bytes = std::fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(KEY));
    click(&mut cx, w, "rep-close");
    flush(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, main, "reputation-preferences");
    flush(&mut cx);
    let w = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    });
    ready(&mut cx, w, "rep-save");
    reveal(&mut cx, w, "rep-threshold".into());
    assert_eq!(
        cx.update_window(w, |_, w, _| w
            .find("rep-threshold")
            .value()
            .unwrap()
            .to_string())
            .unwrap(),
        "3"
    );
    checkpoint(
        &mut cx,
        w,
        &evidence.join("03-reputation-settings-reopened.json"),
    );
    manual(&mut cx, w, "skip.test");
    result(&mut cx, w, Status::Excluded);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("04-reputation-exclusion-no-network.json"),
    );
    responses
        .lock()
        .unwrap()
        .push_back((200, payload(true), false));
    manual(&mut cx, w, "manual-warning.test");
    let report = result(&mut cx, w, Status::Complete);
    assert_eq!(report.reports.len(), 1);
    assert_eq!(
        report.reports[0].verdict,
        crate::reputation::Verdict::Warning
    );
    assert!(!report.reports[0].details.contains(KEY));
    reveal(&mut cx, w, "rep-details-0".into());
    click(&mut cx, w, "rep-details-0");
    flush(&mut cx);
    reveal(&mut cx, w, "rep-copy-0".into());
    click(&mut cx, w, "rep-copy-0");
    let excerpt = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(excerpt.contains("last_analysis_stats"));
    assert!(!excerpt.contains(KEY));
    checkpoint(
        &mut cx,
        w,
        &evidence.join("05-reputation-report-redacted-excerpt.json"),
    );
    responses
        .lock()
        .unwrap()
        .push_back((500, "{}".into(), false));
    manual(&mut cx, w, "limited.test");
    result(&mut cx, w, Status::Failed);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("06-reputation-service-error.json"),
    );
    hold.store(true, Ordering::Release);
    responses
        .lock()
        .unwrap()
        .push_back((200, payload(true), true));
    manual(&mut cx, w, "manual-held.test");
    result(&mut cx, w, Status::Checking);
    reveal(&mut cx, w, "rep-cancel".into());
    ready(&mut cx, w, "rep-cancel");
    click(&mut cx, w, "rep-cancel");
    result(&mut cx, w, Status::Cancelled);
    hold.store(false, Ordering::Release);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("07-cancel-held-reputation-check.json"),
    );
    click(&mut cx, w, "rep-tab-settings");
    flush(&mut cx);
    let mut external: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    external["settings"]["minimum_threshold"] = 4.into();
    let external = serde_json::to_vec_pretty(&external).unwrap();
    std::fs::write(&path, &external).unwrap();
    reveal(&mut cx, w, "rep-threshold".into());
    fill(&mut cx, w, "rep-threshold", "5");
    click(&mut cx, w, "rep-save");
    ready(&mut cx, w, "rep-save");
    assert_eq!(std::fs::read(&path).unwrap(), external);
    assert_eq!(reputation.snapshot().settings.minimum_threshold, 3);
    checkpoint(
        &mut cx,
        w,
        &evidence.join("08-stale-reputation-settings-refused.json"),
    );
    reveal(&mut cx, w, "rep-reload".into());
    click(&mut cx, w, "rep-reload");
    ready(&mut cx, w, "rep-save");
    assert_eq!(reputation.snapshot().settings.minimum_threshold, 4);
    click(&mut cx, w, "rep-close");
    flush(&mut cx);
    let before_hits = hits.load(Ordering::SeqCst);
    hold.store(true, Ordering::Release);
    responses
        .lock()
        .unwrap()
        .push_back((200, payload(true), true));
    let ask = asking(&runtime, &daemon, "held-prompt");
    let prompt = popup(&mut cx, &view);
    wait_for(&mut cx, |_cx| hits.load(Ordering::SeqCst) > before_hits);
    assert!(hits.load(Ordering::SeqCst) > before_hits);
    assert_eq!(probe(&mut cx, prompt), (false, false));
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("09-permission-during-held-reputation.json"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(ask).unwrap().action, "deny");
    hold.store(false, Ordering::Release);
    flush(&mut cx);
    responses
        .lock()
        .unwrap()
        .push_back((200, payload(true), false));
    let ask = asking(&runtime, &daemon, "early-warning");
    let prompt = popup(&mut cx, &view);
    warning(&mut cx, prompt, true);
    let fallback = backend.handle.preferences.snapshot();
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("10-untouched-warning-reject-once.json"),
    );
    let footer_bounds = cx
        .update_window(prompt, |_, w, cx| {
            w.render_frame(cx);
            [
                w.find("reputation-reject-once").bounds(),
                w.find("prompt-more").bounds(),
            ]
        })
        .unwrap();
    for _ in 0..3 {
        cx.update_window(prompt, |_, w, cx| {
            let position = w.find("prompt-content").bounds().center();
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
                    delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-180.))),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            w.render_frame(cx);
        })
        .unwrap();
        flush(&mut cx);
    }
    cx.update_window(prompt, |_, w, cx| {
        w.render_frame(cx);
        let after = [
            w.find("reputation-reject-once").bounds(),
            w.find("prompt-more").bounds(),
        ];
        assert_eq!(
            after, footer_bounds,
            "Decision and advanced-details footer stay fixed while actual content scrolls"
        );
        for bounds in after {
            assert!(
                bounds.top() >= px(0.)
                    && bounds.left() >= px(0.)
                    && bounds.bottom() <= w.viewport_size().height
                    && bounds.right() <= w.viewport_size().width,
                "Whole footer controls stay inside viewport: {bounds:?}"
            );
        }
    })
    .unwrap();
    std::fs::write(evidence.join("prompt-footer-scroll.json"),serde_json::to_vec_pretty(&serde_json::json!({"actual_wheel_events":3,"footer_positions_unchanged":true,"entire_reject_and_more_bounds_inside_viewport":true})).unwrap()).unwrap();
    click(&mut cx, prompt, "reputation-reject-once");
    let reply = runtime.block_on(ask).unwrap();
    assert_eq!(reply.action, "reject");
    assert_eq!(reply.duration, "once");
    assert_eq!(backend.handle.preferences.snapshot(), fallback);
    flush(&mut cx);
    hold.store(true, Ordering::Release);
    responses
        .lock()
        .unwrap()
        .push_back((200, payload(true), true));
    let ask = asking(&runtime, &daemon, "late-warning");
    let prompt = popup(&mut cx, &view);
    click(&mut cx, prompt, "prompt-more");
    pick(&mut cx, prompt, "prompt-duration", "5 minutes");
    assert!(backend.handle.prompts.snapshot()[0].reviewing);
    hold.store(false, Ordering::Release);
    warning(&mut cx, prompt, false);
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("11-late-warning-preserves-review.json"),
    );
    let before = cx.update(|cx| cx.windows());
    prompt_report(&mut cx, prompt);
    flush(&mut cx);
    let inspect = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    });
    let captured = result(&mut cx, inspect, Status::Complete).capture;
    let count = cx.update(|cx| cx.windows().len());
    prompt_report(&mut cx, prompt);
    wait_for(&mut cx, |cx| (cx.windows().len()) == (count));

    assert_eq!(cx.update(|cx| cx.windows().len()), count);
    assert!(cx.update_window(inspect, |_, _, _| ()).is_ok());
    checkpoint(
        &mut cx,
        inspect,
        &evidence.join("12-captured-inspection-singleton.json"),
    );
    click(&mut cx, prompt, "allow-selected");
    let reply = runtime.block_on(ask).unwrap();
    assert_eq!(reply.action, "allow");
    assert_eq!(reply.duration, "5m");
    flush(&mut cx);
    responses
        .lock()
        .unwrap()
        .push_back((200, payload(true), false));
    let ask = asking(&runtime, &daemon, "next-capture");
    let prompt = popup(&mut cx, &view);
    warning(&mut cx, prompt, true);
    let before = cx.update(|cx| cx.windows());
    prompt_report(&mut cx, prompt);
    flush(&mut cx);
    let next = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    });
    let new_capture = result(&mut cx, next, Status::Complete).capture;
    assert_ne!(new_capture, captured);
    assert!(cx.update_window(inspect, |_, _, _| ()).is_err());
    checkpoint(
        &mut cx,
        next,
        &evidence.join("13-new-capture-replaces-inspection.json"),
    );
    click(&mut cx, prompt, "deny-once");
    runtime.block_on(ask).unwrap();
    click(&mut cx, next, "rep-close");
    flush(&mut cx);
    hold.store(true, Ordering::Release);
    responses
        .lock()
        .unwrap()
        .push_back((200, payload(true), true));
    let before_hits = hits.load(Ordering::SeqCst);
    let ask = asking(&runtime, &daemon, "old-capture-held");
    let prompt = popup(&mut cx, &view);
    wait_for(&mut cx, |_cx| hits.load(Ordering::SeqCst) > before_hits);
    assert!(
        hits.load(Ordering::SeqCst) > before_hits,
        "Old captured HTTP request actually started"
    );
    click(&mut cx, prompt, "deny-once");
    runtime.block_on(ask).unwrap();
    responses
        .lock()
        .unwrap()
        .push_back((200, payload(false), false));
    let ask = asking(&runtime, &daemon, "new-capture-clean");
    let prompt = popup(&mut cx, &view);
    hold.store(false, Ordering::Release);
    for _ in 0..50 {
        flush(&mut cx);
        if cx
            .update_window(prompt, |_, w, _| {
                w.try_find("prompt-reputation-details")
                    .is_some_and(|e| e.disabled() != Some(true))
            })
            .unwrap()
        {
            break;
        }
    }
    assert_eq!(probe(&mut cx, prompt), (false, false));
    let before = cx.update(|cx| cx.windows());
    prompt_report(&mut cx, prompt);
    flush(&mut cx);
    let clean = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    });
    let clean_report = result(&mut cx, clean, Status::Complete);
    assert_eq!(
        clean_report.reports[0].subject.value(),
        "new-capture-clean.test"
    );
    assert_eq!(
        clean_report.reports[0].verdict,
        crate::reputation::Verdict::NotFlagged
    );
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("14-old-result-isolated-from-next-prompt.json"),
    );
    click(&mut cx, prompt, "deny-once");
    runtime.block_on(ask).unwrap();
    click(&mut cx, clean, "rep-close");
    flush(&mut cx);
    responses
        .lock()
        .unwrap()
        .push_back((200, payload(true), false));
    let ask = asking(&runtime, &daemon, "disable-warning");
    let prompt = popup(&mut cx, &view);
    warning(&mut cx, prompt, true);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, main, "reputation-preferences");
    flush(&mut cx);
    let w = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|w| !before.contains(w))
            .unwrap()
    });
    click(&mut cx, w, "rep-tab-settings");
    reveal(&mut cx, w, "rep-enabled".into());
    click(&mut cx, w, "rep-enabled");
    click(&mut cx, w, "rep-save");
    ready(&mut cx, w, "rep-save");
    wait_for(&mut cx, |cx| probe(cx, prompt) == (false, false));
    assert_eq!(probe(&mut cx, prompt), (false, false));
    assert!(!reputation.snapshot().settings.enabled);
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("15-disable-clears-current-warning.json"),
    );
    click(&mut cx, prompt, "deny-once");
    runtime.block_on(ask).unwrap();
    let hits = hits.load(Ordering::SeqCst);
    manual(&mut cx, w, "disabled-manual.test");
    result(&mut cx, w, Status::Disabled);
    assert_eq!(hits, paths.lock().unwrap().len());
    assert!(
        paths
            .lock()
            .unwrap()
            .iter()
            .all(|p| p.starts_with("GET /api/v3/domains/") && !p.contains(KEY))
    );
    std::fs::write(
        evidence.join("saved-reputation-preferences.json"),
        std::fs::read(&path).unwrap(),
    )
    .unwrap();
    click(&mut cx, w, "rep-close");
    drop(view);
    shutdown(cx);
    drop(daemon);
    worker.join();
    http.abort();
    drop(runtime);
    drop(handle);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-reputation-default-disabled.json","02-invalid-reputation-settings.json","03-reputation-settings-reopened.json","04-reputation-exclusion-no-network.json","05-reputation-report-redacted-excerpt.json","06-reputation-service-error.json","07-cancel-held-reputation-check.json","08-stale-reputation-settings-refused.json","09-permission-during-held-reputation.json","10-untouched-warning-reject-once.json","11-late-warning-preserves-review.json","12-captured-inspection-singleton.json","13-new-capture-replaces-inspection.json","14-old-result-isolated-from-next-prompt.json","15-disable-clears-current-warning.json"],"proof_boundary":"Actual native settings save/reopen/validation/exclusions/stale/reload; owned GET-only HTTP reports/redacted detail/copy/service-error/cancel; real permission while HTTP held; untouched Reject once, late warning preserves selected5m review and fallback; captured singleton/newcapture isolation and old held-result exclusion; disabling clears current warning. Dummy0600keyfile and fixed owned loopback endpoint only. No official VirusTotal/API key/upload/browser report-link click, real system daemon or native compositor proof."})).unwrap()).unwrap();
}
