//! Kit controls for local policy presets and reviewed global state transitions.
use super::*;
use gpui_kit::component::{Selectable, checkbox::Checkbox};
impl FirewallView {
    pub(super) fn render_policy(&self, cx: &mut Context<Self>) -> AnyElement {
        let writable = self.writable();
        let locked = self.busy || !writable;
        let Some(loaded) = &self.loaded else {
            return div()
                .child("Load a system-firewall snapshot first")
                .into_any_element();
        };
        let mut choices = div()
            .id("fw-policy-chains")
            .w(px(260.))
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        for (g, group) in loaded.document.system_rules.iter().enumerate() {
            for (c, chain) in group.chains.iter().enumerate() {
                choices = choices.child(
                    Button::new(("fw-policy-chain", (g * 256 + c) as u64))
                        .label(format!(
                            "{} / {} / {}",
                            chain.family, chain.table, chain.name
                        ))
                        .selected(self.chain == Some((g, c)))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.chain = Some((g, c));
                            this.selected_rule = None;
                            this.offset = 0;
                            cx.notify();
                        })),
                );
            }
        }
        let selected = self.chain.and_then(|(g, c)| loaded.document.chain(g, c));
        let policy=div().flex_1().flex().flex_col().gap_3()
            .child(selected.map(|c|format!("{} · {} / {} · current draft policy {}",c.name,c.kind,c.hook,c.policy)).unwrap_or_else(||"Select a chain".into()))
            .child("Accept removes only the built-in profile's known rule IDs. Drop adds its required exceptions without replacing user rules or chain metadata. Changes stay in this draft until Review and Apply.")
            .child(div().flex().gap_2()
                .child(Button::new("fw-policy-accept").label("Use accept preset").disabled(locked||selected.is_none()).on_click(cx.listener(|this,_,window,cx|{if let Some((group,chain))=this.chain{this.mutate(Command::Policy{group,chain,drop:false},window,cx);}})))
                .child(Button::new("fw-policy-drop").label("Use drop preset").disabled(locked||selected.is_none()).on_click(cx.listener(|this,_,window,cx|{if let Some((group,chain))=this.chain{this.mutate(Command::Policy{group,chain,drop:true},window,cx);}}))))
            .child("An output drop policy can block outgoing traffic if the daemon stops. Review the complete rule set before applying it.");
        div().flex_1().min_h_0().flex().flex_col().gap_3()
            .child(Checkbox::new("fw-system-enabled").label("Enable system firewall (draft)").checked(loaded.document.enabled).disabled(locked).on_change(cx.listener(|this,value:&bool,window,cx|this.mutate(Command::Enabled(*value),window,cx))))
            .child("Disabling prepares accept policies on reported base chains. Review shows any active policy-preparation reload first, then the disabled configuration. Each stage waits for its own acknowledgement and saved cache; failure stops the sequence.")
            .child(Button::new("fw-prepare-disable").label("Prepare disabling and accept base-chain policies").disabled(locked).on_click(cx.listener(|this,_,window,cx|this.mutate(Command::Enabled(false),window,cx))))
            .child(div().flex_1().min_h_0().flex().gap_3().child(choices).child(policy))
            .child(Button::new("fw-policy-review").label("Review complete changes…").disabled(locked).on_click(cx.listener(|this,_,window,cx|this.review(window,cx))))
            .into_any_element()
    }
}
