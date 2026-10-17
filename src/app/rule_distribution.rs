//! Native reviewed cross-node rule copies; the background engine owns all I/O.
use crate::{
    backend::Handle,
    rules::{
        distribute::{self, Destination, Progress, Review, Status},
        transfer::ExportScope,
    },
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Clone)]
enum Source {
    Catalogue(Box<ExportScope>),
    Draft {
        rules: Arc<Vec<crate::protocol::Rule>>,
        rename_from: Option<String>,
    },
    Import,
}
pub(super) fn open(
    backend: Handle,
    scope: ExportScope,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    open_source(
        backend,
        Source::Catalogue(Box::new(scope)),
        None,
        false,
        None,
        cx,
    )
}
pub(super) fn open_import(backend: Handle, cx: &mut App) -> gpui::Result<AnyWindowHandle> {
    let preferred = backend.status().session.map(|s| s.id);
    open_source(backend, Source::Import, preferred, false, None, cx)
}
pub(super) fn open_draft(
    backend: Handle,
    rule: crate::protocol::Rule,
    original: Option<String>,
    preferred: u64,
    complete: tokio::sync::oneshot::Sender<()>,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let replace = original.as_ref() == Some(&rule.name);
    let rename_from = original.filter(|old| *old != rule.name);
    open_source(
        backend,
        Source::Draft {
            rules: Arc::new(vec![rule]),
            rename_from,
        },
        Some(preferred),
        replace,
        Some(complete),
        cx,
    )
}
fn open_source(
    backend: Handle,
    source: Source,
    preferred: Option<u64>,
    replace: bool,
    complete: Option<tokio::sync::oneshot::Sender<()>>,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(940.), px(720.)),
                cx,
            ))),
            window_min_size: Some(size(px(720.), px(560.))),
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Rustleblower — rules to nodes").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| {
            cx.new(|cx| Copies::new(backend, source, preferred, replace, complete, window, cx))
        },
    )?;
    crate::i18n::register_title(window, "Rustleblower — rules to nodes", cx);
    Ok(window)
}
pub(super) struct Copies {
    backend: Handle,
    source: Source,
    path: Entity<InputState>,
    clipboard: Option<Arc<String>>,
    import_invalid: bool,
    complete: Option<tokio::sync::oneshot::Sender<()>>,
    nodes: Vec<Destination>,
    selected: BTreeSet<u64>,
    all: bool,
    replace: bool,
    review: Option<Arc<Review>>,
    progress: Option<Progress>,
    rules_tab: bool,
    busy: bool,
    applying: bool,
    error: Option<String>,
    message: String,
    stop: Arc<AtomicBool>,
    focus: FocusHandle,
}
impl Drop for Copies {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(reply) = self.complete.take() {
            let _ = reply.send(());
        }
    }
}
impl Copies {
    fn new(
        backend: Handle,
        source: Source,
        preferred: Option<u64>,
        replace: bool,
        complete: Option<tokio::sync::oneshot::Sender<()>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.on_window_should_close(cx, |window, _| {
            window.minimize_window();
            false
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "JSON file or rule directory",
                    window,
                    cx,
                ))
                .validate(|value, _| value.len() <= 4096)
        });
        cx.subscribe(&path, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.clipboard = None;
                this.import_invalid = false;
                this.error = None;
                cx.notify();
            }
        })
        .detach();
        if matches!(source, Source::Import) {
            path.focus_handle(cx).focus(window, cx);
        }
        let mut this = Self {
            backend,
            all: matches!(&source,Source::Catalogue(scope) if scope.names.is_none()),
            source,
            path,
            clipboard: None,
            import_invalid: false,
            complete,
            nodes: Vec::new(),
            selected: BTreeSet::new(),
            replace,
            review: None,
            progress: None,
            rules_tab: false,
            busy: false,
            applying: false,
            error: None,
            message: String::new(),
            stop: Arc::new(AtomicBool::new(false)),
            focus,
        };
        this.refresh();
        if let Some(id) = preferred
            && this.nodes.iter().any(|d| d.session == id)
        {
            this.selected.insert(id);
        }
        this
    }
    fn refresh(&mut self) {
        let chosen = self
            .nodes
            .iter()
            .filter(|d| self.selected.contains(&d.session))
            .map(|d| d.name.clone())
            .collect::<BTreeSet<_>>();
        self.nodes = self
            .backend
            .status()
            .sessions
            .values()
            .filter(|s| {
                s.notifications
                    && !matches!(&self.source,Source::Catalogue(scope) if s.name==scope.query.node)
            })
            .map(|s| Destination {
                session: s.id,
                name: s.name.clone(),
            })
            .collect();
        self.nodes.sort_by(|a, b| a.name.cmp(&b.name));
        self.selected = self
            .nodes
            .iter()
            .filter(|d| chosen.contains(&d.name))
            .map(|d| d.session)
            .collect();
    }
    fn prepare(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if matches!(self.source, Source::Import) && self.import_invalid {
            self.error = Some(crate::i18n::tr(
                "Capture valid clipboard text or choose a file path before review",
            ));
            cx.notify();
            return;
        }
        let destinations = self
            .nodes
            .iter()
            .filter(|d| self.selected.contains(&d.session))
            .cloned()
            .collect::<Vec<_>>();
        let mut source = self.source.clone();
        if let Source::Catalogue(scope) = &mut source
            && self.all
        {
            scope.names = None;
        }
        self.busy = true;
        self.error = None;
        self.message = "Reading source rules and destination inventories…".into();
        let backend = self.backend.clone();
        let replace = self.replace;
        let path = std::path::PathBuf::from(self.path.read(cx).value().to_string());
        let clipboard = self.clipboard.clone();
        let task = cx.background_executor().spawn(async move {
            match source {
                Source::Catalogue(scope) => {
                    distribute::prepare(&backend, &scope, destinations, replace)
                }
                Source::Draft { rules, rename_from } => distribute::prepare_rules(
                    &backend,
                    "Unsaved rule draft",
                    rules,
                    destinations,
                    replace,
                    rename_from,
                ),
                Source::Import => {
                    let imported = if let Some(text) = clipboard {
                        crate::rules::transfer::Import::from_text(&text)?
                    } else {
                        crate::rules::transfer::Import::from_path(&path)?
                    };
                    distribute::prepare_rules(
                        &backend,
                        "Imported rules",
                        Arc::new(imported.rules),
                        destinations,
                        replace,
                        None,
                    )
                }
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(review) => {
                        this.progress = Some(review.progress());
                        this.review = Some(Arc::new(review));
                        this.message =
                            "Review destinations and source rules before copying.".into();
                    }
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(review) = self.review.clone() else {
            return;
        };
        let Some(progress) = self.progress.clone() else {
            return;
        };
        self.busy = true;
        self.applying = true;
        self.error = None;
        self.stop.store(false, Ordering::Release);
        let stop = self.stop.clone();
        let engine_stop = stop.clone();
        let backend = self.backend.clone();
        let (tx, mut rx) = tokio::sync::watch::channel(progress.clone());
        let task = cx.background_executor().spawn(async move {
            distribute::run(backend, review, progress, engine_stop, tx).await
        });
        cx.spawn_in(window,async move|this,cx|{while rx.changed().await.is_ok(){let progress=rx.borrow_and_update().clone();if this.update_in(cx,|this,_,cx|{this.progress=Some(progress);cx.notify();}).is_err(){stop.store(true,Ordering::Release);}}
   let progress=task.await;let _=this.update_in(cx,|this,_,cx|{this.busy=false;this.applying=false;this.message=crate::i18n::tr_args("%{arg0} copies confirmed. Completed copies remain applied; unfinished targets keep their captured connection and inventory.", &[("arg0", &format!("{}", progress.confirmed()))]);this.progress=Some(progress);cx.notify();});}).detach();
        cx.notify();
    }
    fn copy_rule(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(review) = self.review.clone() else {
            return;
        };
        self.busy = true;
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            let text = serde_json::to_string_pretty(&crate::rules::document::rule_json(
                &review.rules[index],
            ))?;
            anyhow::ensure!(
                text.len() <= crate::rules::MAX_RULE_BYTES * 8,
                "Rule JSON exceeds 512 KiB"
            );
            Ok::<_, anyhow::Error>(text)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(text) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        this.message = "Copied exact source rule JSON.".into();
                    }
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn capture_clipboard(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        match cx.read_from_clipboard().and_then(|item| item.text()) {
            Some(text) if text.len() <= crate::rules::document::MAX_DOCUMENT_BYTES => {
                self.clipboard = Some(Arc::new(text));
                self.import_invalid = false;
                self.error = None;
                self.message =
                    "Clipboard text captured. Review validates this exact captured content.".into();
            }
            Some(_) => {
                self.clipboard = None;
                self.import_invalid = true;
                self.error = Some(crate::i18n::tr("Clipboard import exceeds 8 MiB"));
            }
            None => {
                self.clipboard = None;
                self.import_invalid = true;
                self.error = Some(crate::i18n::tr("The clipboard has no rule JSON text"));
            }
        }
        cx.notify();
    }
    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Choose rule JSON or a rule directory".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = prompt.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            this.path.update(cx, |input, cx| {
                                input.set_value(path.to_string_lossy().to_string(), window, cx)
                            });
                            this.clipboard = None;
                        }
                    }
                    Ok(Ok(None)) => {}
                    other => {
                        this.error = Some(crate::i18n::tr_args(
                            "Cannot choose import path: %{other}",
                            &[("other", &format!("{:?}", other))],
                        ))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn source_label(&self) -> String {
        match &self.source {
            Source::Catalogue(scope) => crate::i18n::tr_args(
                "Source: %{arg0}",
                &[("arg0", &scope.query.node.to_string())],
            ),
            Source::Draft { .. } => {
                "Unsaved rule draft; choose every node that should receive it.".into()
            }
            Source::Import => {
                if self.clipboard.is_some() {
                    "Source: captured clipboard text".into()
                } else {
                    "Source: JSON file or rule directory".into()
                }
            }
        }
    }
    fn copy_results(&self, cx: &mut Context<Self>) {
        if let (Some(review), Some(progress)) = (&self.review, &self.progress) {
            let text = review
                .targets
                .iter()
                .zip(&progress.targets)
                .map(|(target, p)| {
                    crate::i18n::tr_args("%{arg0}: %{arg1}; %{arg2} confirmed; %{arg3} skipped; %{arg4} originals removed; %{arg5}/%{arg6} processed", &[("arg0", &target.destination.name.to_string()), ("arg1", &crate::i18n::tr(&p.status.label()).to_string()), ("arg2", &format!("{}", p.confirmed)), ("arg3", &format!("{}", p.skipped)), ("arg4", &format!("{}", p.removed)), ("arg5", &format!("{}", p.next)), ("arg6", &format!("{}", review.rules.len()))])
                })
                .collect::<Vec<_>>()
                .join("\n");
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn test_state(&self) -> (bool, usize, Vec<Status>) {
        (
            self.busy,
            self.review.as_ref().map_or(0, |r| r.rules.len()),
            self.progress.as_ref().map_or(Vec::new(), |p| {
                p.targets.iter().map(|t| t.status.clone()).collect()
            }),
        )
    }
}
impl Render for Copies {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let busy = self.busy;
        let mut body = div().flex_1().min_h_0().px_5().flex().flex_col().gap_3();
        if let Some(review) = &self.review {
            body=body.child(crate::i18n::tr_args("%{arg0} source rules from %{arg1} · %{arg2} destinations · %{arg3}", &[("arg0", &(review.rules.len()).to_string()), ("arg1", &(review.source).to_string()), ("arg2", &(review.targets.len()).to_string()), ("arg3", (if review.replace{"Replace conflicting confirmed names"}else{"Skip existing names"}))]))
    .when_some(review.rename_from.as_ref(),|v,old|v.child(crate::i18n::tr_args("Rename: confirm %{arg0} first, then remove %{arg1} on each destination where it exists.", &[("arg0", &(review.rules[0].name).to_string()), ("arg1", &(old).to_string())])))
    .child(div().text_sm().text_color(muted).child(crate::i18n::tr("The captured source data stays unchanged. Paths, users and networks retain their exact source values on each destination. Identical rules and names awaiting observation are always skipped. No cross-node rollback is possible.")))
    .child(div().flex().gap_2().child(Button::new("copy-destinations-tab").label(crate::i18n::tr("Destinations & results")).selected(!self.rules_tab).on_click(cx.listener(|this,_,_,cx|{this.rules_tab=false;cx.notify();})))
      .child(Button::new("copy-rules-tab").label(crate::i18n::tr("Source rules")).selected(self.rules_tab).on_click(cx.listener(|this,_,_,cx|{this.rules_tab=true;cx.notify();}))));
            let count = if self.rules_tab {
                review.rules.len()
            } else {
                review.targets.len()
            };
            let rows=uniform_list("copy-review",count,cx.processor(|this,range:std::ops::Range<usize>,_,cx|range.map(|index|{
    let review=this.review.as_ref().unwrap();let row=div().w_full().h(px(100.)).px_3().py_2().border_b_1().border_color(cx.theme().border).flex().flex_col().gap_1();
    if this.rules_tab {let rule=&review.rules[index];row.child(div().flex().justify_between().child(div().flex_1().truncate().child(rule.name.clone())).child(Button::new(("copy-source-json",index)).label(crate::i18n::tr("Copy full JSON")).disabled(this.busy).on_click(cx.listener(move|this,_,window,cx|this.copy_rule(index,window,cx)))))
     .child(format!("{} · {} · {}",rule.action,rule.duration,if rule.enabled{"enabled"}else{"disabled"})).child(div().text_sm().truncate().child(rule.operator.as_ref().map(|op|format!("{} · {} · {}",op.r#type,op.operand,op.data)).unwrap_or_default()))
    }else{let target=&review.targets[index];let progress=this.progress.as_ref().map(|p|&p.targets[index]);row.child(target.destination.name.clone()).child(crate::i18n::tr_args("%{arg0} to apply · %{arg1} conflicts skipped · %{arg2} identical · %{arg3} awaiting · %{arg4} original removals", &[("arg0", &(target.copies).to_string()), ("arg1", &(target.existing).to_string()), ("arg2", &(target.identical).to_string()), ("arg3", &(target.awaiting).to_string()), ("arg4", &(target.rename_removals).to_string())]))
     .child(div().text_sm().truncate().child(if let Some(reason)=&target.blocked{crate::i18n::tr_args("Blocked: %{reason}", &[("reason", &reason.to_string())])}else if let Some(p)=progress{crate::i18n::tr_args("%{arg0} · %{arg1} confirmed · %{arg2} skipped · %{arg3} originals removed · %{arg4}/%{arg5} processed", &[("arg0", &crate::i18n::tr(&p.status.label()).to_string()), ("arg1", &format!("{}", p.confirmed)), ("arg2", &format!("{}", p.skipped)), ("arg3", &format!("{}", p.removed)), ("arg4", &format!("{}", p.next)), ("arg5", &format!("{}", review.rules.len()))])}else{crate::i18n::tr("Ready")}))}
   }).collect::<Vec<_>>())).w_full().flex_1().min_h_0();
            body = body.child(rows);
        } else {
            body = body.child(crate::i18n::presentation(&self.source_label()));
            if matches!(self.source, Source::Import) {
                body = body
                    .child(Input::new(&self.path).id("copy-import-path").disabled(busy))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("copy-import-browse")
                                    .label(crate::i18n::tr("Choose file / directory…"))
                                    .disabled(busy)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.browse(window, cx)),
                                    ),
                            )
                            .child(
                                Button::new("copy-import-clipboard")
                                    .label(crate::i18n::tr("Capture clipboard"))
                                    .disabled(busy)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.capture_clipboard(cx)),
                                    ),
                            )
                            .child(
                                Button::new("copy-import-use-path")
                                    .label(crate::i18n::tr("Use file path"))
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.clipboard = None;
                                        this.import_invalid = false;
                                        this.error = None;
                                        this.message.clear();
                                        cx.notify();
                                    })),
                            ),
                    );
            }
            if let Source::Catalogue(scope) = &self.source {
                body = body.child(
                    Checkbox::new("copy-all-rules")
                        .label(crate::i18n::tr(
                            "Copy every rule matching the captured filter, across all pages",
                        ))
                        .checked(self.all)
                        .disabled(busy || scope.names.is_none())
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.all = *value;
                            cx.notify();
                        })),
                );
            }
            body=body
    .child(Checkbox::new("copy-replace").label(crate::i18n::tr("Replace conflicting confirmed names on the destinations")).checked(self.replace).disabled(busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.replace = *value;cx.notify();})))
    .child(div().flex().gap_2().child(Button::new("copy-refresh-nodes").label(crate::i18n::tr("Refresh destinations")).disabled(busy).on_click(cx.listener(|this,_,_,cx|{this.refresh();cx.notify();}))).child(Button::new("copy-select-all").label(crate::i18n::tr("Select all destinations")).disabled(busy).on_click(cx.listener(|this,_,_,cx|{this.selected=this.nodes.iter().map(|d|d.session).collect();cx.notify();}))))
    .child(crate::i18n::tr_args("%{arg0} selected destinations. Rule values are copied exactly; review paths and user IDs for each destination.", &[("arg0", &(self.selected.len()).to_string())]));
            let rows = uniform_list(
                "copy-destinations",
                self.nodes.len(),
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|index| {
                            let dest = &this.nodes[index];
                            let id = dest.session;
                            div().w_full().h(px(52.)).px_3().child(
                                Checkbox::new(("copy-node", index))
                                    .label(dest.name.clone())
                                    .checked(this.selected.contains(&id))
                                    .disabled(this.busy)
                                    .on_change(cx.listener(move |this, value: &bool, _, cx| {
                                        if *value {
                                            this.selected.insert(id);
                                        } else {
                                            this.selected.remove(&id);
                                        }
                                        cx.notify();
                                    })),
                            )
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .w_full()
            .flex_1()
            .min_h_0();
            body = body
                .when(self.nodes.is_empty(), |v| {
                    v.child(crate::i18n::tr(
                        "Connect another node to choose a copy destination.",
                    ))
                })
                .child(rows);
        }
        let resumable = self
            .progress
            .as_ref()
            .is_some_and(|p| p.targets.iter().any(|p| p.status != Status::Complete));
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(border)
                    .text_lg()
                    .child(match self.source {
                        Source::Catalogue(_) => crate::i18n::tr("Copy application rules to nodes"),
                        Source::Draft { .. } => crate::i18n::tr("Apply rule draft to nodes"),
                        Source::Import => crate::i18n::tr("Import application rules to nodes"),
                    }),
            )
            .when_some(self.error.clone(), |v, error| {
                v.child(super::banner(error, cx))
            })
            .child(
                div()
                    .px_5()
                    .py_2()
                    .text_sm()
                    .child(crate::i18n::presentation(&self.message)),
            )
            .child(body)
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_t_1()
                    .border_color(border)
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .when(self.applying, |v| {
                        v.child(
                            Button::new("copy-stop")
                                .label(crate::i18n::tr("Stop after current reply"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.stop.store(true, Ordering::Release);
                                    cx.notify();
                                })),
                        )
                    })
                    .when(self.review.is_some(), |v| {
                        v.child(
                            Button::new("copy-back")
                                .label(crate::i18n::tr("Change destinations or rebuild"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.review = None;
                                    this.progress = None;
                                    this.error = None;
                                    this.message.clear();
                                    cx.notify();
                                })),
                        )
                    })
                    .when(self.review.is_some(), |v| {
                        v.child(
                            Button::new("copy-results")
                                .label(crate::i18n::tr("Copy results"))
                                .on_click(cx.listener(|this, _, _, cx| this.copy_results(cx))),
                        )
                    })
                    .child(
                        Button::new("copy-close")
                            .label(crate::i18n::tr("Close"))
                            .disabled(busy)
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    )
                    .when(self.review.is_none(), |v| {
                        v.child(
                            Button::new("copy-review-button")
                                .label(crate::i18n::tr("Review copy"))
                                .disabled(busy || self.selected.is_empty() || self.import_invalid)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.prepare(window, cx)),
                                ),
                        )
                    })
                    .when(self.review.is_some(), |v| {
                        v.child(
                            Button::new("copy-apply")
                                .label(
                                    if self.progress.as_ref().is_some_and(|p| {
                                        p.targets.iter().any(|p| p.status != Status::Ready)
                                    }) {
                                        crate::i18n::tr("Continue unfinished copies")
                                    } else {
                                        crate::i18n::tr("Copy reviewed rules")
                                    },
                                )
                                .disabled(busy || !resumable)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.apply(window, cx)),
                                ),
                        )
                    }),
            )
    }
}
