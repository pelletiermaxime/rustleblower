//! One bounded freedesktop notification worker; no per-notification threads.
use crate::{
    desktop_notifications::{Mailbox, Notice, Target},
    preferences::Store,
};
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use zbus::Variant;

const DESTINATION: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
pub(super) struct Client {
    actions: mpsc::Receiver<Target>,
    stop: CancellationToken,
}
impl Client {
    pub fn start(backend: &crate::backend::Handle) -> Option<Self> {
        let receiver = backend.desktop_notices.take_receiver()?;
        let (sender, actions) = mpsc::channel(1);
        let stop = backend.cancel.child_token();
        let cancel = stop.clone();
        let mailbox = backend.desktop_notices.clone();
        let preferences = backend.preferences.clone();
        let started = std::thread::Builder::new()
            .name("rustleblower-notifications".into())
            .spawn(move || {
                if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    runtime.block_on(run(mailbox, preferences, receiver, sender, cancel, None));
                } else {
                    mailbox.failed("Cannot start desktop notification worker");
                }
            });
        if let Err(error) = started {
            backend.desktop_notices.failed(error);
            return None;
        }
        Some(Self { actions, stop })
    }
    pub fn open_requested(&mut self) -> Option<Target> {
        self.actions.try_recv().ok()
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub fn fixture() -> (Self, mpsc::Sender<Target>) {
        let (sender, actions) = mpsc::channel(1);
        (
            Self {
                actions,
                stop: CancellationToken::new(),
            },
            sender,
        )
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

async fn connect(address: Option<&str>) -> zbus::Result<zbus::Connection> {
    let builder = if let Some(address) = address {
        zbus::connection::Builder::address(address)
    } else {
        zbus::connection::Builder::session()
    };
    builder
        .max_queued(std::num::NonZeroUsize::new(16).unwrap())
        .method_timeout(Duration::from_secs(1))
        .build()
        .await
}
async fn close(proxy: &zbus::Proxy<'_>, id: u32) {
    if id != 0 {
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            proxy.call::<_, _, ()>("CloseNotification", &id),
        )
        .await;
    }
}
async fn send(proxy: &zbus::Proxy<'_>, id: u32, notice: &Notice) -> zbus::Result<u32> {
    let icon = match notice.severity {
        0 => "dialog-error",
        1 => "dialog-warning",
        _ => "dialog-information",
    };
    let hints = HashMap::from([
        ("urgency", Variant::U8(notice.urgency())),
        ("category", Variant::from("network")),
        (
            "desktop-entry",
            Variant::from("dev.rustleblower.Rustleblower"),
        ),
    ]);
    let action_label = crate::i18n::tr(notice.target.label());
    let title = if notice.target == Target::Alerts {
        crate::i18n::tr_args(
            "Rustleblower — %{severity}",
            &[(
                "severity",
                &crate::i18n::presentation(&crate::alerts::severity(notice.severity)),
            )],
        )
    } else {
        crate::i18n::tr(&notice.title())
    };
    proxy
        .call(
            "Notify",
            &(
                "Rustleblower",
                id,
                icon,
                title,
                notice.body(),
                vec![
                    "default",
                    action_label.as_str(),
                    notice.target.action(),
                    action_label.as_str(),
                ],
                hints,
                10_000_i32,
            ),
        )
        .await
}
async fn run(
    mailbox: Arc<Mailbox>,
    preferences: Arc<Store>,
    mut receiver: mpsc::Receiver<Notice>,
    actions: mpsc::Sender<Target>,
    stop: CancellationToken,
    address: Option<&str>,
) {
    // Connection failures retry on this worker. Queued notices and responses stay bounded.
    while !stop.is_cancelled() {
        let connection = tokio::select! {
            _ = stop.cancelled() => break,
            result = tokio::time::timeout(Duration::from_secs(2), connect(address)) => match result {
                Ok(Ok(connection)) => connection,
                result => {
                    mailbox.failed(format!("Desktop notifications unavailable: {result:?}"));
                    tokio::select! { _=stop.cancelled()=>break, _=tokio::time::sleep(Duration::from_secs(2))=>continue }
                }
            }
        };
        let result = connected(
            &connection,
            &mailbox,
            &preferences,
            &mut receiver,
            &actions,
            &stop,
        )
        .await;
        if let Err(error) = result {
            mailbox.failed(error);
        }
        tokio::select! { _=stop.cancelled()=>break, _=tokio::time::sleep(Duration::from_secs(2))=>{} }
    }
}
async fn connected(
    connection: &zbus::Connection,
    mailbox: &Mailbox,
    preferences: &Store,
    receiver: &mut mpsc::Receiver<Notice>,
    actions: &mpsc::Sender<Target>,
    stop: &CancellationToken,
) -> anyhow::Result<()> {
    let proxy = zbus::Proxy::new(connection, DESTINATION, PATH, DESTINATION).await?;
    // Subscribe before Notify to avoid losing an immediate activation. Proxy subscriptions
    // resolve the service owner; other session-bus clients cannot spoof an action.
    let mut invoked = proxy.receive_signal("ActionInvoked").await?;
    let mut closed = proxy.receive_signal("NotificationClosed").await?;
    let mut current = 0;
    let mut current_target = Target::Alerts;
    let mut latest = None;
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            notice = receiver.recv() => {
                let Some(notice) = notice else { break };
                if latest.replace(notice).is_some() {
                    mailbox.coalesced.fetch_add(1, Ordering::Relaxed);
                }
            }
            signal = invoked.next() => {
                let Some(signal) = signal else { break };
                if let Ok((id, action)) = signal.body().deserialize::<(u32, String)>()
                    && id != 0 && id == current
                    && (action == "default" || action == current_target.action())
                {
                    let _ = actions.try_send(current_target);
                    close(&proxy, current).await;
                    current = 0;
                }
            }
            signal = closed.next() => {
                let Some(signal) = signal else { break };
                if let Ok((id, _)) = signal.body().deserialize::<(u32, u32)>() && id == current {
                    current = 0;
                }
            }
            _ = interval.tick() => {
                if !preferences.snapshot().desktop_notifications {
                    latest = None;
                    close(&proxy, current).await;
                    current = 0;
                    continue;
                }
                if let Some(notice) = latest.take() {
                    match send(&proxy, current, &notice).await {
                        Ok(id) => {
                            if current != id { close(&proxy, current).await; }
                            current = id;
                            current_target = notice.target;
                            mailbox.accepted.fetch_add(1, Ordering::Relaxed);
                            *mailbox.error.lock().unwrap() = None;
                        }
                        Err(error) => {
                            mailbox.failed(error);
                            close(&proxy, current).await;
                            current = 0;
                        }
                    }
                }
            }
        }
    }
    close(&proxy, current).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::BufRead,
        process::{Child, Command, Stdio},
        sync::atomic::{AtomicBool, AtomicU32},
    };
    use zbus::OwnedVariant;
    struct Bus(Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    #[derive(Debug)]
    struct Posted {
        id: u32,
        replaces: u32,
        title: String,
        body: String,
        actions: Vec<String>,
        urgency: u8,
    }
    struct Server {
        posted: mpsc::Sender<Posted>,
        closes: mpsc::Sender<u32>,
        next: AtomicU32,
        hold: Arc<AtomicBool>,
        release: Arc<tokio::sync::Notify>,
    }
    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Server {
        #[allow(clippy::too_many_arguments)] // The standard Notify wire signature has eight arguments.
        async fn notify(
            &self,
            app_name: &str,
            replaces_id: u32,
            app_icon: &str,
            summary: &str,
            body: &str,
            actions: Vec<String>,
            hints: HashMap<String, OwnedVariant>,
            expire_timeout: i32,
        ) -> u32 {
            assert_eq!(app_name, "Rustleblower");
            assert!(app_icon.starts_with("dialog-"));
            assert_eq!(expire_timeout, 10_000);
            assert_eq!(
                <&str>::try_from(hints.get("category").unwrap()).unwrap(),
                "network"
            );
            let id = if replaces_id == 0 {
                self.next.fetch_add(1, Ordering::Relaxed)
            } else {
                replaces_id
            };
            let _ = self.posted.try_send(Posted {
                id,
                replaces: replaces_id,
                title: summary.into(),
                body: body.into(),
                actions,
                urgency: u8::try_from(hints.get("urgency").unwrap()).unwrap(),
            });
            if self.hold.load(Ordering::Relaxed) {
                self.release.notified().await;
            }
            id
        }
        fn close_notification(&self, id: u32) {
            let _ = self.closes.try_send(id);
        }
    }
    async fn received<T>(rx: &mut mpsc::Receiver<T>) -> T {
        tokio::time::timeout(Duration::from_secs(4), rx.recv())
            .await
            .unwrap()
            .unwrap()
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn private_bus_replacement_owner_actions_mute_failure_and_shutdown() -> anyhow::Result<()>
    {
        let child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut bus = Bus(child);
        let mut address = String::new();
        std::io::BufReader::new(bus.0.stdout.take().unwrap()).read_line(&mut address)?;
        let address = address.trim();
        anyhow::ensure!(
            !address.is_empty(),
            "Private dbus-daemon could not start; socket permission is required"
        );
        let (posted, mut posts) = mpsc::channel(16);
        let (closes, mut closed) = mpsc::channel(16);
        let hold = Arc::new(AtomicBool::new(false));
        let release = Arc::new(tokio::sync::Notify::new());
        let service = zbus::connection::Builder::address(address)
            .name(DESTINATION)
            .serve_at(
                PATH,
                Server {
                    posted,
                    closes,
                    next: AtomicU32::new(40),
                    hold: hold.clone(),
                    release: release.clone(),
                },
            )
            .build()
            .await?;
        let dir = tempfile::tempdir()?;
        let backend = crate::backend::Backend::start(crate::config::Config {
            transport: Default::default(),
            socket: dir.path().join("ui.sock"),
            database: dir.path().join("history.sqlite3"),
            prompt_timeout: 30,
            timeout_action: crate::config::Decision::Deny,
            history_days: 7,
            history_mib: 16,
            headless: true,
        })?;
        let preferences = backend.handle.preferences.clone();
        let mailbox = backend.handle.desktop_notices.clone();
        let rx = mailbox.take_receiver().unwrap();
        let (actions, mut action) = mpsc::channel(1);
        let stop = CancellationToken::new();
        let task = tokio::spawn({
            let mailbox = mailbox.clone();
            let preferences = preferences.clone();
            let stop = stop.clone();
            let address = address.to_owned();
            async move { run(mailbox, preferences, rx, actions, stop, Some(&address)).await }
        });
        let alert = crate::protocol::Alert {
            r#type: 0,
            priority: 2,
            data: Some(crate::protocol::alert::Data::Text(
                "<error>& details".into(),
            )),
            ..Default::default()
        };
        mailbox.post("owned-node", &alert);
        let first = tokio::time::timeout(Duration::from_secs(4), posts.recv())
            .await
            .unwrap_or_else(|_| panic!("No Notify received: {:?}", mailbox.error.lock().unwrap()))
            .unwrap();
        assert_eq!(first.replaces, 0);
        assert_eq!(first.urgency, 2);
        assert!(first.title.contains("Error"));
        assert_eq!(first.body, "owned-node\n&lt;error&gt;&amp; details");
        assert_eq!(
            first.actions,
            ["default", "Open alerts", "open-alerts", "Open alerts"]
        );
        for _ in 0..8 {
            mailbox.post("newest-node", &alert);
        }
        let second = received(&mut posts).await;
        assert_eq!(second.id, first.id);
        assert_eq!(second.replaces, first.id);
        assert!(second.body.starts_with("newest-node"));
        assert!(mailbox.coalesced.load(Ordering::Relaxed) > 0);
        // Signals from another client, stale IDs and unknown actions must do nothing.
        let impostor = connect(Some(address)).await?;
        impostor
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(first.id, "default"),
            )
            .await?;
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(first.id + 1, "default"),
            )
            .await?;
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(first.id, "allow"),
            )
            .await?;
        assert!(
            tokio::time::timeout(Duration::from_millis(150), action.recv())
                .await
                .is_err()
        );
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(first.id, "default"),
            )
            .await?;
        assert_eq!(received(&mut action).await, Target::Alerts);
        assert_eq!(received(&mut closed).await, first.id);
        // Dismissal invalidates the action; reopening does not duplicate callbacks.
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(first.id, "default"),
            )
            .await?;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), action.recv())
                .await
                .is_err()
        );
        mailbox.post("replace-kind", &alert);
        let replaced_kind = received(&mut posts).await;
        mailbox.post_subscription("owned-list", "Updated <list> & ready", false);
        let subscription = received(&mut posts).await;
        assert_eq!(subscription.id, replaced_kind.id);
        assert_eq!(subscription.replaces, replaced_kind.id);
        assert!(subscription.title.contains("List subscriptions"));
        assert_eq!(
            subscription.body,
            "owned-list\nUpdated &lt;list&gt; &amp; ready"
        );
        assert_eq!(subscription.urgency, 1);
        assert_eq!(
            subscription.actions,
            [
                "default",
                "Open subscriptions",
                "open-subscriptions",
                "Open subscriptions"
            ]
        );
        // A reused notification ID must carry its new target, and a stale alert
        // action must not open the wrong screen after replacement.
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(subscription.id, "open-alerts"),
            )
            .await?;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), action.recv())
                .await
                .is_err()
        );
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(subscription.id, "open-subscriptions"),
            )
            .await?;
        assert_eq!(received(&mut action).await, Target::Subscriptions);
        assert_eq!(received(&mut closed).await, subscription.id);
        crate::desktop_notifications::missed::post_timeout(
            &mailbox,
            &crate::preferences::Popup::default(),
            &crate::protocol::Connection {
                process_path: "/usr/bin/owned-client".into(),
                process_args: vec!["owned-client".into(), "--fixture".into()],
                ..Default::default()
            },
            &crate::protocol::Rule {
                action: "deny".into(),
                ..Default::default()
            },
            "",
            "owned-host",
            "timeout",
        );
        let missed = received(&mut posts).await;
        assert!(missed.title.contains("Missed permission popup"));
        assert_eq!(
            missed.body,
            "/usr/bin/owned-client\ndeny action applied \nCommand line: owned-client --fixture"
        );
        assert_eq!(missed.urgency, 1);
        assert_eq!(
            missed.actions,
            [
                "default",
                "Open application rules",
                "open-rules",
                "Open application rules"
            ]
        );
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(missed.id, "open-subscriptions"),
            )
            .await?;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), action.recv())
                .await
                .is_err()
        );
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(missed.id, "open-rules"),
            )
            .await?;
        assert_eq!(received(&mut action).await, Target::Rules);
        assert_eq!(received(&mut closed).await, missed.id);
        mailbox.post_update(&crate::updates::Notice {
            target: crate::updates::Target::SelectedDaemon,
            node: "owned-daemon".into(),
            current: "1.8.1".into(),
            latest: "1.9.0".into(),
            release_url: "https://github.com/evilsocket/opensnitch/releases/tag/v1.9.0".into(),
        });
        let update = received(&mut posts).await;
        assert!(update.title.contains("Update available"));
        assert_eq!(update.urgency, 1);
        assert_eq!(
            update.body,
            "owned-daemon\n1.8.1 → 1.9.0\nhttps://github.com/evilsocket/opensnitch/releases/tag/v1.9.0"
        );
        assert_eq!(
            update.actions,
            ["default", "Open updates", "open-updates", "Open updates"]
        );
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(update.id, "open-rules"),
            )
            .await?;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), action.recv())
                .await
                .is_err()
        );
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(update.id, "open-updates"),
            )
            .await?;
        assert_eq!(received(&mut action).await, Target::Updates);
        assert_eq!(received(&mut closed).await, update.id);
        mailbox.post_raw_download(&crate::raw_downloads::Notice {
            group: "Owned files".into(),
            success: false,
            message: "One resource failed; previous file retained".into(),
            failed_names: vec!["Owned resource".into()],
            failed_count: 1,
        });
        let raw = received(&mut posts).await;
        assert!(raw.title.contains("Raw downloads"));
        assert_eq!(raw.urgency, 1);
        assert_eq!(
            raw.body,
            "Owned files\nOne resource failed; previous file retained"
        );
        assert_eq!(
            raw.actions,
            [
                "default",
                "Open raw downloads",
                "open-raw-downloads",
                "Open raw downloads"
            ]
        );
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(raw.id, "open-updates"),
            )
            .await?;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), action.recv())
                .await
                .is_err()
        );
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(raw.id, "open-raw-downloads"),
            )
            .await?;
        assert_eq!(received(&mut action).await, Target::RawDownloads);
        assert_eq!(received(&mut closed).await, raw.id);
        mailbox.post("dismissed", &alert);
        let dismissed = received(&mut posts).await;
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "NotificationClosed",
                &(dismissed.id, 2_u32),
            )
            .await?;
        service
            .emit_signal(
                None::<&str>,
                PATH,
                DESTINATION,
                "ActionInvoked",
                &(dismissed.id, "open-alerts"),
            )
            .await?;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), action.recv())
                .await
                .is_err()
        );
        mailbox.post("close-on-mute", &alert);
        let visible = received(&mut posts).await;
        let mut prefs = preferences.snapshot();
        prefs.desktop_notifications = false;
        preferences.save(prefs)?;
        assert_eq!(received(&mut closed).await, visible.id);
        mailbox.post("muted", &alert);
        assert!(
            tokio::time::timeout(Duration::from_millis(650), posts.recv())
                .await
                .is_err()
        );
        let mut prefs = preferences.snapshot();
        prefs.desktop_notifications = true;
        preferences.save(prefs)?;
        hold.store(true, Ordering::Relaxed);
        mailbox.post("held", &alert);
        let held = received(&mut posts).await;
        assert_ne!(held.id, first.id);
        use hyper_util::rt::TokioIo;
        let path = dir.path().join("ui.sock");
        let channel = tonic::transport::Endpoint::try_from("http://localhost")?
            .connect_with_connector(tower::service_fn(move |_| {
                let path = path.clone();
                async move {
                    tokio::net::UnixStream::connect(path)
                        .await
                        .map(TokioIo::new)
                }
            }))
            .await?;
        let mut daemon = crate::protocol::ui_client::UiClient::new(channel);
        daemon
            .subscribe(crate::protocol::ClientConfig {
                name: "notice-test".into(),
                ..Default::default()
            })
            .await?;
        let started = std::time::Instant::now();
        tokio::time::timeout(
            Duration::from_millis(500),
            daemon.ping(crate::protocol::PingRequest {
                id: 99,
                ..Default::default()
            }),
        )
        .await??;
        let asking = tokio::spawn(async move {
            daemon
                .ask_rule(crate::protocol::Connection {
                    process_path: "/bin/notification-independent".into(),
                    ..Default::default()
                })
                .await
        });
        let prompt = tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                if let Some(prompt) = backend.handle.prompts.snapshot().first() {
                    break prompt.id;
                }
                tokio::time::sleep(Duration::from_millis(5)).await
            }
        })
        .await?;
        assert!(
            backend
                .handle
                .decide(prompt, crate::config::Decision::Allow)
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(500), asking)
                .await???
                .into_inner()
                .action,
            "allow"
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        tokio::time::timeout(Duration::from_secs(3), async {
            while mailbox.error.lock().unwrap().is_none() {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
        })
        .await?;
        release.notify_one();
        hold.store(false, Ordering::Relaxed);
        mailbox.post("recovered", &alert);
        let recovered = received(&mut posts).await;
        assert!(recovered.body.starts_with("recovered"));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(mailbox.error.lock().unwrap().is_none());
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(2), task).await??;
        assert_eq!(received(&mut closed).await, recovered.id);
        assert!(mailbox.accepted.load(Ordering::Relaxed) >= 3);
        drop(service);
        drop(impostor);
        drop(bus);
        backend.join();
        Ok(())
    }
}
