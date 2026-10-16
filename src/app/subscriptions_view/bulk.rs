use super::*;
use crate::subscriptions::bulk as data;
const REVIEW_PAGE: usize = 16;
pub(super) struct Bulk {
    pub root_focus: FocusHandle,
    matching: bool,
    search: String,
    ids: Vec<u64>,
    enabled: Picker,
    groups: Picker,
    format: Picker,
    fields: Vec<Entity<InputState>>,
    overrides: [bool; 3],
    pub review: Option<data::Review>,
    offset: usize,
}
pub(super) fn picker(
    labels: Vec<SharedString>,
    window: &mut Window,
    cx: &mut Context<Dashboard>,
) -> Picker {
    let state = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(labels),
            Some(IndexPath::new(0)),
            window,
            cx,
        )
        .searchable(true)
    });
    cx.subscribe(
        &state,
        |_, _, _: &SelectEvent<SearchableVec<SharedString>>, cx| cx.notify(),
    )
    .detach();
    state
}
fn value(state: &Picker, cx: &App) -> String {
    state
        .read(cx)
        .selected_value()
        .map(ToString::to_string)
        .unwrap_or_default()
}
impl Bulk {
    pub fn set_review(&mut self, review: data::Review) {
        self.review = Some(review);
        self.offset = 0;
    }
    fn request(&self, cx: &App) -> Result<Request, String> {
        let groups = self.fields[0]
            .read(cx)
            .value()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        let override_value = |index: usize| -> Result<Option<Option<u64>>, String> {
            if !self.overrides[index] {
                return Ok(None);
            }
            let text = self.fields[index + 1].read(cx).value();
            if text.trim().is_empty() {
                Ok(Some(None))
            } else {
                Ok(Some(Some(
                    text.trim()
                        .parse()
                        .map_err(|_| "Enter a whole nonnegative limit")?,
                )))
            }
        };
        let format = value(&self.format, cx);
        let patch = data::Patch {
            enabled: match value(&self.enabled, cx).as_str() {
                "Enable" => Some(true),
                "Disable" => Some(false),
                _ => None,
            },
            groups: match value(&self.groups, cx).as_str() {
                "Add groups" => data::Groups::Add(groups),
                "Remove groups" => data::Groups::Remove(groups),
                "Replace groups" => data::Groups::Replace(groups),
                "Clear groups" => data::Groups::Clear,
                _ => data::Groups::Keep,
            },
            format: FORMATS
                .iter()
                .find(|(_, label)| *label == format)
                .map(|(key, _)| Format::parse(key))
                .transpose()?,
            interval: override_value(0)?
                .map(|v| {
                    v.map(u32::try_from)
                        .transpose()
                        .map_err(|_| "Interval is too large")
                })
                .transpose()?,
            timeout: override_value(1)?
                .map(|v| {
                    v.map(u32::try_from)
                        .transpose()
                        .map_err(|_| "Timeout is too large")
                })
                .transpose()?,
            max_bytes: override_value(2)?
                .map(|v| {
                    v.map(|n| n.checked_mul(1024).ok_or("Size is too large"))
                        .transpose()
                })
                .transpose()?,
        };
        patch.validate()?;
        let scope = if self.matching {
            data::Scope::Matching(self.search.clone())
        } else {
            data::Scope::Selected(self.ids.clone())
        };
        scope.validate()?;
        Ok(Request::ReviewBulk { scope, patch })
    }
}
impl Dashboard {
    pub(in crate::app) fn open_subscription_bulk(
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
        let search = view.input.read(cx).value().to_string();
        let ids = view.marked.iter().copied().collect::<Vec<_>>();
        let enabled = picker(
            ["Keep current status", "Enable", "Disable"]
                .map(SharedString::from)
                .to_vec(),
            window,
            cx,
        );
        let groups = picker(
            [
                "Keep groups",
                "Add groups",
                "Remove groups",
                "Replace groups",
                "Clear groups",
            ]
            .map(SharedString::from)
            .to_vec(),
            window,
            cx,
        );
        let format = picker(
            std::iter::once(SharedString::from("Keep input format"))
                .chain(FORMATS.iter().map(|(_, s)| SharedString::from(*s)))
                .collect(),
            window,
            cx,
        );
        let fields = ["", "", "", ""]
            .iter()
            .enumerate()
            .map(|(i, v)| field(v, if i == 0 { 576 } else { 16 }, window, cx))
            .collect();
        enabled.focus_handle(cx).focus(window, cx);
        self.subscriptions.bulk = Some(Bulk {
            root_focus: self.focus.clone(),
            matching: ids.is_empty(),
            search,
            ids,
            enabled,
            groups,
            format,
            fields,
            overrides: [false; 3],
            review: None,
            offset: 0,
        });
        self.subscriptions.error = None;
        self.subscriptions.message = None;
        cx.notify();
    }
    pub(in crate::app) fn review_subscription_bulk(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = &self.subscriptions.bulk {
            match view.request(cx) {
                Ok(request) => {
                    self.subscriptions
                        .send(&self.backend.subscriptions, request, After::Action)
                }
                Err(error) => self.subscriptions.error = Some(error),
            }
            self.focus.focus(window, cx);
            cx.notify();
        }
    }
    pub(in crate::app) fn apply_subscription_bulk(&mut self, cx: &mut Context<Self>) {
        if let Some(review) = self
            .subscriptions
            .bulk
            .as_ref()
            .and_then(|v| v.review.as_ref())
            .filter(|r| !r.entries.is_empty())
            .cloned()
        {
            self.subscriptions.send(
                &self.backend.subscriptions,
                Request::ApplyBulk(review),
                After::Action,
            );
            cx.notify();
        }
    }
    pub(super) fn render_subscription_bulk(&self, cx: &mut Context<Self>) -> Div {
        let view = self.subscriptions.bulk.as_ref().unwrap();
        let busy = self.subscriptions.pending.is_some();
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child("Bulk subscription changes");
        if let Some(review) = &view.review {
            body = body.child(format!("{} examined · {} changes · {} unchanged",review.scanned,review.entries.len(),review.scanned-review.entries.len()))
                .child(div().text_sm().child("Only these reviewed changes will apply. Enabled state, groups and input format change collection membership. Existing rule definitions and retained direct-list data are preserved. A concurrent edit rejects the entire batch; rebuild the review to include it."));
            let mut rows = div()
                .id("subs-bulk-review-rows")
                .min_h(px(96.))
                .max_h(px(300.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_3();
            for entry in review.entries.iter().skip(view.offset).take(REVIEW_PAGE) {
                rows = rows.child(div().text_sm().child(format!(
                    "{} (#{})\n{}",
                    entry.before.name,
                    entry.id,
                    changes(entry)
                )));
            }
            body = body
                .child(rows)
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("subs-bulk-prev")
                                .label("Previous changes")
                                .disabled(busy || view.offset == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(v) = &mut this.subscriptions.bulk {
                                        v.offset = v.offset.saturating_sub(REVIEW_PAGE);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(format!(
                            "{}–{} of {}",
                            if review.entries.is_empty() {
                                0
                            } else {
                                view.offset + 1
                            },
                            (view.offset + REVIEW_PAGE).min(review.entries.len()),
                            review.entries.len()
                        ))
                        .child(
                            Button::new("subs-bulk-next")
                                .label("Next changes")
                                .disabled(busy || view.offset + REVIEW_PAGE >= review.entries.len())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(v) = &mut this.subscriptions.bulk {
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
                            Button::new("subs-bulk-back")
                                .label("Back to fields")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(v) = &mut this.subscriptions.bulk {
                                        v.review = None;
                                    }
                                    this.subscriptions.error = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("subs-bulk-rebuild")
                                .label("Rebuild review")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.review_subscription_bulk(window, cx)
                                })),
                        )
                        .child(
                            Button::new("subs-bulk-apply")
                                .label("Apply reviewed changes")
                                .disabled(busy || review.entries.is_empty())
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.apply_subscription_bulk(cx)),
                                ),
                        ),
                );
        } else {
            body = body
                .child(format!(
                    "{} marked across pages · filter captured: {}",
                    view.ids.len(),
                    if view.search.is_empty() {
                        "all active subscriptions"
                    } else {
                        &view.search
                    }
                ))
                .child(
                    Checkbox::new("subs-bulk-matching")
                        .label("All active subscriptions matching this filter, across every page")
                        .checked(view.matching)
                        .disabled(busy)
                        .on_change(cx.listener(|this, v: &bool, _, cx| {
                            if let Some(b) = &mut this.subscriptions.bulk {
                                b.matching = *v;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    div().h_8().flex_shrink_0().child(
                        Select::new(&view.enabled)
                            .id("subs-bulk-enabled")
                            .disabled(busy),
                    ),
                )
                .child(
                    div().h_8().flex_shrink_0().child(
                        Select::new(&view.groups)
                            .id("subs-bulk-groups-mode")
                            .disabled(busy),
                    ),
                )
                .child("Group labels, separated by commas")
                .child(
                    Input::new(&view.fields[0])
                        .id("subs-bulk-groups")
                        .disabled(busy),
                )
                .child(
                    div().h_8().flex_shrink_0().child(
                        Select::new(&view.format)
                            .id("subs-bulk-format")
                            .disabled(busy),
                    ),
                );
            body = body.child(div().text_sm().child(
                "Check a limit to change it; an empty checked field inherits the global default.",
            ));
            for (index, label) in [
                "Change interval (seconds)",
                "Change timeout (seconds)",
                "Change maximum list size (KiB)",
            ]
            .iter()
            .enumerate()
            {
                body = body.child(
                    div()
                        .flex()
                        .gap_3()
                        .items_center()
                        .child(
                            Checkbox::new(("subs-bulk-override", index))
                                .label(*label)
                                .checked(view.overrides[index])
                                .disabled(busy)
                                .on_change(cx.listener(move |this, v: &bool, _, cx| {
                                    if let Some(b) = &mut this.subscriptions.bulk {
                                        b.overrides[index] = *v;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Input::new(&view.fields[index + 1])
                                .id(("subs-bulk-limit", index))
                                .disabled(busy || !view.overrides[index]),
                        ),
                );
            }
            body =
                body.child(
                    Button::new("subs-bulk-review")
                        .label("Review changes")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.review_subscription_bulk(window, cx)
                        })),
                );
        }
        body.child(
            Button::new("subs-bulk-cancel")
                .label("Cancel bulk changes")
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.subscriptions.bulk = None;
                    this.subscriptions.error = None;
                    this.subscriptions.input.focus_handle(cx).focus(window, cx);
                    cx.notify();
                })),
        )
    }
}
fn changes(entry: &data::Entry) -> String {
    let (before, after) = (&entry.before, &entry.after);
    let mut lines = Vec::new();
    if before.enabled != after.enabled {
        lines.push(format!("Enabled: {} → {}", before.enabled, after.enabled));
    }
    if before.groups != after.groups {
        lines.push(format!(
            "Groups: [{}] → [{}]",
            before.groups.join(", "),
            after.groups.join(", ")
        ));
    }
    if before.format != after.format {
        lines.push(format!(
            "Input format: {} → {}",
            before.format.key(),
            after.format.key()
        ));
    }
    let limit = |value: Option<crate::subscriptions::limits::Override>, size| {
        use crate::subscriptions::limits::Override;
        match value {
            None => "inherit count and unit".into(),
            Some(Override::Absolute(n)) => {
                format!("{n} {}", if size { "bytes" } else { "seconds" })
            }
            Some(Override::Parts(p)) => format!(
                "{} × {}",
                p.count
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "inherit count".into()),
                p.unit.map(|u| u.label()).unwrap_or("inherit unit")
            ),
        }
    };
    for (label, old, new, size) in [
        ("Interval", before.interval, after.interval, false),
        ("Timeout", before.timeout, after.timeout, false),
        ("Maximum list", before.max_bytes, after.max_bytes, true),
    ] {
        if old != new {
            lines.push(format!(
                "{label}: {} → {}",
                limit(old, size),
                limit(new, size)
            ));
        }
    }
    lines.join("\n")
}
