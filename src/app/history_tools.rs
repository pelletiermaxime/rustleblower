//! Native history filters, column settings and background export.
use crate::history::{
    self, History, Query,
    export::Format,
    query::{COMPARISONS, Comparison, FIELDS, Field, Filter, Logic},
    settings::Options,
};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
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
    Filters(Vec<Filter>, Logic),
    Sort(Option<(Field, bool)>),
    Mark {
        id: i64,
        generation: u64,
        extend: bool,
    },
}
type Picker<T> = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<T>>>>;
fn picker<T: Clone + PartialEq + 'static>(
    items: Vec<crate::i18n::LocalizedOption<T>>,
    window: &mut Window,
    cx: &mut Context<Tools>,
) -> Picker<T> {
    cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(items),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
        .searchable(true)
    })
}
pub(super) fn open(
    history: History,
    query: Query,
    selection: Option<Arc<history::selection::Selection>>,
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
                title: Some(crate::i18n::tr("Rustleblower — history tools").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| Tools::new(history, query, selection, sender, window, cx)),
    )?;
    crate::i18n::register_title(handle, "Rustleblower — history tools", cx);
    Ok(handle)
}
struct Tools {
    history: History,
    query: Query,
    selection: Option<Arc<history::selection::Selection>>,
    sender: SyncSender<Message>,
    options: Options,
    field: Picker<Field>,
    comparison: Picker<Comparison>,
    value: Entity<InputState>,
    formula: Entity<InputState>,
    new_group: bool,
    path: Entity<InputState>,
    format: Picker<Format>,
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
        mut query: Query,
        selection: Option<Arc<history::selection::Selection>>,
        sender: SyncSender<Message>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = picker(
            FIELDS
                .iter()
                .map(|field| crate::i18n::LocalizedOption::new(*field, field.label()))
                .collect(),
            window,
            cx,
        );
        let comparison = picker(
            COMPARISONS
                .iter()
                .map(|(op, label)| crate::i18n::LocalizedOption::new(*op, *label))
                .collect(),
            window,
            cx,
        );
        let format = picker(
            vec![
                crate::i18n::LocalizedOption::new(Format::Csv, "CSV"),
                crate::i18n::LocalizedOption::new(Format::JsonLines, "JSON Lines"),
            ],
            window,
            cx,
        );
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "Value; time accepts RFC3339 UTC",
                    window,
                    cx,
                ))
                .validate(|value, _| value.len() <= 256)
        });
        let path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "/absolute/path/history.csv",
                    window,
                    cx,
                ))
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
        query.logic = query.logic.clone().normalized(query.filters.len());
        let formula = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(query.logic.formula.clone())
                .validate(|value, _| value.len() <= 256 && value.is_ascii())
        });
        cx.subscribe(&formula, |this, state, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.query.logic.formula = state.read(cx).value().to_string();
                cx.notify();
            }
        })
        .detach();
        Self {
            history,
            query,
            selection,
            sender,
            options,
            field,
            comparison,
            value,
            formula,
            new_group: false,
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
    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self
            .field
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(Field::Process);
        let comparison = self
            .comparison
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(Comparison::Contains);
        let filter = Filter {
            field,
            comparison,
            value: self.value.read(cx).value().to_string(),
        };
        if self.query.filters.len() >= 8 {
            self.error = Some(crate::i18n::tr("Use at most eight conditions"));
        } else if let Err(error) = filter.validate() {
            self.error = Some(error.to_string());
        } else {
            let had_formula = !self.query.logic.formula.trim().is_empty();
            self.query
                .logic
                .append(self.new_group, self.query.filters.len());
            self.query.filters.push(filter);
            self.formula
                .update(cx, |state, cx| state.set_value("", window, cx));
            self.new_group = false;
            self.error = None;
            self.message =
                had_formula.then(|| "Condition numbers changed; the formula was cleared.".into());
        }
        cx.notify();
    }
    fn apply_filters(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.query.logic.validate(self.query.filters.len()) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        match self.sender.try_send(Message::Filters(
            self.query.filters.clone(),
            self.query.logic.clone(),
        )) {
            Ok(()) => {
                self.query.before = None;
                self.query.cursor = None;
                self.query.ceiling = None;
                self.error = None;
                self.message = Some(crate::i18n::tr(
                    "Filters and groups applied to history. Export uses this same expression.",
                ));
            }
            Err(_) => {
                self.error = Some(crate::i18n::tr(
                    "The history view is busy or closed; try again",
                ))
            }
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
    }
    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(reply) = self.history.query(self.query.clone()) else {
            self.error = Some(crate::i18n::tr("History is busy; try copying again"));
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
                        this.message = Some(crate::i18n::tr("Matching page copied as CSV."));
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn copy_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = self
            .selection
            .clone()
            .filter(|selection| !selection.is_empty())
        else {
            self.error = Some(crate::i18n::tr(
                "Select history events in the main window, then reopen these tools.",
            ));
            cx.notify();
            return;
        };
        let columns = self.options.columns.clone();
        let database = self.history.database_path().to_owned();
        let format = self
            .format
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(Format::Csv);
        self.stop.store(false, Ordering::Relaxed);
        let stop = self.stop.clone();
        self.busy = true;
        self.exporting = true;
        self.error = None;
        self.message = None;
        let task = cx.background_executor().spawn(async move {
            history::export::copy_selection(&database, &selection, &columns, format, &stop)
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                this.exporting = false;
                if this.stop.load(Ordering::Relaxed) {
                    this.error = Some(crate::i18n::tr("History copy stopped"));
                    cx.notify();
                    return;
                }
                match result {
                    Ok((text, report)) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        this.message = Some(crate::i18n::tr_args(
                            "Copied %{count} selected events.",
                            &[("count", &report.rows.to_string())],
                        ));
                        this.error = None;
                    }
                    Err(error) => {
                        this.error = Some(error);
                        this.message = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn export(&mut self, selected: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selection = if selected {
            self.selection
                .clone()
                .filter(|selection| !selection.is_empty())
        } else {
            None
        };
        if selected && selection.is_none() {
            self.error = Some(crate::i18n::tr(
                "Select history events in the main window, then reopen these tools.",
            ));
            cx.notify();
            return;
        }
        let target = PathBuf::from(self.path.read(cx).value().as_ref());
        let database = self.history.database.clone();
        let query = self.query.clone();
        let columns = self.options.columns.clone();
        let replace = self.replace;
        let format = self
            .format
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(Format::Csv);
        self.stop.store(false, Ordering::Relaxed);
        let stop = self.stop.clone();
        self.busy = true;
        self.exporting = true;
        self.error = None;
        self.message = Some(crate::i18n::tr(
            "Creating a private history snapshot and exporting…",
        ));
        let task = cx.background_executor().spawn(async move {
            let result = if let Some(selection) = selection {
                history::export::save_selection(
                    &database, &selection, &columns, format, &target, replace, &stop,
                )
            } else {
                history::export::save(&database, &query, &columns, format, &target, replace, &stop)
            };
            result.map_err(|error| format!("{error:#}"))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                this.exporting = false;
                match result {
                    Ok(report) => {
                        this.message = Some(crate::i18n::tr_args(
                            if selected {
                                "Exported %{arg0} selected events (%{arg1} bytes)."
                            } else {
                                "Exported %{arg0} matching events (%{arg1} bytes)."
                            },
                            &[
                                ("arg0", &format!("{}", report.rows)),
                                ("arg1", &format!("{}", report.bytes)),
                            ],
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
            panel=panel.child(div().text_sm().child(crate::i18n::tr("Match all or any groups, and all or any conditions within each group. Groups apply alongside the main search/action filter. Text matches are literal; numeric comparisons use integers and times accept RFC3339 UTC.")))
                .child(Button::new("history-filter-root-mode").label(crate::i18n::tr_args("Match groups: %{arg0}", &[("arg0", &(crate::i18n::tr(self.query.logic.mode.label())).to_string())])).disabled(self.busy || !self.query.logic.formula.trim().is_empty()).on_click(cx.listener(|this,_,_,cx|{this.query.logic.mode=this.query.logic.mode.toggled();cx.notify();})))
                .child(div().h_8().flex_shrink_0().child(Select::new(&self.field).id("history-filter-field").w_full().disabled(self.busy)))
                .child(div().h_8().flex_shrink_0().child(Select::new(&self.comparison).id("history-filter-comparison").w_full().disabled(self.busy)))
                .child(div().h_8().flex_shrink_0().child(Input::new(&self.value).id("history-filter-value").disabled(self.busy)))
                .child(Checkbox::new("history-filter-new-group").label(crate::i18n::tr("Start a new group with the next condition")).checked(self.new_group).disabled(self.busy || self.query.filters.is_empty()).on_change(cx.listener(|this,value:&bool,_,cx|{this.new_group= *value;cx.notify();})))
                .child(Button::new("history-filter-add").label(crate::i18n::tr("Add condition")).disabled(self.busy || self.query.filters.len()>=8).on_click(cx.listener(|this,_,window,cx|this.add(window,cx))));
            let mut group_start = 0;
            let mut group_index = 0;
            for (index, filter) in self.query.filters.iter().enumerate() {
                if index == group_start
                    && let Some(group) = self.query.logic.groups.get(group_index)
                {
                    let current = group_index;
                    panel = panel.child(
                        Button::new(("history-filter-group-mode", current))
                            .label(crate::i18n::tr_args(
                                "Group %{arg0}: %{arg1}",
                                &[
                                    ("arg0", &(current + 1).to_string()),
                                    ("arg1", &(crate::i18n::tr(group.mode.label())).to_string()),
                                ],
                            ))
                            .disabled(self.busy || !self.query.logic.formula.trim().is_empty())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let group = &mut this.query.logic.groups[current];
                                group.mode = group.mode.toggled();
                                cx.notify();
                            })),
                    );
                    group_start += usize::from(group.count);
                    group_index += 1;
                }
                panel = panel.child(
                    div()
                        .flex()
                        .gap_3()
                        .items_center()
                        .child(div().flex_1().text_sm().child(format!(
                                "{}. {} · {} · {}",
                                index + 1,
                                crate::i18n::tr(filter.field.label()),
                                crate::i18n::tr(
                                    COMPARISONS
                                        .iter()
                                        .find(|(op, _)| *op == filter.comparison)
                                        .unwrap()
                                        .1
                                ),
                                filter.value
                            )))
                        .child(
                            Button::new(("history-filter-remove", index))
                                .label(crate::i18n::tr("Remove"))
                                .disabled(self.busy)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let had_formula = !this.query.logic.formula.trim().is_empty();
                                    this.query.logic.remove(index, this.query.filters.len());
                                    this.query.filters.remove(index);
                                    this.formula
                                        .update(cx, |state, cx| state.set_value("", window, cx));
                                    this.message = had_formula.then(|| {
                                        "Condition numbers changed; the formula was cleared.".into()
                                    });
                                    cx.notify();
                                })),
                        ),
                );
            }
            panel=panel.child(div().text_sm().child(crate::i18n::tr("Formula (optional): reference every condition once, for example 1 AND (2 OR 3). AND binds before OR. Empty uses the group buttons. Adding or removing conditions clears the formula.")))
                .child(div().h_8().flex_shrink_0().child(Input::new(&self.formula).id("history-filter-formula").disabled(self.busy)))
                .child(div().flex().gap_3().child(Button::new("history-filter-apply").label(crate::i18n::tr("Apply filters")).disabled(self.busy || self.history.health.indexing.load(Ordering::Acquire)).on_click(cx.listener(|this,_,_,cx|this.apply_filters(cx))))
                .child(Button::new("history-filter-clear").label(crate::i18n::tr("Clear conditions")).disabled(self.busy).on_click(cx.listener(|this,_,window,cx|{this.query.filters.clear();this.query.logic=Logic::default();this.formula.update(cx,|state,cx|state.set_value("",window,cx));this.new_group=false;cx.notify();}))));
        } else if self.tab == 1 {
            panel=panel.child(Checkbox::new("history-table-mode").label(crate::i18n::tr("Use table view")).checked(self.options.table).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.options.table= *value;cx.notify();})))
                .child(div().text_sm().child(crate::i18n::tr("Table headers sort matching history, not just this page. Columns scroll horizontally. Compact view keeps the two-line summary. Saved choices also select export columns.")));
            for field in FIELDS {
                let field = *field;
                panel = panel.child(
                    Checkbox::new(SharedString::from(format!(
                        "history-column-{}",
                        field.key()
                    )))
                    .label(crate::i18n::tr(field.label()))
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
                    .label(crate::i18n::tr("Save history view"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.save_columns(window, cx))),
            );
        } else {
            panel=panel.child(div().text_sm().child(crate::i18n::tr_args("Captured search: %{arg0}; action: %{arg1}; %{arg2} conditions. File export includes all matching history, independently of the current page. Clipboard copies a matching page of at most 200 rows. Search, action, sort and page position were captured when this window opened; reopen it after changing the main view.", &[("arg0", &format!("{:?}", self.query.search)), ("arg1", &format!("{:?}", self.query.action)), ("arg2", &(self.query.filters.len()).to_string())])))
                .child(div().h_8().flex_shrink_0().child(Input::new(&self.path).id("history-export-path").disabled(self.busy)))
                .child(Button::new("history-export-browse").label(crate::i18n::tr("Choose file…")).disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.browse(window,cx))))
                .child(div().h_8().flex_shrink_0().child(Select::new(&self.format).id("history-export-format").w_full().disabled(self.busy)))
                .child(Checkbox::new("history-export-replace").label(crate::i18n::tr("Replace an existing export file")).checked(self.replace).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.replace= *value;cx.notify();})))
                .child(div().flex().gap_3().child(Button::new("history-export-save").label(crate::i18n::tr("Export all matching events")).disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.export(false,window,cx))))
                    .child(Button::new("history-copy-page").label(crate::i18n::tr("Copy matching page as CSV")).disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.copy(window,cx)))))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(crate::i18n::tr("Selected events were captured when these tools opened. Reopen to use a different selection. Chosen columns and format apply to selected output.")))
                .child(div().flex().gap_3().flex_wrap()
                    .child(Button::new("history-export-selected").label(crate::i18n::tr("Export selected events")).disabled(self.busy||self.selection.as_ref().is_none_or(|selection|selection.is_empty())).on_click(cx.listener(|this,_,window,cx|this.export(true,window,cx))))
                    .child(Button::new("history-copy-selected").label(crate::i18n::tr("Copy selected events")).disabled(self.busy||self.selection.as_ref().is_none_or(|selection|selection.is_empty())).on_click(cx.listener(|this,_,window,cx|this.copy_selected(window,cx)))))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(crate::i18n::tr("Exports are private files capped at 256 MiB; clipboard is capped at 4 MiB. Stop leaves the destination unchanged. Live metadata such as environment variables is not part of history.")));
        }
        #[cfg(feature = "headless-tests")]
        let panel = panel.test_support();
        div()
            .size_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .track_focus(&self.focus)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .text_xl()
                    .child(crate::i18n::tr("History filters, columns & export")),
            )
            .child(
                TabBar::new("history-tools-tabs")
                    .selected_index(self.tab)
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.tab = *index;
                        cx.notify();
                    }))
                    .child(Tab::new().label(crate::i18n::tr("Filters")))
                    .child(Tab::new().label(crate::i18n::tr("Columns")))
                    .child(Tab::new().label(crate::i18n::tr("Export"))),
            )
            .child(panel)
            .when_some(self.history.health.metadata_warning(), |body, message| {
                body.child(super::banner(message, cx))
            })
            .when_some(self.error.clone(), |body, error| {
                body.child(super::banner(error, cx))
            })
            .when_some(self.message.clone(), |body, message| {
                body.child(div().text_sm().child(crate::i18n::presentation(&message)))
            })
            .child(
                div()
                    .flex()
                    .gap_3()
                    .justify_end()
                    .child(
                        Button::new("history-export-stop")
                            .label(crate::i18n::tr("Stop export"))
                            .disabled(!self.exporting)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.stop.store(true, Ordering::Relaxed);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("history-tools-close")
                            .label(crate::i18n::tr("Close"))
                            .disabled(self.busy)
                            .on_click(|_, window, _| window.remove_window()),
                    ),
            )
    }
}

impl Drop for Tools {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
