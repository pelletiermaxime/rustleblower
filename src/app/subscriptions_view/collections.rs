use super::*;
use crate::subscriptions::collections as data;
pub(super) struct Collections {
    pub input: Entity<InputState>,
    pub query: data::Query,
    pub page: Option<data::Page>,
}
impl Collections {
    pub fn new(window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search group or list type…")
                .validate(|s, _| s.len() <= 256)
        });
        cx.subscribe(&input, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                if let Some(view) = &mut this.subscriptions.collections {
                    view.query.offset = 0;
                }
                this.subscriptions.dirty = true;
                cx.notify();
            }
        })
        .detach();
        Self {
            input,
            query: data::Query::default(),
            page: None,
        }
    }
}
impl Dashboard {
    pub(in crate::app) fn open_collections(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.subscriptions.pending.is_some()
            || self.subscriptions.rule_busy
            || self.subscriptions.form.is_some()
            || self.subscriptions.inspection.is_some()
            || self.subscriptions.bulk.is_some()
            || self.subscriptions.transfer.is_some()
        {
            return;
        }
        let view = Collections::new(window, cx);
        view.input.focus_handle(cx).focus(window, cx);
        self.subscriptions.collections = Some(view);
        self.subscriptions.dirty = true;
        self.subscriptions.error = None;
        cx.notify();
    }
    pub(super) fn render_collections(&mut self, cx: &mut Context<Self>) -> Div {
        let view = self.subscriptions.collections.as_ref().unwrap();
        let busy = self.subscriptions.pending.is_some() || self.subscriptions.rule_busy;
        let node = self.backend.status().session;
        let ready = node
            .as_ref()
            .is_some_and(|s| s.notifications && self.backend.history.health.catalogues.ready(s.id));
        let mut rows = div()
            .id("subs-collection-rows")
            .flex_1()
            .min_h(px(96.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        if let Some(page) = &view.page {
            if page.rows.is_empty() {
                rows=rows.child("No matching collections. Add groups to subscriptions and refresh their data to populate group collections.");
            }
            for (index, collection) in page.rows.iter().enumerate() {
                let review = collection.clone();
                let inspect = review.clone();
                rows = rows.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .items_center()
                        .child(div().flex_1().child(format!(
                                "{} · {} · {} active sources",
                                collection
                                    .group
                                    .as_deref()
                                    .unwrap_or("All enabled subscriptions"),
                                collection.format.directory(),
                                collection.members
                            )))
                        .child(
                            Button::new(("subs-collection-review", index))
                                .label("Review new rule…")
                                .disabled(busy || !ready || self.rules.editor_window.is_some())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.prepare_collection_rules(review.clone(), true, window, cx)
                                })),
                        )
                        .child(
                            Button::new(("subs-collection-inspect", index))
                                .label("Associated rules…")
                                .disabled(busy || node.is_none())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.prepare_collection_rules(
                                        inspect.clone(),
                                        false,
                                        window,
                                        cx,
                                    )
                                })),
                        ),
                );
            }
        } else {
            rows = rows.child("Loading collections…");
        }
        let offset = view.query.offset;
        let total = view.page.as_ref().map_or(0, |p| p.total);
        let mut sorts = div().flex().flex_wrap().gap_2().child("Sort all matches:");
        for (index, label, sort) in [
            (0usize, "Group", data::Sort::Group),
            (1, "Type", data::Sort::Kind),
            (2, "Active sources", data::Sort::Members),
        ] {
            sorts = sorts.child(
                Button::new(("subs-collection-sort", index))
                    .label(label)
                    .selected(
                        std::mem::discriminant(&view.query.sort) == std::mem::discriminant(&sort),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(view) = &mut this.subscriptions.collections {
                            view.query.sort = sort;
                            view.query.offset = 0;
                        }
                        this.subscriptions.dirty = true;
                        cx.notify();
                    })),
            );
        }
        sorts = sorts.child(
            Checkbox::new("subs-collection-descending")
                .label("Descending")
                .checked(view.query.descending)
                .on_change(cx.listener(|this, value: &bool, _, cx| {
                    if let Some(view) = &mut this.subscriptions.collections {
                        view.query.descending = *value;
                        view.query.offset = 0;
                    }
                    this.subscriptions.dirty = true;
                    cx.notify();
                })),
        );
        div().flex_1().min_h_0().flex().flex_col().gap_3()
            .child("Rule collections")
            .child(div().text_sm().child("Existing collection rules follow enabled membership and refreshed data. Each new rule is reviewed for the selected local node and your user ID. Empty collections match nothing until sources are added."))
            .child(div().text_sm().child(format!("Target node: {}",node.as_ref().map(|s|s.name.as_str()).unwrap_or("none selected; choose a local node on Nodes"))))
            .child(Input::new(&view.input).id("subs-collection-search")).child(sorts).child(rows)
            .child(div().flex().flex_wrap().gap_2()
                .child(Button::new("subs-collection-previous").label("Previous").disabled(busy||offset==0).on_click(cx.listener(|this,_,_,cx|{if let Some(view)=&mut this.subscriptions.collections{view.query.offset=view.query.offset.saturating_sub(PAGE);}this.subscriptions.dirty=true;cx.notify();})))
                .child(format!("{}–{} of {total}",if total==0{0}else{offset+1},(offset+PAGE).min(total)))
                .child(Button::new("subs-collection-next").label("Next").disabled(busy||offset+PAGE>=total).on_click(cx.listener(|this,_,_,cx|{if let Some(view)=&mut this.subscriptions.collections{view.query.offset+=PAGE;}this.subscriptions.dirty=true;cx.notify();})))
                .child(Button::new("subs-collection-back").label("Back to subscriptions").disabled(busy).on_click(cx.listener(|this,_,window,cx|{this.subscriptions.collections=None;this.subscriptions.dirty=true;this.subscriptions.input.focus_handle(cx).focus(window,cx);cx.notify();}))))
    }
}
