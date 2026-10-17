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
    let hold = Arc::new(AtomicBool::new(true));
    let accept = Arc::new(AtomicBool::new(true));
    let observed = Arc::new(Mutex::new(Vec::<protocol::Notification>::new()));
    let (daemon,task)=runtime.block_on(async {
  use hyper_util::rt::TokioIo;let path=config.socket.clone();let channel=tonic::transport::Endpoint::try_from("http://localhost").unwrap().connect_with_connector(tower::service_fn(move |_|{let path=path.clone();async move{tokio::net::UnixStream::connect(path).await.map(TokioIo::new)}})).await.unwrap();let mut daemon=protocol::ui_client::UiClient::new(channel);
  daemon.subscribe(protocol::ClientConfig{name:"preferences-fixture".into(),is_firewall_running:true,config:r#"{"DefaultAction":"deny","DefaultDuration":"once","ProcMonitorMethod":"proc","Rules":{"Path":"/old/rules","Future":true},"Server":{"Address":"unix:///fixture.sock","Authentication":{"Type":"simple","Future":"keep"}},"Future":{"precise":1.2345678901234567890123456789}}"#.into(),..Default::default()}).await.unwrap();let(tx,rx)=tokio::sync::mpsc::channel(8);let mut stream=daemon.notifications(tokio_stream::wrappers::ReceiverStream::new(rx)).await.unwrap().into_inner();let hold=hold.clone();let accept=accept.clone();let observed=observed.clone();let task=tokio::spawn(async move{while let Some(command)=stream.message().await.unwrap(){assert!(observed.lock().unwrap().len()<16);observed.lock().unwrap().push(command.clone());while hold.load(Ordering::Acquire){tokio::time::sleep(Duration::from_millis(5)).await;}tx.send(protocol::NotificationReply{id:command.id,code:if accept.load(Ordering::Acquire){protocol::NotificationReplyCode::Ok}else{protocol::NotificationReplyCode::Error}as i32,data:if accept.load(Ordering::Acquire){""}else{"synthetic config save failure"}.into()}).await.unwrap();}});backend.handle.history.flush().await.unwrap();(daemon,task)
 });
    let original = nodes::detail(&config.database, "preferences-fixture")
        .unwrap()
        .config;
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
        window.press("ctrl-4", cx);
    })
    .unwrap();
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    flush(&mut cx);
    let before = cx.update(|cx| cx.windows());
    click(&mut cx, window, "nodes-config");
    flush(&mut cx);
    let editor = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|handle| !before.contains(handle))
            .unwrap()
    });
    checkpoint(&mut cx, editor, &evidence.join("01-daemon-general.json"));
    pick(&mut cx, editor, "daemon-default-action", "Allow");
    pick(&mut cx, editor, "daemon-default-duration", "Forever");
    fill(&mut cx, editor, "daemon-rules-path", "/new/rules");
    cx.update_window(editor, |_, window, cx| {
        window.scroll(
            "daemon-rules-path",
            gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-250.))),
            cx,
        )
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, editor, "daemon-checksums");
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-setting-groups").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    pick(&mut cx, editor, "daemon-log-level", "Warning");
    click(&mut cx, editor, "daemon-log-utc");
    checkpoint(&mut cx, editor, &evidence.join("02-daemon-logging.json"));
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-setting-groups").click(3usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    fill(&mut cx, editor, "daemon-max-events", "-1");
    click(&mut cx, editor, "node-config-review");
    flush(&mut cx);
    assert!(observed.lock().unwrap().is_empty());
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("03-invalid-daemon-limit.json"),
    );
    fill(&mut cx, editor, "daemon-max-events", "300");
    fill(&mut cx, editor, "daemon-max-stats", "75");
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-setting-groups").click(2usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    fill(&mut cx, editor, "daemon-firewall-interval", "1m30s");
    checkpoint(&mut cx, editor, &evidence.join("04-daemon-firewall.json"));
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-setting-groups").click(4usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    pick(&mut cx, editor, "daemon-auth-type", "Mutual TLS");
    fill(&mut cx, editor, "daemon-ca-cert", "/daemon/ca.pem");
    checkpoint(&mut cx, editor, &evidence.join("05-daemon-connection.json"));
    cx.update_window(editor, |_, window, cx| window.press("ctrl-enter", cx))
        .unwrap();
    flush(&mut cx);
    assert!(observed.lock().unwrap().is_empty());
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("06-review-daemon-changes.json"),
    );
    click(&mut cx, editor, "node-config-cancel");
    flush(&mut cx);
    // Switching through JSON and back must preserve staged typed values and unknown numbers.
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-config-modes").click(1usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("07-complete-daemon-json.json"),
    );
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-config-modes").click(0usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, editor, "node-config-review");
    click(&mut cx, editor, "node-config-confirm");
    flush(&mut cx);
    assert_eq!(observed.lock().unwrap().len(), 1);
    assert_eq!(
        nodes::detail(&config.database, "preferences-fixture")
            .unwrap()
            .config,
        original
    );
    let command = observed.lock().unwrap()[0].clone();
    assert_eq!(command.r#type, protocol::Action::ChangeConfig as i32);
    let value: serde_json::Value = serde_json::from_str(&command.data).unwrap();
    assert_eq!(value["DefaultAction"], "allow");
    assert_eq!(value["DefaultDuration"], "always");
    assert_eq!(value["Rules"]["Path"], "/new/rules");
    assert_eq!(value["Rules"]["EnableChecksums"], true);
    assert_eq!(value["Rules"]["Future"], true);
    assert_eq!(value["LogLevel"], 3);
    assert_eq!(value["LogUTC"], true);
    assert_eq!(value["Stats"]["MaxEvents"], 300);
    assert_eq!(value["Stats"]["MaxStats"], 75);
    assert!(value["Stats"].get("Workers").is_none());
    assert_eq!(value["FwOptions"]["MonitorInterval"], "1m30s");
    assert!(value["FwOptions"].get("QueueNum").is_none());
    assert_eq!(value["Server"]["Address"], "unix:///fixture.sock");
    assert_eq!(value["Server"]["Authentication"]["Type"], "tls-mutual");
    assert_eq!(value["Server"]["Authentication"]["Future"], "keep");
    assert_eq!(
        value["Server"]["Authentication"]["TLSOptions"]["CACert"],
        "/daemon/ca.pem"
    );
    assert!(command.data.contains("1.2345678901234567890123456789"));
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("08-awaiting-daemon-save.json"),
    );
    let mut asking = daemon.clone();
    let ask = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/bin/config-prompt".into(),
                dst_ip: "192.0.2.1".into(),
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
        &evidence.join("09-permission-during-config.json"),
    );
    click(&mut cx, prompt, "allow-forever");
    flush(&mut cx);
    let reply = runtime.block_on(ask).unwrap();
    assert_eq!(reply.action, "allow");
    assert_eq!(reply.duration, "always");
    hold.store(false, Ordering::Release);
    flush(&mut cx);
    assert_eq!(
        nodes::detail(&config.database, "preferences-fixture")
            .unwrap()
            .config,
        command.data
    );
    checkpoint(
        &mut cx,
        editor,
        &evidence.join("10-confirmed-daemon-settings.json"),
    );
    // A subsequent error must preserve the first confirmed document.
    cx.update_window(editor, |_, window, cx| {
        window.within("daemon-setting-groups").click(0usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    pick(&mut cx, editor, "daemon-default-action", "Reject");
    accept.store(false, Ordering::Release);
    click(&mut cx, editor, "node-config-review");
    click(&mut cx, editor, "node-config-confirm");
    flush(&mut cx);
    assert_eq!(observed.lock().unwrap().len(), 2);
    assert_eq!(
        nodes::detail(&config.database, "preferences-fixture")
            .unwrap()
            .config,
        command.data
    );
    assert!(
        backend
            .handle
            .history
            .health
            .catalogues
            .uncertain(backend.handle.node("preferences-fixture").unwrap().id)
    );
    checkpoint(&mut cx, editor, &evidence.join("11-daemon-save-error.json"));
    click(&mut cx, editor, "node-config-close");
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    shutdown(cx);
    task.abort();
    drop(daemon);
    drop(runtime);
    backend.join();
    assert!(!config.socket.exists());
    let checkpoints = (1..=11)
        .zip([
            "daemon-general",
            "daemon-logging",
            "invalid-daemon-limit",
            "daemon-firewall",
            "daemon-connection",
            "review-daemon-changes",
            "complete-daemon-json",
            "awaiting-daemon-save",
            "permission-during-config",
            "confirmed-daemon-settings",
            "daemon-save-error",
        ])
        .map(|(index, name)| format!("{index:02}-{name}.json"))
        .collect::<Vec<_>>();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":checkpoints,"boundary":"Actual Kit typed settings, review/JSON modes and input, isolated gRPC ACK/error, SQLite persistence and independent permission; real daemon untouched"})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Enter selects first node; open native daemon forms", "Edit policy/rules/logging/statistics/firewall/TLS fields, reject negative limit before sending", "Review/back/JSON/settings preserves only changed fields and exact unknown numbers", "Hold ACK without cache update, answer Allow forever despite daemon duration change, confirm save", "Second configuration ERROR preserves confirmed data and blocks uncertain node changes"]).unwrap()).unwrap();
}
