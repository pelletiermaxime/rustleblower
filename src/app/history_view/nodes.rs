//! One bounded background query; node filtering never needs a live daemon.
use super::*;
use crate::i18n::LocalizedOption;
use gpui_kit::component::select::{SearchableVec, SelectEvent, SelectState};

type Pending = (u64, oneshot::Receiver<Result<Vec<String>, String>>);

pub(super) struct State {
    pub picker: Entity<SelectState<SearchableVec<LocalizedOption<String>>>>,
    names: Vec<String>,
    selected: String,
    pending: Option<Pending>,
    revision: u64,
    next: Instant,
    pub error: Option<String>,
}
impl State {
    pub fn new(selected: &str, window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Self::items(&[], selected)),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        picker.update(cx, |state, cx| {
            state.set_selected_value(&selected.to_owned(), window, cx)
        });
        cx.subscribe(
            &picker,
            |this, _, event: &SelectEvent<SearchableVec<LocalizedOption<String>>>, cx| {
                if let SelectEvent::Confirm(Some(name)) = event
                    && this.history.query.node != *name
                {
                    this.history.query.node = name.clone();
                    this.history.reset();
                    cx.notify();
                }
            },
        )
        .detach();
        Self {
            picker,
            names: Vec::new(),
            selected: selected.into(),
            pending: None,
            revision: u64::MAX,
            next: Instant::now(),
            error: None,
        }
    }
    fn items(names: &[String], selected: &str) -> Vec<LocalizedOption<String>> {
        let mut items = vec![LocalizedOption::new(String::new(), "All nodes")];
        items.extend(
            names
                .iter()
                .cloned()
                .map(|name| LocalizedOption::raw(name.clone(), name)),
        );
        // Keep a saved selection explicit if retention removed its last event.
        if !selected.is_empty() && !names.iter().any(|name| name == selected) {
            items.push(LocalizedOption::raw(selected.to_owned(), selected));
        }
        items
    }
    pub fn poll(
        &mut self,
        history: &History,
        selected: &str,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> bool {
        let mut changed = false;
        let old_error = self.error.clone();
        if let Some((revision, receive)) = self.pending.as_mut() {
            match receive.try_recv() {
                Ok(result) => {
                    let revision = *revision;
                    self.pending = None;
                    self.revision = revision;
                    match result {
                        Ok(names) => {
                            self.error = None;
                            if self.names != names {
                                self.names = names;
                                changed = true;
                            }
                        }
                        Err(_) => {
                            self.error =
                                Some(crate::i18n::tr("Cached node choices are unavailable"))
                        }
                    }
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending = None;
                    self.error = Some(crate::i18n::tr("Cached node choices are unavailable"));
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        if changed || self.selected != selected {
            self.selected = selected.into();
            let items = Self::items(&self.names, selected);
            self.picker.update(cx, |state, cx| {
                state.set_items(SearchableVec::new(items), window, cx);
                state.set_selected_value(&selected.to_owned(), window, cx);
            });
            changed = true;
        }
        let revision = history.health.revision.load(Ordering::Acquire);
        if self.pending.is_none() && revision != self.revision && Instant::now() >= self.next {
            let (send, receive) = oneshot::channel();
            let history = history.clone();
            cx.background_executor()
                .spawn(async move {
                    let ready = match history.fence() {
                        Ok(fence) => fence.await.is_ok(),
                        Err(_) => false,
                    };
                    let result = if ready {
                        crate::history::query::node_choices(history.database_path())
                            .map_err(|e| e.to_string())
                    } else {
                        Err("Cached node choices are unavailable".into())
                    };
                    let _ = send.send(result);
                })
                .detach();
            self.pending = Some((revision, receive));
            self.next = Instant::now() + Duration::from_secs(1);
        }
        changed || self.error != old_error
    }
}
