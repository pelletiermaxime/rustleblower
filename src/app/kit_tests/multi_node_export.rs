use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::rules::{export_nodes, transfer::Import};
    type Client = protocol::ui_client::UiClient<tonic::transport::Channel>;
    fn rule(i: usize, action: &str) -> protocol::Rule {
        protocol::Rule {
            name: format!("export.{i:03}"),
            description: format!("Export rule {i}"),
            enabled: true,
            created: 123456,
            precedence: true,
            nolog: true,
            action: action.into(),
            duration: "always".into(),
            operator: Some(protocol::Operator {
                r#type: "simple".into(),
                operand: "process.path".into(),
                data: format!("/owned/export-{i}"),
                sensitive: true,
                ..Default::default()
            }),
        }
    }
    async fn connect(path: PathBuf, name: &str, rules: Vec<protocol::Rule>) -> Client {
        use hyper_util::rt::TokioIo;
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
        let mut client = Client::new(channel);
        client
            .subscribe(protocol::ClientConfig {
                name: name.into(),
                rules,
                ..Default::default()
            })
            .await
            .unwrap();
        client
    }
    fn state(cx: &mut impl AppContext, w: AnyWindowHandle) -> (bool, usize) {
        cx.update_window(w, |_, window, cx| {
            gpui_kit::base::Root::read(window, cx)
                .view()
                .clone()
                .downcast::<crate::app::rule_exports::Exports>()
                .unwrap()
                .read(cx)
                .test_state()
        })
        .unwrap()
    }
    fn finish(cx: &mut TestAppContext, w: AnyWindowHandle) {
        wait_for(cx, |cx| !state(cx, w).0);
    }
    fn folders(parent: &Path) -> Vec<PathBuf> {
        let mut p = std::fs::read_dir(parent)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_dir())
            .collect::<Vec<_>>();
        p.sort();
        p
    }
    fn verify(
        folder: &Path,
        alpha: Vec<protocol::Rule>,
        beta: Vec<protocol::Rule>,
        offline: Vec<protocol::Rule>,
    ) {
        let m: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.join("manifest.json")).unwrap()).unwrap();
        for n in m["nodes"].as_array().unwrap() {
            let mut got = Import::from_path(&folder.join(n["directory"].as_str().unwrap()))
                .map(|i| i.rules)
                .unwrap_or_default();
            got.sort_by(|a, b| a.name.cmp(&b.name));
            let expected = match n["name"].as_str().unwrap() {
                "alpha" => &alpha,
                "beta" => &beta,
                _ => &offline,
            };
            assert_eq!(&got, expected);
        }
    }
    let evidence = evidence();
    let output = evidence.join("exports");
    std::fs::create_dir(&output).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (mut alpha, beta) = runtime.block_on(async {
        let a = connect(
            config.socket.clone(),
            "alpha",
            (0..250).map(|i| rule(i, "allow")).collect(),
        )
        .await;
        let b = connect(config.socket.clone(), "beta", vec![rule(0, "deny")]).await;
        backend
            .handle
            .history
            .replace_rules(900, "offline".into(), vec![rule(205, "reject")]);
        backend.handle.history.flush().await.unwrap();
        (a, b)
    });
    assert_eq!(
        export_nodes::catalogue(&config.database).unwrap(),
        vec!["alpha", "beta", "offline"]
    );
    let mut cx = context(test_cx);
    let mut view = None;
    let main = cx
        .open_test_window(size(px(1240.), px(940.)), |window, cx| {
            let v = cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(v.clone());
            cx.new(|cx| gpui_kit::base::Root::new(v, window, cx))
        })
        .unwrap();
    let main: AnyWindowHandle = main.into();
    let view = view.unwrap();
    flush(&mut cx);
    cx.update_window(main, |_, w, cx| w.press("ctrl-2", cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(main, |_, w, cx| {
        w.press("ctrl-f", cx);
        w.input("export-205", cx);
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, main, "rules-export-nodes");
    let export = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    finish(&mut cx, export);
    fill(
        &mut cx,
        export,
        "nodes-export-path",
        output.to_str().unwrap(),
    );
    click(&mut cx, export, "nodes-export-all");
    checkpoint(
        &mut cx,
        export,
        &evidence.join("01-cached-node-selection.json"),
    );
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    assert_eq!(state(&mut cx, export).1, 252);
    checkpoint(
        &mut cx,
        export,
        &evidence.join("02-complete-node-review.json"),
    );
    click(&mut cx, export, "nodes-export-close");
    flush(&mut cx);
    assert!(folders(&output).is_empty());
    checkpoint(
        &mut cx,
        main,
        &evidence.join("03-cancel-publishes-nothing.json"),
    );
    click(&mut cx, main, "rules-export-nodes");
    let export = cx.read_entity(&view, |v, _| v.rules.editor_window.unwrap());
    finish(&mut cx, export);
    fill(
        &mut cx,
        export,
        "nodes-export-path",
        output.to_str().unwrap(),
    );
    click(&mut cx, export, "nodes-export-all");
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    assert_eq!(state(&mut cx, export).1, 252);
    // Replacing the live inventory after review must not change the snapshot.
    runtime.block_on(async {
        alpha
            .subscribe(protocol::ClientConfig {
                name: "alpha".into(),
                rules: vec![rule(205, "reject")],
                ..Default::default()
            })
            .await
            .unwrap();
        backend.handle.history.flush().await.unwrap();
    });
    let mut asking = beta.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/export-prompt".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    let prompt = permission_window(&mut cx, &view);
    checkpoint(
        &mut cx,
        prompt,
        &evidence.join("04-permission-with-export-review.json"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    let first = folders(&output);
    assert_eq!(first.len(), 1);
    verify(
        &first[0],
        (0..250).map(|i| rule(i, "allow")).collect(),
        vec![rule(0, "deny")],
        vec![rule(205, "reject")],
    );
    checkpoint(
        &mut cx,
        export,
        &evidence.join("05-frozen-export-completed.json"),
    );
    click(&mut cx, export, "nodes-export-back");
    click(&mut cx, export, "nodes-export-filter");
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    assert_eq!(state(&mut cx, export).1, 2);
    checkpoint(
        &mut cx,
        export,
        &evidence.join("06-filtered-node-review.json"),
    );
    let marker = output.join("existing-file");
    std::fs::write(&marker, b"preserve").unwrap();
    fill(
        &mut cx,
        export,
        "nodes-export-path",
        marker.to_str().unwrap(),
    );
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    assert_eq!(folders(&output).len(), 1);
    assert_eq!(std::fs::read(&marker).unwrap(), b"preserve");
    checkpoint(
        &mut cx,
        export,
        &evidence.join("07-invalid-parent-preserved.json"),
    );
    fill(
        &mut cx,
        export,
        "nodes-export-path",
        output.to_str().unwrap(),
    );
    click(&mut cx, export, "nodes-export-apply");
    finish(&mut cx, export);
    let both = folders(&output);
    assert_eq!(both.len(), 2);
    let second = both.iter().find(|p| !first.contains(p)).unwrap();
    verify(
        second,
        vec![rule(205, "reject")],
        vec![],
        vec![rule(205, "reject")],
    );
    checkpoint(
        &mut cx,
        export,
        &evidence.join("08-filtered-export-completed.json"),
    );
    click(&mut cx, export, "nodes-export-close");
    drop(view);
    shutdown(cx);
    drop(alpha);
    drop(beta);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let frames = [
        "cached-node-selection",
        "complete-node-review",
        "cancel-publishes-nothing",
        "permission-with-export-review",
        "frozen-export-completed",
        "filtered-node-review",
        "invalid-parent-preserved",
        "filtered-export-completed",
    ]
    .iter()
    .enumerate()
    .map(|(i, n)| format!("{:02}-{n}.json", i + 1))
    .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":frames,"proof_boundary":"Actual cached-node selection, typed path, Cancel, immutable full/filter review, filesystem error/retry and complete per-node JSON outputs. Real synthetic Subscribe/AskRule; offline inventory supplied through production history boundary. Permission proof is while export review is open, not under a deliberately stalled disk operation. No native OS picker, system daemon or kernel enforcement proof."})).unwrap()).unwrap();
}
