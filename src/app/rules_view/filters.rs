//! Staged native rule filters. Applying uses a single bounded UI message; all
//! catalogue reads still belong to the existing background SQLite worker.
use crate::{
    history::query::{COMPARISONS, Comparison, Logic},
    rules::{
        Query,
        query::{FIELDS, Field, Filter, Predicate},
    },
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
};
use std::sync::mpsc::SyncSender;
pub(super) struct Update {
    pub session: u64,
    pub node: String,
    pub filters: Vec<Filter>,
    pub logic: Logic,
}
type Picker<T> = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<T>>>>;
pub(super) fn open(
    query: Query,
    session: u64,
    sender: SyncSender<Update>,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(900.), px(760.)),
                cx,
            ))),
            window_min_size: Some(size(px(720.), px(600.))),
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Rustleblower — rule filters").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| Editor::new(query, session, sender, window, cx)),
    )?;
    crate::i18n::register_title(window, "Rustleblower — rule filters", cx);
    Ok(window)
}
struct Editor {
    query: Query,
    session: u64,
    sender: SyncSender<Update>,
    field: Picker<Field>,
    comparison: Picker<Comparison>,
    value: Entity<InputState>,
    formula: Entity<InputState>,
    new_group: bool,
    error: Option<String>,
    message: Option<String>,
    focus: FocusHandle,
}
impl Editor {
    fn new(
        mut query: Query,
        session: u64,
        sender: SyncSender<Update>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    FIELDS
                        .iter()
                        .map(|field| crate::i18n::LocalizedOption::new(*field, field.label()))
                        .collect::<Vec<_>>(),
                ),
                Some(IndexPath::new(1)),
                window,
                cx,
            )
            .searchable(true)
        });
        let comparison = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    COMPARISONS
                        .iter()
                        .map(|(op, label)| crate::i18n::LocalizedOption::new(*op, *label))
                        .collect::<Vec<_>>(),
                ),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
            .searchable(true)
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "Value; dates accept Unix seconds or RFC3339 UTC",
                    window,
                    cx,
                ))
                .validate(|value, _| value.len() <= 256)
        });
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
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            query,
            session,
            sender,
            field,
            comparison,
            value,
            formula,
            new_group: false,
            error: None,
            message: None,
            focus,
        }
    }
    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self
            .field
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(Field::Name);
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
            self.error = Some(crate::i18n::tr("Use at most eight rule conditions"));
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
    fn apply(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = Predicate::new(&self.query) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        match self.sender.try_send(Update {
            session: self.session,
            node: self.query.node.clone(),
            filters: self.query.filters.clone(),
            logic: self.query.logic.clone(),
        }) {
            Ok(()) => {
                self.error = None;
                self.message = Some(crate::i18n::tr(
                    "Conditions sent to the rule catalogue. Exports and reviews use the applied conditions.",
                ));
            }
            Err(_) => {
                self.error = Some(crate::i18n::tr(
                    "The rule catalogue is busy or closed; try again.",
                ))
            }
        }
        cx.notify();
    }
}
impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body=div().id("rule-filter-body").flex_1().min_h_0().overflow_y_scroll().flex().flex_col().gap_3()
            .child(div().text_sm().child(crate::i18n::tr_args("Conditions for %{arg0} combine with the current search and Permanent/Temporary scope. Created uses Unix seconds; booleans use 0 or 1. Root operator fields inspect the outer operator, while Executable finds its process path.", &[("arg0", &(self.query.node).to_string())])))
            .child(Button::new("rule-filter-root-mode").label(crate::i18n::tr_args("Match groups: %{arg0}", &[("arg0", &(crate::i18n::tr(self.query.logic.mode.label())).to_string())])) .disabled(!self.query.logic.formula.trim().is_empty()).on_click(cx.listener(|this,_,_,cx|{this.query.logic.mode=this.query.logic.mode.toggled();cx.notify();})))
            .child(div().h_8().flex_shrink_0().child(Select::new(&self.field).id("rule-filter-field").w_full()))
            .child(div().h_8().flex_shrink_0().child(Select::new(&self.comparison).id("rule-filter-comparison").w_full()))
            .child(div().h_8().flex_shrink_0().child(Input::new(&self.value).id("rule-filter-value")))
            .child(Checkbox::new("rule-filter-new-group").label(crate::i18n::tr("Start a new group with the next condition")).checked(self.new_group).disabled(self.query.filters.is_empty()).on_change(cx.listener(|this,value:&bool,_,cx|{this.new_group= *value;cx.notify();})))
            .child(Button::new("rule-filter-add").label(crate::i18n::tr("Add condition")).disabled(self.query.filters.len()>=8).on_click(cx.listener(|this,_,window,cx|this.add(window,cx))));
        let (mut start, mut group_index) = (0, 0);
        for (index, filter) in self.query.filters.iter().enumerate() {
            if index == start
                && let Some(group) = self.query.logic.groups.get(group_index)
            {
                let current = group_index;
                body = body.child(
                    Button::new(("rule-filter-group-mode", current))
                        .label(crate::i18n::tr_args(
                            "Group %{arg0}: %{arg1}",
                            &[
                                ("arg0", &(current + 1).to_string()),
                                ("arg1", &(crate::i18n::tr(group.mode.label())).to_string()),
                            ],
                        ))
                        .disabled(!self.query.logic.formula.trim().is_empty())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let group = &mut this.query.logic.groups[current];
                            group.mode = group.mode.toggled();
                            cx.notify();
                        })),
                );
                start += usize::from(group.count);
                group_index += 1;
            }
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
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
                        Button::new(("rule-filter-remove", index))
                            .label(crate::i18n::tr("Remove"))
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
        body = body.child(div().text_sm().child(crate::i18n::tr("Formula (optional): reference every condition once, for example 1 AND (2 OR 3). AND binds before OR. Empty uses the group buttons. Adding or removing conditions clears the formula.")))
            .child(div().h_8().flex_shrink_0().child(Input::new(&self.formula).id("rule-filter-formula")));
        #[cfg(feature = "headless-tests")]
        let body = body.test_support();
        div()
            .size_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .track_focus(&self.focus)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_xl().child(crate::i18n::tr("Rule filters")))
            .child(body)
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(crate::i18n::presentation(&error)),
                )
            })
            .when_some(self.message.clone(), |view, message| {
                view.child(div().text_sm().child(crate::i18n::presentation(&message)))
            })
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        Button::new("rule-filter-apply")
                            .label(crate::i18n::tr("Apply conditions"))
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    )
                    .child(
                        Button::new("rule-filter-clear")
                            .label(crate::i18n::tr("Clear staged conditions"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.query.filters.clear();
                                this.query.logic = Logic::default();
                                this.formula
                                    .update(cx, |state, cx| state.set_value("", window, cx));
                                this.new_group = false;
                                this.message = None;
                                this.error = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("rule-filter-close")
                            .label(crate::i18n::tr("Close"))
                            .on_click(|_, window, _| window.remove_window()),
                    ),
            )
    }
}
