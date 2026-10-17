//! Multi-node export setup/review; rule bodies stay in the worker's disk snapshot.
use crate::{
    backend::Handle,
    rules::{
        Query,
        export_nodes::{self, Review},
    },
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
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
pub(super) fn open(backend: Handle, filter: Query, cx: &mut App) -> gpui::Result<AnyWindowHandle> {
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(940.), px(720.)),
                cx,
            ))),
            window_min_size: Some(size(px(720.), px(560.))),
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Rustleblower — export nodes").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| {
            cx.new(|cx| {
                let mut view = Exports::new(backend, filter, window, cx);
                view.refresh(window, cx);
                view
            })
        },
    )?;
    crate::i18n::register_title(window, "Rustleblower — export nodes", cx);
    Ok(window)
}
pub(super) struct Exports {
    backend: Handle,
    filter: Query,
    filtered: bool,
    nodes: Vec<String>,
    selected: BTreeSet<String>,
    path: Entity<InputState>,
    review: Option<Arc<Review>>,
    busy: bool,
    error: Option<String>,
    message: String,
    stop: Arc<AtomicBool>,
    focus: FocusHandle,
}
impl Drop for Exports {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Exports {
    fn new(backend: Handle, filter: Query, window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.on_window_should_close(cx, |window, _| {
            window.minimize_window();
            false
        });
        let path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "Parent directory for a new export folder",
                    window,
                    cx,
                ))
                .validate(|v, _| v.len() <= 4096)
        });
        path.focus_handle(cx).focus(window, cx);
        Self {
            backend,
            selected: BTreeSet::from([filter.node.clone()]),
            filter,
            filtered: false,
            nodes: Vec::new(),
            path,
            review: None,
            busy: false,
            error: None,
            message: String::new(),
            stop: Arc::new(AtomicBool::new(false)),
            focus: cx.focus_handle(),
        }
    }
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let db = self.backend.history.database_path().to_owned();
        let task = cx
            .background_executor()
            .spawn(async move { export_nodes::catalogue(&db) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(nodes) => {
                        this.selected.retain(|n| nodes.contains(n));
                        this.nodes = nodes;
                    }
                    Err(e) => this.error = Some(format!("{e:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.message = "Capturing selected cached rules on disk…".into();
        self.stop.store(false, Ordering::Release);
        let names = self.selected.iter().cloned().collect::<Vec<_>>();
        let filter = if self.filtered {
            self.filter.clone()
        } else {
            Query {
                policy: self.filter.policy,
                ..Default::default()
            }
        };
        let db = self.backend.history.database_path().to_owned();
        let stop = self.stop.clone();
        let task = cx
            .background_executor()
            .spawn(async move { export_nodes::capture(&db, &names, &filter, &stop) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(review) => {
                        this.message = crate::i18n::tr_args("%{arg0} rules captured for %{arg1} nodes. Review the counts before exporting.", &[("arg0", &format!("{}", review.rules)), ("arg1", &format!("{}", review.nodes.len()))]);
                        this.review = Some(Arc::new(review));
                    }
                    Err(e) => {
                        this.message.clear();
                        this.error = Some(format!("{e:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn publish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(review) = self.review.clone() else {
            return;
        };
        let parent = PathBuf::from(self.path.read(cx).value().to_string());
        self.busy = true;
        self.error = None;
        self.message = "Writing the reviewed export…".into();
        self.stop.store(false, Ordering::Release);
        let stop = self.stop.clone();
        let task = cx
            .background_executor()
            .spawn(async move { export_nodes::publish(&review, &parent, &stop) });
        cx.spawn_in(window,async move|this,cx|{let result=task.await;let _=this.update_in(cx,|this,_,cx|{this.busy=false;match result{Ok(path)=>this.message=crate::i18n::tr_args("Export complete: %{arg0}. Each node folder contains importable OpenSnitch JSON rules.", &[("arg0", &format!("{}", path.display()))]),Err(e)=>{this.message="Export did not complete".into();this.error=Some(format!("{e:#}"));}}cx.notify();});}).detach();
        cx.notify();
    }
    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose the parent export directory".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = prompt.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            this.path.update(cx, |i, cx| {
                                i.set_value(path.to_string_lossy().to_string(), window, cx)
                            });
                        }
                    }
                    Ok(Ok(None)) => {}
                    other => {
                        this.error = Some(crate::i18n::tr_args(
                            "Cannot choose export directory: %{other}",
                            &[("other", &format!("{:?}", other))],
                        ))
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn test_state(&self) -> (bool, usize) {
        (self.busy, self.review.as_ref().map_or(0, |r| r.rules))
    }
}
impl Render for Exports {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.busy;
        let border = cx.theme().border;
        let mut body = div().flex_1().min_h_0().px_5().flex().flex_col().gap_3();
        body = body
            .child(
                Input::new(&self.path)
                    .id("nodes-export-path")
                    .disabled(busy),
            )
            .child(
                Button::new("nodes-export-browse")
                    .label(crate::i18n::tr("Choose parent directory…"))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.browse(window, cx))),
            );
        if let Some(review) = &self.review {
            body=body.child(crate::i18n::tr_args("%{arg0} rules · %{arg1} nodes · %{arg2} KiB of JSON. The captured contents stay unchanged if nodes reconnect or their rules change.", &[("arg0", &(review.rules).to_string()), ("arg1", &(review.nodes.len()).to_string()), ("arg2", &(review.bytes.div_ceil(1024)).to_string())]))
        .child(crate::i18n::tr("A new folder contains one subfolder per node plus manifest.json. Import a node subfolder to restore its rules. Proposals awaiting observation remain unconfirmed."))
        .child(uniform_list("nodes-export-review",review.nodes.len(),cx.processor(|this,range:std::ops::Range<usize>,_,_|range.map(|i|{let n=&this.review.as_ref().unwrap().nodes[i];div().w_full().h(px(84.)).flex().flex_col().gap_1().child(n.name.clone()).child(crate::i18n::tr_args("%{arg0} rules · %{arg1} awaiting observation · %{arg2} KiB", &[("arg0", &(n.rules).to_string()), ("arg1", &(n.awaiting).to_string()), ("arg2", &(n.bytes.div_ceil(1024)).to_string())])).child(div().text_sm().truncate().child(n.directory.clone()))}).collect::<Vec<_>>())).flex_1().min_h_0());
        } else {
            body=body.child(crate::i18n::tr("Choose cached nodes, including offline nodes. All rules are included unless you select the captured filter below."))
        .child(Checkbox::new("nodes-export-filter").label(crate::i18n::tr_args("Use captured filter on every node: search %{arg0}, %{arg1}", &[("arg0", &format!("{:?}", self.filter.search)), ("arg1", (match self.filter.temporary{Some(true)=>"temporary",Some(false)=>"permanent",None=>"all durations"}))])).checked(self.filtered).disabled(busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.filtered = *value;cx.notify();})))
        .child(div().flex().gap_2().child(Button::new("nodes-export-refresh").label(crate::i18n::tr("Refresh cached nodes")).disabled(busy).on_click(cx.listener(|this,_,window,cx|this.refresh(window,cx)))).child(Button::new("nodes-export-all").label(crate::i18n::tr("Select all nodes")).disabled(busy).on_click(cx.listener(|this,_,_,cx|{this.selected=this.nodes.iter().cloned().collect();cx.notify();}))))
        .child(crate::i18n::tr_args("%{arg0} selected nodes", &[("arg0", &(self.selected.len()).to_string())]))
        .child(uniform_list("nodes-export-choices",self.nodes.len(),cx.processor(|this,range:std::ops::Range<usize>,_,cx|range.map(|i|{let name=this.nodes[i].clone();div().w_full().h(px(52.)).child(Checkbox::new(("nodes-export-node",i)).label(name.clone()).checked(this.selected.contains(&name)).disabled(this.busy).on_change(cx.listener(move|this,value:&bool,_,cx|{if *value{this.selected.insert(name.clone());}else{this.selected.remove(&name);}cx.notify();})))}).collect::<Vec<_>>())).flex_1().min_h_0());
        }
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
                    .child(crate::i18n::tr("Export rules from nodes")),
            )
            .when_some(self.error.clone(), |v, e| v.child(super::banner(e, cx)))
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
                    .child(
                        Button::new("nodes-export-close")
                            .label(crate::i18n::tr("Cancel / close"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.stop.store(true, Ordering::Release);
                                window.remove_window();
                                cx.notify();
                            })),
                    )
                    .when(busy, |v| {
                        v.child(
                            Button::new("nodes-export-stop")
                                .label(crate::i18n::tr("Cancel work"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.stop.store(true, Ordering::Release);
                                    cx.notify();
                                })),
                        )
                    })
                    .when(self.review.is_some(), |v| {
                        v.child(
                            Button::new("nodes-export-back")
                                .label(crate::i18n::tr("Change nodes / rebuild"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.review = None;
                                    this.error = None;
                                    this.message.clear();
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new("nodes-export-apply")
                            .label(if self.review.is_some() {
                                crate::i18n::tr("Export reviewed folders")
                            } else {
                                crate::i18n::tr("Review export")
                            })
                            .disabled(busy || (self.review.is_none() && self.selected.is_empty()))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.review.is_some() {
                                    this.publish(window, cx)
                                } else {
                                    this.capture(window, cx)
                                }
                            })),
                    ),
            )
    }
}
