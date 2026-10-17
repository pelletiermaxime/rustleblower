//! StatusNotifierItem integration, isolated from GPUI and permission delivery.
use ksni::TrayMethods;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
    mpsc::{Receiver, SyncSender, sync_channel},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Open,
    Startup,
    Quit,
}

pub(super) struct Client {
    available: Arc<AtomicU8>,
    actions: Receiver<Action>,
    stop: CancellationToken,
}

impl Client {
    pub fn start() -> Self {
        let (sender, actions) = sync_channel(4);
        let available = Arc::new(AtomicU8::new(0));
        let stop = CancellationToken::new();
        let item = Item {
            sender,
            available: available.clone(),
            icons: super::brand_icons::tray_icons(),
        };
        let cancel = stop.clone();
        // zbus uses async-io, matching GPUI's portal dependencies. Its runtime
        // and all session-bus communication stay off the foreground executor.
        std::thread::Builder::new()
            .name("rustleblower-tray".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let Ok(runtime) = runtime else { return };
                runtime.block_on(async move {
                    loop {
                        item.available.store(0, Ordering::Release);
                        let service = tokio::select! {
                            _ = cancel.cancelled() => break,
                            result = item.clone().spawn() => result,
                        };
                        if let Ok(service) = service {
                            // A watcher-offline callback may race with initial registration.
                            // Never overwrite that callback with a stale successful result.
                            let _ = item.available.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
                            loop {
                                tokio::select! {
                                    _ = cancel.cancelled() => break,
                                    _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
                                }
                                if cancel.is_cancelled() || service.is_closed() {
                                    break;
                                }
                            }
                            item.available.store(2, Ordering::Release);
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_secs(1),
                                service.shutdown(),
                            ).await;
                        }
                        tokio::select! {
                            _ = cancel.cancelled() => break,
                            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                        }
                    }
                    item.available.store(2, Ordering::Release);
                });
            })
            .ok();
        Self {
            available,
            actions,
            stop,
        }
    }

    pub fn available(&self) -> bool {
        self.available.load(Ordering::Acquire) == 1
    }

    pub fn action(&self) -> Option<Action> {
        self.actions.try_recv().ok()
    }

    #[cfg(all(test, feature = "headless-tests"))]
    pub fn fixture() -> (Self, SyncSender<Action>, Arc<AtomicU8>) {
        let (sender, actions) = sync_channel(4);
        let available = Arc::new(AtomicU8::new(1));
        (
            Self {
                available: available.clone(),
                actions,
                stop: CancellationToken::new(),
            },
            sender,
            available,
        )
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[derive(Clone)]
struct Item {
    sender: SyncSender<Action>,
    available: Arc<AtomicU8>,
    icons: Vec<ksni::Icon>,
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "dev.rustleblower.Rustleblower".into()
    }
    fn title(&self) -> String {
        "Rustleblower".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icons.clone()
    }
    fn activate(&mut self, _: i32, _: i32) {
        let _ = self.sender.try_send(Action::Open);
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            ksni::menu::StandardItem {
                label: crate::i18n::tr("Open Rustleblower"),
                activate: Box::new(|item: &mut Self| {
                    let _ = item.sender.try_send(Action::Open);
                }),
                ..Default::default()
            }
            .into(),
            ksni::menu::StandardItem {
                label: crate::i18n::tr("Startup preferences…"),
                activate: Box::new(|item: &mut Self| {
                    let _ = item.sender.try_send(Action::Startup);
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            ksni::menu::StandardItem {
                label: crate::i18n::tr("Quit Rustleblower"),
                activate: Box::new(|item: &mut Self| {
                    let _ = item.sender.try_send(Action::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
    fn watcher_online(&self) {
        self.available.store(1, Ordering::Release);
    }
    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        self.available.store(2, Ordering::Release);
        true
    }
}
