//! Native import/export review. Disk/JSON work runs on GPUI's background pool;
//! daemon mutations remain sequential, acknowledged and independently stoppable.
use crate::{
    backend::Handle,
    protocol::{Action, Rule},
    rules::transfer::{self, ExportScope, Import},
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputState},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Mode {
    Import,
    Export,
    Bulk {
        action: Action,
        label: &'static str,
        decision: Option<&'static str>,
    },
}

pub(super) fn open(
    backend: Handle,
    session: u64,
    scope: ExportScope,
    mode: Mode,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    open_with_rules(backend, session, scope, mode, Vec::new(), cx)
}
pub(super) fn open_batch(
    backend: Handle,
    session: u64,
    scope: ExportScope,
    rules: Vec<Rule>,
    mode: Mode,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    open_with_rules(backend, session, scope, mode, rules, cx)
}
fn open_with_rules(
    backend: Handle,
    session: u64,
    scope: ExportScope,
    mode: Mode,
    rules: Vec<Rule>,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(900.), px(700.)),
                cx,
            ))),
            window_min_size: Some(size(px(720.), px(560.))),
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Rustleblower — rule transfer").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| {
            let view =
                cx.new(|cx| RuleTransfer::new(backend, session, scope, mode, rules, window, cx));
            if matches!(mode, Mode::Bulk { .. }) {
                view.update(cx, |view, cx| view.load_bulk_scope(false, window, cx));
            }
            view
        },
    )?;
    crate::i18n::register_title(window, "Rustleblower — rule transfer", cx);
    Ok(window)
}

struct RuleTransfer {
    backend: Handle,
    session: u64,
    scope: ExportScope,
    mode: Mode,
    path: Entity<InputState>,
    rules: Vec<Rule>,
    existing: BTreeSet<String>,
    replace: bool,
    bulk_all: bool,
    busy: bool,
    applying: bool,
    message: String,
    error: Option<String>,
    stop: Arc<AtomicBool>,
    focus: FocusHandle,
}
#[cfg(all(test, feature = "headless-tests"))]
pub(super) fn test_idle(window: &Window, cx: &App) -> bool {
    let view = gpui_kit::base::Root::read(window, cx)
        .view()
        .clone()
        .downcast::<RuleTransfer>()
        .unwrap();
    !view.read(cx).busy
}
impl RuleTransfer {
    fn new(
        backend: Handle,
        session: u64,
        scope: ExportScope,
        mode: Mode,
        rules: Vec<Rule>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.on_window_should_close(cx, |window, _| {
            window.minimize_window();
            false
        });
        let path = cx.new(|cx| {
            InputState::new(window, cx).placeholder(if mode == Mode::Import {
                "JSON file or directory"
            } else {
                "JSON file path, or directory for a folder export"
            })
        });
        path.focus_handle(cx).focus(window, cx);
        Self {
            backend,
            session,
            scope,
            mode,
            path,
            message: if matches!(mode, Mode::Bulk { .. }) {
                crate::i18n::tr_args(
                    "Review %{arg0} selected rules. Changes are confirmed one rule at a time.",
                    &[("arg0", &format!("{}", rules.len()))],
                )
            } else {
                String::new()
            },
            rules,
            existing: BTreeSet::new(),
            replace: false,
            bulk_all: false,
            busy: false,
            applying: false,
            error: None,
            stop: Arc::new(AtomicBool::new(false)),
            focus: cx.focus_handle(),
        }
    }
    fn load_bulk_scope(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Mode::Bulk {
            action, decision, ..
        } = self.mode
        else {
            return;
        };
        self.bulk_all = all;
        self.rules.clear();
        self.busy = true;
        self.error = None;
        self.message = "Reading a consistent rule snapshot for review…".into();
        let database = self.backend.history.database_path().to_owned();
        let mut scope = self.scope.clone();
        if all {
            scope.names = None;
        }
        let task = cx.background_executor().spawn(async move {
            let mut rules = transfer::load_rules(&database, &scope)?;
            for rule in &mut rules {
                if let Some(decision) = decision {
                    rule.action = decision.into();
                }
                if action == Action::EnableRule {
                    rule.enabled = true;
                }
                if action == Action::DisableRule {
                    rule.enabled = false;
                }
            }
            Ok::<_, anyhow::Error>(rules)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(rules) => {
                        this.message = crate::i18n::tr_args(
                            "Review %{arg0} rules · %{arg1}",
                            &[
                                ("arg0", &format!("{}", rules.len())),
                                (
                                    "arg1",
                                    (if all {
                                        "every page matching the filter"
                                    } else {
                                        "selected rules"
                                    }),
                                ),
                            ],
                        );
                        this.rules = rules;
                    }
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        this.message = "Rule review could not be loaded".into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn browse(&mut self, directory: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        if self.mode == Mode::Export && !directory {
            let prompt = cx.prompt_for_new_path(Path::new("."), Some("rustleblower-rules.json"));
            cx.spawn_in(window, async move |this, cx| {
                let response = prompt.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.busy = false;
                    match response {
                        Ok(Ok(Some(path))) => this.path.update(cx, |input, cx| {
                            input.set_value(path.to_string_lossy().to_string(), window, cx)
                        }),
                        Ok(Ok(None)) => {}
                        other => {
                            this.error = Some(crate::i18n::tr_args(
                                "Cannot choose an export path: %{other}",
                                &[("other", &format!("{:?}", other))],
                            ))
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        } else {
            let prompt = cx.prompt_for_paths(PathPromptOptions {
                files: !directory,
                directories: directory,
                multiple: false,
                prompt: Some("Choose a rule transfer path".into()),
            });
            cx.spawn_in(window, async move |this, cx| {
                let response = prompt.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.busy = false;
                    match response {
                        Ok(Ok(Some(paths))) => {
                            if let Some(path) = paths.first() {
                                this.path.update(cx, |input, cx| {
                                    input.set_value(path.to_string_lossy().to_string(), window, cx)
                                });
                            }
                        }
                        Ok(Ok(None)) => {}
                        other => {
                            this.error = Some(crate::i18n::tr_args(
                                "Cannot choose a rule transfer path: %{other}",
                                &[("other", &format!("{:?}", other))],
                            ))
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }
    fn load(&mut self, clipboard: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let text = if clipboard {
            match cx.read_from_clipboard().and_then(|item| item.text()) {
                Some(text) => Some(text),
                None => {
                    self.error = Some(crate::i18n::tr("The clipboard has no rule JSON text"));
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let path = PathBuf::from(self.path.read(cx).value().to_string());
        let database = self.backend.history.database_path().to_owned();
        let node = self.scope.query.node.clone();
        self.busy = true;
        self.rules.clear();
        self.error = None;
        self.message = "Reading and validating rule JSON…".into();
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            let imported = match text {
                Some(text) => Import::from_text(&text),
                None => Import::from_path(&path),
            }?;
            let names = transfer::existing_names(&database, &node)?;
            Ok::<_, anyhow::Error>((imported, names))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok((imported, names)) => {
                        this.message = crate::i18n::tr_args(
                            "%{arg0} rules ready · %{arg1} KiB JSON · %{arg2} existing names",
                            &[
                                ("arg0", &format!("{}", imported.rules.len())),
                                ("arg1", &format!("{}", imported.bytes.div_ceil(1024))),
                                (
                                    "arg2",
                                    &format!(
                                        "{}",
                                        imported
                                            .rules
                                            .iter()
                                            .filter(|rule| names.contains(&rule.name))
                                            .count()
                                    ),
                                ),
                            ],
                        );
                        this.rules = imported.rules;
                        this.existing = names;
                    }
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        this.message.clear();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.rules.is_empty() {
            return;
        }
        let mut rules = std::mem::take(&mut self.rules);
        let before = rules.len();
        if !self.replace {
            rules.retain(|rule| !self.existing.contains(&rule.name));
        }
        let skipped = before - rules.len();
        let total = rules.len();
        self.stop.store(false, Ordering::Release);
        self.busy = true;
        self.applying = true;
        self.error = None;
        self.message = crate::i18n::tr_args(
            "Waiting for confirmation · 0/%{total} · %{skipped} existing names skipped",
            &[
                ("total", &format!("{}", total)),
                ("skipped", &format!("{}", skipped)),
            ],
        );
        cx.notify();
        let stop = self.stop.clone();
        let control = self.backend.control.clone();
        let session = self.session;
        let action = match self.mode {
            Mode::Bulk { action, .. } => action,
            _ => Action::ChangeRule,
        };
        cx.spawn_in(window, async move |this, cx| {
            let mut remaining = std::collections::VecDeque::from(rules);
            let mut confirmed = 0;
            let mut error = None;
            while !stop.load(Ordering::Acquire) {
                let Some(rule) = remaining.pop_front() else {
                    break;
                };
                let result = match control.submit(session, action, rule.clone()) {
                    Ok(reply) => reply
                        .await
                        .unwrap_or_else(|_| Err("Rule command stopped before confirmation".into())),
                    Err(e) => Err(e),
                };
                if let Err(e) = result {
                    remaining.push_front(rule);
                    error = Some(e);
                    break;
                }
                confirmed += 1;
                if confirmed % 10 == 0 || confirmed == total {
                    let _ = this.update_in(cx, |this, _, cx| {
                        this.message =
                            crate::i18n::tr_args("Daemon confirmed %{confirmed}/%{total} · %{skipped} skipped", &[("confirmed", &format!("{}", confirmed)), ("total", &format!("{}", total)), ("skipped", &format!("{}", skipped))]);
                        cx.notify();
                    });
                }
            }
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                this.applying = false;
                this.rules = remaining.into();
                this.message = crate::i18n::tr_args("Daemon confirmed %{confirmed}/%{total} · %{skipped} skipped · %{arg0} remaining", &[("confirmed", &format!("{}", confirmed)), ("total", &format!("{}", total)), ("skipped", &format!("{}", skipped)), ("arg0", &format!("{}", this.rules.len()))]);
                this.error = error;
                if stop.load(Ordering::Acquire) && !this.rules.is_empty() {
                    this.message.push_str(" · stopped after the current reply");
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn export(&mut self, target: u8, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let database = self.backend.history.database_path().to_owned();
        let scope = self.scope.clone();
        let path = PathBuf::from(self.path.read(cx).value().to_string());
        self.busy = true;
        self.error = None;
        self.message = "Exporting a consistent rule snapshot…".into();
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            match target {
                0 => transfer::export_text(&database, &scope).map(|(text, count)| {
                    (
                        Some(text),
                        crate::i18n::tr_args(
                            "Copied %{count} rules to the clipboard",
                            &[("count", &format!("{}", count))],
                        ),
                    )
                }),
                1 => transfer::export_file(&database, &scope, &path).map(|count| {
                    (
                        None,
                        crate::i18n::tr_args(
                            "Exported %{count} rules to %{arg0}",
                            &[
                                ("count", &format!("{}", count)),
                                ("arg0", &format!("{}", path.display())),
                            ],
                        ),
                    )
                }),
                _ => transfer::export_directory(&database, &scope, &path).map(|(path, count)| {
                    (
                        None,
                        crate::i18n::tr_args(
                            "Exported %{count} rules to %{arg0}",
                            &[
                                ("count", &format!("{}", count)),
                                ("arg0", &format!("{}", path.display())),
                            ],
                        ),
                    )
                }),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok((text, message)) => {
                        if let Some(text) = text {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                        this.message = message;
                    }
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        this.message = "Export did not complete".into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
impl Render for RuleTransfer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let import = self.mode == Mode::Import;
        let review = self.mode != Mode::Export;
        let bulk = matches!(self.mode, Mode::Bulk { .. });
        let busy = self.busy;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let rows = uniform_list(
            "transfer-rules",
            self.rules.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| {
                        let rule = &this.rules[index];
                        div()
                            .w_full()
                            .h(px(52.))
                            .px_4()
                            .py_2()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .flex()
                            .items_center()
                            .gap_4()
                            .justify_between()
                            .child(div().flex_1().min_w_0().truncate().child(rule.name.clone()))
                            .child(format!(
                                "{} · {}{}",
                                rule.action,
                                rule.duration,
                                if this.existing.contains(&rule.name) {
                                    " · existing"
                                } else {
                                    ""
                                }
                            ))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .w_full()
        .flex_1()
        .min_h_0();
        let controls = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new("transfer-browse-file")
                    .label(if import {
                        crate::i18n::tr("Choose file…")
                    } else {
                        crate::i18n::tr("Choose JSON destination…")
                    })
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.browse(false, window, cx))),
            )
            .child(
                Button::new("transfer-browse-directory")
                    .label(crate::i18n::tr("Choose directory…"))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.browse(true, window, cx))),
            )
            .when(import, |v| {
                v.child(
                    Button::new("transfer-read")
                        .label(crate::i18n::tr("Read path"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.load(false, window, cx))),
                )
                .child(
                    Button::new("transfer-paste")
                        .label(crate::i18n::tr("Read clipboard"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.load(true, window, cx))),
                )
            })
            .when(!import, |v| {
                v.child(
                    Button::new("transfer-copy")
                        .label(crate::i18n::tr("Copy JSON"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.export(0, window, cx))),
                )
                .child(
                    Button::new("transfer-export-file")
                        .label(crate::i18n::tr("Save JSON file"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.export(1, window, cx))),
                )
                .child(
                    Button::new("transfer-export-directory")
                        .label(crate::i18n::tr("Export rule folder"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.export(2, window, cx))),
                )
            });
        div().size_full().flex().flex_col().track_focus(&self.focus).bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(div().px_5().py_3().border_b_1().border_color(border).text_lg().child(if let Mode::Bulk{label,..}=self.mode {crate::i18n::tr(label)} else if import{crate::i18n::tr("Import application rules")}else{crate::i18n::tr("Export application rules")}))
            .child(div().p_5().flex().flex_col().gap_3()
                .child(crate::i18n::tr_args("Node: %{arg0}", &[("arg0", &(self.scope.query.node).to_string())]))
                .when(!review,|v|v.child(if let Some(names)=&self.scope.names{crate::i18n::tr_args("%{arg0} selected rules", &[("arg0", &format!("{}", names.len()))])}else{crate::i18n::tr("All rules matching the current filter, across every page")}))
                .when(!bulk,|v|v.child(Input::new(&self.path).id("transfer-path").disabled(busy)).child(controls))
                .child(div().text_sm().text_color(muted).child(if bulk {crate::i18n::tr("Apply affects the reviewed rules. Completed changes remain applied if you stop or a later rule fails.")} else if import{crate::i18n::tr("Read a JSON file, an OpenSnitch rule directory, or clipboard JSON. Review the rules before applying them.")}else{crate::i18n::tr("JSON files and folders preserve OpenSnitch rule fields. Folder export creates a new child directory.")}))
                .when(import,|v|v.child(Checkbox::new("transfer-replace").label(crate::i18n::tr("Replace rules that have the same names")).checked(self.replace).disabled(busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.replace = *value;cx.notify();}))))
                .when(bulk,|v|v.child(Checkbox::new("transfer-all-matching").label(crate::i18n::tr("Use every rule matching the current filter, across all pages")).checked(self.bulk_all).disabled(busy).on_change(cx.listener(|this,value:&bool,window,cx|this.load_bulk_scope(*value,window,cx))))))
            .when_some(self.error.clone(),|v,error|v.child(super::banner(error,cx)))
            .child(div().px_5().py_2().text_sm().child(crate::i18n::presentation(&self.message)))
            .when(review,|v|v.child(rows))
            .when(!review,|v|v.child(div().flex_1()))
            .child(div().px_5().py_3().border_t_1().border_color(border).flex().justify_end().gap_2()
                .when(review&&self.applying,|v|v.child(Button::new("transfer-stop").label(crate::i18n::tr("Stop after current rule")).on_click(cx.listener(|this,_,_,cx|{this.stop.store(true,Ordering::Release);cx.notify();}))))
                .child(Button::new("transfer-close").label(crate::i18n::tr("Close")).disabled(busy).on_click(cx.listener(|_,_,window,_|window.remove_window())))
                .when(review,|v|v.child(Button::new("transfer-apply").label(if matches!(self.mode,Mode::Bulk{action:Action::DeleteRule,..}){crate::i18n::tr("Delete reviewed rules")}else{crate::i18n::tr("Apply reviewed rules")}).disabled(busy||self.rules.is_empty()).on_click(cx.listener(|this,_,window,cx|this.apply(window,cx))))))
    }
}
