//! A single coalescing background writer for history presentation choices.
use crate::history::settings::{Presentation, Store};
use futures_util::future::{Either, select};
use gpui::App;
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
#[derive(Clone)]
struct Update {
    value: Presentation,
    generation: u64,
    flush: bool,
}
#[derive(Clone, Default)]
struct Status {
    saved: u64,
    error: Option<String>,
}
pub(super) struct Client {
    updates: watch::Sender<Option<Update>>,
    status: watch::Receiver<Status>,
    last: Presentation,
    generation: u64,
}
impl Client {
    pub fn start(store: Arc<Store>, cx: &App) -> Self {
        let last = store.snapshot().presentation;
        let (updates, mut rx) = watch::channel::<Option<Update>>(None);
        let (status, read) = watch::channel(Status {
            error: store.load_error(),
            ..Default::default()
        });
        let executor = cx.background_executor().clone();
        executor.clone().spawn(async move {
            while rx.changed().await.is_ok() {
                let Some(mut update) = rx.borrow_and_update().clone() else {continue};
                while !update.flush {
                    if !matches!(select(Box::pin(rx.changed()), Box::pin(executor.timer(Duration::from_millis(500)))).await, Either::Left((Ok(()),_))) {break}
                    if let Some(next) = rx.borrow_and_update().clone() {update=next;}
                }
                match store.save_presentation(update.value) {
                    Ok(()) => { status.send_replace(Status {saved:update.generation, error:None}); }
                    Err(e) => {
                        status.send_replace(Status {saved:0,error:Some(crate::history::bounded(&crate::i18n::tr_args("History view is session-only: %{e}. Restart after repairing the settings file to resume saving.", &[("e", &format!("{:#}", e))]),1024))});
                        break;
                    }
                }
            }
        }).detach();
        Self {
            updates,
            status: read,
            last,
            generation: 0,
        }
    }
    pub fn record(&mut self, value: Presentation, flush: bool) {
        if self.status.borrow().error.is_some() || (!flush && value == self.last) {
            return;
        }
        self.last = value.clone();
        self.generation += 1;
        self.updates.send_replace(Some(Update {
            value,
            generation: self.generation,
            flush,
        }));
    }
    pub fn finish(&self) -> impl Future<Output = ()> + use<> {
        let target = self.generation;
        let mut status = self.status.clone();
        async move {
            while status.borrow().saved < target && status.borrow().error.is_none() {
                if status.changed().await.is_err() {
                    break;
                }
            }
        }
    }
    pub fn error(&self) -> Option<String> {
        self.status.borrow().error.clone()
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub fn settled(&self) -> bool {
        self.status.borrow().saved >= self.generation || self.status.borrow().error.is_some()
    }
}
