use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::rules::catalogue_policy::{Origin, Preset, Status};
    use gpui::InputEvent as _;
    fn control(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) {
        let footer = cx
            .update_window(w, |_, w, _| {
                w.find(id).path().contains(&"settings-footer".into())
            })
            .unwrap();
        if !footer {
            for _ in 0..32 {
                let delta = cx
                    .update_window(w, |_, w, cx| {
                        w.render_frame(cx);
                        let body = w.find("catalogue-preferences-scroll").bounds();
                        let target = w.find(id).bounds();
                        if target.center().y < body.top() {
                            Some(120.)
                        } else if target.center().y > body.bottom() {
                            Some(-120.)
                        } else {
                            None
                        }
                    })
                    .unwrap();
                let Some(delta) = delta else { break };
                cx.update_window(w, |_, w, cx| {
                    let position = w.find("catalogue-preferences-scroll").bounds().center();
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
                            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(delta))),
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
        }
        click(cx, w, id);
        flush(cx);
    }
    let evidence = evidence();
    let phase = std::env::var("RUSTLEBLOWER_CATALOGUE_TEST_PHASE").ok();
    if phase.is_none() {
        // Two real OS processes are essential: process_run_id is process-scoped.
        let owned = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut phases = Vec::new();
        for name in ["seed", "reopen"] {
            let log = std::fs::File::create(evidence.join(format!("{name}.log"))).unwrap();
            let mut child = std::process::Command::new(&executable)
                .args([
                    "app::kit_tests::catalogue_policy::journey",
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("RUSTLEBLOWER_CATALOGUE_TEST_PHASE", name)
                .env("RUSTLEBLOWER_CATALOGUE_TEST_ROOT", owned.path())
                .stdout(log.try_clone().unwrap())
                .stderr(log)
                .spawn()
                .unwrap();
            let pid = child.id();
            let started = Instant::now();
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if started.elapsed() > Duration::from_secs(150) {
                    child.kill().unwrap();
                    let _ = child.wait();
                    panic!(
                        "Owned catalogue child {name} PID{pid} exceeded150seconds; retained phase log"
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            phases.push(serde_json::json!({"phase":name,"pid":pid,"success":status.success(),"elapsed_seconds":started.elapsed().as_secs_f64()}));
            std::fs::write(
                evidence.join("owned-processes.json"),
                serde_json::to_vec_pretty(&phases).unwrap(),
            )
            .unwrap();
            assert!(
                status.success(),
                "Owned catalogue {name} failed; inspect retained phase log"
            );
            assert!(!owned.path().join("ui.sock").exists());
        }
        let a: serde_json::Value =
            serde_json::from_slice(&std::fs::read(evidence.join("seed-state.json")).unwrap())
                .unwrap();
        let b: serde_json::Value =
            serde_json::from_slice(&std::fs::read(evidence.join("reopen-state.json")).unwrap())
                .unwrap();
        assert_ne!(a["pid"], b["pid"]);
        assert_ne!(a["run"], b["run"]);
        assert_eq!(
            a["wire"], b["wire"],
            "Actual OS restart changes no cached daemon rule fields"
        );
        let checkpoints = std::fs::read_dir(&evidence)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".json"))
            .collect::<Vec<_>>();
        std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":checkpoints,"proof_boundary":"Two owned OS processes and actual Kit catalogue controls; persisted filtering, offline prior-run status, unchanged daemon wire, independent permission response. No host daemon or policy changes."})).unwrap()).unwrap();
        return;
    }
    let phase = phase.unwrap();
    assert!(matches!(phase.as_str(), "seed" | "reopen"));
    let owned = PathBuf::from(std::env::var_os("RUSTLEBLOWER_CATALOGUE_TEST_ROOT").unwrap());
    let config = options(&owned);
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let mut daemon = if phase == "seed" {
        Some(runtime.block_on(async {
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
                    name: "catalogue-fixture".into(),
                    rules: ["once", "30s", "5m", "always", "2h", "until restart"]
                        .into_iter()
                        .enumerate()
                        .map(|(i, duration)| protocol::Rule {
                            name: format!("catalogue.{i}"),
                            description: format!("Owned {duration} catalogue rule"),
                            duration: duration.into(),
                            created: crate::history::now_nanos() / 1_000_000_000 - 10,
                            enabled: true,
                            action: "deny".into(),
                            operator: Some(protocol::Operator {
                                r#type: "simple".into(),
                                operand: "process.path".into(),
                                data: format!("/owned/catalogue-{i}"),
                                ..Default::default()
                            }),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                })
                .await
                .unwrap();
            daemon
        }))
    } else {
        None
    };
    let mut cx = context(test_cx);
    let mut view = None;
    let main = cx
        .open_test_window(size(px(1320.), px(980.)), |w, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), w, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, w, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = view.unwrap();
    cx.update_window(main, |_, w, cx| w.within("main-tabs").click(1usize, cx))
        .unwrap();
    for _ in 0..60 {
        flush(&mut cx);
        if backend.handle.catalogue_policy.snapshot().ready
            && cx.read_entity(&view, |v, _| v.rules.page.rules.len())
                == if phase == "seed" { 6 } else { 4 }
        {
            break;
        }
    }
    if phase == "reopen" {
        pick(&mut cx, main, "rules-cache-node", "catalogue-fixture");
    }
    flush(&mut cx);
    let wire = cx.read_entity(&view, |v, _| {
        v.rules
            .page
            .rules
            .iter()
            .map(crate::rules::document::rule_json)
            .collect::<Vec<_>>()
    });
    if phase == "seed" {
        assert_eq!(wire.len(), 6);
        checkpoint(&mut cx, main, &evidence.join("01-default-all-presets.json"));
        let before = cx.update(|cx| cx.windows());
        click(&mut cx, main, "rules-catalogue-preferences");
        flush(&mut cx);
        let preferences = cx.update(|cx| {
            cx.windows()
                .into_iter()
                .find(|w| !before.contains(w))
                .unwrap()
        });
        control(&mut cx, preferences, "catalogue-ignore");
        control(&mut cx, preferences, "catalogue-threshold");
        cx.update_window(preferences, |_, w, cx| {
            w.press("down", cx);
            w.press("enter", cx)
        })
        .unwrap();
        flush(&mut cx);
        control(&mut cx, preferences, "catalogue-review");
        checkpoint(
            &mut cx,
            preferences,
            &evidence.join("02-exact-filter-review.json"),
        );
        assert_eq!(
            cx.read_entity(&view, |v, _| v.rules.page.rules.len()),
            6,
            "Review leaves catalogue unchanged"
        );
        control(&mut cx, preferences, "catalogue-keep-editing");
        control(&mut cx, preferences, "catalogue-review");
        control(&mut cx, preferences, "catalogue-confirm");
        wait_for(&mut cx, |cx| {
            backend
                .handle
                .catalogue_policy
                .snapshot()
                .settings
                .threshold
                == Preset::FiveMinutes
                && cx.read_entity(&view, |v, _| v.rules.page.rules.len()) == 4
        });
        assert_eq!(
            backend
                .handle
                .catalogue_policy
                .snapshot()
                .settings
                .threshold,
            Preset::FiveMinutes
        );
        assert_eq!(
            cx.read_entity(&view, |v, _| v
                .rules
                .page
                .rules
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<_>>()),
            ["catalogue.2", "catalogue.3", "catalogue.4", "catalogue.5"]
        );
        checkpoint(
            &mut cx,
            main,
            &evidence.join("03-short-presets-hidden.json"),
        );
        click(&mut cx, main, "rules-catalogue-preferences");
        assert_eq!(
            cx.update(|cx| cx.windows()).len(),
            before.len() + 1,
            "Actual opener reuses the preferences singleton"
        );
        let mut client = daemon.take().unwrap();
        let ask = runtime.spawn(async move {
            client
                .ask_rule(protocol::Connection {
                    process_path: "/owned/catalogue-permission".into(),
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
        checkpoint(
            &mut cx,
            popup,
            &evidence.join("04-permission-with-catalogue-preferences.json"),
        );
        click(&mut cx, popup, "deny-once");
        assert_eq!(runtime.block_on(ask).unwrap().action, "deny");
        let pref_path = crate::rules::catalogue_policy::path_for(&config.database);
        let old_revision = backend.handle.catalogue_policy.snapshot().revision.clone();
        control(&mut cx, preferences, "catalogue-ignore");
        control(&mut cx, preferences, "catalogue-review");
        let mut external = b"\n".to_vec();
        external.extend(std::fs::read(&pref_path).unwrap());
        std::fs::write(&pref_path, &external).unwrap();
        control(&mut cx, preferences, "catalogue-confirm");
        assert_eq!(std::fs::read(&pref_path).unwrap(), external);
        assert_eq!(
            backend.handle.catalogue_policy.snapshot().revision,
            old_revision
        );
        assert_eq!(cx.read_entity(&view, |v, _| v.rules.page.rules.len()), 4);
        checkpoint(
            &mut cx,
            preferences,
            &evidence.join("05-stale-review-refused.json"),
        );
        control(&mut cx, preferences, "catalogue-reload");
        wait_for(&mut cx, |_cx| {
            backend.handle.catalogue_policy.snapshot().revision != old_revision
        });
        assert_ne!(
            backend.handle.catalogue_policy.snapshot().revision,
            old_revision
        );
        control(&mut cx, preferences, "catalogue-close");
    } else {
        assert_eq!(wire.len(), 4);
        assert!(cx.read_entity(&view, |v, _| {
            v.rules
                .page
                .origins
                .values()
                .all(|o| *o == Origin::Restored)
        }));
        assert_eq!(
            cx.read_entity(&view, |v, _| v.rules.page.cache_status["catalogue.5"]
                .status),
            Status::ExpiredAtUiRestart
        );
        assert!(cx.read_entity(&view, |v, _| v.rules.page.rules.iter().all(|r| r.enabled)));
        checkpoint(
            &mut cx,
            main,
            &evidence.join("06-real-process-restart-offline-status.json"),
        );
    }
    let visible_wire = cx.read_entity(&view, |v, _| {
        v.rules
            .page
            .rules
            .iter()
            .map(crate::rules::document::rule_json)
            .collect::<Vec<_>>()
    });
    let state = serde_json::json!({"pid":std::process::id(),"run":backend.handle.catalogue_run.unwrap().0,"wire":visible_wire,"settings":backend.handle.catalogue_policy.snapshot().settings});
    std::fs::write(
        evidence.join(format!("{phase}-state.json")),
        serde_json::to_vec_pretty(&state).unwrap(),
    )
    .unwrap();
    drop(view);
    shutdown(cx);
    drop(daemon);
    backend.join();
    drop(runtime);
    assert!(!config.socket.exists());
}
