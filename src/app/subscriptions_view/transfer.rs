use super::*;
use crate::subscriptions::{bulk::Scope, transfer as data};
const REVIEW_PAGE: usize = 16;
pub(super) struct Transfer {
    pub root_focus: FocusHandle,
    path: Entity<InputState>,
    encoding: Picker,
    search: String,
    ids: Vec<u64>,
    matching: bool,
    replace: bool,
    defaults: bool,
    catalogue: bool,
    pub review: Option<Box<data::Review>>,
    offset: usize,
}
impl Transfer {
    pub fn set_review(&mut self, review: Box<data::Review>) {
        self.review = Some(review);
        self.offset = 0;
    }
    fn import_request(&self, cx: &App) -> Request {
        Request::ReviewImport {
            path: self.path.read(cx).value().trim().into(),
            replace: self.replace,
            defaults: self.defaults,
            catalogue: self.catalogue,
        }
    }
}
impl Dashboard {
    pub(in crate::app) fn open_subscription_transfer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = &self.subscriptions;
        if view.pending.is_some()
            || view.rule_busy
            || view.form.is_some()
            || view.collections.is_some()
            || view.inspection.is_some()
            || view.bulk.is_some()
            || view.transfer.is_some()
            || view.query.removed
            || view.remove
            || view.detail.is_some()
        {
            return;
        }
        let path = field("", 4096, window, cx);
        let encoding = bulk::picker(
            ["Rustleblower configuration", "OpenSnitch action file"]
                .map(SharedString::from)
                .to_vec(),
            window,
            cx,
        );
        let ids = view.marked.iter().copied().collect::<Vec<_>>();
        let transfer = Transfer {
            root_focus: self.focus.clone(),
            path,
            encoding,
            search: self.subscriptions.input.read(cx).value().to_string(),
            matching: ids.is_empty(),
            ids,
            replace: false,
            defaults: false,
            catalogue: false,
            review: None,
            offset: 0,
        };
        transfer.path.focus_handle(cx).focus(window, cx);
        self.subscriptions.transfer = Some(transfer);
        self.subscriptions.error = None;
        self.subscriptions.message = None;
        cx.notify();
    }
    pub(in crate::app) fn review_subscription_import(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = &self.subscriptions.transfer {
            let request = view.import_request(cx);
            self.subscriptions
                .send(&self.backend.subscriptions, request, After::Action);
            self.focus.focus(window, cx);
            cx.notify();
        }
    }
    pub(in crate::app) fn apply_subscription_import(&mut self, cx: &mut Context<Self>) {
        if let Some(review) = self
            .subscriptions
            .transfer
            .as_ref()
            .and_then(|v| v.review.as_ref())
            .filter(|r| r.conflicts() == 0)
            .cloned()
        {
            self.subscriptions.send(
                &self.backend.subscriptions,
                Request::ApplyImport(review),
                After::Action,
            );
            cx.notify();
        }
    }
    pub(super) fn render_subscription_transfer(&self, cx: &mut Context<Self>) -> Div {
        let view = self.subscriptions.transfer.as_ref().unwrap();
        let busy = self.subscriptions.pending.is_some();
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child("Import / export subscriptions");
        if let Some(review) = &view.review {
            body=body.child(format!("{} entries · {} changes · {} conflicts · {} retained removals",review.entries.len(),review.changed(),review.conflicts(),review.removals.len()))
                .child(div().text_sm().child("Import saves the reviewed metadata together and retains existing downloads and rules. Source directories are not adopted. Other actions and imported notification settings are retained for export only."));
            body = body.child(format!(
                "Default count/unit before: {}",
                review.settings_before.1.count_units_label()
            ));
            if review.defaults {
                let before = &review.settings_before.1;
                let after = &review.document.settings;
                body = body.child(format!(
                    "Default count/unit after: {}",
                    after.count_units_label()
                ));
                body=body.child(div().text_sm().child(format!(
                    "Global defaults before → after\nAutomatic updates: {} → {}\nInterval: {} → {} seconds · Timeout: {} → {} seconds\nMaximum list: {} → {} bytes · Disk budget: {} → {} MiB\nUser agent: {} → {}",
                    before.automatic,after.automatic,before.interval,after.interval,before.timeout,after.timeout,before.max_bytes,after.max_bytes,before.disk_bytes/1024/1024,after.disk_bytes/1024/1024,before.user_agent,after.user_agent
                )));
            } else {
                body = body.child("Keep current global defaults and automatic-update state.");
            }
            let mut rows = div()
                .id("subs-transfer-review-rows")
                .min_h(px(96.))
                .max_h(px(240.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_3();
            if review.replace_catalogue {
                body=body.child(div().text_sm().child("Complete catalogue replacement: absent active entries will stop updating and leave group/all collections. Their downloaded files remain for existing direct-list rules. No daemon rule is rewritten."));
            }
            for index in (view.offset..review.review_len()).take(REVIEW_PAGE) {
                if let Some(entry) = review.entries.get(index) {
                    let status = if entry.conflict {
                        "Configuration conflict: enable replacement"
                    } else {
                        match &entry.before {
                            None => "New subscription",
                            Some(b) if b.removed => "Restore retained subscription",
                            Some(b) if b.spec == entry.after && !entry.metadata_changed => {
                                "Unchanged"
                            }
                            Some(_) => "Replace existing configuration",
                        }
                    };
                    let before = entry
                        .before
                        .as_ref()
                        .map(|b| {
                            format!(
                                "#{} {}",
                                b.id,
                                serde_json::to_string_pretty(&b.spec).unwrap()
                            )
                        })
                        .unwrap_or_else(|| "(new)".into());
                    let settings = if review.defaults {
                        &review.document.settings
                    } else {
                        &review.settings_before.1
                    };
                    let limits = effective(&entry.after, settings)
                        .map(|(i, t, m)| {
                            format!("{i}s interval · {t}s timeout · {m} bytes maximum")
                        })
                        .unwrap_or_else(|e| e);
                    rows = rows.child(div().text_sm().child(format!("Effective after: {limits}")));
                    rows = rows.child(div().text_sm().child(format!(
                        "{} · {status}\nIdentity: {}\nBefore: {before}\nAfter: {}{}",
                        entry.after.name,
                        entry.resolution,
                        serde_json::to_string_pretty(&entry.after).unwrap(),
                        if entry.metadata_changed {
                            "\nRetained extra fields will be replaced by this file's values."
                        } else {
                            ""
                        }
                    )));
                } else {
                    let removed = &review.removals[index - review.entries.len()];
                    rows=rows.child(div().text_sm().child(format!("Remove #{} {} from updates and collections\nKeep downloaded files for existing direct-list rules.\n{}",removed.id,removed.spec.name,serde_json::to_string_pretty(&removed.spec).unwrap())));
                }
            }
            body = body
                .child(rows)
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("subs-transfer-prev")
                                .label("Previous entries")
                                .disabled(busy || view.offset == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(v) = &mut this.subscriptions.transfer {
                                        v.offset = v.offset.saturating_sub(REVIEW_PAGE);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(format!(
                            "{}–{} of {}",
                            if review.review_len() == 0 {
                                0
                            } else {
                                view.offset + 1
                            },
                            (view.offset + REVIEW_PAGE).min(review.review_len()),
                            review.review_len()
                        ))
                        .child(
                            Button::new("subs-transfer-next")
                                .label("Next entries")
                                .disabled(busy || view.offset + REVIEW_PAGE >= review.review_len())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(v) = &mut this.subscriptions.transfer {
                                        v.offset += REVIEW_PAGE;
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("subs-transfer-back")
                                .label("Back to file options")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(v) = &mut this.subscriptions.transfer {
                                        v.review = None;
                                    }
                                    this.subscriptions.error = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("subs-transfer-rebuild")
                                .label("Rebuild from file")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.review_subscription_import(window, cx)
                                })),
                        )
                        .child(
                            Button::new("subs-transfer-apply")
                                .label("Import reviewed configuration")
                                .disabled(busy || review.conflicts() > 0)
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.apply_subscription_import(cx)
                                    }),
                                ),
                        ),
                );
        } else {
            body=body.child(div().text_sm().child("Choose an absolute JSON path. Import accepts Rustleblower version 1, an OpenSnitch list_subscriptions action file, or its config object. Export creates a new file and preserves existing destinations. Downloaded files and daemon rule definitions are separate."))
                .child(Input::new(&view.path).id("subs-transfer-path").disabled(busy))
                .child(Checkbox::new("subs-transfer-replace").label("Import: replace or restore matching existing configurations and extra fields").checked(view.replace).disabled(busy).on_change(cx.listener(|this,v:&bool,_,cx|{if let Some(t)=&mut this.subscriptions.transfer{t.replace = *v;}cx.notify();})))
                .child(Checkbox::new("subs-transfer-catalogue").label("Import: replace complete active catalogue; retain absent entries and files").checked(view.catalogue).disabled(busy).on_change(cx.listener(|this,v:&bool,_,cx|{if let Some(t)=&mut this.subscriptions.transfer{t.catalogue = *v;}cx.notify();})))
                .child(Checkbox::new("subs-transfer-defaults").label("Import: include global defaults and automatic-update state in review").checked(view.defaults).disabled(busy).on_change(cx.listener(|this,v:&bool,_,cx|{if let Some(t)=&mut this.subscriptions.transfer{t.defaults = *v;}cx.notify();})))
                .child(Button::new("subs-transfer-review").label("Review import…").disabled(busy).on_click(cx.listener(|this,_,window,cx|this.review_subscription_import(window,cx))))
                .child(format!("Export: {} marked across pages · captured filter: {}",view.ids.len(),if view.search.is_empty(){"all active subscriptions"}else{&view.search}))
                .child(Checkbox::new("subs-transfer-matching").label("Export all active matches across every page").checked(view.matching).disabled(busy).on_change(cx.listener(|this,v:&bool,_,cx|{if let Some(t)=&mut this.subscriptions.transfer{t.matching = *v;}cx.notify();})))
                .child(div().text_sm().child("Native export preserves exact names and filenames. OpenSnitch export normalizes typed filenames and makes duplicates distinct."))
                .child(div().h_8().flex_shrink_0().child(Select::new(&view.encoding).id("subs-transfer-encoding").disabled(busy)))
                .child(Button::new("subs-transfer-export").label("Export to new file").disabled(busy).on_click(cx.listener(|this,_,_,cx|{
                    if let Some(v)=&this.subscriptions.transfer {
                        let scope=if v.matching{Scope::Matching(v.search.clone())}else{Scope::Selected(v.ids.clone())};
                        let encoding=if v.encoding.read(cx).selected_value().is_some_and(|v|v.as_ref()=="OpenSnitch action file"){data::Encoding::OpenSnitch}else{data::Encoding::Native};
                        let request=Request::Export{path:v.path.read(cx).value().trim().into(),scope,encoding};
                        this.subscriptions.send(&this.backend.subscriptions,request,After::Action);
                    }cx.notify();
                })));
        }
        body.child(
            Button::new("subs-transfer-cancel")
                .label("Cancel / return to subscriptions")
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.subscriptions.transfer = None;
                    this.subscriptions.error = None;
                    this.subscriptions.input.focus_handle(cx).focus(window, cx);
                    cx.notify();
                })),
        )
    }
}
