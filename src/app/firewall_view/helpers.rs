//! Native Kit controls for optional statement/verdict constructors.
use super::{Field, FirewallView};
use crate::firewall::{
    document::Expression,
    helpers::{self, KINDS, Kind, OPERATORS, VERDICTS},
    suggestions::Catalogue,
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    Disableable, IndexPath,
    button::Button,
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
type Picker = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<SharedString>>>>;
pub(super) enum Guided {
    Statement {
        kind: Picker,
        options: Vec<Picker>,
        operator: Picker,
        value: Field,
        catalogue: std::sync::Arc<Catalogue>,
        services: Picker,
        interfaces: Picker,
    },
    Verdict {
        target: Picker,
        value: Field,
    },
}
pub(super) enum Kept {
    Expression(Expression),
    Verdict(String, String),
}
fn picker(
    labels: &[&str],
    selected: usize,
    window: &mut Window,
    cx: &mut Context<FirewallView>,
) -> Picker {
    let labels = labels
        .iter()
        .map(|s| crate::i18n::LocalizedOption::new(SharedString::from(*s), *s))
        .collect::<Vec<_>>();
    let picker = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(labels),
            Some(IndexPath::new(selected)),
            window,
            cx,
        )
        .searchable(true)
    });
    cx.subscribe(
        &picker,
        |_, _, _: &SelectEvent<SearchableVec<crate::i18n::LocalizedOption<SharedString>>>, cx| {
            cx.notify()
        },
    )
    .detach();
    picker
}
fn value(picker: &Picker, cx: &App) -> String {
    picker
        .read(cx)
        .selected_value()
        .map(ToString::to_string)
        .unwrap_or_default()
}
fn suggestion_picker(
    labels: Vec<SharedString>,
    window: &mut Window,
    cx: &mut Context<FirewallView>,
) -> Picker {
    let picker = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(
                labels
                    .into_iter()
                    .map(|label| {
                        crate::i18n::LocalizedOption::raw(label.clone(), label.to_string())
                    })
                    .collect::<Vec<_>>(),
            ),
            None,
            window,
            cx,
        )
        .searchable(true)
    });
    cx.subscribe(
        &picker,
        |_, _, _: &SelectEvent<SearchableVec<crate::i18n::LocalizedOption<SharedString>>>, cx| {
            cx.notify()
        },
    )
    .detach();
    picker
}
// Kit Select's host uses size_full; constrain its row so the hit bounds match
// the painted trigger instead of consuming the helper's remaining height.
fn select_row(picker: &Picker, id: &'static str, locked: bool) -> Div {
    div()
        .h_8()
        .flex_shrink_0()
        .child(Select::new(picker).id(id).disabled(locked))
}
impl Guided {
    pub(super) fn statement(
        catalogue: std::sync::Arc<Catalogue>,
        window: &mut Window,
        cx: &mut Context<FirewallView>,
    ) -> Self {
        Self::Statement {
            services: suggestion_picker(
                catalogue
                    .services
                    .iter()
                    .map(|s| SharedString::from(s.label()))
                    .collect(),
                window,
                cx,
            ),
            interfaces: suggestion_picker(
                catalogue
                    .interfaces
                    .iter()
                    .cloned()
                    .map(SharedString::from)
                    .collect(),
                window,
                cx,
            ),
            catalogue,
            kind: picker(
                &KINDS.iter().map(|k| k.label()).collect::<Vec<_>>(),
                0,
                window,
                cx,
            ),
            options: KINDS
                .iter()
                .map(|k| picker(k.options(), 0, window, cx))
                .collect(),
            operator: picker(OPERATORS, 0, window, cx),
            value: Field::new("fw-helper-value", "Value", String::new(), window, cx),
        }
    }
    pub(super) fn refresh_suggestions(
        &mut self,
        next: std::sync::Arc<Catalogue>,
        window: &mut Window,
        cx: &mut Context<FirewallView>,
    ) {
        if let Self::Statement {
            catalogue,
            services,
            interfaces,
            ..
        } = self
        {
            *services = suggestion_picker(
                next.services
                    .iter()
                    .map(|s| SharedString::from(s.label()))
                    .collect(),
                window,
                cx,
            );
            *interfaces = suggestion_picker(
                next.interfaces
                    .iter()
                    .cloned()
                    .map(SharedString::from)
                    .collect(),
                window,
                cx,
            );
            *catalogue = next;
        }
    }
    pub(super) fn verdict(
        current: &str,
        window: &mut Window,
        cx: &mut Context<FirewallView>,
    ) -> Self {
        Self::Verdict {
            target: picker(
                VERDICTS,
                VERDICTS.iter().position(|v| *v == current).unwrap_or(0),
                window,
                cx,
            ),
            value: Field::new(
                "fw-helper-verdict-value",
                "Parameter (:port, IP:port or queue number)",
                String::new(),
                window,
                cx,
            ),
        }
    }
    fn kind(picker: &Picker, cx: &App) -> Kind {
        let label = value(picker, cx);
        KINDS
            .iter()
            .copied()
            .find(|k| k.label() == label)
            .unwrap_or(KINDS[0])
    }
    pub(super) fn result(&self, hook: &str, cx: &App) -> Result<Kept, String> {
        match self {
            Self::Statement {
                kind,
                options,
                operator,
                value: input,
                catalogue,
                ..
            } => {
                let kind = Self::kind(kind, cx);
                let index = KINDS.iter().position(|k| *k == kind).unwrap();
                let protocol = value(&options[index], cx);
                let mut text = input.value(cx);
                if matches!(kind, Kind::DestinationPort | Kind::SourcePort)
                    && let Some(port) = catalogue.port(&text, &protocol)?
                {
                    text = port.to_string();
                }
                helpers::statement(kind, &protocol, &value(operator, cx), &text)
                    .map(Kept::Expression)
            }
            Self::Verdict {
                target,
                value: input,
            } => helpers::verdict(&value(target, cx), &input.value(cx), hook)
                .map(|(target, parameters)| Kept::Verdict(target, parameters)),
        }
    }
    pub(super) fn render(&self, hook: &str, locked: bool, cx: &mut Context<FirewallView>) -> Div {
        let mut body = div().flex_1().min_h_0().flex().flex_col().gap_3();
        body=match self {
            Self::Statement{kind,options,operator,value:input,catalogue,services,interfaces}=>{
                let selected=Self::kind(kind,cx);
                let index=KINDS.iter().position(|k|*k==selected).unwrap();
                body.child(crate::i18n::tr("Guided statement"))
                    .child(select_row(kind,"fw-helper-kind",locked))
                    .when(selected.options().len()>1,|v|v.child(select_row(&options[index],"fw-helper-option",locked)))
                    .when(selected.has_operator(),|v|v.child(select_row(operator,"fw-helper-operator",locked)))
                    .child(div().text_sm().child(crate::i18n::tr(selected.hint())))
                    .child(input.render(locked))
                    .when(matches!(selected,Kind::DestinationPort|Kind::SourcePort),|v|v
                        .child(div().text_sm().child(crate::i18n::tr("Local services (using a suggestion also sets its protocol)")))
                        .child(select_row(services,"fw-helper-service",locked))
                        .child(Button::new("fw-helper-use-service").label(crate::i18n::tr("Use port and protocol")).disabled(locked || services.read(cx).selected_value().is_none()).on_click(cx.listener(|this,_,window,cx|{
                            if let Some(form)=&mut this.form
                                && let Some(Self::Statement{kind,options,value:input,catalogue,services,..})=&mut form.helper {
                                let label=value(services,cx);
                                if let Some(service)=catalogue.services.iter().find(|s|s.label()==label) {
                                    let selected=Self::kind(kind,cx);
                                    let index=KINDS.iter().position(|k|*k==selected).unwrap();
                                    if let Some(protocol)=selected.options().iter().position(|p|*p==service.protocol) {
                                        options[index].update(cx,|state,cx|state.set_selected_index(Some(IndexPath::new(protocol)),window,cx));
                                        input.state.update(cx,|state,cx|state.set_value(service.port.to_string(),window,cx));
                                    }
                                }
                            }
                            cx.notify();
                        }))))
                    .when(matches!(selected,Kind::InputInterface|Kind::OutputInterface),|v|v
                        .child(div().text_sm().child(crate::i18n::tr("Interfaces on this computer")))
                        .child(select_row(interfaces,"fw-helper-interface",locked))
                        .child(Button::new("fw-helper-use-interface").label(crate::i18n::tr("Use interface")).disabled(locked || interfaces.read(cx).selected_value().is_none()).on_click(cx.listener(|this,_,window,cx|{
                            if let Some(form)=&mut this.form
                                && let Some(Self::Statement{value:input,interfaces,..})=&mut form.helper {
                                let name=value(interfaces,cx);
                                if !name.is_empty() {input.state.update(cx,|state,cx|state.set_value(name,window,cx));}
                            }
                            cx.notify();
                        }))))
                    .when_some(catalogue.warning.as_ref(),|v,message|v.child(div().text_sm().child(message.clone())))
                    .child(Button::new("fw-helper-reload-suggestions").label(crate::i18n::tr("Reload local suggestions")).disabled(locked).on_click(cx.listener(|this,_,window,cx|this.load_suggestions(window,cx))))
            },
            Self::Verdict{target,value:input}=>{
                body.child(crate::i18n::tr("Guided verdict"))
                    .child(crate::i18n::tr_args("Selected chain hook: %{arg0}", &[("arg0", &if hook.is_empty(){crate::i18n::tr("regular chain (no base hook)")}else{hook.to_owned()})]))
                    .child(select_row(target,"fw-helper-verdict",locked))
                    .child(div().text_sm().child(crate::i18n::tr("Queue uses a number (0–65535). Redirect/TPROXY/DNAT/SNAT use :port or IP:port. Other choices take no parameters. Advanced verdict parameters remain editable in the rule fields.")))
                    .child(input.render(locked))
            },
        };
        body.child(div().text_sm().child(crate::i18n::tr("Keep changes only this local rule draft. Review the complete firewall before applying it to the captured node.")))
            .child(div().flex().gap_2()
                .child(Button::new("fw-helper-keep").label(crate::i18n::tr("Keep in rule draft")).disabled(locked).on_click(cx.listener(|this,_,window,cx|{
                    if let Some(form)=&mut this.form {
                        match form.keep_helper(window,cx) {Ok(())=>{this.error=None;this.focus.focus(window,cx);},Err(e)=>this.error=Some(e)}
                    }
                    cx.notify();
                })))
                .child(Button::new("fw-helper-cancel").label(crate::i18n::tr("Cancel helper")).disabled(locked).on_click(cx.listener(|this,_,window,cx|{
                    if let Some(form)=&mut this.form {form.helper=None;}
                    this.error=None;this.focus.focus(window,cx);cx.notify();
                }))))
    }
}
