//! Full native rule editor. Drafts are local until the daemon acknowledges Save.
use crate::{
    backend::Handle,
    protocol::{Action, Operator, Rule},
    rules::document,
};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputState, Textarea, TextareaState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
};

type Picker = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<String>>>>;
const ACTIONS: &[(&str, &str)] = &[("allow", "Allow"), ("deny", "Deny"), ("reject", "Reject")];
const DURATIONS: &[(&str, &str)] = &[
    ("once", "Once"),
    ("30s", "30 seconds"),
    ("5m", "5 minutes"),
    ("15m", "15 minutes"),
    ("30m", "30 minutes"),
    ("1h", "1 hour"),
    ("12h", "12 hours"),
    ("until restart", "Until daemon restart"),
    ("always", "Forever"),
];
const KINDS: &[(&str, &str)] = &[
    ("simple", "Equals"),
    ("regexp", "Regular expression"),
    ("network", "IP network / CIDR"),
    ("lists", "Watched list directory"),
    ("range", "Port range (inclusive)"),
];
const FIELDS: &[(&str, &str)] = &[
    ("process.path", "Executable path"),
    ("process.parent.path", "Parent executable"),
    ("process.command", "Command line"),
    ("process.id", "Process ID"),
    ("user.id", "User ID"),
    ("user.name", "User name"),
    ("dest.host", "Destination host"),
    ("dest.ip", "Destination IP"),
    ("dest.port", "Destination port"),
    ("dest.network", "Destination network"),
    ("source.ip", "Source IP"),
    ("source.port", "Source port"),
    ("source.network", "Source network"),
    ("protocol", "Protocol"),
    ("iface.in", "Incoming interface"),
    ("iface.out", "Outgoing interface"),
    ("process.env.", "Environment variable"),
    ("process.hash.md5", "Executable MD5"),
    ("process.hash.sha1", "Executable SHA1"),
    ("lists.domains", "Domain list"),
    ("lists.domains_regexp", "Domain regex list"),
    ("lists.ips", "IP list"),
    ("lists.nets", "Network list"),
    ("lists.hash.md5", "MD5 list"),
    ("true", "Match all connections"),
];

fn picker(
    options: &[(&str, &str)],
    value: &str,
    window: &mut Window,
    cx: &mut Context<RuleEditor>,
) -> Picker {
    let mut labels = options
        .iter()
        .map(|(value, label)| crate::i18n::LocalizedOption::new((*value).to_owned(), *label))
        .collect::<Vec<_>>();
    let index = options
        .iter()
        .position(|(key, _)| *key == value)
        .unwrap_or_else(|| {
            labels.push(crate::i18n::LocalizedOption::raw(value.to_string(), value));
            labels.len() - 1
        });
    let state = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(labels),
            Some(IndexPath::new(index)),
            window,
            cx,
        )
        .searchable(true)
    });
    cx.subscribe(
        &state,
        |_, _, _: &SelectEvent<SearchableVec<crate::i18n::LocalizedOption<String>>>, cx| {
            cx.notify()
        },
    )
    .detach();
    state
}
fn selected(state: &Picker, _options: &[(&str, &str)], cx: &App) -> String {
    state.read(cx).selected_value().cloned().unwrap_or_default()
}
fn input(
    value: impl Into<SharedString>,
    hint: &'static str,
    window: &mut Window,
    cx: &mut Context<RuleEditor>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value)
            .placeholder(crate::i18n::placeholder(hint, window, cx))
    })
}

struct Condition {
    id: u64,
    original: Operator,
    field: Picker,
    kind: Picker,
    value: Entity<InputState>,
    environment: Entity<InputState>,
    sensitive: bool,
}
impl Condition {
    fn new(id: u64, op: Operator, window: &mut Window, cx: &mut Context<RuleEditor>) -> Self {
        let (field, environment) = op
            .operand
            .strip_prefix("process.env.")
            .map_or((op.operand.as_str(), ""), |name| ("process.env.", name));
        let value = input(
            op.data.clone(),
            "Match value (empty is allowed)",
            window,
            cx,
        );
        let environment = input(
            environment.to_string(),
            "Environment variable name",
            window,
            cx,
        );
        Self {
            id,
            field: picker(FIELDS, field, window, cx),
            kind: picker(KINDS, &op.r#type, window, cx),
            value,
            environment,
            sensitive: op.sensitive,
            original: op,
        }
    }
    fn operator(&self, cx: &App) -> Result<Operator, String> {
        let mut op = self.original.clone();
        op.operand = selected(&self.field, FIELDS, cx);
        op.r#type = selected(&self.kind, KINDS, cx);
        if op.operand == "process.env." {
            let name = self.environment.read(cx).value().to_string();
            if name.is_empty() {
                return Err("Enter an environment variable name".into());
            }
            op.operand.push_str(&name);
        }
        op.data = self.value.read(cx).value().to_string();
        op.sensitive = self.sensitive;
        if op.r#type != "list" {
            op.list.clear();
        }
        if op.r#type == "network"
            && !matches!(op.operand.as_str(), "source.network" | "dest.network")
        {
            return Err(
                "IP network matching requires the Source network or Destination network field"
                    .into(),
            );
        }
        if op.r#type == "lists" && !op.operand.starts_with("lists.") {
            return Err(
                "A watched directory requires a domain, IP, network or checksum list field".into(),
            );
        }
        if matches!(op.operand.as_str(), "source.network" | "dest.network")
            && op.r#type != "network"
        {
            return Err("Network fields require the IP network / CIDR match type".into());
        }
        if op.operand.starts_with("lists.") && op.r#type != "lists" {
            return Err("List fields require the Watched list directory match type".into());
        }
        Ok(op)
    }
}

pub(super) fn open(
    backend: Handle,
    session: u64,
    rule: Rule,
    original: Option<Rule>,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(900.), px(760.)),
                cx,
            ))),
            window_min_size: Some(size(px(720.), px(540.))),
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Rustleblower — application rule").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| {
            cx.new(|cx| RuleEditor::new(backend, session, rule, original, window, cx))
        },
    )?;
    crate::i18n::register_title(window, "Rustleblower — application rule", cx);
    Ok(window)
}

pub(super) struct RuleEditor {
    backend: Handle,
    session: u64,
    base: Rule,
    original: Option<Rule>,
    name: Entity<InputState>,
    description: Entity<InputState>,
    action: Picker,
    duration: Picker,
    custom_duration: Entity<InputState>,
    custom_duration_enabled: bool,
    conditions: Vec<Condition>,
    next: u64,
    raw: Entity<TextareaState>,
    raw_mode: bool,
    enabled: bool,
    priority: bool,
    logging: bool,
    busy: bool,
    distributing: bool,
    error: Option<String>,
    focus: FocusHandle,
}

impl RuleEditor {
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn test_error(&self) -> Option<String> {
        self.error.clone()
    }
    pub(super) fn new(
        backend: Handle,
        session: u64,
        rule: Rule,
        original: Option<Rule>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.on_window_should_close(cx, |window, _| {
            window.minimize_window();
            false
        });
        let op = rule.operator.clone().unwrap_or(Operator {
            r#type: "simple".into(),
            operand: "process.path".into(),
            ..Default::default()
        });
        let ops = if op.r#type == "list" && !op.list.is_empty() {
            op.list.clone()
        } else {
            vec![op.clone()]
        };
        let raw_mode = ops.len() > 64 || ops.iter().any(|op| op.r#type == "list");
        let conditions = ops
            .into_iter()
            .take(64)
            .enumerate()
            .map(|(i, op)| Condition::new(i as u64, op, window, cx))
            .collect::<Vec<_>>();
        let raw_text = serde_json::to_string_pretty(&document::operator_json(&op)).unwrap();
        let raw = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(12)
                .default_value(raw_text)
        });
        let name = input(rule.name.clone(), "Rule name", window, cx);
        let description = input(rule.description.clone(), "Description", window, cx);
        let action = picker(ACTIONS, &rule.action, window, cx);
        let duration = picker(DURATIONS, &rule.duration, window, cx);
        let custom_duration = input(
            rule.duration.clone(),
            "Custom duration, e.g. 90s or 2h30m",
            window,
            cx,
        );
        let focus = cx.focus_handle();
        name.focus_handle(cx).focus(window, cx);
        Self {
            backend,
            session,
            name,
            description,
            action,
            duration,
            custom_duration,
            custom_duration_enabled: !DURATIONS.iter().any(|(key, _)| *key == rule.duration),
            next: conditions.len() as u64,
            conditions,
            raw,
            raw_mode,
            enabled: rule.enabled,
            priority: rule.precedence,
            logging: !rule.nolog,
            base: rule,
            original,
            busy: false,
            distributing: false,
            error: None,
            focus,
        }
    }

    fn operator(&self, cx: &App) -> Result<Operator, String> {
        if self.raw_mode {
            return document::operator_from_text(self.raw.read(cx).value().as_ref());
        }
        let mut operators = self
            .conditions
            .iter()
            .map(|condition| condition.operator(cx))
            .collect::<Result<Vec<_>, _>>()?;
        if operators.is_empty() {
            return Err("Add a match condition, or choose Match all connections explicitly".into());
        }
        if operators.len() == 1
            && !self
                .base
                .operator
                .as_ref()
                .is_some_and(|o| o.r#type == "list")
        {
            return Ok(operators.remove(0));
        }
        let mut op = self
            .base
            .operator
            .clone()
            .filter(|o| o.r#type == "list")
            .unwrap_or(Operator {
                r#type: "list".into(),
                operand: "list".into(),
                sensitive: true,
                ..Default::default()
            });
        op.data.clear();
        op.list = operators;
        Ok(op)
    }
    fn draft(&self, cx: &App) -> Result<Rule, String> {
        let mut rule = self.base.clone();
        rule.name = self.name.read(cx).value().to_string();
        rule.description = self.description.read(cx).value().to_string();
        rule.action = selected(&self.action, ACTIONS, cx);
        rule.duration = if self.custom_duration_enabled {
            self.custom_duration.read(cx).value().to_string()
        } else {
            selected(&self.duration, DURATIONS, cx)
        };
        rule.enabled = self.enabled;
        rule.precedence = self.priority;
        rule.nolog = !self.logging;
        rule.operator = Some(self.operator(cx)?);
        document::validate_edit(&rule)?;
        Ok(rule)
    }
    fn distribute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let rule = match self.draft(cx) {
            Ok(rule) => rule,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let original = self.original.as_ref().map(|rule| rule.name.clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        match super::rule_distribution::open_draft(
            self.backend.clone(),
            rule,
            original,
            self.session,
            tx,
            cx,
        ) {
            Ok(_) => {
                self.busy = true;
                self.distributing = true;
                self.error = None;
                cx.spawn_in(window, async move |this, cx| {
                    let _ = rx.await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.busy = false;
                        this.distributing = false;
                        this.name.focus_handle(cx).focus(window, cx);
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(error) => {
                self.error = Some(crate::i18n::tr_args(
                    "Cannot review destinations: %{error}",
                    &[("error", &format!("{:#}", error))],
                ))
            }
        }
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let rule = match self.draft(cx) {
            Ok(rule) => rule,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        let delete_old = self.original.clone().filter(|old| old.name != rule.name);
        let unique = self
            .original
            .as_ref()
            .is_none_or(|old| old.name != rule.name);
        let database = self.backend.history.database_path().to_owned();
        let node = self
            .backend
            .session(self.session)
            .map(|session| session.name);
        let name = rule.name.clone();
        let check=cx.background_executor().spawn(async move {
            let node=node.ok_or("Daemon changed or disconnected; reopen this editor")?;
            if unique&&crate::rules::transfer::find_rule(&database,&node,&name).map_err(|error|crate::i18n::tr_args("Cannot check rule name: %{error}", &[("error", &format!("{:#}", error))]))?.is_some() {
                return Err("A rule with this name already exists; choose a different name or edit the existing rule".to_string());
            }
            Ok::<_,String>(())
        });
        self.busy = true;
        self.error = None;
        cx.notify();
        let control = self.backend.control.clone();
        let session = self.session;
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                check.await?;
                let first = control.submit(session, Action::ChangeRule, rule)?;
                first
                    .await
                    .map_err(|_| "Rule command stopped before confirmation".to_string())??;
                if let Some(old) = delete_old {
                    let rx = control
                        .submit(session, Action::DeleteRule, old)
                        .map_err(|e| {
                            crate::i18n::tr_args(
                                "New rule was saved, but the original could not be deleted: %{e}",
                                &[("e", &e.to_string())],
                            )
                        })?;
                    rx.await
                        .map_err(|_| {
                            "New rule was saved, but deleting the original was not confirmed"
                                .to_string()
                        })?
                        .map_err(|e| {
                            crate::i18n::tr_args(
                                "New rule was saved, but the original could not be deleted: %{e}",
                                &[("e", &e.to_string())],
                            )
                        })?;
                }
                Ok::<_, String>(())
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => window.remove_window(),
                    Err(e) => {
                        this.error = Some(e);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}

impl Render for RuleEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let busy = self.busy;
        let metadata=div().flex().flex_col().gap_3()
            .child(div().text_sm().child(self.backend.session(self.session).map(|s|crate::i18n::tr_args("Target node: %{arg0}", &[("arg0", &s.name.to_string())])).unwrap_or_else(||"Target node disconnected; reopen this editor before saving.".into())))
            .child(div().text_sm().text_color(muted).child(crate::i18n::tr("Name and description")))
            .child(Input::new(&self.name).id("editor-name").disabled(busy))
            .child(Input::new(&self.description).id("editor-description").disabled(busy))
            .child(div().flex().gap_3().items_center()
                .child(Select::new(&self.action).id("editor-action").accessibility_label(crate::i18n::tr("Rule action")).w(px(180.)).disabled(busy))
                .child(Select::new(&self.duration).id("editor-duration").accessibility_label(crate::i18n::tr("Rule duration")).w(px(220.)).disabled(busy||self.custom_duration_enabled))
                .child(Checkbox::new("editor-custom-duration-enabled").label(crate::i18n::tr("Custom duration")).checked(self.custom_duration_enabled).disabled(busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.custom_duration_enabled = *value;cx.notify();}))))
            .when(self.custom_duration_enabled,|v|v.child(Input::new(&self.custom_duration).id("editor-custom-duration").disabled(busy)))
            .child(div().flex().gap_4()
                .child(Checkbox::new("editor-enabled").label(crate::i18n::tr("Enabled")).checked(self.enabled).disabled(busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.enabled = *value;cx.notify();})))
                .child(Checkbox::new("editor-priority").label(crate::i18n::tr("Priority")).checked(self.priority).disabled(busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.priority = *value;cx.notify();})))
                .child(Checkbox::new("editor-logging").label(crate::i18n::tr("Log matching connections")).checked(self.logging).disabled(busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.logging = *value;cx.notify();}))))
            .child(div().text_xs().text_color(muted).child(crate::i18n::tr("Priority stops evaluation when this rule matches. Other deny/reject rules can override a non-priority allow.")));
        let mut matches = div().flex().flex_col().gap_3();
        if self.raw_mode {
            matches = matches
                .child(crate::i18n::tr("OpenSnitch match JSON"))
                .child(
                    div()
                        .id("editor-match-json")
                        .child(Textarea::new(&self.raw).disabled(busy)),
                );
        } else {
            for row in &self.conditions {
                let id = row.id;
                let is_env = selected(&row.field, FIELDS, cx) == "process.env.";
                let is_range = selected(&row.kind, KINDS, cx) == "range";
                matches = matches.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_3()
                        .border_1()
                        .border_color(border)
                        .rounded_lg()
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    Select::new(&row.field)
                                        .id(("editor-field", id))
                                        .accessibility_label(crate::i18n::tr("Match field"))
                                        .w(px(230.))
                                        .disabled(busy),
                                )
                                .child(
                                    Select::new(&row.kind)
                                        .id(("editor-match", id))
                                        .accessibility_label(crate::i18n::tr("Match type"))
                                        .w(px(230.))
                                        .disabled(busy),
                                )
                                .child(
                                    Checkbox::new(("editor-case", id))
                                        .label(crate::i18n::tr("Case sensitive"))
                                        .checked(row.sensitive)
                                        .disabled(busy)
                                        .on_change(cx.listener(
                                            move |this, value: &bool, _, cx| {
                                                if let Some(row) =
                                                    this.conditions.iter_mut().find(|r| r.id == id)
                                                {
                                                    row.sensitive = *value;
                                                }
                                                cx.notify();
                                            },
                                        )),
                                ),
                        )
                        .when(is_range, |v| v.child(
                            div().text_xs().text_color(muted).child(crate::i18n::tr("Inclusive min-max, e.g. 8000-8080. Ports run from 0 to 65535. Use Source port or Destination port.")
                            )
                        ))
                        .when(is_range && !self.backend.control.range_supported(self.session), |v| v.child(
                            div().text_xs().text_color(muted).child(crate::i18n::tr("Range support is unconfirmed for this daemon. Saving requires OpenSnitch 1.9 or an existing reported range rule.")
                            )
                        ))
                        .when(is_env, |v| {
                            v.child(
                                Input::new(&row.environment)
                                    .id(("editor-environment", id))
                                    .disabled(busy),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    div().flex_1().child(
                                        Input::new(&row.value)
                                            .id(("editor-value", id))
                                            .disabled(busy),
                                    ),
                                )
                                .child(
                                    Button::new(("editor-remove", id))
                                        .label(crate::i18n::tr("Remove"))
                                        .disabled(busy)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.conditions.retain(|row| row.id != id);
                                            cx.notify();
                                        })),
                                ),
                        ),
                );
            }
        }
        let body=div().id("rule-editor-body").flex_1().min_h_0().overflow_y_scroll().p_5().flex().flex_col().gap_4()
            .child(metadata).child(div().flex().items_center().justify_between().child(crate::i18n::tr("Match all of these conditions"))
                .child(Button::new("editor-json-toggle").label(if self.raw_mode{crate::i18n::tr("Structured conditions")}else{crate::i18n::tr("Match JSON")}).disabled(busy)
                    .on_click(cx.listener(|this,_,window,cx|{
                        if this.raw_mode {
                            match document::operator_from_text(this.raw.read(cx).value().as_ref()) {
                                Ok(op)=>{let ops=if op.r#type=="list"{op.list.clone()}else{vec![op]};if ops.len()>64||ops.iter().any(|o|o.r#type=="list"){this.error=Some(crate::i18n::tr("This match requires the JSON view to preserve all its nested conditions"));}else{this.conditions=ops.into_iter().map(|op|{let id=this.next;this.next+=1;Condition::new(id,op,window,cx)}).collect();this.raw_mode=false;}},
                                Err(e)=>this.error=Some(e)
                            }
                        } else {match this.operator(cx){Ok(op)=>{let text=serde_json::to_string_pretty(&document::operator_json(&op)).unwrap();this.raw.update(cx,|raw,cx|raw.set_value(text,window,cx));this.raw_mode=true;},Err(e)=>this.error=Some(e)}}
                        cx.notify();
                    }))))
            .child(matches)
            .when(!self.raw_mode,|v|v.child(Button::new("editor-add-condition").label(crate::i18n::tr("Add condition")).disabled(busy||self.conditions.len()>=64)
                .on_click(cx.listener(|this,_,window,cx|{let id=this.next;this.next+=1;this.conditions.push(Condition::new(id,Operator {r#type:"simple".into(),operand:"dest.host".into(),sensitive:true,..Default::default()},window,cx));cx.notify();}))))
            .child(div().text_xs().text_color(muted).child(crate::i18n::tr("Network conditions use CIDR or daemon aliases such as LAN. Watched lists refer to directories on the daemon. Checksums require daemon checksum monitoring.")));
        #[cfg(feature = "headless-tests")]
        let body = body.test_support();
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
                    .child(if self.original.is_some() {
                        crate::i18n::tr("Edit application rule")
                    } else {
                        crate::i18n::tr("New application rule")
                    }),
            )
            .when_some(self.error.clone(), |v, error| {
                v.child(
                    div()
                        .px_5()
                        .py_2()
                        .bg(cx.theme().warning.opacity(0.15))
                        .text_color(cx.theme().warning)
                        .child(crate::i18n::presentation(&error)),
                )
            })
            .child(body)
            .child(
                div()
                    .px_5()
                    .py_3()
                    .border_t_1()
                    .border_color(border)
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(if self.distributing {
                        crate::i18n::tr("Reviewing destination nodes…")
                    } else if busy {
                        crate::i18n::tr("Waiting for daemon confirmation…")
                    } else {
                        crate::i18n::tr("Saved only after the daemon confirms the change.")
                    })
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("editor-cancel")
                                    .label(crate::i18n::tr("Cancel"))
                                    .disabled(busy)
                                    .on_click(
                                        cx.listener(|_, _, window, _| window.remove_window()),
                                    ),
                            )
                            .child(
                                Button::new("editor-nodes")
                                    .label(crate::i18n::tr("Apply to nodes…"))
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.distribute(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("editor-save")
                                    .label(crate::i18n::tr("Save rule"))
                                    .disabled(busy)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.save(window, cx)),
                                    ),
                            ),
                    ),
            )
    }
}
