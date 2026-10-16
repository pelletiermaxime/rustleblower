//! A node-targeted firewall snapshot, bounded browser and reviewed full reload.
use crate::{
    backend::Handle,
    firewall::{self, document::Document},
    nodes,
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::Button,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    list::ListItem,
    tab::{Tab, TabBar},
};
use std::time::Duration;
mod files;
mod forms;
mod policy;
use crate::firewall::document::Command;
actions!(
    firewall_view,
    [
        KeepDraft,
        ConfirmDraft,
        EditSelectedChain,
        NewChainDraft,
        CancelDraft,
        ShowPolicies,
        ToggleSystemDraft
    ]
);
pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-enter", KeepDraft, Some("SystemFirewall")),
        KeyBinding::new("ctrl-shift-enter", ConfirmDraft, Some("SystemFirewall")),
        KeyBinding::new("alt-e", EditSelectedChain, Some("SystemFirewall")),
        KeyBinding::new("ctrl-n", NewChainDraft, Some("SystemFirewall")),
        KeyBinding::new("escape", CancelDraft, Some("SystemFirewall")),
        KeyBinding::new("alt-p", ShowPolicies, Some("SystemFirewall")),
        KeyBinding::new("ctrl-shift-e", ToggleSystemDraft, Some("SystemFirewall")),
    ]);
}
use forms::{Form, Target};

pub(super) fn open(
    backend: Handle,
    session: u64,
    node: String,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1000.), px(780.)),
                cx,
            ))),
            window_min_size: Some(size(px(760.), px(580.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Rustleblower — system firewall".into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| FirewallView::new(backend, session, node, window, cx)),
    )?;
    Ok(window)
}
struct Loaded {
    snapshot: firewall::Snapshot,
    document: Document,
    text: String,
}
struct Stage {
    plan: firewall::policy::Plan,
    summary: Vec<String>,
}
struct FirewallView {
    backend: Handle,
    session: u64,
    node: String,
    loaded: Option<Loaded>,
    input: Entity<TextareaState>,
    search: Entity<InputState>,
    last_text: String,
    tab: usize,
    chain: Option<(usize, usize)>,
    offset: usize,
    stage: Option<Stage>,
    busy: bool,
    error: Option<String>,
    message: Option<String>,
    focus: FocusHandle,
    form: Option<Form>,
    selected_rule: Option<usize>,
    delete: Option<Command>,
    files: files::Files,
    operation: Option<std::sync::Arc<std::sync::Mutex<firewall::policy::Progress>>>,
}
impl FirewallView {
    fn keep_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.delete.is_some() || self.stage.is_some() {
            return;
        }
        if let Some(form) = &mut self.form {
            match form.keep_active(cx) {
                Ok(true) => {
                    self.error = None;
                    self.focus.focus(window, cx);
                    cx.notify();
                }
                Ok(false) => self.commit_form(window, cx),
                Err(e) => {
                    self.error = Some(e);
                    cx.notify();
                }
            }
        } else {
            self.review(window, cx);
        }
    }
    fn cancel_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some(form) = &mut self.form {
            if !form.cancel_active() {
                self.form = None;
            }
        } else {
            self.delete = None;
            self.stage = None;
        }
        self.error = None;
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn new(
        backend: Handle,
        session: u64,
        node: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx));
        cx.subscribe_in(&input, window, |this, state, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                let text = state.read(cx).value().to_string();
                if text.len() > firewall::MAX_JSON {
                    state.update(cx, |state, cx| {
                        state.set_value(this.last_text.clone(), window, cx)
                    });
                    this.error = Some("Firewall JSON must stay below 1 MiB".into());
                } else {
                    this.last_text = text;
                    this.stage = None;
                }
                cx.notify();
            }
        })
        .detach();
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter rules in the selected chain…")
                .validate(|v, _| v.len() <= 256)
        });
        cx.subscribe(&search, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.offset = 0;
                this.selected_rule = None;
                cx.notify();
            }
        })
        .detach();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut this = Self {
            backend,
            session,
            node,
            loaded: None,
            input,
            search,
            last_text: String::new(),
            tab: 0,
            chain: None,
            offset: 0,
            stage: None,
            busy: false,
            error: None,
            message: None,
            focus,
            form: None,
            selected_rule: None,
            delete: None,
            files: files::Files::new(window, cx),
            operation: None,
        };
        this.load(window, cx);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        this.poll_operation(window, cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        this
    }
    fn writable(&self) -> bool {
        !self.backend.control.firewall_busy(self.session)
            && self.loaded.as_ref().is_some_and(|loaded| {
                loaded.snapshot.session == self.session
                    && loaded.snapshot.config.version == 1
                    && self
                        .backend
                        .control
                        .firewall_current(self.session, loaded.snapshot.fingerprint)
            })
            && self
                .backend
                .session(self.session)
                .is_some_and(|s| s.name == self.node && s.notifications)
            && !self
                .backend
                .history
                .health
                .catalogues
                .uncertain(self.session)
    }
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.stage = None;
        self.form = None;
        self.delete = None;
        self.selected_rule = None;
        let path = self.backend.history.database_path().to_owned();
        let node = self.node.clone();
        let work = cx.background_executor().spawn(async move {
            let _job = firewall::read_job()
                .ok_or("A previous firewall read is finishing; try Reload cached snapshot again")?;
            let _permit = nodes::wait_read_permit().await?;
            let snapshot = firewall::read(&path, &node).map_err(|e| format!("{e:#}"))?;
            let document = Document::from(snapshot.config.clone());
            let text = document.json()?;
            Ok::<_, String>(Loaded {
                snapshot,
                document,
                text,
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(loaded) => {
                        this.chain =
                            loaded.document.system_rules.iter().enumerate().find_map(
                                |(g, group)| (!group.chains.is_empty()).then_some((g, 0)),
                            );
                        this.input.update(cx, |state, cx| {
                            state.set_value(loaded.text.clone(), window, cx)
                        });
                        this.last_text = loaded.text.clone();
                        this.loaded = Some(loaded);
                        if this.tab == 1 {
                            this.input.focus_handle(cx).focus(window, cx);
                        }
                        this.offset = 0;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn read_draft(&mut self, target: Option<Target>, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || (target.is_some() && !self.writable()) {
            return;
        }
        let destination = if target.is_none() { self.tab } else { 0 };
        let text = self.input.read(cx).value().to_string();
        if target.is_none() && self.loaded.as_ref().is_some_and(|l| l.text == text) {
            self.tab = destination;
            self.focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        let work = cx
            .background_executor()
            .spawn(async move { Document::parse(&text) });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(document) => {
                        if let Some(loaded) = &mut this.loaded {
                            loaded.document = document;
                        }
                        if this.chain.is_none_or(|(g, c)| {
                            this.loaded
                                .as_ref()
                                .is_none_or(|l| l.document.chain(g, c).is_none())
                        }) {
                            this.chain =
                                this.loaded.as_ref().and_then(|l| {
                                    l.document.system_rules.iter().enumerate().find_map(
                                        |(g, group)| (!group.chains.is_empty()).then_some((g, 0)),
                                    )
                                });
                            this.selected_rule = None;
                            this.offset = 0;
                        }
                        this.tab = destination;
                        if let Some(target) = target {
                            if let Some(loaded) = &this.loaded {
                                match Form::new(target, &loaded.document, window, cx) {
                                    Ok(form) => this.form = Some(form),
                                    Err(e) => this.error = Some(e),
                                }
                            }
                        } else {
                            this.focus.focus(window, cx);
                        }
                    }
                    Err(e) => {
                        this.error = Some(e);
                        this.tab = 1;
                        this.input.focus_handle(cx).focus(window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn commit_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &self.form else {
            return;
        };
        match form.command(cx) {
            Ok(command) => self.mutate(command, window, cx),
            Err(e) => {
                self.error = Some(e);
                cx.notify();
            }
        }
    }
    fn mutate(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        // An already-open form can still keep a local draft after disconnect.
        // The shared review/apply path continues to require the captured daemon.
        if self.busy || (!self.writable() && self.form.is_none()) {
            return;
        }
        let Some(loaded) = &self.loaded else {
            return;
        };
        let mut document = loaded.document.clone();
        let created_chain = match &command {
            Command::Chain {
                group, index: None, ..
            } => Some((
                *group,
                document
                    .system_rules
                    .get(*group)
                    .map_or(0, |g| g.chains.len()),
            )),
            _ => None,
        };
        self.busy = true;
        self.error = None;
        self.message = None;
        let work = cx.background_executor().spawn(async move {
            document.command(command)?;
            let text = document.json()?;
            Ok::<_, String>((document, text))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((document, text)) => {
                        if let Some(loaded) = &mut this.loaded {
                            loaded.document = document;
                        }
                        this.last_text = text.clone();
                        this.input
                            .update(cx, |state, cx| state.set_value(text, window, cx));
                        this.chain = created_chain.or(this.chain);
                        this.form = None;
                        this.delete = None;
                        this.stage = None;
                        this.selected_rule = None;
                        if this.chain.is_none_or(|(g, c)| {
                            this.loaded
                                .as_ref()
                                .is_none_or(|l| l.document.chain(g, c).is_none())
                        }) {
                            this.chain =
                                this.loaded.as_ref().and_then(|l| {
                                    l.document.system_rules.iter().enumerate().find_map(
                                        |(g, group)| (!group.chains.is_empty()).then_some((g, 0)),
                                    )
                                });
                            this.offset = 0;
                        }
                        this.focus.focus(window, cx);
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.writable() {
            return;
        }
        let Some(loaded) = &self.loaded else {
            return;
        };
        let before = loaded.snapshot.config.clone();
        let expected = loaded.snapshot.fingerprint;
        let text = self.input.read(cx).value().to_string();
        self.busy = true;
        self.error = None;
        self.message = None;
        let work = cx.background_executor().spawn(async move {
            let document = Document::parse(&text)?;
            let plan = firewall::policy::Plan::new(expected, &before, document.into())?;
            let summary = plan.summary().to_vec();
            Ok::<_, String>(Stage { plan, summary })
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(stage) => {
                        this.stage = Some(stage);
                        this.focus.focus(window, cx);
                    }
                    Err(error) => {
                        this.error = Some(error);
                        this.input.focus_handle(cx).focus(window, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn apply(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.writable() || self.stage.is_none() {
            return;
        }
        let stage = self.stage.take().unwrap();
        // Reserving fails before starting work. Preserve the review on failure
        // by rebuilding it from the unchanged complete draft on the next Review.
        match stage.plan.begin(&self.backend.control, self.session) {
            Ok(operation) => {
                self.operation = Some(operation.progress());
                self.busy = true;
                self.error = None;
                cx.background_executor().spawn(operation.run()).detach();
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
    fn poll_operation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self
            .operation
            .as_ref()
            .and_then(|p| p.lock().unwrap().outcome.clone());
        let Some(result) = result else {
            return;
        };
        self.operation = None;
        self.busy = false;
        match result {
            Ok(steps) => {
                self.message = Some(format!(
                    "Daemon accepted {steps} reload stage(s), and the local cache was saved. Acceptance does not prove enforcement; awaiting a fresh daemon report."
                ));
                self.load(window, cx);
            }
            Err(error) => self.error = Some(error),
        }
    }
}
impl Render for FirewallView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = self.input.clone();
        let weak = cx.weak_entity();
        let writable = self.writable();
        let locked = self.busy || self.stage.is_some();
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let mut body = div().flex_1().min_h_0().flex().flex_col().gap_2();
        if let Some(operation) = &self.operation {
            let progress = operation.lock().unwrap().clone();
            body=body.child(format!("System-firewall operation: {} of {} confirmed cache updates",progress.completed,progress.total))
                .child(progress.label).child("Waiting for the daemon and saved cache before continuing. Closing this window lets the reviewed operation continue in the background.");
        } else if let Some(command) = self.delete.clone() {
            body=body.child("Remove this item from the local draft? The daemon changes only after Review and Apply.")
                .child(div().flex().gap_2().child(Button::new("fw-delete-confirm").label("Remove from draft").disabled(self.busy||!writable).on_click(cx.listener(move|this,_,window,cx|this.mutate(command.clone(),window,cx))))
                .child(Button::new("fw-delete-cancel").label("Cancel removal").disabled(self.busy).on_click(cx.listener(|this,_,window,cx|{this.delete=None;this.focus.focus(window,cx);cx.notify();}))));
        } else if let Some(form) = &self.form {
            body = form.render(self.busy, cx);
        } else if let Some(stage) = &self.stage {
            body=body.child(div().text_sm().child("Replace this node's complete system-firewall configuration? Unrelated application rules remain separate. The daemon can reject or partly apply a reload; refresh after any error. Long review values are shortened; Back opens the complete draft."))
                .child(div().id("firewall-review-body").flex_1().min_h_0().overflow_y_scroll().children(stage.summary.iter().cloned().map(|s|div().text_sm().child(s))))
                .child(div().flex().gap_2().child(Button::new("firewall-back").label("Back to complete draft").disabled(self.busy).on_click(cx.listener(|this,_,window,cx|{this.stage=None;if this.tab==1{this.input.focus_handle(cx).focus(window,cx);}else{this.focus.focus(window,cx);}cx.notify();})))
                .child(Button::new("firewall-confirm").label("Apply reviewed firewall reload").disabled(self.busy||!writable).on_click(cx.listener(|this,_,window,cx|this.apply(window,cx)))));
        } else if self.tab == 3 {
            body = body.child(self.render_policy(cx));
        } else if self.tab == 2 {
            body = body.child(self.render_files(cx));
        } else if self.tab == 1 {
            body=body.child(div().text_sm().child("Edits replace this node's complete configuration. Review the changes before applying. Enable/disable transitions use the same review; prepare accept policies before disabling."))
                .child(div().flex_1().min_h_0().child(Textarea::new(&self.input).accessibility_id("firewall-json").size_full().disabled(!writable||locked).on_paste(move|clipboard,_,cx| {
                    let state=input.read(cx);let remaining=state.value().len().saturating_sub(state.selected_value().len());
                    if clipboard.text().is_some_and(|text|text.len().saturating_add(remaining)>firewall::MAX_JSON) {
                        let _=weak.update(cx,|this,cx|{this.error=Some("Paste would exceed the 1 MiB firewall document limit".into());cx.notify();});true
                    }else{false}
                })))
                .child(Button::new("firewall-review").label("Review complete changes…").disabled(!writable||locked).on_click(cx.listener(|this,_,window,cx|this.review(window,cx))));
        } else if let Some(loaded) = &self.loaded {
            let selected = self.chain;
            let mut choices = div()
                .id("firewall-chains")
                .w(px(230.))
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col();
            for (g, group) in loaded.document.system_rules.iter().enumerate() {
                for (c, chain) in group.chains.iter().enumerate() {
                    choices = choices.child(
                        Button::new(("firewall-chain", (g * 256 + c) as u64))
                            .label(format!(
                                "{} / {} / {}",
                                chain.family, chain.table, chain.name
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.chain = Some((g, c));
                                this.offset = 0;
                                this.selected_rule = None;
                                cx.notify();
                            })),
                    );
                }
            }
            let mut rows = div().flex_1().min_w_0().min_h_0().flex().flex_col().gap_2();
            if let Some(chain) = selected.and_then(|(g, c)| loaded.document.chain(g, c)) {
                let search = self.search.read(cx).value().to_lowercase();
                let matches = chain
                    .rules
                    .iter()
                    .enumerate()
                    .filter_map(|(i, r)| {
                        (search.is_empty()
                            || r.uuid.to_lowercase().contains(&search)
                            || r.description.to_lowercase().contains(&search)
                            || r.target.to_lowercase().contains(&search))
                        .then_some(i)
                    })
                    .collect::<Vec<_>>();
                let total = matches.len();
                let offset = self.offset.min(total);
                let end = (offset + firewall::PAGE).min(matches.len());
                rows = rows
                    .child(format!(
                        "{} · type {} · hook {} · priority {} · policy {}",
                        chain.name, chain.kind, chain.hook, chain.priority, chain.policy
                    ))
                    .child(Input::new(&self.search).id("firewall-search"))
                    .child(format!(
                        "{} matching rules · showing {}–{}",
                        matches.len(),
                        if matches.is_empty() { 0 } else { offset + 1 },
                        end
                    ))
                    .child(
                        uniform_list(
                            "firewall-rules",
                            end - offset,
                            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                let Some(chain) = this.loaded.as_ref().and_then(|l| {
                                    this.chain.and_then(|(g, c)| l.document.chain(g, c))
                                }) else {
                                    return vec![];
                                };
                                range
                                    .filter_map(|i| {
                                        matches.get(offset + i).map(|n| (*n, &chain.rules[*n]))
                                    })
                                    .map(|(i, rule)| {
                                        ListItem::new(("firewall-rule", i))
                                            .selected(this.selected_rule == Some(i))
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .child(format!(
                                                        "{} · {} · {} · {}",
                                                        rule.position,
                                                        rule.uuid,
                                                        rule.target,
                                                        if rule.enabled {
                                                            "enabled"
                                                        } else {
                                                            "disabled"
                                                        }
                                                    ))
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .child(rule.description.clone()),
                                                    ),
                                            )
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.selected_rule = Some(i);
                                                cx.notify();
                                            }))
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .flex_1()
                        .min_h_0(),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("firewall-previous")
                                    .label("Previous")
                                    .disabled(offset == 0)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.offset = this.offset.saturating_sub(firewall::PAGE);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("firewall-next")
                                    .label("Next")
                                    .disabled(end >= total)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.offset += firewall::PAGE;
                                        cx.notify();
                                    })),
                            ),
                    );
            } else {
                rows = rows.child(
                    "No reported chains. Legacy wrappers remain available in complete JSON.",
                );
            }
            body = body.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .gap_3()
                    .child(choices)
                    .child(rows),
            );
            let chain = self.chain;
            let selected = self.selected_rule;
            let count = chain
                .and_then(|(g, c)| loaded.document.chain(g, c))
                .map_or(0, |c| c.rules.len());
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("fw-chain-new")
                            .label("New chain…")
                            .disabled(!writable || locked)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.read_draft(
                                    Some(Target::Chain {
                                        group: this.chain.map_or(0, |p| p.0),
                                        index: None,
                                    }),
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("fw-chain-edit")
                            .label("Edit chain…")
                            .disabled(!writable || locked || chain.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some((group, index)) = this.chain {
                                    this.read_draft(
                                        Some(Target::Chain {
                                            group,
                                            index: Some(index),
                                        }),
                                        window,
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child(
                        Button::new("fw-chain-delete")
                            .label("Remove chain…")
                            .disabled(!writable || locked || chain.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some((group, chain)) = this.chain {
                                    this.delete = Some(Command::DeleteChain { group, chain });
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new("fw-rule-new")
                            .label("New rule…")
                            .disabled(!writable || locked || chain.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some((group, chain)) = this.chain {
                                    this.read_draft(
                                        Some(Target::Rule {
                                            group,
                                            chain,
                                            index: None,
                                            duplicate: false,
                                        }),
                                        window,
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child(
                        Button::new("fw-rule-edit")
                            .label("Edit rule…")
                            .disabled(!writable || locked || selected.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some((group, chain)) = this.chain
                                    && let Some(index) = this.selected_rule
                                {
                                    this.read_draft(
                                        Some(Target::Rule {
                                            group,
                                            chain,
                                            index: Some(index),
                                            duplicate: false,
                                        }),
                                        window,
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child(
                        Button::new("fw-rule-duplicate")
                            .label("Duplicate rule…")
                            .disabled(!writable || locked || selected.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some((group, chain)) = this.chain
                                    && let Some(index) = this.selected_rule
                                {
                                    this.read_draft(
                                        Some(Target::Rule {
                                            group,
                                            chain,
                                            index: Some(index),
                                            duplicate: true,
                                        }),
                                        window,
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child(
                        Button::new("fw-rule-toggle")
                            .label("Toggle enabled in draft")
                            .disabled(!writable || locked || selected.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some((group, chain)) = this.chain
                                    && let Some(rule) = this.selected_rule
                                {
                                    this.mutate(
                                        Command::ToggleRule { group, chain, rule },
                                        window,
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child(
                        Button::new("fw-rule-delete")
                            .label("Remove rule…")
                            .disabled(!writable || locked || selected.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some((group, chain)) = this.chain
                                    && let Some(rule) = this.selected_rule
                                {
                                    this.delete = Some(Command::DeleteRule { group, chain, rule });
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new("fw-rule-up")
                            .label("Move rule up")
                            .disabled(!writable || locked || selected.is_none_or(|i| i == 0))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some((group, chain)) = this.chain
                                    && let Some(from) = this.selected_rule
                                    && from > 0
                                {
                                    this.mutate(
                                        Command::MoveRule {
                                            group,
                                            chain,
                                            from,
                                            to: from - 1,
                                        },
                                        window,
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child(
                        Button::new("fw-rule-down")
                            .label("Move rule down")
                            .disabled(
                                !writable || locked || selected.is_none_or(|i| i + 1 >= count),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some((group, chain)) = this.chain
                                    && let Some(from) = this.selected_rule
                                    && from + 1 < count
                                {
                                    this.mutate(
                                        Command::MoveRule {
                                            group,
                                            chain,
                                            from,
                                            to: from + 1,
                                        },
                                        window,
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child(
                        Button::new("fw-draft-review")
                            .label("Review draft changes…")
                            .disabled(!writable || locked)
                            .on_click(cx.listener(|this, _, window, cx| this.review(window, cx))),
                    ),
            );
        } else {
            body = body.child("Loading cached system-firewall snapshot…");
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .track_focus(&self.focus)
            .key_context("SystemFirewall")
            .on_action(cx.listener(|this, _: &ShowPolicies, window, cx| {
                if !this.busy
                    && this.form.is_none()
                    && this.stage.is_none()
                    && this.delete.is_none()
                {
                    this.tab = 3;
                    this.read_draft(None, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleSystemDraft, window, cx| {
                if this.tab == 3
                    && this.form.is_none()
                    && this.stage.is_none()
                    && this.delete.is_none()
                    && let Some(loaded) = &this.loaded
                {
                    this.mutate(Command::Enabled(!loaded.document.enabled), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &KeepDraft, window, cx| this.keep_key(window, cx)))
            .on_action(cx.listener(|this, _: &ConfirmDraft, window, cx| this.apply(window, cx)))
            .on_action(cx.listener(|this, _: &CancelDraft, window, cx| this.cancel_key(window, cx)))
            .on_action(cx.listener(|this, _: &EditSelectedChain, window, cx| {
                if this.form.is_none()
                    && this.stage.is_none()
                    && this.delete.is_none()
                    && let Some((group, index)) = this.chain
                {
                    this.read_draft(
                        Some(Target::Chain {
                            group,
                            index: Some(index),
                        }),
                        window,
                        cx,
                    );
                }
            }))
            .on_action(cx.listener(|this, _: &NewChainDraft, window, cx| {
                if this.form.is_none() && this.stage.is_none() && this.delete.is_none() {
                    this.read_draft(
                        Some(Target::Chain {
                            group: this.chain.map_or(0, |p| p.0),
                            index: None,
                        }),
                        window,
                        cx,
                    );
                }
            }))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(format!("System firewall · {}", self.node))
            .when(
                self.loaded
                    .as_ref()
                    .is_some_and(|l| l.text != self.last_text),
                |view| {
                    view.child(div().text_sm().child(
                        "Local draft changes · Review and Apply to send them to this daemon",
                    ))
                },
            )
            .child(
                div().text_sm().text_color(muted).child(
                    self.loaded
                        .as_ref()
                        .map(|l| {
                            format!(
                                "Backend {} · format {} · enabled {} · {} · {}",
                                l.snapshot.backend,
                                l.snapshot.config.version,
                                l.snapshot.config.enabled,
                                if l.snapshot.reported {
                                    "reported by daemon"
                                } else {
                                    "accepted reload; awaiting fresh daemon report"
                                },
                                if writable {
                                    "connected"
                                } else {
                                    "read only / stale / unsupported"
                                }
                            )
                        })
                        .unwrap_or_else(|| "No snapshot loaded".into()),
                ),
            )
            .child(
                TabBar::new("firewall-tabs")
                    .selected_index(self.tab)
                    .on_click(cx.listener(|this, index: &usize, window, cx| {
                        if !this.busy
                            && this.stage.is_none()
                            && this.form.is_none()
                            && this.delete.is_none()
                        {
                            if *index != 1 && this.tab == 1 {
                                this.tab = *index;
                                this.read_draft(None, window, cx);
                                return;
                            }
                            this.tab = *index;
                            if *index == 1 {
                                this.input.focus_handle(cx).focus(window, cx);
                            }
                            cx.notify();
                        }
                    }))
                    .child(Tab::new().label("Chains and rules"))
                    .child(Tab::new().label("Complete JSON"))
                    .child(Tab::new().label("Files and profiles"))
                    .child(Tab::new().label("Policies and state")),
            )
            .when_some(self.error.clone(), |v, e| v.child(super::banner(e, cx)))
            .when_some(self.message.clone(), |v, m| {
                v.child(div().text_sm().child(m))
            })
            .child(body)
            .child(
                div()
                    .border_t_1()
                    .border_color(border)
                    .pt_2()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("firewall-reload")
                            .label("Reload cached snapshot")
                            .disabled(locked || self.form.is_some() || self.delete.is_some())
                            .on_click(cx.listener(|this, _, window, cx| this.load(window, cx))),
                    )
                    .child(
                        Button::new("firewall-copy")
                            .label("Copy complete draft JSON")
                            .disabled(
                                self.loaded.is_none()
                                    || self.busy
                                    || self.form.is_some()
                                    || self.delete.is_some(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    this.input.read(cx).value().to_string(),
                                ))
                            })),
                    )
                    .child(
                        Button::new("firewall-close")
                            .label("Close")
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    ),
            )
    }
}
