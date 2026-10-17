use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::nodes;
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let(mut alpha,beta,alpha_tx,mut alpha_commands,beta_tx,mut beta_commands)=runtime.block_on(async{
  use hyper_util::rt::TokioIo;
  async fn connect(path:PathBuf,name:&str)->(protocol::ui_client::UiClient<tonic::transport::Channel>,tokio::sync::mpsc::Sender<protocol::NotificationReply>,tonic::Streaming<protocol::Notification>){let channel=tonic::transport::Endpoint::try_from("http://localhost").unwrap().connect_with_connector(tower::service_fn(move |_|{let path=path.clone();async move{tokio::net::UnixStream::connect(path).await.map(TokioIo::new)}})).await.unwrap();let mut daemon=protocol::ui_client::UiClient::new(channel);daemon.subscribe(protocol::ClientConfig{name:name.into(),version:"synthetic-kernel".into(),is_firewall_running:true,log_level:2,config:r#"{"DefaultAction":"deny","DefaultDuration":"once","Future":{"keep":true}}"#.into(),rules:vec![protocol::Rule{name:format!("{name}-only-rule"),action:"deny".into(),duration:"always".into(),enabled:true,operator:Some(protocol::Operator{r#type:"simple".into(),operand:"process.path".into(),data:format!("/bin/{name}"),..Default::default()}),..Default::default()}],..Default::default()}).await.unwrap();let(tx,rx)=tokio::sync::mpsc::channel(8);let stream=daemon.notifications(tokio_stream::wrappers::ReceiverStream::new(rx)).await.unwrap().into_inner();(daemon,tx,stream)}
  let(a,at,as_)=connect(config.socket.clone(),"alpha-node").await;let(b,bt,bs)=connect(config.socket.clone(),"beta-node").await;backend.handle.history.flush().await.unwrap();(a,b,at,as_,bt,bs)
 });
    let mut cx = context(test_cx);
    let mut view = None;
    let window = cx
        .open_test_window(size(px(1180.), px(900.)), |window, cx| {
            let dashboard =
                cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx));
            view = Some(dashboard.clone());
            cx.new(|cx| gpui_kit::base::Root::new(dashboard, window, cx))
        })
        .unwrap();
    let window: AnyWindowHandle = window.into();
    let view = view.unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.within("main-tabs").click(3usize, cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        (cx.read_entity(&view, |view, _| view.nodes.page.len())) == (2)
    });

    assert_eq!(cx.read_entity(&view, |view, _| view.nodes.page.len()), 2);
    checkpoint(&mut cx, window, &evidence.join("01-connected-nodes.json"));
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 1usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, window, "nodes-rules");
    wait_for(&mut cx, |_cx| {
        (backend.handle.status().session.unwrap().name) == ("beta-node")
    });

    assert_eq!(backend.handle.status().session.unwrap().name, "beta-node");
    assert_eq!(
        cx.read_entity(&view, |view, _| view.rules.page.rules[0].name.clone()),
        "beta-node-only-rule"
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("02-selected-node-rules.json"),
    );
    cx.update_window(window, |_, window, cx| window.press("ctrl-4", cx))
        .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 0usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        None
    );
    click(&mut cx, window, "nodes-remember-pause");
    flush(&mut cx);
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        Some(true)
    );
    click(&mut cx, window, "nodes-interception");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("03-stop-interception-review.json"),
    );
    click(&mut cx, window, "nodes-cancel");
    assert!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    click(&mut cx, window, "nodes-interception");
    click(&mut cx, window, "nodes-confirm");
    flush(&mut cx);
    let command = runtime.block_on(alpha_commands.message()).unwrap().unwrap();
    assert_eq!(command.client_name, "alpha-node");
    assert_eq!(command.r#type, protocol::Action::DisableInterception as i32);
    assert!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("04-held-interception-ack.json"),
    );
    // Another node's actual AskRule remains usable while interception ACK is held.
    let mut asking = beta.clone();
    let ask = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/bin/beta-prompt".into(),
                dst_ip: "192.0.2.10".into(),
                dst_port: 443,
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
        &evidence.join("05-other-node-permission.json"),
    );
    click(&mut cx, prompt, "allow-forever");
    flush(&mut cx);
    assert_eq!(runtime.block_on(ask).unwrap().action, "allow");
    runtime
        .block_on(alpha_tx.send(protocol::NotificationReply {
            id: command.id,
            code: protocol::NotificationReplyCode::Ok as i32,
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx);
    assert!(
        !nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    assert!(
        nodes::detail(&config.database, "beta-node")
            .unwrap()
            .row
            .running
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("06-confirmed-interception.json"),
    );
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window, "nodes-config");
    flush(&mut cx);
    let editor = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|handle| !before.contains(handle))
            .unwrap()
    });
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-config-modes").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(editor, |_, window, cx| {
        window.press("ctrl-a", cx);
        window.input("[]", cx)
    })
    .unwrap();
    click(&mut cx, editor, "node-config-review");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("07-invalid-node-config.json"),
    );
    let document = r#"{"DefaultAction":"allow","DefaultDuration":"5m","Future":{"keep":true}}"#;
    cx.update_window(editor, |_, window, cx| {
        window.press("tab", cx);
    })
    .unwrap();
    // The initial textarea is the first focusable control; click Review restored
    // button focus, so Shift+Tab returns to the actual multiline editor.
    cx.update_window(editor, |_, window, cx| {
        window.press("shift-tab", cx);
        window.press("shift-tab", cx);
        window.press("ctrl-a", cx);
        window.input(document, cx)
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, editor, "node-config-review");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("08-review-node-config.json"),
    );
    click(&mut cx, editor, "node-config-cancel");
    flush(&mut cx);
    // Review again uses the staged user document, not a state setter.
    click(&mut cx, editor, "node-config-review");
    click(&mut cx, editor, "node-config-confirm");
    flush(&mut cx);
    let command = runtime.block_on(alpha_commands.message()).unwrap().unwrap();
    assert_eq!(command.r#type, protocol::Action::ChangeConfig as i32);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&command.data).unwrap()["Future"]["keep"],
        true
    );
    assert_ne!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .config,
        document
    );
    runtime
        .block_on(alpha_tx.send(protocol::NotificationReply {
            id: command.id,
            code: protocol::NotificationReplyCode::Ok as i32,
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx);
    assert_eq!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .config,
        document
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("09-confirmed-node-config.json"),
    );
    click(&mut cx, editor, "node-config-close");
    flush(&mut cx);
    // An ERROR keeps beta's confirmed running state and marks only beta uncertain.
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 1usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, window, "nodes-interception");
    click(&mut cx, window, "nodes-confirm");
    flush(&mut cx);
    let command = runtime.block_on(beta_commands.message()).unwrap().unwrap();
    runtime
        .block_on(beta_tx.send(protocol::NotificationReply {
            id: command.id,
            code: protocol::NotificationReplyCode::Error as i32,
            data: "synthetic interception failure".into(),
        }))
        .unwrap();
    flush(&mut cx);
    assert!(
        nodes::detail(&config.database, "beta-node")
            .unwrap()
            .row
            .running
    );
    assert!(
        !backend
            .handle
            .history
            .health
            .catalogues
            .uncertain(backend.handle.node("alpha-node").unwrap().id)
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("10-node-command-error.json"),
    );
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.input("absent-node", cx)
    })
    .unwrap();
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| view.nodes.page.is_empty())
    });

    assert!(cx.read_entity(&view, |view, _| view.nodes.page.is_empty()));
    checkpoint(&mut cx, window, &evidence.join("11-node-search-empty.json"));
    cx.update_window(window, |_, window, cx| {
        window.press("ctrl-f", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.click(("node-row", 0usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        Some(false)
    );
    checkpoint(&mut cx, window, &evidence.join("12-remembered-pause.json"));
    drop(alpha_commands);
    drop(alpha_tx);
    let (alpha_tx, mut alpha_commands) = runtime.block_on(async {
        let saved = nodes::detail(&config.database, "alpha-node").unwrap();
        alpha
            .subscribe(protocol::ClientConfig {
                name: "alpha-node".into(),
                version: "reconnected-fixture-kernel".into(),
                is_firewall_running: true,
                config: saved.config,
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let stream = alpha
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        backend.handle.history.flush().await.unwrap();
        (tx, stream)
    });
    let command = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), alpha_commands.message())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    });
    assert_eq!(command.r#type, protocol::Action::DisableInterception as i32);
    assert_eq!(command.client_name, "alpha-node");
    flush(&mut cx);
    assert!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("13-restoration-awaits-ack.json"),
    );
    let mut asking = beta.clone();
    let reply = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/owned/pause-restore-independent".into(),
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
        &evidence.join("14-permission-during-restoration.json"),
    );
    click(&mut cx, prompt, "deny-once");
    assert_eq!(runtime.block_on(reply).unwrap().action, "deny");
    runtime
        .block_on(alpha_tx.send(protocol::NotificationReply {
            id: command.id,
            code: protocol::NotificationReplyCode::Ok as i32,
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx);
    flush(&mut cx);
    assert!(
        !nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        Some(false)
    );
    checkpoint(&mut cx, window, &evidence.join("15-restored-pause.json"));
    flush(&mut cx);
    assert!(
        !backend
            .handle
            .control
            .node_busy(backend.handle.node("alpha-node").unwrap().id)
    );
    click(&mut cx, window, "nodes-remember-pause");
    flush(&mut cx);
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        None
    );
    assert!(
        !nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(50), alpha_commands.message()).await
            })
            .is_err()
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("16-opt-out-keeps-current-pause.json"),
    );
    click(&mut cx, window, "nodes-interception");
    click(&mut cx, window, "nodes-cancel");
    assert!(
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_millis(50), alpha_commands.message()).await
            })
            .is_err()
    );
    click(&mut cx, window, "nodes-interception");
    click(&mut cx, window, "nodes-confirm");
    flush(&mut cx);
    let command = runtime.block_on(alpha_commands.message()).unwrap().unwrap();
    assert_eq!(command.r#type, protocol::Action::EnableInterception as i32);
    runtime
        .block_on(alpha_tx.send(protocol::NotificationReply {
            id: command.id,
            code: protocol::NotificationReplyCode::Ok as i32,
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx);
    assert!(
        nodes::detail(&config.database, "alpha-node")
            .unwrap()
            .row
            .running
    );
    assert_eq!(
        backend.handle.history.health.interception.get("alpha-node"),
        None
    );
    checkpoint(
        &mut cx,
        window,
        &evidence.join("17-manual-resume-after-opt-out.json"),
    );
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    shutdown(cx);
    drop(alpha_tx);
    drop(beta_tx);
    drop(alpha_commands);
    drop(beta_commands);
    drop(alpha);
    drop(beta);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let checkpoints = (1..=17)
        .zip([
            "connected-nodes",
            "selected-node-rules",
            "stop-interception-review",
            "held-interception-ack",
            "other-node-permission",
            "confirmed-interception",
            "invalid-node-config",
            "review-node-config",
            "confirmed-node-config",
            "node-command-error",
            "node-search-empty",
            "remembered-pause",
            "restoration-awaits-ack",
            "permission-during-restoration",
            "restored-pause",
            "opt-out-keeps-current-pause",
            "manual-resume-after-opt-out",
        ])
        .map(|(index, name)| format!("{index:02}-{name}.json"))
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":checkpoints,"boundary":"Actual GPUI input, two isolated gRPC daemon fixtures, acknowledged node controls and SQLite; opt-in per-node pause, current-generation reconnect restoration under held ACK, independent permission, opt-out without auto-resume and manual resume; system daemon/firewall unchanged"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Select Nodes through main tab; choose beta for its rules", "Cancel then confirm alpha interception; hold ACK, answer beta permission through popup; ACK updates only alpha", "Reject nonobject JSON; edit complete configuration, review/back/confirm; ACK persists unknown fields", "Beta ERROR preserves confirmed values and marks only beta uncertain; typed search reaches empty state"]).unwrap()).unwrap();
}
