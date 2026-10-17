use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let mut config = options(dir.path());
    config.prompt_timeout = 30;
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
        let mut client = protocol::ui_client::UiClient::new(channel);
        client
            .subscribe(protocol::ClientConfig {
                name: "icon-fixture".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        client
    });
    let data = evidence.join("desktop-data");
    assert_eq!(
        PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap()),
        data
    );
    let applications = data.join("applications");
    std::fs::create_dir_all(&applications).unwrap();
    let png_app = evidence.join("PNG sample app");
    let svg_app = evidence.join("SVG sample app");
    for path in [&png_app, &svg_app] {
        std::fs::write(path, "fixture: never executed").unwrap();
    }
    let png = evidence.join("fixture.png");
    image::RgbaImage::from_fn(64, 64, |x, _| {
        image::Rgba(if x < 32 {
            [250, 20, 20, 255]
        } else {
            [20, 230, 240, 255]
        })
    })
    .save(&png)
    .unwrap();
    let svg = evidence.join("fixture.svg");
    std::fs::write(&svg, r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><path fill="#19e632" d="M0 0h32v64H0z"/><path fill="#e6ba19" d="M32 0h32v64H32z"/></svg>"##).unwrap();
    for (name, app, icon) in [("png", &png_app, &png), ("svg", &svg_app, &svg)] {
        std::fs::write(
            applications.join(format!("{name}.desktop")),
            format!(
                "[Desktop Entry]\nType=Application\nName=Fixture\nExec=\"{}\" %U\nIcon={}\n",
                app.display(),
                icon.display()
            ),
        )
        .unwrap();
    }
    let mut cx = context(test_cx);
    let mut window = None;
    let checkpoints = [
        "01-desktop-png.json",
        "02-busy-fallback.json",
        "03-desktop-svg.json",
        "04-missing-icon.json",
    ];
    for (case, checkpoint_name) in checkpoints.iter().enumerate() {
        let held = (case == 1).then(crate::app::app_icon::hold_workers);
        let app = match case {
            0 => png_app.clone(),
            1 | 2 => svg_app.clone(),
            _ => evidence.join("unregistered-app"),
        };
        let mut asking = daemon.clone();
        let reply = runtime.spawn(async move {
            asking
                .ask_rule(protocol::Connection {
                    process_path: app.to_string_lossy().into_owned(),
                    dst_ip: "192.0.2.42".into(),
                    dst_port: 443,
                    protocol: "tcp".into(),
                    ..Default::default()
                })
                .await
                .unwrap()
                .into_inner()
        });
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                while backend.handle.prompts.snapshot().is_empty() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
        });
        let prompt = *window.get_or_insert_with(|| {
            cx.update(|cx| {
                crate::app::prompt_window::open(backend.handle.clone(), 1., false, cx).unwrap()
            })
        });
        flush(&mut cx);
        let icon_loaded = |window: &mut gpui::Window, cx: &mut App| {
            gpui_kit::base::Root::read(window, cx)
                .view()
                .clone()
                .downcast::<PromptWindow>()
                .unwrap()
                .read(cx)
                .test_icon_loaded()
        };
        if case == 0 || case == 2 {
            wait_window(&mut cx, prompt, icon_loaded);
        } else {
            assert!(
                !cx.update_window(prompt, |_, window, cx| icon_loaded(window, cx))
                    .unwrap(),
                "busy/missing lookup uses a fresh fallback"
            );
        }
        checkpoint(&mut cx, prompt, &evidence.join(checkpoint_name));
        let started = Instant::now();
        click(&mut cx, prompt, "allow-forever");
        let rule = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), reply)
                .await
                .unwrap()
                .unwrap()
        });
        assert_eq!(
            (rule.action.as_str(), rule.duration.as_str()),
            ("allow", "always")
        );
        if case == 1 {
            assert!(
                started.elapsed() < Duration::from_millis(500),
                "icon worker saturation must not delay a permission reply"
            );
        }
        drop(held);
        flush(&mut cx);
    }
    runtime.block_on(backend.handle.history.flush()).unwrap();
    cx.update_window(window.unwrap(), |_, window, _| window.remove_window())
        .unwrap();
    shutdown(cx);
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(evidence.join("headless.json"), serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","checkpoints":checkpoints,"framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"proof_boundary":"Production desktop-entry lookup, PNG/SVG icon loading state, request reset and independent actual gRPC replies while both icon workers are held. Synthetic daemon only."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"), serde_json::to_vec_pretty(&["Show an icon resolved from a quoted desktop Exec and PNG Icon", "Hold both lookup workers: show initials instead of the previous icon and answer immediately", "Release workers and show SVG icon loading on a fresh request", "An unregistered app uses the fallback, not the previous SVG; every request returns allow/always"]).unwrap()).unwrap();
}
