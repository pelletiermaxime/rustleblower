//! Native history filters, column settings and background export.
use crate::history::{
    self, History, Query,
    export::Format,
    query::{COMPARISONS, Comparison, FIELDS, Field, Filter},
    settings::Options,
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectState},
    tab::{Tab, TabBar},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
};
pub(super) enum Message {
    Filters(Vec<Filter>),
    Sort(Option<(Field, bool)>),
}
type Picker = Entity<SelectState<SearchableVec<SharedString>>>;
pub(super) fn open(
    history: History,
    query: Query,
    sender: SyncSender<Message>,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(900.), px(760.)),
                cx,
            ))),
            window_min_size: Some(size(px(720.), px(600.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Rustleblower — history tools".into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| Tools::new(history, query, sender, window, cx)),
    )?;
    Ok(handle)
}
struct Tools {
    history: History,
    query: Query,
    sender: SyncSender<Message>,
    options: Options,
    field: Picker,
    comparison: Picker,
    value: Entity<InputState>,
    path: Entity<InputState>,
    format: Picker,
    tab: usize,
    busy: bool,
    exporting: bool,
    replace: bool,
    stop: Arc<AtomicBool>,
    message: Option<String>,
    error: Option<String>,
    focus: FocusHandle,
}
impl Tools {
    fn new(
        history: History,
        query: Query,
        sender: SyncSender<Message>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let picker = |labels: Vec<SharedString>, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(labels),
                    Some(IndexPath::new(0)),
                    window,
                    cx,
                )
                .searchable(true)
            })
        };
        let field = picker(
            FIELDS.iter().map(|field| field.label().into()).collect(),
            window,
            cx,
        );
        let comparison = picker(
            COMPARISONS
                .iter()
                .map(|(_, label)| (*label).into())
                .collect(),
            window,
            cx,
        );
        let format = picker(vec!["CSV".into(), "JSON Lines".into()], window, cx);
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Value; time accepts RFC3339 UTC")
                .validate(|value, _| value.len() <= 256)
        });
        let path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/absolute/path/history.csv")
                .validate(|value, _| value.len() <= 4096)
        });
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let busy = weak.upgrade().is_some_and(|view| view.read(cx).busy);
            if busy {
                window.minimize_window();
            }
            !busy
        });
        if history.health.indexing.load(Ordering::Acquire) {
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(250))
                        .await;
                    let again = this
                        .update_in(cx, |this, _, cx| {
                            let again = this.history.health.indexing.load(Ordering::Acquire);
                            if !again {
                                cx.notify();
                            }
                            again
                        })
                        .unwrap_or(false);
                    if !again {
                        break;
                    }
                }
            })
            .detach();
        }
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let options = history.settings.snapshot();
        Self {
            history,
            query,
            sender,
            options,
            field,
            comparison,
            value,
            path,
            format,
            tab: 0,
            busy: false,
            exporting: false,
            replace: false,
            stop: Arc::new(AtomicBool::new(false)),
            message: None,
            error: None,
            focus,
        }
    }
    fn add(&mut self, cx: &mut Context<Self>) {
        let field = self
            .field
            .read(cx)
            .selected_value()
            .and_then(|label| {
                FIELDS
                    .iter()
                    .copied()
                    .find(|field| field.label() == label.as_ref())
            })
            .unwrap_or(Field::Process);
        let comparison = self
            .comparison
            .read(cx)
            .selected_value()
            .and_then(|label| {
                COMPARISONS
                    .iter()
                    .find(|(_, text)| *text == label.as_ref())
                    .map(|(op, _)| *op)
            })
            .unwrap_or(Comparison::Contains);
        let filter = Filter {
            field,
            comparison,
            value: self.value.read(cx).value().to_string(),
        };
        if self.query.filters.len() >= 8 {
            self.error = Some("Use at most eight conditions".into());
        } else if let Err(error) = filter.validate() {
            self.error = Some(error.to_string());
        } else {
            self.query.filters.push(filter);
            self.error = None;
        }
        cx.notify();
    }
    fn save_columns(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.options.validate() {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.busy = true;
        let store = self.history.settings.clone();
        let options = self.options.clone();
        let task = cx.background_executor().spawn(async move {
            store
                .save_columns(options)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.error = None;
                        this.message = Some(
                            "History view saved. Applies immediately and after restart.".into(),
                        );
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        let prompt = cx.prompt_for_new_path(Path::new("."), Some("rustleblower-history.csv"));
        cx.spawn_in(window, async move |this, cx| {
            let response = prompt.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match response {
                    Ok(Ok(Some(path))) => this.path.update(cx, |input, cx| {
                        input.set_value(path.to_string_lossy().to_string(), window, cx)
                    }),
                    Ok(Ok(None)) => {}
                    other => this.error = Some(format!("Cannot choose an export path: {other:?}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(reply) = self.history.query(self.query.clone()) else {
            self.error = Some("History is busy; try copying again".into());
            cx.notify();
            return;
        };
        let columns = self.options.columns.clone();
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = match reply.await {
                Ok(Ok(page)) => {
                    cx.background_executor()
                        .spawn(async move {
                            history::export::page_csv(&page.rows, &columns)
                                .map_err(|error| error.to_string())
                        })
                        .await
                }
                Ok(Err(error)) => Err(error),
                Err(_) => Err("History worker stopped".into()),
            };
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(text) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        this.error = None;
                        this.message = Some("Matching page copied as CSV.".into());
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = PathBuf::from(self.path.read(cx).value().as_ref());
        let database = self.history.database.clone();
        let query = self.query.clone();
        let columns = self.options.columns.clone();
        let replace = self.replace;
        let format = if self
            .format
            .read(cx)
            .selected_value()
            .is_some_and(|label| label.as_ref() == "JSON Lines")
        {
            Format::JsonLines
        } else {
            Format::Csv
        };
        self.stop.store(false, Ordering::Relaxed);
        let stop = self.stop.clone();
        self.busy = true;
        self.exporting = true;
        self.error = None;
        self.message = Some("Creating a private history snapshot and exporting…".into());
        let task = cx.background_executor().spawn(async move {
            history::export::save(&database, &query, &columns, format, &target, replace, &stop)
                .map_err(|error| format!("{error:#}"))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                this.exporting = false;
                match result {
                    Ok(report) => {
                        this.message = Some(format!(
                            "Exported {} matching events ({} bytes).",
                            report.rows, report.bytes
                        ));
                        this.error = None;
                    }
                    Err(error) => {
                        this.message = None;
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
impl Render for Tools {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut panel = div()
            .id("history-tools-panel")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3();
        if self.tab == 0 {
            panel=panel.child(div().text_sm().child("Conditions combine with AND and apply alongside the main search/action filter. Numeric comparisons use integers; times accept RFC3339 UTC. Unknown older values stay unreported."))
                .child(div().h_8().flex_shrink_0().child(Select::new(&self.field).id("history-filter-field").w_full().disabled(self.busy)))
                .child(div().h_8().flex_shrink_0().child(Select::new(&self.comparison).id("history-filter-comparison").w_full().disabled(self.busy)))
                .child(div().h_8().flex_shrink_0().child(Input::new(&self.value).id("history-filter-value").disabled(self.busy)))
                .child(Button::new("history-filter-add").label("Add condition").disabled(self.busy || self.query.filters.len()>=8).on_click(cx.listener(|this,_,_,cx|this.add(cx))));
            for (index, filter) in self.query.filters.iter().enumerate() {
                panel = panel.child(
                    div()
                        .flex()
                        .gap_3()
                        .items_center()
                        .child(div().flex_1().text_sm().child(format!(
                                "{} · {} · {}",
                                filter.field.label(),
                                COMPARISONS
                                    .iter()
                                    .find(|(op, _)| *op == filter.comparison)
                                    .unwrap()
                                    .1,
                                filter.value
                            )))
                        .child(
                            Button::new(("history-filter-remove", index))
                                .label("Remove")
                                .disabled(self.busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.query.filters.remove(index);
                                    cx.notify();
                                })),
                        ),
                );
            }
            panel=panel.child(div().flex().gap_3().child(Button::new("history-filter-apply").label("Apply filters").disabled(self.busy || self.history.health.indexing.load(Ordering::Acquire)).on_click(cx.listener(|this,_,_,cx|{match this.sender.try_send(Message::Filters(this.query.filters.clone())){Ok(())=>{this.query.before=None;this.query.cursor=None;this.query.ceiling=None;this.error=None;this.message=Some("Filters applied to history. Export uses these captured conditions.".into());},Err(_)=>this.error=Some("The history view is busy or closed; try again".into())}cx.notify();})))
                .child(Button::new("history-filter-clear").label("Clear conditions").disabled(self.busy).on_click(cx.listener(|this,_,_,cx|{this.query.filters.clear();cx.notify();}))));
        } else if self.tab == 1 {
            panel=panel.child(Checkbox::new("history-table-mode").label("Use table view").checked(self.options.table).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.options.table= *value;cx.notify();})))
                .child(div().text_sm().child("Table headers sort matching history, not just this page. Columns scroll horizontally. Compact view keeps the two-line summary. Saved choices also select export columns."));
            for field in FIELDS {
                let field = *field;
                panel = panel.child(
                    Checkbox::new(SharedString::from(format!(
                        "history-column-{}",
                        field.key()
                    )))
                    .label(field.label())
                    .checked(self.options.columns.contains(&field))
                    .disabled(self.busy)
                    .on_change(cx.listener(
                        move |this, value: &bool, _, cx| {
                            if *value {
                                if !this.options.columns.contains(&field) {
                                    this.options.columns.push(field);
                                }
                            } else {
                                this.options.columns.retain(|candidate| *candidate != field);
                            }
                            cx.notify();
                        },
                    )),
                );
            }
            panel = panel.child(
                Button::new("history-columns-save")
                    .label("Save history view")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.save_columns(window, cx))),
            );
        } else {
            panel=panel.child(div().text_sm().child(format!("Captured search: {:?}; action: {:?}; {} conditions. File export includes all matching history, independently of the current page. Clipboard copies a matching page of at most 200 rows. Search, action, sort and page position were captured when this window opened; reopen it after changing the main view.",self.query.search,self.query.action,self.query.filters.len())))
                .child(div().h_8().flex_shrink_0().child(Input::new(&self.path).id("history-export-path").disabled(self.busy)))
                .child(Button::new("history-export-browse").label("Choose file…").disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.browse(window,cx))))
                .child(div().h_8().flex_shrink_0().child(Select::new(&self.format).id("history-export-format").w_full().disabled(self.busy)))
                .child(Checkbox::new("history-export-replace").label("Replace an existing export file").checked(self.replace).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.replace= *value;cx.notify();})))
                .child(div().flex().gap_3().child(Button::new("history-export-save").label("Export all matching events").disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.export(window,cx))))
                    .child(Button::new("history-copy-page").label("Copy matching page as CSV").disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.copy(window,cx)))))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Exports are private files capped at 256 MiB; clipboard is capped at 4 MiB. Stop leaves the destination unchanged. Live metadata such as environment variables is not part of history."));
        }
        div()
            .size_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .track_focus(&self.focus)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_xl().child("History filters, columns & export"))
            .child(
                TabBar::new("history-tools-tabs")
                    .selected_index(self.tab)
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.tab = *index;
                        cx.notify();
                    }))
                    .child(Tab::new().label("Filters"))
                    .child(Tab::new().label("Columns"))
                    .child(Tab::new().label("Export")),
            )
            .child(panel)
            .when_some(self.history.health.metadata_warning(), |body, message| {
                body.child(super::banner(message, cx))
            })
            .when_some(self.error.clone(), |body, error| {
                body.child(super::banner(error, cx))
            })
            .when_some(self.message.clone(), |body, message| {
                body.child(div().text_sm().child(message))
            })
            .child(
                div()
                    .flex()
                    .gap_3()
                    .justify_end()
                    .child(
                        Button::new("history-export-stop")
                            .label("Stop export")
                            .disabled(!self.exporting)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.stop.store(true, Ordering::Relaxed);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("history-tools-close")
                            .label("Close")
                            .disabled(self.busy)
                            .on_click(|_, window, _| window.remove_window()),
                    ),
            )
    }
}
