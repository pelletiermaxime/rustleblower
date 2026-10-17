use anyhow::Result;
use rustleblower::{
    backend::Backend,
    config::{Config, Decision},
    desktop_notifications::{CAPACITY, Mailbox, Target, missed},
    preferences::{Popup, Store},
    protocol::{self, Connection, Rule, ui_client::UiClient},
};
use std::{path::Path, sync::atomic::Ordering, time::Duration};
use tonic::transport::{Channel, Endpoint};

fn connection() -> Connection {
    Connection {
        src_ip: "192.0.2.1".into(),
        src_port: 12345,
        dst_ip: "192.0.2.42".into(),
        dst_host: "api.example.test".into(),
        dst_port: 443,
        protocol: "tcp".into(),
        process_path: "/usr/bin/owned-client".into(),
        process_args: vec!["owned-client".into(), "--fixture".into()],
        process_cwd: "/home/example".into(),
        process_id: 123,
        user_id: 1000,
        ..Default::default()
    }
}
#[test]
fn templates_expand_once_preserve_unknown_fields_and_bound_utf8() {
    let mut con = connection();
    let rule = Rule {
        action: "reject".into(),
        ..Default::default()
    };
    let template = "%conn.srcip%:%conn.srcport% %conn.dsthost% %conn.dstip%:%conn.dstport% %conn.proto% %conn.process% %conn.process_cwd% %conn.pid% %conn.uid% %rule.action% %conn.action% %node.addr% %node.hostname% %unknown%";
    assert_eq!(
        missed::render(template, &con, &rule, "on node 192.0.2.8", "owned-host"),
        "192.0.2.1:12345 api.example.test 192.0.2.42:443 tcp /usr/bin/owned-client /home/example 123 1000 reject reject on node 192.0.2.8 owned-host %unknown%"
    );
    con.process_args = vec!["%rule.action%".into(), "<b>unsafe & text</b>\0\n".into()];
    assert_eq!(
        missed::render("%conn.process_args%", &con, &rule, "", ""),
        "%rule.action% <b>unsafe & text</b>  "
    );
    assert!(
        missed::render("", &con, &rule, "", "").starts_with("reject action applied \nProcess:")
    );
    assert_eq!(
        missed::render("unclosed %thing", &con, &rule, "", ""),
        "unclosed %thing"
    );
    assert_eq!(missed::render("%%", &con, &rule, "", ""), "%%");
    let text = format!("{}é%rule.action%", "x".repeat(missed::MAX_BODY - 1));
    assert_eq!(
        missed::render(&text, &con, &rule, "", ""),
        "x".repeat(missed::MAX_BODY - 1)
    );
    for template in [
        "literal\\path\nline\ttab",
        missed::DEFAULT_TEMPLATE,
        "",
        "é",
    ] {
        assert_eq!(
            missed::decode_input(&missed::encode_input(template)).unwrap(),
            template
        );
    }
    for value in ["bad\\", "bad\\q", "bad\0", "bad\r"] {
        assert!(missed::decode_input(value).is_err());
    }
    assert!(missed::validate(&"x".repeat(1025)).is_err());
    assert!(missed::decode_input(&"x".repeat(2049)).is_err());
    assert!(missed::decode_input(&"x".repeat(1025)).is_err());
    assert_eq!(
        missed::decode_input(&"\\n".repeat(1024)).unwrap().len(),
        1024
    );
}
#[test]
fn preference_migration_reopen_validation_and_bounded_mailbox() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let database = dir.path().join("history.sqlite3");
    let store = Store::open(&database)?;
    store.save(Popup::default())?;
    let path = dir.path().join("history.sqlite3.preferences.json");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(saved["version"], 5);
    for version in 1..=4 {
        let mut legacy = saved.clone();
        legacy["version"] = serde_json::json!(version);
        legacy
            .as_object_mut()
            .unwrap()
            .remove("missed_popup_notifications");
        legacy
            .as_object_mut()
            .unwrap()
            .remove("missed_popup_template");
        std::fs::write(&path, serde_json::to_vec(&legacy)?)?;
        let prefs = Store::open(&database)?.snapshot();
        assert!(prefs.missed_popup_notifications);
        assert_eq!(prefs.missed_popup_template, missed::DEFAULT_TEMPLATE);
    }
    let prefs = Popup {
        missed_popup_notifications: false,
        missed_popup_template: "Custom\n%conn.dsthost%".into(),
        ..Default::default()
    };
    store.save(prefs.clone())?;
    assert_eq!(Store::open(&database)?.snapshot(), prefs);
    let bytes = std::fs::read(&path)?;
    assert!(
        store
            .save(Popup {
                missed_popup_template: "x".repeat(1025),
                ..prefs.clone()
            })
            .is_err()
    );
    assert_eq!(store.snapshot(), prefs);
    assert_eq!(std::fs::read(&path)?, bytes);
    for (key, value) in [
        ("missed_popup_notifications", serde_json::json!(1)),
        ("missed_popup_template", serde_json::json!(false)),
        ("missed_popup_template", serde_json::json!("bad\0message")),
    ] {
        let mut invalid = saved.clone();
        invalid[key] = value;
        let bytes = serde_json::to_vec(&invalid)?;
        std::fs::write(&path, &bytes)?;
        assert!(Store::open(&database).is_err());
        assert_eq!(std::fs::read(&path)?, bytes);
    }
    let mailbox = Mailbox::default();
    let mut rx = mailbox.take_receiver().unwrap();
    let mut con = connection();
    con.process_path = "<app>\0".into();
    let rule = Rule {
        action: "deny".into(),
        ..Default::default()
    };
    let prefs = Popup {
        missed_popup_template: "<b>%rule.action%</b> & %conn.dsthost%".into(),
        ..Default::default()
    };
    for reason in [
        "decision",
        "overload",
        "popups-disabled",
        "shutdown",
        "reconnected",
        "disconnected",
    ] {
        missed::post_timeout(&mailbox, &prefs, &con, &rule, "", "", reason);
    }
    assert!(rx.try_recv().is_err());
    for prefs in [
        Popup {
            desktop_notifications: false,
            ..prefs.clone()
        },
        Popup {
            missed_popup_notifications: false,
            ..prefs.clone()
        },
    ] {
        missed::post_timeout(&mailbox, &prefs, &con, &rule, "", "", "timeout");
    }
    assert!(rx.try_recv().is_err());
    for _ in 0..CAPACITY + 3 {
        missed::post_timeout(&mailbox, &prefs, &con, &rule, "", "", "timeout");
    }
    assert_eq!(mailbox.skipped.load(Ordering::Relaxed), 3);
    let notice = rx.try_recv().unwrap();
    assert_eq!(notice.target, Target::Rules);
    assert_eq!(notice.urgency(), 1);
    assert!(notice.title().contains("Missed permission"));
    assert_eq!(
        notice.body(),
        "&lt;app&gt; \n&lt;b&gt;deny&lt;/b&gt; &amp; api.example.test"
    );
    Ok(())
}
async fn client(path: &Path) -> Result<UiClient<Channel>> {
    let path = path.to_owned();
    let channel = Endpoint::try_from("http://localhost")?
        .connect_with_connector(tower::service_fn(move |_| {
            let path = path.clone();
            async move {
                tokio::net::UnixStream::connect(path)
                    .await
                    .map(hyper_util::rt::TokioIo::new)
            }
        }))
        .await?;
    let mut client = UiClient::new(channel);
    client
        .subscribe(protocol::ClientConfig {
            name: "owned-daemon".into(),
            ..Default::default()
        })
        .await?;
    Ok(client)
}
fn config(path: &Path) -> Config {
    Config {
        socket: path.join("ui.sock"),
        database: path.join("history.sqlite3"),
        transport: Default::default(),
        prompt_timeout: 1,
        timeout_action: Decision::Deny,
        history_days: 7,
        history_mib: 16,
        headless: true,
    }
}
async fn pending(backend: &Backend) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(2), async {
        while backend.handle.prompts.snapshot().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_uses_actual_captured_fallback_but_current_notice_settings() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = config(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut daemon = client(&config.socket).await?;
    let mut notices = backend.handle.desktop_notices.take_receiver().unwrap();
    backend.handle.preferences.save(Popup {
        fallback: Some(Decision::Reject),
        missed_popup_template: "Old %rule.action%".into(),
        ..Default::default()
    })?;
    let mut ask = daemon.clone();
    let task = tokio::spawn(async move { ask.ask_rule(connection()).await });
    pending(&backend).await?;
    backend.handle.preferences.save(Popup {
        fallback: Some(Decision::Allow),
        missed_popup_template: "New %rule.action%|%conn.dsthost%|%node.hostname%|%node.addr%"
            .into(),
        ..Default::default()
    })?;
    let rule = tokio::time::timeout(Duration::from_secs(2), task)
        .await???
        .into_inner();
    assert_eq!((&*rule.action, &*rule.duration), ("reject", "once"));
    let notice = notices.try_recv()?;
    assert_eq!(notice.target, Target::Rules);
    assert_eq!(notice.summary, "New reject|api.example.test|owned-daemon|");
    assert!(notices.try_recv().is_err());
    let mut ask = daemon.clone();
    let task = tokio::spawn(async move { ask.ask_rule(connection()).await });
    pending(&backend).await?;
    assert!(
        backend
            .handle
            .decide(backend.handle.prompts.snapshot()[0].id, Decision::Allow)
    );
    assert_eq!(task.await??.into_inner().action, "allow");
    assert!(notices.try_recv().is_err());
    daemon.ping(protocol::PingRequest::default()).await?;
    backend.handle.shutdown();
    drop(daemon);
    backend.join();
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_delivery_queue_and_immediate_mute_do_not_hold_fallback_replies() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let config = config(dir.path());
    let backend = Backend::start(config.clone())?;
    let mut daemon = client(&config.socket).await?;
    let mut notices = backend.handle.desktop_notices.take_receiver().unwrap();
    for _ in 0..CAPACITY {
        backend
            .handle
            .desktop_notices
            .post_subscription("owned", "queued", false);
    }
    let mut request = tonic::Request::new(connection());
    request.set_timeout(Duration::from_millis(600));
    let rule = daemon.ask_rule(request).await?.into_inner();
    assert_eq!((&*rule.action, &*rule.duration), ("deny", "once"));
    assert_eq!(
        backend
            .handle
            .desktop_notices
            .skipped
            .load(Ordering::Relaxed),
        1
    );
    for _ in 0..CAPACITY {
        assert_eq!(notices.try_recv()?.target, Target::Subscriptions);
    }
    let mut ask = daemon.clone();
    let task = tokio::spawn(async move {
        let mut request = tonic::Request::new(connection());
        request.set_timeout(Duration::from_millis(600));
        ask.ask_rule(request).await
    });
    pending(&backend).await?;
    backend.handle.preferences.save(Popup {
        missed_popup_notifications: false,
        ..Default::default()
    })?;
    assert_eq!(task.await??.into_inner().action, "deny");
    assert!(notices.try_recv().is_err());
    backend.handle.preferences.save(Popup {
        popups: false,
        ..Default::default()
    })?;
    assert_eq!(
        daemon.ask_rule(connection()).await?.into_inner().action,
        "deny"
    );
    assert!(notices.try_recv().is_err());
    backend.handle.shutdown();
    drop(daemon);
    backend.join();
    Ok(())
}
