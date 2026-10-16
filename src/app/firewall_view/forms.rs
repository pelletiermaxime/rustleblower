//! Bounded native field editors for chains, rules and ordered expression values.
use super::FirewallView;
use crate::firewall::{
    self,
    document::{Chain, Command, Document, Expression, Rule, Statement, Value},
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    Disableable,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputState},
    list::ListItem,
    tab::{Tab, TabBar},
};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy)]
pub(super) enum Target {
    Chain {
        group: usize,
        index: Option<usize>,
    },
    Rule {
        group: usize,
        chain: usize,
        index: Option<usize>,
        duplicate: bool,
    },
}
struct Field {
    id: &'static str,
    label: &'static str,
    state: Entity<InputState>,
}
impl Field {
    fn new(
        id: &'static str,
        label: &'static str,
        value: String,
        window: &mut Window,
        cx: &mut Context<FirewallView>,
    ) -> Self {
        let max = value.len().clamp(8192, crate::nodes::FIREWALL_BYTES);
        Self {
            id,
            label,
            state: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .validate(move |v, _| v.len() <= max && !v.contains('\0'))
            }),
        }
    }
    fn value(&self, cx: &App) -> String {
        self.state.read(cx).value().to_string()
    }
    fn render(&self, locked: bool) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_sm().child(self.label))
            .child(Input::new(&self.state).id(self.id).disabled(locked))
    }
}
struct ValueForm {
    index: Option<usize>,
    key: Field,
    value: Field,
}
struct StatementForm {
    index: Option<usize>,
    name: Field,
    op: Field,
    values: Vec<Value>,
    selected: Option<usize>,
    offset: usize,
    value: Option<ValueForm>,
}
pub(super) struct Form {
    target: Target,
    fields: Vec<Field>,
    enabled: bool,
    expressions: Vec<Expression>,
    selected: Option<usize>,
    offset: usize,
    statement: Option<StatementForm>,
    tab: usize,
}
fn new_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "rustleblower-{:x}-{:x}-{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
impl Form {
    pub fn keep_active(&mut self, cx: &App) -> Result<bool, String> {
        if self.statement.as_ref().is_some_and(|s| s.value.is_some()) {
            self.save_value(cx)?;
            return Ok(true);
        }
        if self.statement.is_some() {
            self.save_statement(cx)?;
            return Ok(true);
        }
        Ok(false)
    }
    pub fn cancel_active(&mut self) -> bool {
        if let Some(statement) = &mut self.statement {
            if statement.value.take().is_some() {
                return true;
            }
            self.statement = None;
            return true;
        }
        false
    }
    pub fn new(
        target: Target,
        doc: &Document,
        window: &mut Window,
        cx: &mut Context<FirewallView>,
    ) -> Result<Self, String> {
        let (fields, enabled, expressions) = match target {
            Target::Chain { group, index } => {
                let chain = match index {
                    Some(index) => doc
                        .chain(group, index)
                        .ok_or("Select a current chain")?
                        .clone(),
                    None => Chain {
                        table: "filter".into(),
                        family: "inet".into(),
                        kind: "filter".into(),
                        hook: "input".into(),
                        policy: "accept".into(),
                        ..Default::default()
                    },
                };
                let specs = [
                    ("fw-chain-name", "Chain name", chain.name),
                    ("fw-chain-table", "Table", chain.table),
                    ("fw-chain-family", "Family (inet, ip, ip6)", chain.family),
                    ("fw-chain-priority", "Priority", chain.priority),
                    ("fw-chain-type", "Type", chain.kind),
                    ("fw-chain-hook", "Hook", chain.hook),
                    ("fw-chain-policy", "Policy", chain.policy),
                ];
                (
                    specs
                        .into_iter()
                        .map(|(id, label, value)| Field::new(id, label, value, window, cx))
                        .collect(),
                    false,
                    Vec::new(),
                )
            }
            Target::Rule {
                group,
                chain,
                index,
                duplicate,
            } => {
                let chain = doc.chain(group, chain).ok_or("Select a current chain")?;
                let mut rule = match index {
                    Some(i) => chain.rules.get(i).ok_or("Select a current rule")?.clone(),
                    None => Rule {
                        enabled: true,
                        target: "accept".into(),
                        position: 0,
                        ..Default::default()
                    },
                };
                if index.is_none() || duplicate {
                    rule.uuid = new_id();
                }
                let specs = [
                    (
                        "fw-rule-id",
                        "Rule ID (legacy rules may have none)",
                        rule.uuid,
                    ),
                    ("fw-rule-description", "Description", rule.description),
                    ("fw-rule-target", "Verdict / target", rule.target),
                    (
                        "fw-rule-target-parameters",
                        "Target parameters",
                        rule.target_parameters,
                    ),
                    (
                        "fw-rule-position",
                        "Insertion position (daemon value; default 0)",
                        rule.position.to_string(),
                    ),
                    ("fw-rule-table", "Legacy table", rule.table),
                    ("fw-rule-chain", "Legacy chain", rule.chain),
                    ("fw-rule-parameters", "Legacy parameters", rule.parameters),
                ];
                (
                    specs
                        .into_iter()
                        .map(|(id, label, value)| Field::new(id, label, value, window, cx))
                        .collect(),
                    rule.enabled,
                    rule.expressions,
                )
            }
        };
        let form = Self {
            target,
            fields,
            enabled,
            expressions,
            selected: None,
            offset: 0,
            statement: None,
            tab: 0,
        };
        form.fields[0].state.focus_handle(cx).focus(window, cx);
        Ok(form)
    }
    pub fn command(&self, cx: &App) -> Result<Command, String> {
        let v = |i: usize| self.fields[i].value(cx);
        match self.target {
            Target::Chain { group, index } => Ok(Command::Chain {
                group,
                index,
                fields: Chain {
                    name: v(0),
                    table: v(1),
                    family: v(2),
                    priority: v(3),
                    kind: v(4),
                    hook: v(5),
                    policy: v(6),
                    rules: Vec::new(),
                },
            }),
            Target::Rule {
                group,
                chain,
                index,
                duplicate,
            } => Ok(Command::Rule {
                group,
                chain,
                index: if duplicate { None } else { index },
                rule: Rule {
                    uuid: v(0),
                    description: v(1),
                    target: v(2),
                    target_parameters: v(3),
                    position: v(4)
                        .parse()
                        .map_err(|_| "Position must be an unsigned 64-bit integer")?,
                    table: v(5),
                    chain: v(6),
                    parameters: v(7),
                    enabled: self.enabled,
                    expressions: self.expressions.clone(),
                },
            }),
        }
    }
    fn open_statement(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<FirewallView>,
    ) {
        let statement = index
            .and_then(|i| self.expressions.get(i))
            .and_then(|e| e.statement.clone())
            .unwrap_or_default();
        let name = Field::new(
            "fw-statement-name",
            "Statement name (tcp, ip, ct, log…)",
            statement.name,
            window,
            cx,
        );
        name.state.focus_handle(cx).focus(window, cx);
        self.statement = Some(StatementForm {
            index,
            name,
            op: Field::new(
                "fw-statement-op",
                "Operator (empty, ==, !=, in…)",
                statement.op,
                window,
                cx,
            ),
            values: statement.values,
            selected: None,
            offset: 0,
            value: None,
        });
    }
    fn save_statement(&mut self, cx: &App) -> Result<(), String> {
        let Some(s) = &self.statement else {
            return Ok(());
        };
        let name = s.name.value(cx);
        if name.is_empty() {
            return Err("Enter a statement name".into());
        }
        let expression = Expression {
            statement: Some(Statement {
                name,
                op: s.op.value(cx),
                values: s.values.clone(),
            }),
        };
        self.expressions = self.checked_expressions(s.index, expression, cx)?;
        self.statement = None;
        Ok(())
    }
    fn open_value(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<FirewallView>,
    ) {
        let Some(statement) = &mut self.statement else {
            return;
        };
        let value = index
            .and_then(|i| statement.values.get(i))
            .cloned()
            .unwrap_or_default();
        let key = Field::new(
            "fw-value-key",
            "Value key (dport, saddr, state…)",
            value.key,
            window,
            cx,
        );
        key.state.focus_handle(cx).focus(window, cx);
        statement.value = Some(ValueForm {
            index,
            key,
            value: Field::new("fw-value-value", "Value", value.value, window, cx),
        });
    }
    fn save_value(&mut self, cx: &App) -> Result<(), String> {
        let Some(statement) = &self.statement else {
            return Ok(());
        };
        let Some(form) = &statement.value else {
            return Ok(());
        };
        let value = Value {
            key: form.key.value(cx),
            value: form.value.value(cx),
        };
        let mut values = statement.values.clone();
        if let Some(index) = form.index {
            *values.get_mut(index).ok_or("Value selection changed")? = value;
        } else {
            if statement.values.len() >= firewall::MAX_VALUES {
                return Err("Value limit reached".into());
            }
            values.push(value);
        }
        self.checked_expressions(
            statement.index,
            Expression {
                statement: Some(Statement {
                    name: statement.name.value(cx),
                    op: statement.op.value(cx),
                    values: values.clone(),
                }),
            },
            cx,
        )?;
        if let Some(statement) = &mut self.statement {
            statement.values = values;
            statement.value = None;
        }
        Ok(())
    }
    fn checked_expressions(
        &self,
        index: Option<usize>,
        expression: Expression,
        cx: &App,
    ) -> Result<Vec<Expression>, String> {
        let mut expressions = self.expressions.clone();
        if let Some(index) = index {
            *expressions
                .get_mut(index)
                .ok_or("Statement selection changed")? = expression;
        } else {
            if expressions.len() >= firewall::MAX_EXPRESSIONS {
                return Err("Expression limit reached".into());
            }
            expressions.push(expression);
        }
        let v = |i: usize| self.fields[i].value(cx);
        let rule = Rule {
            table: v(5),
            chain: v(6),
            uuid: v(0),
            description: v(1),
            parameters: v(7),
            target: v(2),
            target_parameters: v(3),
            position: u64::MAX,
            enabled: self.enabled,
            expressions: expressions.clone(),
        };
        let config = crate::protocol::SysFirewall {
            version: 1,
            system_rules: vec![crate::protocol::FwChains {
                rule: Some(rule.into()),
                chains: Vec::new(),
            }],
            ..Default::default()
        };
        firewall::admit(&config)?;
        Ok(expressions)
    }
    pub fn render(&self, locked: bool, cx: &mut Context<FirewallView>) -> Div {
        if let Some(statement) = &self.statement {
            if let Some(value) = &statement.value {
                return div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child("Expression value")
                    .child(value.key.render(locked))
                    .child(value.value.render(locked))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("fw-value-save")
                                    .label("Keep value")
                                    .disabled(locked)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(form) = &mut this.form {
                                            match form.save_value(cx) {
                                                Ok(()) => {
                                                    this.error = None;
                                                    this.focus.focus(window, cx);
                                                }
                                                Err(e) => this.error = Some(e),
                                            }
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("fw-value-cancel")
                                    .label("Cancel value")
                                    .disabled(locked)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(s) =
                                            this.form.as_mut().and_then(|f| f.statement.as_mut())
                                        {
                                            s.value = None;
                                        }
                                        this.focus.focus(window, cx);
                                        cx.notify();
                                    })),
                            ),
                    );
            }
            let offset = statement.offset.min(statement.values.len());
            let end = (offset + firewall::PAGE).min(statement.values.len());
            let total = statement.values.len();
            let selected = statement.selected;
            return div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_2()
                .child("Expression statement")
                .child(statement.name.render(locked))
                .child(statement.op.render(locked))
                .child(format!(
                    "Ordered values · {}–{} of {total}",
                    if total == 0 { 0 } else { offset + 1 },
                    end
                ))
                .child(
                    uniform_list(
                        "fw-value-list",
                        end - offset,
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            let Some(s) = this.form.as_ref().and_then(|f| f.statement.as_ref())
                            else {
                                return vec![];
                            };
                            range
                                .map(|i| {
                                    let index = offset + i;
                                    let value = &s.values[index];
                                    ListItem::new(("fw-value-row", index))
                                        .selected(selected == Some(index))
                                        .child(format!(
                                            "{} = {}",
                                            crate::history::bounded(&value.key, 128),
                                            crate::history::bounded(&value.value, 256)
                                        ))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(s) = this
                                                .form
                                                .as_mut()
                                                .and_then(|f| f.statement.as_mut())
                                            {
                                                s.selected = Some(index);
                                            }
                                            cx.notify();
                                        }))
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .flex_1()
                    .min_h_0(),
                )
                .child(value_actions(locked, selected, offset, end, total, cx))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("fw-statement-save")
                                .label("Keep statement")
                                .disabled(locked)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Some(form) = &mut this.form {
                                        match form.save_statement(cx) {
                                            Ok(()) => {
                                                this.error = None;
                                                this.focus.focus(window, cx);
                                            }
                                            Err(e) => this.error = Some(e),
                                        }
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("fw-statement-cancel")
                                .label("Cancel statement")
                                .disabled(locked)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Some(form) = &mut this.form {
                                        form.statement = None;
                                    }
                                    this.focus.focus(window, cx);
                                    cx.notify();
                                })),
                        ),
                );
        }
        let chain = matches!(self.target, Target::Chain { .. });
        let mut body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(if chain { "Chain draft" } else { "Rule draft" });
        if !chain {
            body = body.child(
                TabBar::new("fw-rule-tabs")
                    .selected_index(self.tab)
                    .on_click(cx.listener(|this, index: &usize, window, cx| {
                        if let Some(form) = &mut this.form {
                            form.tab = *index;
                            this.focus.focus(window, cx);
                        }
                        cx.notify();
                    }))
                    .child(Tab::new().label("Rule"))
                    .child(Tab::new().label("Expressions"))
                    .child(Tab::new().label("Legacy fields")),
            );
        }
        if chain || self.tab != 1 {
            let fields = if chain {
                0..self.fields.len()
            } else if self.tab == 0 {
                0..5
            } else {
                5..8
            };
            body = body.child(
                div()
                    .id("fw-form-fields")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(fields.map(|i| self.fields[i].render(locked)))
                    .when(!chain && self.tab == 0, |view| {
                        view.child(
                            Checkbox::new("fw-rule-enabled")
                                .label("Rule enabled")
                                .checked(self.enabled)
                                .disabled(locked)
                                .on_change(cx.listener(|this, value: &bool, _, cx| {
                                    if let Some(form) = &mut this.form {
                                        form.enabled = *value;
                                    }
                                    cx.notify();
                                })),
                        )
                    }),
            );
        } else {
            let offset = self.offset.min(self.expressions.len());
            let end = (offset + firewall::PAGE).min(self.expressions.len());
            let total = self.expressions.len();
            let selected = self.selected;
            body = body
                .child(format!(
                    "Ordered expressions · {}–{} of {total}",
                    if total == 0 { 0 } else { offset + 1 },
                    end
                ))
                .child(
                    uniform_list(
                        "fw-expression-list",
                        end - offset,
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            let Some(form) = &this.form else {
                                return vec![];
                            };
                            range
                                .map(|i| {
                                    let index = offset + i;
                                    let statement = form.expressions[index].statement.as_ref();
                                    ListItem::new(("fw-expression-row", index))
                                        .selected(selected == Some(index))
                                        .child(
                                            statement
                                                .map(|s| {
                                                    format!(
                                                        "{} {} · {} values",
                                                        crate::history::bounded(&s.name, 128),
                                                        crate::history::bounded(&s.op, 64),
                                                        s.values.len()
                                                    )
                                                })
                                                .unwrap_or_else(|| {
                                                    "Missing statement; edit to repair".into()
                                                }),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(form) = &mut this.form {
                                                form.selected = Some(index);
                                            }
                                            cx.notify();
                                        }))
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .flex_1()
                    .min_h_0(),
                )
                .child(expression_actions(locked, selected, offset, end, total, cx));
        }
        body.child(
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("fw-form-save")
                        .label("Keep in draft")
                        .disabled(locked)
                        .on_click(cx.listener(|this, _, window, cx| this.commit_form(window, cx))),
                )
                .child(
                    Button::new("fw-form-cancel")
                        .label("Cancel form")
                        .disabled(locked)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.form = None;
                            this.error = None;
                            this.focus.focus(window, cx);
                            cx.notify();
                        })),
                ),
        )
    }
}

fn expression_actions(
    locked: bool,
    selected: Option<usize>,
    offset: usize,
    end: usize,
    total: usize,
    cx: &mut Context<FirewallView>,
) -> Div {
    div()
        .flex()
        .flex_wrap()
        .gap_2()
        .child(
            Button::new("fw-expression-new")
                .label("Add statement…")
                .disabled(locked || total >= firewall::MAX_EXPRESSIONS)
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(form) = &mut this.form {
                        form.open_statement(None, window, cx);
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-expression-edit")
                .label("Edit statement…")
                .disabled(locked || selected.is_none())
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(form) = &mut this.form {
                        form.open_statement(form.selected, window, cx);
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-expression-delete")
                .label("Remove statement")
                .disabled(locked || selected.is_none())
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(form) = &mut this.form
                        && let Some(i) = form.selected.take()
                    {
                        form.expressions.remove(i);
                        form.offset = 0;
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-expression-up")
                .label("Move up")
                .disabled(locked || selected.is_none_or(|i| i == 0))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(form) = &mut this.form
                        && let Some(i) = form.selected
                        && i > 0
                    {
                        form.expressions.swap(i, i - 1);
                        form.selected = Some(i - 1);
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-expression-down")
                .label("Move down")
                .disabled(locked || selected.is_none_or(|i| i + 1 >= total))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(form) = &mut this.form
                        && let Some(i) = form.selected
                        && i + 1 < form.expressions.len()
                    {
                        form.expressions.swap(i, i + 1);
                        form.selected = Some(i + 1);
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-expression-previous")
                .label("Previous")
                .disabled(locked || offset == 0)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(form) = &mut this.form {
                        form.offset = form.offset.saturating_sub(firewall::PAGE);
                        form.selected = None;
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-expression-next")
                .label("Next")
                .disabled(locked || end >= total)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(form) = &mut this.form {
                        form.offset += firewall::PAGE;
                        form.selected = None;
                    }
                    cx.notify();
                })),
        )
}
fn value_actions(
    locked: bool,
    selected: Option<usize>,
    offset: usize,
    end: usize,
    total: usize,
    cx: &mut Context<FirewallView>,
) -> Div {
    div()
        .flex()
        .flex_wrap()
        .gap_2()
        .child(
            Button::new("fw-value-new")
                .label("Add value…")
                .disabled(locked || total >= firewall::MAX_VALUES)
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(form) = &mut this.form {
                        form.open_value(None, window, cx);
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-value-edit")
                .label("Edit value…")
                .disabled(locked || selected.is_none())
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(form) = &mut this.form {
                        let index = form.statement.as_ref().and_then(|s| s.selected);
                        form.open_value(index, window, cx);
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-value-delete")
                .label("Remove value")
                .disabled(locked || selected.is_none())
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(s) = this.form.as_mut().and_then(|f| f.statement.as_mut())
                        && let Some(i) = s.selected.take()
                    {
                        s.values.remove(i);
                        s.offset = 0;
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-value-up")
                .label("Move up")
                .disabled(locked || selected.is_none_or(|i| i == 0))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(s) = this.form.as_mut().and_then(|f| f.statement.as_mut())
                        && let Some(i) = s.selected
                        && i > 0
                    {
                        s.values.swap(i, i - 1);
                        s.selected = Some(i - 1);
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-value-down")
                .label("Move down")
                .disabled(locked || selected.is_none_or(|i| i + 1 >= total))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(s) = this.form.as_mut().and_then(|f| f.statement.as_mut())
                        && let Some(i) = s.selected
                        && i + 1 < s.values.len()
                    {
                        s.values.swap(i, i + 1);
                        s.selected = Some(i + 1);
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-value-previous")
                .label("Previous")
                .disabled(locked || offset == 0)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(s) = this.form.as_mut().and_then(|f| f.statement.as_mut()) {
                        s.offset = s.offset.saturating_sub(firewall::PAGE);
                        s.selected = None;
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("fw-value-next")
                .label("Next")
                .disabled(locked || end >= total)
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(s) = this.form.as_mut().and_then(|f| f.statement.as_mut()) {
                        s.offset += firewall::PAGE;
                        s.selected = None;
                    }
                    cx.notify();
                })),
        )
}
