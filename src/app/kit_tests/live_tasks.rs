use super::*;

#[gpui_kit::test]
fn journey(test_cx: &mut TestAppContext) {
    use crate::app::live_view::View;
    let evidence = evidence();
    let dir = tempfile::tempdir().unwrap();
    let config = options(dir.path());
    let backend = Backend::start(config.clone()).unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (daemon, replies, mut commands) = runtime.block_on(async {
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
                name: "live-fixture".into(),
                version: "test-kernel".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let commands = daemon
            .notifications(tokio_stream::wrappers::ReceiverStream::new(rx))
            .await
            .unwrap()
            .into_inner();
        (daemon, tx, commands)
    });
    let recorded = Arc::new(Mutex::new(Vec::<protocol::Notification>::new()));
    let log = recorded.clone();
    let reply = replies.clone();
    let task=runtime.spawn(async move{
 let mut periodic=tokio::task::JoinSet::new();
 let mut running=std::collections::BTreeMap::<String,tokio::task::AbortHandle>::new();
 while let Ok(Some(command))=commands.message().await{
  eprintln!("Live fixture command {}: {}",command.r#type,command.data);log.lock().unwrap().push(command.clone());
 if command.r#type==14 {if let Some(task)=running.remove(&command.data){task.abort();}continue;}
 if command.r#type!=13{continue;}
 assert!(command.id > 10_000, "OpenSnitch routes low task IDs to alerts instead of live replies");
  let doc:serde_json::Value=serde_json::from_str(&command.data).unwrap();let name=doc["name"].as_str().unwrap();if name=="node-monitor"{continue;}
  let data=if name=="sockets-monitor"{
   let table=(0..225).filter(|i|doc["data"]["proto"]==0||doc["data"]["proto"]==if i%2==0{6}else{17}).filter(|i|doc["data"]["family"]==0||doc["data"]["family"]==if i%2==0{10}else{2}).map(|i|serde_json::json!({"PID":42,"Proto":if i%2==0{6}else{17},"Iface":"eth0","Socket":{"ID":{"Source":if i%2==0{"2001:db8::1"}else{"192.0.2.1"},"Destination":if i%2==0{"2001:db8::2"}else{"198.51.100.2"},"SourcePort":30000+i,"DestinationPort":443,"Cookie":[1,2]},"Family":if i%2==0{10}else{2},"State":1,"UID":1000,"INode":5000+i,"RQueue":7,"WQueue":9}})).collect::<Vec<_>>();
   serde_json::json!({"Table":table,"Processes":{"42":{"Path":"/usr/bin/live-fixture","Comm":"live-fixture"}}})
  }else{
   assert_eq!(doc["data"]["pid"],"42");serde_json::json!({"PID":42,"Path":"/usr/bin/live-fixture","Comm":"live-fixture","CWD":"/tmp/owned","Args":["live-fixture","two words"],"Status":"Name:\tlive-fixture\nPid:\t42\nVmRSS:\t4096 kB", "Descriptors":[{"Name":"3","SymLink":"socket:[5000]","Size":0},{"Name":"4","SymLink":"/tmp/owned-file","Size":10}],"IOStats":{"RChar":1234,"WChar":5678,"ReadBytes":8192},"Maps":"1000-2000 r-xp /usr/bin/live-fixture\n2000-3000 rw-p [heap]","Stack":"fixture_frame\nnext_frame","Env":{"SYNTHETIC_LIVE_ENV":"requested explicitly","UNICODE":"Ω"},"Checksums":{"md5":"fixture-md5"},"Tree":[{"key":"/usr/bin/parent","value":1}],"Statm":{"Size":1024,"Resident":128},"Future":18446744073709551615_u64})
  }.to_string();
  for _ in 0..2{if reply.send(protocol::NotificationReply{id:command.id,data:data.clone(),..Default::default()}).await.is_err(){return;}}
  let seconds=doc["data"]["interval"].as_str().unwrap().trim_end_matches('s').parse::<u64>().unwrap();
  let sender=reply.clone();let id=command.id;let key=command.data;
  assert!(!running.contains_key(&key),"fixture task identity must be stopped before restart");
  let abort=periodic.spawn(async move {loop {
   tokio::time::sleep(Duration::from_secs(seconds)).await;
   if sender.send(protocol::NotificationReply{id,data:data.clone(),..Default::default()}).await.is_err(){break;}
  }});
  running.insert(key,abort);
  while periodic.try_join_next().is_some(){}
 }});
    let mut cx = context(test_cx);
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(crate::app::dashboard_options(), cx, |window, cx| {
            cx.new(|cx| Dashboard::new(backend.handle.clone(), config.clone(), window, cx))
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| window.press("ctrl-6", cx))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, window, "live-start");
    flush(&mut cx);
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| {
            view.live.snapshot.revision >= 2 && view.live.parsed.is_some()
        })
    });
    cx.read_entity(&view, |view, _| {
        assert!(view.live.snapshot.revision >= 2);
        let Some(View::Sockets { data, matches }) = &view.live.parsed else {
            panic!("live sockets must parse")
        };
        assert_eq!(data.rows.len(), 225);
        assert_eq!(matches.len(), 225);
    });
    checkpoint(&mut cx, window, &evidence.join("01-live-sockets.json"));
    click(&mut cx, window, "live-next");
    checkpoint(
        &mut cx,
        window,
        &evidence.join("02-live-sockets-page-two.json"),
    );
    click(&mut cx, window, "live-pause");
    flush(&mut cx);
    pick(&mut cx, window, "live-protocol", "TCP");
    pick(&mut cx, window, "live-family", "IPv6");
    pick(&mut cx, window, "live-state", "Established");
    pick(&mut cx, window, "live-interval", "2 seconds");
    click(&mut cx, window, "live-start");
    flush(&mut cx);
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view,_| matches!(&view.live.parsed,Some(View::Sockets{data,..}) if data.rows.len()==113))
    });
    cx.read_entity(&view, |view, _| {
        let Some(View::Sockets { data, .. }) = &view.live.parsed else {
            panic!()
        };
        assert_eq!(data.rows.len(), 113);
        assert!(
            data.rows
                .iter()
                .all(|r| r.title.contains("TCP") && r.title.contains("IPv6"))
        );
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("03-live-daemon-filters.json"),
    );
    click(&mut cx, window, "live-pause");
    flush(&mut cx);
    cx.update_window(window, |_, window, cx| {
        window.within("table").click(("row", 0usize), cx)
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, window, "live-copy-row");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("RQueue")
    );
    click(&mut cx, window, "live-process");
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |v, _| {
            matches!(&v.live.parsed, Some(View::Fields { .. }))
        })
    });
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("04-live-process-status.json"),
    );
    pick(&mut cx, window, "live-field", "Env");
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("05-live-process-environment.json"),
    );
    click(&mut cx, window, "live-copy-field");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("SYNTHETIC_LIVE_ENV")
    );
    pick(&mut cx, window, "live-field", "Descriptors");
    flush(&mut cx);
    fill(&mut cx, window, "live-search", "socket:[");
    flush(&mut cx);
    cx.read_entity(&view, |view, _| {
        let Some(View::Fields { lines, .. }) = &view.live.parsed else {
            panic!()
        };
        assert_eq!(lines.len(), 1);
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("06-live-process-sockets.json"),
    );
    fill(&mut cx, window, "live-search", "");
    pick(&mut cx, window, "live-field", "Maps");
    flush(&mut cx);
    checkpoint(&mut cx, window, &evidence.join("07-live-process-maps.json"));
    click(&mut cx, window, "live-copy-json");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("18446744073709551615")
    );
    let pid_command = recorded
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|n| n.r#type == 13)
        .unwrap()
        .id;
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: pid_command,
            code: 1,
            data: "Synthetic process exited".into(),
        }))
        .unwrap();
    flush(&mut cx);
    wait_for(&mut cx, |cx| {
        cx.read_entity(&view, |view, _| !view.live.snapshot.running)
    });
    cx.read_entity(&view, |view, _| {
        assert!(!view.live.snapshot.running);
        assert!(
            view.live
                .snapshot
                .error
                .as_ref()
                .unwrap()
                .contains("exited")
        );
        assert!(view.live.parsed.is_some());
    });
    checkpoint(
        &mut cx,
        window,
        &evidence.join("08-live-process-error-retained.json"),
    );
    cx.update_window(window, |_, window, cx| {
        window.within("live-tabs").click(2usize, cx)
    })
    .unwrap();
    flush(&mut cx);
    click(&mut cx, window, "live-start");
    flush(&mut cx);
    let mut asking = daemon.clone();
    let request = runtime.spawn(async move {
        asking
            .ask_rule(protocol::Connection {
                process_path: "/usr/bin/live-permission".into(),
                dst_host: "live.example".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
    });
    flush(&mut cx);
    let popup = permission_window(&mut cx, &view);
    checkpoint(
        &mut cx,
        popup,
        &evidence.join("09-live-permission-while-awaiting.json"),
    );
    click(&mut cx, popup, "deny-once");
    assert_eq!(runtime.block_on(request).unwrap().action, "deny");
    let resource = recorded
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|n| n.r#type == 13)
        .unwrap()
        .clone();
    let resource_config: serde_json::Value = serde_json::from_str(&resource.data).unwrap();
    assert_eq!(resource_config["data"]["node"], "live-fixture");
    let sysinfo=serde_json::json!({"Uptime":3600,"Loads":[65536,32768,16384],"Totalram":8388608,"Freeram":4194304,"Sharedram":0,"Bufferram":1024,"Totalswap":2097152,"Freeswap":1048576,"Procs":123,"Unit":1}).to_string();
    runtime
        .block_on(replies.send(protocol::NotificationReply {
            id: resource.id,
            data: sysinfo,
            ..Default::default()
        }))
        .unwrap();
    flush(&mut cx);
    checkpoint(
        &mut cx,
        window,
        &evidence.join("10-live-node-resources.json"),
    );
    click(&mut cx, window, "live-copy-json");
    assert!(
        cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .contains("Uptime")
    );
    cx.update_window(window, |_, window, cx| window.press("ctrl-1", cx))
        .unwrap();
    flush(&mut cx);
    cx.read_entity(&view, |view, _| {
        assert!(
            view.screen == crate::app::Screen::History,
            "Ctrl+1 must leave Live"
        )
    });
    for _ in 0..10 {
        if recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|n| n.r#type == 14)
            .count()
            == 4
        {
            break;
        }
        flush(&mut cx);
    }
    let messages = recorded.lock().unwrap().clone();
    assert_eq!(messages.iter().filter(|n| n.r#type == 13).count(), 4);
    assert_eq!(messages.iter().filter(|n| n.r#type == 14).count(), 4);
    assert!(messages.iter().all(|n| [13, 14].contains(&n.r#type)));
    let filtered: serde_json::Value = serde_json::from_str(
        &messages
            .iter()
            .filter(|n| n.r#type == 13)
            .nth(1)
            .unwrap()
            .data,
    )
    .unwrap();
    assert_eq!(
        filtered["data"],
        serde_json::json!({"interval":"2s","state":1,"proto":6,"family":10})
    );
    // Closing the owned dashboard also requests stop, even with a held first update.
    cx.update_window(window, |_, window, cx| window.press("ctrl-6", cx))
        .unwrap();
    flush(&mut cx);
    click(&mut cx, window, "live-start");
    flush(&mut cx);
    let (tray, tray_actions, _available) = crate::app::tray::Client::fixture();
    cx.update_entity(&view, |view, _| view.tray = Some(tray));
    cx.update_window(window, |_, window, cx| {
        assert!(view.update(cx, |view, _| view.should_close_dashboard(window)));
        window.remove_window();
    })
    .unwrap();
    flush(&mut cx);
    tray_actions.send(crate::app::tray::Action::Open).unwrap();
    flush(&mut cx);
    let restored = cx.read_entity(&view, |view, _| {
        assert!(!view.live.snapshot.running);
        view.main_window.unwrap()
    });
    checkpoint(
        &mut cx,
        restored,
        &evidence.join("11-live-close-stopped.json"),
    );
    for _ in 0..10 {
        if recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|n| n.r#type == 14)
            .count()
            == 5
        {
            break;
        }
        flush(&mut cx);
    }
    let messages = recorded.lock().unwrap().clone();
    assert_eq!(messages.iter().filter(|n| n.r#type == 13).count(), 5);
    assert_eq!(messages.iter().filter(|n| n.r#type == 14).count(), 5);
    drop(tray_actions);
    drop(daemon);
    drop(replies);
    task.abort();
    drop(view);
    shutdown(cx);
    backend.join();
    assert!(!config.socket.exists());
    std::fs::write(
        evidence.join("commands.json"),
        serde_json::to_vec_pretty(
            &messages
                .iter()
                .map(|n| serde_json::json!({"id":n.id,"action":n.r#type,"data":n.data}))
                .collect::<Vec<_>>(),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::write(evidence.join("headless.json"),serde_json::to_vec_pretty(&serde_json::json!({"status":"passed","framework":"gpui-kit","real_gpu":false,"scoped_socket_removed":true,"checkpoints":["01-live-sockets.json","02-live-sockets-page-two.json","03-live-daemon-filters.json","04-live-process-status.json","05-live-process-environment.json","06-live-process-sockets.json","07-live-process-maps.json","08-live-process-error-retained.json","09-live-permission-while-awaiting.json","10-live-node-resources.json","11-live-close-stopped.json"],"proof_boundary":"Actual Kit start/pause/pickers/paging/selection/search/copy, real streaming gRPC fixture, two updates per task ID, process error preserving data, permission during held task update, exact start/stop JSON and cleanup. TASK_STOP has no success ACK in pinned daemon. No real kernel process/socket or firewall proof."})).unwrap()).unwrap();
    std::fs::write(evidence.join("actions.json"),serde_json::to_vec_pretty(&["Open Live and start; require two updates and 225 sockets, page by actual input","Pause, select TCP/IPv6/Established/2s, restart and require 113 filtered sockets plus exact daemon JSON","Select/copy socket, inspect PID42; read/copy Env, filter descriptors and inspect Maps/full JSON","Inject task ERROR; retain displayed data, request stop without policy uncertainty","Start node monitor with first update held, answer real permission by input, then display/copy Sysinfo resources","Leave Live; require stop. Restart with first update held, close through production boundary, restore and require fifth stop with no firewall commands"]).unwrap()).unwrap();
}
