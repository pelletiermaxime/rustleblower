//! A single bounded off-thread cached-node lookup; no catalogue I/O on GPUI.
use super::*;
use crate::history::History;
use gpui_kit::component::select::{SearchableVec, SelectEvent, SelectState};
type Pending = (u64, oneshot::Receiver<Result<Vec<String>, String>>);
pub(super) struct State {
    pub picker: Entity<SelectState<SearchableVec<SharedString>>>,
    names: Vec<String>,
    pending: Option<Pending>,
    revision: u64,
    next: Instant,
    pub error: Option<String>,
}
impl State {
    pub fn new(window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<SharedString>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        cx.subscribe(
            &picker,
            |this, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
                if let SelectEvent::Confirm(Some(name)) = event
                    && this.rules.query.node != name.as_ref()
                {
                    this.rules.query.node = name.to_string();
                    this.rules.page = Page::default();
                    this.rules.reset();
                    cx.notify();
                }
            },
        )
        .detach();
        Self {
            picker,
            names: Vec::new(),
            pending: None,
            revision: u64::MAX,
            next: Instant::now(),
            error: None,
        }
    }
    pub fn empty(&self) -> bool {
        self.names.is_empty()
    }
    pub fn select(&mut self, name: &str, window: &mut Window, cx: &mut Context<Dashboard>) {
        self.picker.update(cx, |state, cx| {
            state.set_selected_value(&SharedString::from(name.to_owned()), window, cx)
        });
    }
    pub fn poll(
        &mut self,
        history: &History,
        selected: &str,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> (bool, Option<String>) {
        let mut changed = false;
        let mut choice = None;
        if let Some((revision, receiver)) = self.pending.as_mut() {
            match receiver.try_recv() {
                Ok(result) => {
                    let revision = *revision;
                    self.pending = None;
                    match result {
                        Ok(names) => {
                            self.error = None;
                            self.names = names;
                            self.revision = revision;
                            self.picker.update(cx, |state, cx| {
                                state.set_items(
                                    SearchableVec::new(
                                        self.names
                                            .iter()
                                            .cloned()
                                            .map(SharedString::from)
                                            .collect::<Vec<_>>(),
                                    ),
                                    window,
                                    cx,
                                )
                            });
                            let value = if selected.is_empty() {
                                self.names.first().map(String::as_str)
                            } else {
                                Some(selected)
                            };
                            if let Some(value) = value {
                                let value = value.to_owned();
                                self.select(&value, window, cx);
                                if selected.is_empty() {
                                    choice = Some(value);
                                }
                            }
                        }
                        Err(error) => self.error = Some(error),
                    }
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending = None;
                    self.error = Some(crate::i18n::tr("Cached node choices are unavailable"));
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        let revision = history.health.nodes_revision.load(Ordering::Acquire);
        if self.pending.is_none() && revision != self.revision && Instant::now() >= self.next {
            let (send, receive) = oneshot::channel();
            let history = history.clone();
            let path = history.database_path().to_path_buf();
            cx.background_executor()
                .spawn(async move {
                    let ready = match history.fence() {
                        Ok(ready) => ready.await.is_ok(),
                        Err(_) => false,
                    };
                    drop(history);
                    let result = if ready {
                        crate::rules::export_nodes::catalogue(&path)
                            .map_err(|_| "Cached node choices are unavailable".to_owned())
                    } else {
                        Err("Cached node choices are unavailable".to_owned())
                    };
                    let _ = send.send(result);
                })
                .detach();
            self.pending = Some((revision, receive));
            self.next = Instant::now() + Duration::from_secs(1);
        }
        (changed, choice)
    }
}
