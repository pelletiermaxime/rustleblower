//! Native daemon setting controls and a bounded review; disk work stays on the writer.
use crate::{
    backend::Handle,
    nodes::{
        self,
        preferences::{self as model, FIELDS, Field, GROUPS, Group, Kind},
    },
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
};
use serde_json::Value;
use std::{collections::BTreeSet, time::Duration};
use tokio::sync::oneshot;
type Picker = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<usize>>>>;
actions!(daemon_preferences, [ReviewChanges, ConfirmChanges]);
pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-enter", ReviewChanges, Some("DaemonPreferences")),
        KeyBinding::new(
            "ctrl-shift-enter",
            ConfirmChanges,
            Some("DaemonPreferences"),
        ),
    ]);
}
pub(super) fn open(
    backend: Handle,
    session: u64,
    node: String,
    text: String,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(920.), px(740.)),
                cx,
            ))),
            window_min_size: Some(size(px(760.), px(580.))),
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Rustleblower — daemon configuration").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| Configuration::new(backend, session, node, text, window, cx)),
    )?;
    crate::i18n::register_title(handle, "Rustleblower — daemon configuration", cx);
    Ok(handle)
}
enum Control {
    Text(Entity<InputState>),
    Choice {
        state: Picker,
        values: Vec<(SharedString, Option<Value>)>,
    },
    Check(bool),
}
struct Entry {
    field: Field,
    control: Control,
    original: Option<Value>,
    initial: String,
    status: Option<&'static str>,
}
struct Form {
    entries: Vec<Entry>,
    dirty: BTreeSet<usize>,
    _subscriptions: Vec<Subscription>,
}
impl Form {
    fn new(document: &Value, window: &mut Window, cx: &mut Context<Configuration>) -> Self {
        let mut entries = Vec::with_capacity(FIELDS.len());
        let mut subscriptions = Vec::new();
        for (index, field) in FIELDS.iter().copied().enumerate() {
            let original = document.pointer(field.path).cloned();
            let initial = model::display(original.as_ref());
            let mut status = if original.is_none() {
                Some("Not reported; preserved unless edited.")
            } else {
                None
            };
            let control = match field.kind {
                Kind::Boolean => {
                    if original.as_ref().is_some_and(|value| !value.is_boolean()) {
                        status = Some("Unrecognized reported value; preserved unless edited.");
                    }
                    Control::Check(original.as_ref().and_then(Value::as_bool).unwrap_or(false))
                }
                Kind::Choice(options) => {
                    let mut values = vec![(SharedString::from("Not set (daemon default)"), None)];
                    for (key, label) in options {
                        let value = if field.path == "/LogLevel" {
                            Value::from(key.parse::<u32>().unwrap())
                        } else {
                            Value::String((*key).into())
                        };
                        values.push((SharedString::from(*label), Some(value)));
                    }
                    let selected = values
                        .iter()
                        .position(|(_, value)| value == &original)
                        .unwrap_or_else(|| {
                            status = Some("Unrecognized reported value; preserved unless edited.");
                            values.push((
                                SharedString::from(crate::i18n::tr_args(
                                    "Reported: %{arg0}",
                                    &[("arg0", &crate::history::bounded(&initial, 80).to_string())],
                                )),
                                original.clone(),
                            ));
                            values.len() - 1
                        });
                    let state = cx.new(|cx| {
                        SelectState::new(
                            SearchableVec::new(
                                values
                                    .iter()
                                    .enumerate()
                                    .map(|(index, (label, _))| {
                                        crate::i18n::LocalizedOption::new(index, label.to_string())
                                    })
                                    .collect::<Vec<_>>(),
                            ),
                            Some(IndexPath::new(selected)),
                            window,
                            cx,
                        )
                        .searchable(true)
                    });
                    subscriptions.push(cx.subscribe(
                        &state,
                        move |this,
                              _,
                              _: &SelectEvent<
                            SearchableVec<crate::i18n::LocalizedOption<usize>>,
                        >,
                              cx| {
                            this.form.dirty.insert(index);
                            this.message = None;
                            cx.notify();
                        },
                    ));
                    Control::Choice { state, values }
                }
                _ => {
                    let max = match field.kind {
                        Kind::Text(max) => max,
                        Kind::Interval => 128,
                        _ => 20,
                    };
                    let displayed = crate::history::bounded(&initial, max);
                    if initial.len() > max
                        || original.as_ref().is_some_and(|value| {
                            matches!(field.kind, Kind::Text(_) | Kind::Interval)
                                && !value.is_string()
                        })
                    {
                        status = Some(
                            "Unrecognized or oversized reported value; preserved unless edited.",
                        );
                    }
                    let state = cx.new(|cx| {
                        InputState::new(window, cx)
                            .default_value(displayed)
                            .validate(move |text, _| text.len() <= max && !text.contains('\0'))
                    });
                    subscriptions.push(cx.subscribe(&state, move |this, _, event, cx| {
                        if matches!(event, InputEvent::Change) {
                            this.form.dirty.insert(index);
                            this.message = None;
                            cx.notify();
                        }
                    }));
                    Control::Text(state)
                }
            };
            entries.push(Entry {
                field,
                control,
                original,
                initial,
                status,
            });
        }
        Self {
            entries,
            dirty: BTreeSet::new(),
            _subscriptions: subscriptions,
        }
    }
    fn edits(&self, cx: &App) -> Result<model::Edits, String> {
        let mut edits = model::Edits::new();
        for index in &self.dirty {
            let entry = &self.entries[*index];
            let value = match &entry.control {
                Control::Check(checked) => Some(Value::Bool(*checked)),
                Control::Choice { state, values } => {
                    let index = state.read(cx).selected_value().copied();
                    index
                        .and_then(|index| values.get(index))
                        .ok_or("Choose a daemon setting value")?
                        .1
                        .clone()
                }
                Control::Text(state) => {
                    let value = state.read(cx).value();
                    if value.as_ref()
                        == crate::history::bounded(
                            &entry.initial,
                            match entry.field.kind {
                                Kind::Text(max) => max,
                                Kind::Interval => 128,
                                _ => 20,
                            },
                        )
                    {
                        continue;
                    }
                    model::parse(entry.field, &value)?
                }
            };
            if value != entry.original {
                edits.insert(entry.field.path, value);
            }
        }
        Ok(edits)
    }
    fn focus_first(&self, window: &mut Window, cx: &mut App) {
        match &self.entries[0].control {
            Control::Text(state) => state.focus_handle(cx).focus(window, cx),
            Control::Choice { state, .. } => state.focus_handle(cx).focus(window, cx),
            _ => {}
        }
    }
}
struct Stage {
    text: String,
    review: model::Review,
}
pub(super) struct Configuration {
    backend: Handle,
    session: u64,
    node: String,
    input: Entity<TextareaState>,
    original: String,
    last_valid: String,
    form: Form,
    mode: usize,
    group: Group,
    focus: FocusHandle,
    stage: Option<Stage>,
    pending: Option<oneshot::Receiver<Result<(), String>>>,
    pub(super) error: Option<String>,
    message: Option<String>,
}
impl Configuration {
    fn new(
        backend: Handle,
        session: u64,
        node: String,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).default_value(text.clone()));
        cx.subscribe_in(&input, window, |this, input, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                if text.len() > nodes::CONFIG_BYTES {
                    input.update(cx, |input, cx| {
                        input.set_value(this.last_valid.clone(), window, cx)
                    });
                    this.error = Some(crate::i18n::tr(
                        "Daemon configuration must stay below 64 KiB",
                    ));
                } else {
                    this.last_valid = text;
                }
                cx.notify();
            }
        })
        .detach();
        let parsed = nodes::validate_config(&text);
        let error = parsed.as_ref().err().cloned();
        let document = parsed.unwrap_or_else(|_| serde_json::json!({}));
        let form = Form::new(&document, window, cx);
        let mode = if error.is_some() { 1 } else { 0 };
        let focus = cx.focus_handle();
        if mode == 0 {
            form.focus_first(window, cx);
        } else {
            input.focus_handle(cx).focus(window, cx);
        }
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let busy = weak
                .upgrade()
                .is_some_and(|view| view.read(cx).pending.is_some());
            if busy {
                window.minimize_window();
            }
            !busy
        });
        cx.spawn_in(window,async move|this,cx|loop{cx.background_executor().timer(Duration::from_millis(50)).await;if this.update_in(cx,|this,window,cx|{
   if let Some(pending)=this.pending.as_mut(){match pending.try_recv(){Ok(result)=>{this.pending=None;match result{Ok(())=>{if let Some(stage)=this.stage.take(){this.original=stage.text.clone();this.last_valid=stage.text.clone();this.input.update(cx,|input,cx|input.set_value(stage.text.clone(),window,cx));let value=nodes::validate_config(&stage.text).unwrap();this.form=Form::new(&value,window,cx);}this.mode=0;this.message=Some(crate::i18n::tr("Daemon confirmed and saved the configuration."));this.error=None;},Err(error)=>{this.error=Some(error);}}cx.notify();},Err(oneshot::error::TryRecvError::Closed)=>{this.pending=None;this.error=Some(crate::i18n::tr("Configuration command stopped without confirmation; reconnect to verify"));cx.notify();},_=>{}}}
  }).is_err(){break;}}).detach();
        Self {
            backend,
            session,
            node,
            input,
            original: text.clone(),
            last_valid: text,
            form,
            mode,
            group: Group::General,
            focus,
            stage: None,
            pending: None,
            error,
            message: None,
        }
    }
    fn connected(&self) -> bool {
        self.backend
            .session(self.session)
            .is_some_and(|session| session.notifications)
            && !self
                .backend
                .history
                .health
                .catalogues
                .uncertain(self.session)
    }
    fn sync_form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let edits = self.form.edits(cx)?;
        let text = self.input.read(cx).value().to_string();
        let candidate = model::patch(&text, &edits)?;
        let document = nodes::validate_config(&candidate)?;
        self.last_valid = candidate.clone();
        self.input
            .update(cx, |input, cx| input.set_value(candidate, window, cx));
        self.form = Form::new(&document, window, cx);
        Ok(())
    }
    fn stage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() || !self.connected() {
            return;
        }
        let result = (|| -> Result<Stage, String> {
            if self.mode == 0 {
                self.sync_form(window, cx)?;
            }
            let text = self.input.read(cx).value().to_string();
            let review = model::review(&self.original, &text)?;
            if review.total == 0 {
                return Err("No setting changes to apply.".into());
            }
            Ok(Stage { text, review })
        })();
        match result {
            Ok(stage) => {
                self.stage = Some(stage);
                self.mode = 2;
                self.error = None;
                self.message = None;
                self.focus.focus(window, cx);
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
    fn confirm(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_some() || !self.connected() {
            return;
        }
        if let Some(stage) = self.stage.as_ref() {
            match self.backend.control.submit_node(
                self.session,
                nodes::Change::Configuration(stage.text.clone()),
            ) {
                Ok(pending) => self.pending = Some(pending),
                Err(error) => self.error = Some(error),
            }
        }
        cx.notify();
    }
    fn change_mode(&mut self, mode: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        if mode == 2 {
            self.stage(window, cx);
            return;
        }
        if self.mode == 0 && mode == 1 {
            if let Err(error) = self.sync_form(window, cx) {
                self.error = Some(error);
                cx.notify();
                return;
            }
        } else if self.mode != 0 && mode == 0 {
            match nodes::validate_config(&self.input.read(cx).value()) {
                Ok(value) => self.form = Form::new(&value, window, cx),
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
            }
        }
        self.stage = None;
        self.mode = mode;
        self.error = None;
        if mode == 1 {
            self.input.focus_handle(cx).focus(window, cx);
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }
    fn settings(&self, locked: bool, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let mut form = div().flex().flex_col().gap_4().py_2();
        for (index, entry) in self
            .form
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.field.group == self.group)
        {
            let field = entry.field;
            let control = match &entry.control {
                Control::Text(state) => div()
                    .h_8()
                    .flex_shrink_0()
                    .child(Input::new(state).id(field.id).disabled(locked))
                    .into_any_element(),
                Control::Choice { state, .. } => div()
                    .h_8()
                    .flex_shrink_0()
                    .child(Select::new(state).id(field.id).w_full().disabled(locked))
                    .into_any_element(),
                Control::Check(checked) => div()
                    .child(
                        Checkbox::new(field.id)
                            .label(crate::i18n::tr(field.label))
                            .checked(*checked)
                            .disabled(locked)
                            .on_change(cx.listener(move |this, value: &bool, _, cx| {
                                if let Control::Check(checked) =
                                    &mut this.form.entries[index].control
                                {
                                    *checked = *value;
                                }
                                this.form.dirty.insert(index);
                                this.message = None;
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            };
            form = form.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(!matches!(field.kind, Kind::Boolean), |row| {
                        row.child(div().text_sm().child(crate::i18n::tr(field.label)))
                    })
                    .child(control)
                    .when(!field.hint.is_empty(), |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(crate::i18n::tr(field.hint)),
                        )
                    })
                    .when_some(entry.status, |row, status| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(crate::i18n::tr(status)),
                        )
                    }),
            );
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                TabBar::new("daemon-setting-groups")
                    .selected_index(
                        GROUPS
                            .iter()
                            .position(|(group, _)| *group == self.group)
                            .unwrap(),
                    )
                    .children(
                        GROUPS
                            .iter()
                            .map(|(_, label)| Tab::new().label(crate::i18n::tr(label))),
                    )
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.group = GROUPS[*index].0;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id((
                        "daemon-setting-scroll",
                        GROUPS
                            .iter()
                            .position(|(group, _)| *group == self.group)
                            .unwrap(),
                    ))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(form),
            )
    }
}
impl Render for Configuration {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.pending.is_some();
        let locked = busy || self.stage.is_some();
        let connected = self.connected();
        let muted = cx.theme().muted_foreground;
        let content = if self.mode == 0 {
            self.settings(locked, cx).into_any_element()
        } else if self.mode == 1 {
            let input = self.input.clone();
            let weak = cx.weak_entity();
            div()
                .flex_1()
                .min_h_0()
                .child(
                    Textarea::new(&self.input)
                        .accessibility_id("node-config-json")
                        .size_full()
                        .disabled(locked)
                        .on_paste(move |clipboard, _, cx| {
                            let state = input.read(cx);
                            let length = state
                                .value()
                                .len()
                                .saturating_sub(state.selected_value().len());
                            if clipboard.text().is_some_and(|text| {
                                text.len().saturating_add(length) > nodes::CONFIG_BYTES
                            }) {
                                let _ = weak.update(cx, |this, cx| {
                                    this.error = Some(crate::i18n::tr(
                                        "Paste would exceed the 64 KiB configuration limit",
                                    ));
                                    cx.notify();
                                });
                                true
                            } else {
                                false
                            }
                        }),
                )
                .into_any_element()
        } else {
            let count = self.stage.as_ref().map_or(0, |stage| stage.review.total);
            let rows = self
                .stage
                .as_ref()
                .map_or(0, |stage| stage.review.rows.len());
            div().flex_1().min_h_0().flex().flex_col().gap_2().child(div().text_sm().child(crate::i18n::tr_args("Review %{count} changed setting(s) for %{arg0}. Confirming applies and saves them on that daemon.", &[("count", &(count).to_string()), ("arg0", &(self.node).to_string())]))).when(count>model::MAX_REVIEW,|body|body.child(div().text_sm().child(crate::i18n::tr("Only the first 200 changes are listed. Complete JSON retains the entire bounded document."))))
   .child(uniform_list("daemon-review-rows",rows,cx.processor(|this,range:std::ops::Range<usize>,_,cx|range.map(|index|{let row=&this.stage.as_ref().unwrap().review.rows[index];div().w_full().h(px(116.)).px_3().py_2().border_b_1().border_color(cx.theme().border).flex().flex_col().gap_1().child(div().text_sm().child(model::field(&row.path).map_or_else(||row.path.clone(),|field|crate::i18n::tr(field.label)))).child(div().text_xs().truncate().text_color(cx.theme().muted_foreground).child(row.path.clone())).child(div().text_xs().truncate().text_color(cx.theme().muted_foreground).child(crate::i18n::tr_args("Before: %{arg0}", &[("arg0", &(row.before).to_string())]))).child(div().text_xs().truncate().child(crate::i18n::tr_args("After: %{arg0}", &[("arg0", &(row.after).to_string())])))}).collect::<Vec<_>>())).flex_1().min_h_0().w_full()).into_any_element()
        };
        div().size_full().p_5().flex().flex_col().gap_3().bg(cx.theme().background).text_color(cx.theme().foreground).track_focus(&self.focus).key_context("DaemonPreferences")
  .on_action(cx.listener(|this,_:&ReviewChanges,window,cx|this.stage(window,cx))).on_action(cx.listener(|this,_:&ConfirmChanges,_,cx|this.confirm(cx)))
  .child(div().flex_shrink_0().text_xl().child(crate::i18n::tr_args("Daemon configuration — %{arg0}", &[("arg0", &(self.node).to_string())])))
  .child(div().flex_shrink_0().text_sm().text_color(muted).child(crate::i18n::tr("Changes apply to this daemon. Unedited fields are preserved; missing settings keep their reported/default behavior. Popup defaults are separate.")))
  .child(TabBar::new("daemon-config-modes").flex_shrink_0().selected_index(self.mode).children([Tab::new().label(crate::i18n::tr("Settings")),Tab::new().label(crate::i18n::tr("Complete JSON")),Tab::new().label(crate::i18n::tr("Review"))]).on_click(cx.listener(|this,index:&usize,window,cx|this.change_mode(*index,window,cx))))
  .child(content).when_some(self.error.clone(),|body,error|body.child(super::banner(error,cx))).when_some(self.message.clone(),|body,message|body.child(div().text_sm().child(crate::i18n::presentation(&message))))
  .child(div().flex_shrink_0().text_xs().text_color(muted).child(crate::i18n::tr("Ctrl+Enter reviews changes · Ctrl+Shift+Enter confirms after review")))
  .child(div().flex_shrink_0().flex().gap_3().child(Button::new("node-config-review").label(crate::i18n::tr("Review changes")).disabled(locked||!connected).on_click(cx.listener(|this,_,window,cx|this.stage(window,cx))))
   .child(Button::new("node-config-confirm").label(crate::i18n::tr("Confirm apply")).disabled(busy||self.stage.is_none()||!connected).on_click(cx.listener(|this,_,_,cx|this.confirm(cx))))
   .child(Button::new("node-config-cancel").label(crate::i18n::tr("Back to editing")).disabled(busy||self.stage.is_none()).on_click(cx.listener(|this,_,window,cx|this.change_mode(0,window,cx))))
   .child(Button::new("node-config-close").label(crate::i18n::tr("Close")).disabled(busy).on_click(|_,window,_|window.remove_window())))
    }
}
