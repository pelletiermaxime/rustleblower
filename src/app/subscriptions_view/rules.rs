//! Captured rule targets reuse the acknowledged editor and bounded snapshots.
use super::*;
use crate::{
    protocol::Rule,
    subscriptions::{attachment, collections},
};
#[derive(Clone)]
enum TargetKey {
    Single(u64, u64),
    Collection(Option<String>, Format),
}
enum Target {
    Single(Box<Row>),
    Collection(Box<collections::Collection>),
}
impl Target {
    fn label(&self) -> String {
        match self {
            Self::Single(row) => row.spec.name.clone(),
            Self::Collection(c) => format!(
                "{} · {}",
                c.group
                    .as_ref()
                    .map(|g| format!("Group {g}"))
                    .unwrap_or_else(|| "All enabled subscriptions".into()),
                c.format.directory()
            ),
        }
    }
    fn directory(&self) -> Result<std::path::PathBuf, String> {
        match self {
            Self::Single(row) => attachment::directory(
                row.publication
                    .as_ref()
                    .ok_or("Refresh this URL and format before inspecting rules")?,
            ),
            Self::Collection(c) => Ok(c.directory.clone()),
        }
    }
    fn operand(&self) -> &str {
        match self {
            Self::Single(row) => row.spec.format.operand(),
            Self::Collection(c) => c.format.operand(),
        }
    }
    fn draft(&self) -> Result<Rule, String> {
        match self {
            Self::Single(row) => attachment::draft(row),
            Self::Collection(c) => collections::draft(c),
        }
    }
}
pub(super) struct Inspection {
    target: Target,
    key: TargetKey,
    node: String,
    session: u64,
    page: attachment::AttachedPage,
}
#[cfg(all(test, feature = "headless-tests"))]
impl Inspection {
    pub(super) fn counts(&self) -> (usize, usize, usize) {
        (self.page.total, self.page.offset, self.page.rules.len())
    }
}
impl Dashboard {
    pub(super) fn prepare_subscription_rules(
        &mut self,
        create: bool,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.subscriptions.selected() {
            self.prepare_rule_target(
                TargetKey::Single(row.id, row.revision),
                create,
                offset,
                None,
                window,
                cx,
            );
        }
    }
    pub(super) fn prepare_collection_rules(
        &mut self,
        collection: collections::Collection,
        create: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prepare_rule_target(
            TargetKey::Collection(collection.group, collection.format),
            create,
            0,
            None,
            window,
            cx,
        );
    }
    fn prepare_rule_target(
        &mut self,
        key: TargetKey,
        create: bool,
        offset: usize,
        session: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.subscriptions.rule_busy || self.subscriptions.pending.is_some() {
            return;
        }
        let target = match session {
            Some(id) => self.backend.session(id),
            None => self.backend.status().session,
        };
        let Some(node) = target else {
            self.subscriptions.error = Some(
                "The selected local daemon is unavailable; reopen the review from its node".into(),
            );
            cx.notify();
            return;
        };
        let request = match &key {
            TargetKey::Single(id, _) => Request::Get(*id),
            TargetKey::Collection(group, format) => Request::Collection {
                group: group.clone(),
                format: format.clone(),
            },
        };
        let reply = match self.backend.subscriptions.request(request) {
            Ok(reply) => reply,
            Err(error) => {
                self.subscriptions.error = Some(error);
                cx.notify();
                return;
            }
        };
        let database = self.backend.history.database_path().to_owned();
        let guard = self.subscriptions.begin_rule_work();
        self.subscriptions.error = None;
        cx.notify();
        cx.spawn_in(window,async move|this,cx|{let _guard=guard;
            let result=async{
                let reply=reply.await.map_err(|_|"Subscription worker stopped")??;
                let target=match (&key,reply){
                    (TargetKey::Single(_,revision),Reply::Row(row)) if row.revision==*revision=>Target::Single(row),
                    (TargetKey::Collection(..),Reply::Collection(c))=>Target::Collection(c),
                    _=>return Err("Subscription or collection changed; refresh before reviewing rules".into()),
                };
                let directory=target.directory()?;let operand=target.operand().to_owned();let name=node.name.clone();
                let page=if create{attachment::AttachedPage{rules:Vec::new(),total:0,offset:0}}else{
                    cx.background_executor().spawn(async move{attachment::page_for_directory(&database,&name,&operand,&directory,offset).map_err(|e|format!("Cannot inspect associated rules: {e:#}"))}).await?
                };
                Ok::<_,String>((target,page))
            }.await;
            let _=this.update_in(cx,|this,window,cx|{
                this.subscriptions.finish_rule_work();
                match result{
                    Ok((target,page))=>{
                        if create{
                            if !this.backend.session(node.id).is_some_and(|s|s.notifications&&this.backend.history.health.catalogues.ready(s.id)){
                                this.subscriptions.error=Some("The captured daemon changed or its inventory is unavailable; reopen the review".into());
                            }else if this.rules.editor_window.is_some(){this.subscriptions.error=Some("Close the current rule editor first".into());}
                            else{match target.draft().and_then(|rule|super::super::rule_editor::open(this.backend.clone(),node.id,rule,None,cx).map_err(|e|e.to_string())){Ok(window)=>this.rules.editor_window=Some(window),Err(error)=>this.subscriptions.error=Some(error)}}
                        }else{this.subscriptions.inspection=Some(Inspection{target,key,node:node.name,session:node.id,page});this.focus.focus(window,cx);}
                    }
                    Err(error)=>this.subscriptions.error=Some(error),
                }
                cx.notify();
            });
        }).detach();
    }
    fn edit_attached_rule(
        &mut self,
        session: u64,
        rule: Rule,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.rules.editor_window.is_some() || self.subscriptions.rule_busy {
            return;
        }
        let Some(node) = self
            .backend
            .session(session)
            .filter(|s| s.notifications && self.backend.history.health.catalogues.ready(s.id))
        else {
            self.subscriptions.error =
                Some("This daemon is unavailable; refresh before editing".into());
            cx.notify();
            return;
        };
        let database = self.backend.history.database_path().to_owned();
        let task = cx.background_executor().spawn(async move {
            let current =
                crate::rules::transfer::find_confirmed_rule(&database, &node.name, &rule.name)
                    .map_err(|e| format!("Cannot reread current rule: {e:#}"))?;
            match current {
                Some(current) if current == rule => Ok(current),
                _ => Err(
                    "Rule changed or disappeared since this snapshot; refresh the associated rules"
                        .to_string(),
                ),
            }
        });
        let guard = self.subscriptions.begin_rule_work();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let _guard = guard;
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.subscriptions.finish_rule_work();
                match result {
                    Ok(rule)
                        if this.rules.editor_window.is_none()
                            && this.backend.session(session).is_some_and(|s| {
                                s.notifications
                                    && this.backend.history.health.catalogues.ready(s.id)
                            }) =>
                    {
                        match super::super::rule_editor::open(
                            this.backend.clone(),
                            session,
                            rule.clone(),
                            Some(rule),
                            cx,
                        ) {
                            Ok(window) => this.rules.editor_window = Some(window),
                            Err(error) => this.subscriptions.error = Some(error.to_string()),
                        }
                    }
                    Ok(_) => {
                        this.subscriptions.error = Some(
                            "Daemon changed before opening the editor; refresh the snapshot".into(),
                        )
                    }
                    Err(error) => this.subscriptions.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_subscription_rules(&mut self, cx: &mut Context<Self>) -> Div {
        let view = self.subscriptions.inspection.as_ref().unwrap();
        let session = view.session;
        let mut rows = div()
            .id("subs-associated-rows")
            .flex_1()
            .min_h(px(96.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        for (index, rule) in view.page.rules.iter().enumerate() {
            let rule = rule.clone();
            let label = format!(
                "{} · {} · {} · {}",
                rule.name,
                rule.action,
                rule.duration,
                if rule.enabled { "enabled" } else { "disabled" }
            );
            rows = rows.child(
                Button::new(("subs-associated-edit", index))
                    .label(label)
                    .disabled(self.rules.editor_window.is_some() || self.subscriptions.rule_busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_attached_rule(session, rule.clone(), window, cx)
                    })),
            );
        }
        if view.page.rules.is_empty() {
            rows = rows.child("No confirmed rules reference this directory on this node.");
        }
        let offset = view.page.offset;
        let total = view.page.total;
        let previous = view.key.clone();
        let next = previous.clone();
        let refresh = previous.clone();
        div().flex_1().min_h_0().flex().flex_col().gap_3()
            .child(format!("Associated rules · {} · {}",view.target.label(),view.node))
            .child(div().text_sm().child(format!("{} · {}\nSaved inventory only; awaiting popup observations are excluded. Select a rule to review all conditions and edit it.",view.target.operand(),view.target.directory().unwrap_or_default().display())))
            .child(rows)
            .child(div().flex().flex_wrap().gap_2()
                .child(Button::new("subs-associated-previous").label("Previous").disabled(offset==0||self.subscriptions.rule_busy).on_click(cx.listener(move|this,_,window,cx|this.prepare_rule_target(previous.clone(),false,offset.saturating_sub(attachment::PAGE),Some(session),window,cx))))
                .child(format!("{}–{} of {total}",if total==0{0}else{offset+1},(offset+attachment::PAGE).min(total)))
                .child(Button::new("subs-associated-next").label("Next").disabled(offset+attachment::PAGE>=total||self.subscriptions.rule_busy).on_click(cx.listener(move|this,_,window,cx|this.prepare_rule_target(next.clone(),false,offset+attachment::PAGE,Some(session),window,cx))))
                .child(Button::new("subs-associated-refresh").label("Refresh snapshot").disabled(self.subscriptions.rule_busy).on_click(cx.listener(move|this,_,window,cx|this.prepare_rule_target(refresh.clone(),false,offset,Some(session),window,cx))))
                .child(Button::new("subs-associated-back").label(if self.subscriptions.collections.is_some(){"Back to collections"}else{"Back to subscriptions"}).disabled(self.subscriptions.rule_busy).on_click(cx.listener(|this,_,window,cx|{this.subscriptions.inspection=None;this.subscriptions.search_input().focus_handle(cx).focus(window,cx);cx.notify();}))))
    }
}
