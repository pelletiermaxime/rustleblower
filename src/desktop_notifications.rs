//! Bounded, best-effort desktop notices, independent of archive and permissions.
use crate::{alerts, history, protocol};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::mpsc;

pub mod missed;
pub const CAPACITY: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Alerts,
    Subscriptions,
    Rules,
    Updates,
    RawDownloads,
}
impl Target {
    pub fn action(self) -> &'static str {
        match self {
            Self::Alerts => "open-alerts",
            Self::Subscriptions => "open-subscriptions",
            Self::Rules => "open-rules",
            Self::Updates => "open-updates",
            Self::RawDownloads => "open-raw-downloads",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Alerts => "Open alerts",
            Self::Subscriptions => "Open subscriptions",
            Self::Rules => "Open application rules",
            Self::Updates => "Open updates",
            Self::RawDownloads => "Open raw downloads",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Notice {
    pub target: Target,
    pub node: String,
    pub summary: String,
    pub severity: i32,
    pub priority: i32,
}
impl Notice {
    pub fn title(&self) -> String {
        match self.target {
            Target::Alerts => format!("Rustleblower — {}", alerts::severity(self.severity)),
            Target::Subscriptions => "Rustleblower — List subscriptions".into(),
            Target::Rules => "Rustleblower — Missed permission popup".into(),
            Target::Updates => "Rustleblower — Update available".into(),
            Target::RawDownloads => "Rustleblower — Raw downloads".into(),
        }
    }
    pub fn body(&self) -> String {
        // The freedesktop body supports markup. Daemon text is plain text.
        let escape = |text: &str| {
            text.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        };
        format!("{}\n{}", escape(&self.node), escape(&self.summary))
    }
    pub fn urgency(&self) -> u8 {
        match self.priority {
            0 => 0,
            2 => 2,
            _ => 1,
        }
    }
}
pub struct Mailbox {
    sender: mpsc::Sender<Notice>,
    receiver: Mutex<Option<mpsc::Receiver<Notice>>>,
    pub skipped: AtomicU64,
    pub coalesced: AtomicU64,
    pub accepted: AtomicU64,
    pub error: Mutex<Option<String>>,
}
impl Default for Mailbox {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel(CAPACITY);
        Self {
            sender,
            receiver: Mutex::new(Some(receiver)),
            skipped: AtomicU64::new(0),
            coalesced: AtomicU64::new(0),
            accepted: AtomicU64::new(0),
            error: Mutex::new(None),
        }
    }
}
impl Mailbox {
    pub fn post(&self, node: &str, alert: &protocol::Alert) {
        let notice = Notice {
            target: Target::Alerts,
            node: history::bounded(node, 128),
            summary: alerts::summary(alert),
            severity: alert.r#type,
            priority: alert.priority,
        };
        self.enqueue(notice);
    }
    pub fn post_subscription(&self, name: &str, summary: &str, failed: bool) {
        self.enqueue(Notice {
            target: Target::Subscriptions,
            node: history::bounded(name, 128),
            summary: history::bounded(summary, 512),
            severity: if failed { 0 } else { 2 },
            priority: 1,
        });
    }
    pub fn post_update(&self, update: &crate::updates::Notice) {
        let name = match update.target {
            crate::updates::Target::Application => "Rustleblower",
            crate::updates::Target::SelectedDaemon => &update.node,
        };
        self.enqueue(Notice {
            target: Target::Updates,
            node: history::bounded(name, 128),
            summary: history::bounded(
                &format!(
                    "{} → {}\n{}",
                    update.current, update.latest, update.release_url
                ),
                512,
            ),
            severity: 2,
            priority: 1,
        });
    }
    pub fn post_raw_download(&self, result: &crate::raw_downloads::Notice) {
        self.enqueue(Notice {
            target: Target::RawDownloads,
            node: history::bounded(&result.group, 128),
            summary: history::bounded(&result.message, 512),
            severity: if result.success { 2 } else { 0 },
            priority: 1,
        });
    }
    fn enqueue(&self, notice: Notice) {
        if self.sender.try_send(notice).is_err() {
            self.skipped.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn take_receiver(&self) -> Option<mpsc::Receiver<Notice>> {
        self.receiver.lock().unwrap().take()
    }
    pub fn failed(&self, error: impl std::fmt::Display) {
        *self.error.lock().unwrap() = Some(history::bounded(&error.to_string(), 512));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn burst_is_bounded_and_text_is_plain_with_conservative_urgency() {
        let mailbox = Mailbox::default();
        let mut rx = mailbox.take_receiver().unwrap();
        assert!(mailbox.take_receiver().is_none());
        let alert = protocol::Alert {
            data: Some(protocol::alert::Data::Text(
                "<b>unsafe & text</b>\nnext".into(),
            )),
            priority: 72,
            ..Default::default()
        };
        for _ in 0..100 {
            mailbox.post("<node>", &alert);
        }
        assert_eq!(mailbox.skipped.load(Ordering::Relaxed), 92);
        let first = rx.try_recv().unwrap();
        assert!(
            first
                .body()
                .contains("&lt;b&gt;unsafe &amp; text&lt;/b&gt; next")
        );
        assert!(first.body().starts_with("&lt;node&gt;"));
        assert_eq!(first.urgency(), 1);
        assert_eq!(
            Notice {
                priority: 2,
                ..first.clone()
            }
            .urgency(),
            2
        );
        assert_eq!(
            Notice {
                priority: 0,
                ..first
            }
            .urgency(),
            0
        );
        assert_eq!((0..7).filter_map(|_| rx.try_recv().ok()).count(), 7);
        assert!(rx.try_recv().is_err());
    }
}
