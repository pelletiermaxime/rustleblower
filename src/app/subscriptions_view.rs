//! Subscription controls own bounded pages/forms; their worker owns all I/O.
use super::Dashboard;
use crate::subscriptions::{self, Reply, Request, model::*};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath, Selectable,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    list::ListItem,
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use tokio::sync::oneshot;
mod bulk;
mod collections;
mod limits;
mod notifications;
mod rules;
mod transfer;
mod url_test;
type Picker = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<SharedString>>>>;
const FORMATS: &[(&str, &str)] = &[
    ("hosts", "Hosts file"),
    ("domains", "Domain names / patterns"),
    ("ips", "IP addresses and CIDRs"),
    ("nets", "Network prefixes"),
    ("regexp", "Domain regular expressions"),
    ("md5", "MD5 hashes"),
];
struct Form {
    limits: limits::Controls,
    url_test: url_test::UrlTest,
    fields: Vec<Entity<InputState>>,
    format: Option<Picker>,
    enabled: bool,
    previous: Option<(u64, u64)>,
    settings: Option<(u64, Settings)>,
    original: Option<Row>,
}
#[derive(Clone, Copy)]
enum After {
    Page,
    Save,
    Action,
    Settings,
    NotificationsLoad,
    NotificationsSave,
    Preview,
    Logs,
}
pub(super) struct SubscriptionsView {
    pub input: Entity<InputState>,
    pub(super) page: Option<Page>,
    query: Query,
    selected: Option<u64>,
    form: Option<Form>,
    pending: Option<(After, oneshot::Receiver<Result<Reply, String>>)>,
    dirty: bool,
    revision: u64,
    error: Option<String>,
    message: Option<String>,
    detail: Option<String>,
    remove: bool,
    rule_busy: bool,
    rule_guard: Option<oneshot::Receiver<()>>,
    inspection: Option<rules::Inspection>,
    collections: Option<collections::Collections>,
    membership_review: Option<(Request, String)>,
    marked: std::collections::BTreeSet<u64>,
    bulk: Option<bulk::Bulk>,
    transfer: Option<transfer::Transfer>,
    notifications: Option<notifications::Notifications>,
}
fn field(
    value: &str,
    max: usize,
    window: &mut Window,
    cx: &mut Context<Dashboard>,
) -> Entity<InputState> {
    let value = value.to_owned();
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value)
            .validate(move |v, _| v.len() <= max && !v.contains('\0'))
    })
}
impl Form {
    fn spec(
        row: Option<&Row>,
        defaults: (u64, Settings),
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> Self {
        let value = row.map(|r| r.spec.clone()).unwrap_or_default();
        let (limits, limit_values) = limits::Controls::subscription(&value, defaults, window, cx);
        let [interval, timeout, size] = limit_values;
        let values = [
            value.name,
            value.url,
            value.filename,
            value.groups.join(", "),
            interval,
            timeout,
            size,
        ];
        let fields = values
            .iter()
            .enumerate()
            .map(|(i, v)| field(v, if i == 1 { 2048 } else { 576 }, window, cx))
            .collect();
        let index = FORMATS
            .iter()
            .position(|(key, _)| *key == value.format.key())
            .unwrap_or(0);
        let format = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    FORMATS
                        .iter()
                        .map(|(_, label)| {
                            crate::i18n::LocalizedOption::new(SharedString::from(*label), *label)
                        })
                        .collect::<Vec<_>>(),
                ),
                Some(IndexPath::new(index)),
                window,
                cx,
            )
            .searchable(true)
        });
        cx.subscribe(
            &format,
            |_,
             _,
             _: &SelectEvent<SearchableVec<crate::i18n::LocalizedOption<SharedString>>>,
             cx| cx.notify(),
        )
        .detach();
        Self {
            limits,
            url_test: Default::default(),
            fields,
            format: Some(format),
            enabled: value.enabled,
            previous: row.map(|r| (r.id, r.revision)),
            original: row.cloned(),
            settings: None,
        }
    }
    fn settings(
        revision: u64,
        settings: Settings,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> Self {
        let (limits, [interval, timeout, size]) =
            limits::Controls::settings((revision, settings.clone()), window, cx);
        let values = [
            interval,
            timeout,
            size,
            (settings.disk_bytes / 1024 / 1024).to_string(),
            settings.user_agent.clone(),
        ];
        let fields = values.iter().map(|v| field(v, 256, window, cx)).collect();
        Self {
            limits,
            url_test: Default::default(),
            fields,
            format: None,
            enabled: false,
            previous: None,
            original: None,
            settings: Some((revision, settings)),
        }
    }
    fn request(&self, cx: &App) -> Result<Request, String> {
        let value = |i: usize| self.fields[i].read(cx).value().trim().to_owned();
        let number = |i: usize| {
            value(i)
                .parse::<u64>()
                .map_err(|_| "Enter whole nonnegative numbers for the limits".to_string())
        };
        if let Some((revision, settings)) = &self.settings {
            let mut settings = settings.clone();
            self.limits
                .set_defaults([number(0)?, number(1)?, number(2)?], &mut settings, cx)?;
            settings.disk_bytes = number(3)?
                .checked_mul(1024 * 1024)
                .ok_or("Disk budget is too large")?;
            settings.user_agent = value(4);
            settings.validate()?;
            return Ok(Request::SaveSettings {
                revision: *revision,
                settings,
            });
        }
        let [interval, timeout, max_bytes] = self.limits.overrides(self.limit_counts(cx)?, cx)?;
        let label = self
            .format
            .as_ref()
            .unwrap()
            .read(cx)
            .selected_value()
            .map(ToString::to_string)
            .unwrap_or_default();
        let key = FORMATS
            .iter()
            .find(|(_, s)| *s == label)
            .map(|(k, _)| *k)
            .ok_or("Choose a format")?;
        let spec = Spec {
            name: value(0),
            url: value(1),
            filename: value(2),
            groups: value(3)
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            format: Format::parse(key)?,
            enabled: self.enabled,
            interval,
            timeout,
            max_bytes,
        };
        spec.validate()?;
        effective(&spec, &self.limits.defaults.1)?;
        Ok(Request::SaveChecked {
            settings_revision: self.limits.defaults.0,
            previous: self.previous,
            spec,
        })
    }
}
impl SubscriptionsView {
    pub fn new(window: &mut Window, cx: &mut Context<Dashboard>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::i18n::placeholder(
                    "Search subscriptions…",
                    window,
                    cx,
                ))
                .validate(|v, _| v.len() <= 256)
        });
        cx.subscribe(&input, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.subscriptions.query.offset = 0;
                this.subscriptions.selected = None;
                this.subscriptions.marked.clear();
                this.subscriptions.dirty = true;
                cx.notify();
            }
        })
        .detach();
        Self {
            input,
            page: None,
            query: Query::default(),
            selected: None,
            form: None,
            pending: None,
            dirty: true,
            revision: u64::MAX,
            error: None,
            message: None,
            detail: None,
            remove: false,
            rule_busy: false,
            rule_guard: None,
            inspection: None,
            collections: None,
            membership_review: None,
            marked: Default::default(),
            bulk: None,
            transfer: None,
            notifications: None,
        }
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn collection_page(&self) -> Option<&crate::subscriptions::collections::Page> {
        self.collections.as_ref()?.page.as_ref()
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn associated_counts(&self) -> Option<(usize, usize, usize)> {
        self.inspection.as_ref().map(|i| i.counts())
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn bulk_state(&self) -> (usize, Option<(usize, usize)>, Option<&str>) {
        (
            self.marked.len(),
            self.bulk
                .as_ref()
                .and_then(|v| v.review.as_ref())
                .map(|r| (r.scanned, r.entries.len())),
            self.error.as_deref(),
        )
    }
    pub(super) fn has_notifications(&self) -> bool {
        self.notifications.is_some()
    }
    pub(super) fn has_transfer(&self) -> bool {
        self.transfer.is_some()
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn transfer_state(&self) -> (Option<(usize, usize, usize)>, Option<&str>) {
        (
            self.transfer
                .as_ref()
                .and_then(|v| v.review.as_ref())
                .map(|r| (r.entries.len(), r.changed(), r.conflicts())),
            self.error.as_deref(),
        )
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(super) fn transfer_removals(&self) -> Option<usize> {
        self.transfer
            .as_ref()
            .and_then(|v| v.review.as_ref())
            .map(|r| r.removals.len())
    }
    pub fn search_input(&self) -> &Entity<InputState> {
        self.collections
            .as_ref()
            .map(|v| &v.input)
            .unwrap_or(&self.input)
    }
    fn begin_rule_work(&mut self) -> oneshot::Sender<()> {
        let (tx, rx) = oneshot::channel();
        self.rule_guard = Some(rx);
        self.rule_busy = true;
        tx
    }
    fn finish_rule_work(&mut self) {
        self.rule_busy = false;
        self.rule_guard = None;
    }
    fn selected(&self) -> Option<&Row> {
        self.page
            .as_ref()?
            .rows
            .iter()
            .find(|r| Some(r.id) == self.selected)
    }
    fn send(&mut self, handle: &subscriptions::Handle, request: Request, after: After) {
        if self.pending.is_some() {
            return;
        }
        match handle.request(request) {
            Ok(rx) => {
                self.pending = Some((after, rx));
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub fn poll(
        &mut self,
        handle: &subscriptions::Handle,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> bool {
        let mut changed = self
            .form
            .as_mut()
            .is_some_and(|form| url_test::UrlTest::poll(form, cx));
        if self.rule_busy
            && self.rule_guard.as_mut().is_some_and(|rx| {
                !matches!(rx.try_recv(), Err(oneshot::error::TryRecvError::Empty))
            })
        {
            self.finish_rule_work();
            self.error = Some(crate::i18n::tr(
                "Rule review or inspection was interrupted; reopen it to retry",
            ));
            changed = true;
        }
        if let Some((after, rx)) = &mut self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    let after = *after;
                    self.pending = None;
                    changed = true;
                    match result {
                        Ok(Reply::Page(page)) => {
                            if !self.dirty {
                                if page.rows.is_empty() && page.offset > 0 {
                                    self.query.offset = page.total.saturating_sub(1) / PAGE * PAGE;
                                    self.dirty = true;
                                }
                                self.page = Some(page);
                            }
                        }
                        Ok(Reply::Collections(page)) => {
                            if !self.dirty
                                && let Some(view) = &mut self.collections
                            {
                                if page.rows.is_empty() && page.offset > 0 {
                                    view.query.offset = page.total.saturating_sub(1) / PAGE * PAGE;
                                    self.dirty = true;
                                }
                                view.page = Some(page);
                            }
                        }
                        Ok(Reply::ImportReview(review)) => {
                            if let Some(view) = &mut self.transfer {
                                view.set_review(review);
                                view.root_focus.focus(window, cx);
                            }
                        }
                        Ok(Reply::Imported(count)) => {
                            self.transfer = None;
                            self.dirty = true;
                            self.message = Some(crate::i18n::tr_args(
                                "Imported %{count} subscription changes",
                                &[("count", &format!("{}", count))],
                            ));
                            self.input.focus_handle(cx).focus(window, cx);
                        }
                        Ok(Reply::Exported(count)) => {
                            self.message = Some(crate::i18n::tr_args(
                                "Exported %{count} subscriptions to a new file",
                                &[("count", &format!("{}", count))],
                            ));
                        }
                        Ok(Reply::BulkReview(review)) => {
                            if let Some(view) = &mut self.bulk {
                                view.set_review(review);
                                view.root_focus.focus(window, cx);
                            }
                        }
                        Ok(Reply::BulkApplied(count)) => {
                            self.bulk = None;
                            self.dirty = true;
                            self.message = Some(crate::i18n::tr_args(
                                "Updated %{count} subscriptions together",
                                &[("count", &format!("{}", count))],
                            ));
                            self.input.focus_handle(cx).focus(window, cx);
                        }
                        Ok(Reply::Saved(id)) => {
                            self.form = None;
                            self.input.focus_handle(cx).focus(window, cx);
                            self.selected = Some(id);
                            self.dirty = true;
                            self.message = Some(crate::i18n::tr("Subscription saved"));
                        }
                        Ok(Reply::NotificationSettings(revision, config)) => {
                            self.notifications = Some(notifications::Notifications::new(
                                revision, config, window, cx,
                            ));
                        }
                        Ok(Reply::Settings(revision, settings)) => {
                            self.form = Some(Form::settings(revision, settings, window, cx))
                        }
                        Ok(Reply::Preview(text)) => self.detail = Some(text),
                        Ok(Reply::Logs(lines)) => self.detail = Some(lines.join("\n")),
                        Ok(Reply::Done) => {
                            if matches!(after, After::NotificationsSave) {
                                self.notifications = None;
                                self.input.focus_handle(cx).focus(window, cx);
                            }
                            if matches!(after, After::Save) {
                                self.form = None;
                                self.input.focus_handle(cx).focus(window, cx);
                            }
                            self.remove = false;
                            self.dirty = true;
                            self.message = Some(
                                if matches!(after, After::Save | After::NotificationsSave) {
                                    "Subscription settings saved"
                                } else {
                                    "Subscription request accepted"
                                }
                                .into(),
                            );
                        }
                        Ok(Reply::Row(_) | Reply::Collection(_) | Reply::UrlTest(_)) => {}
                        Err(error) => self.error = Some(error),
                    }
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.pending = None;
                    self.error = Some(
                        handle
                            .state()
                            .error
                            .unwrap_or_else(|| "Subscription worker stopped".into()),
                    );
                    changed = true;
                }
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        let state = handle.state();
        if state.revision != self.revision {
            self.revision = state.revision;
            self.dirty = true;
            changed = true;
        }
        if self.pending.is_none()
            && self.dirty
            && self.notifications.is_none()
            && self.form.is_none()
            && self.bulk.is_none()
            && self.transfer.is_none()
            && state.error.is_none()
        {
            let request = if let Some(view) = &mut self.collections {
                view.query.search = view.input.read(cx).value().to_string();
                Request::Collections(view.query.clone())
            } else {
                self.query.search = self.input.read(cx).value().to_string();
                Request::Page(self.query.clone())
            };
            self.dirty = false;
            self.send(handle, request, After::Page);
            changed = true;
        }
        changed
    }
}
impl Dashboard {
    pub(super) fn new_subscription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.subscriptions.pending.is_some()
            || self.subscriptions.notifications.is_some()
            || self.subscriptions.form.is_some()
            || self.subscriptions.collections.is_some()
            || self.subscriptions.inspection.is_some()
            || self.subscriptions.bulk.is_some()
            || self.subscriptions.transfer.is_some()
        {
            return;
        }
        let defaults = self
            .subscriptions
            .page
            .as_ref()
            .map(|p| p.settings.clone())
            .unwrap_or((0, Settings::default()));
        let form = Form::spec(None, defaults, window, cx);
        form.fields[0].focus_handle(cx).focus(window, cx);
        self.subscriptions.form = Some(form);
        self.subscriptions.error = None;
        cx.notify();
    }
    pub(super) fn focus_subscription_search(&self, window: &mut Window, cx: &mut App) {
        if self.subscriptions.inspection.is_some()
            || self.subscriptions.bulk.is_some()
            || self.subscriptions.transfer.is_some()
            || self.subscriptions.notifications.is_some()
            || self.subscriptions.form.is_some()
        {
            self.focus.focus(window, cx);
        } else {
            self.subscriptions
                .search_input()
                .focus_handle(cx)
                .focus(window, cx);
        }
    }
    pub(super) fn subscription_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscriptions.cancel_url_test();
        if self.subscriptions.notifications.is_some() {
            self.save_subscription_notifications(cx);
            return;
        }
        if self.subscriptions.membership_review.is_some() {
            return;
        }
        if let Some(form) = &self.subscriptions.form {
            match form.request(cx) {
                Ok(request) => {
                    let review = match (&request, &form.original) {
                        (
                            Request::Save { spec, .. } | Request::SaveChecked { spec, .. },
                            Some(old),
                        ) if old.removed
                            || old.spec.enabled != spec.enabled
                            || old.spec.groups != spec.groups
                            || old.spec.format != spec.format =>
                        {
                            Some(crate::i18n::tr_args(
                                "%{arg0}\nEnabled for updates and collections: %{arg1} → %{arg2}\nGroups: %{arg3} → %{arg4}\nInput format: %{arg5} → %{arg6}%{arg7}\nExisting group/all rules follow this membership. Retained direct-list files remain available to their rules.",
                                &[
                                    ("arg0", &old.spec.name.to_string()),
                                    ("arg1", &format!("{}", old.spec.enabled)),
                                    ("arg2", &format!("{}", spec.enabled)),
                                    ("arg3", &old.spec.groups.join(", ").to_string()),
                                    ("arg4", &spec.groups.join(", ").to_string()),
                                    ("arg5", old.spec.format.key()),
                                    ("arg6", spec.format.key()),
                                    (
                                        "arg7",
                                        (if old.removed {
                                            "\nRestore this retained subscription into enabled collections."
                                        } else {
                                            ""
                                        }),
                                    ),
                                ],
                            ))
                        }
                        _ => None,
                    };
                    if let Some(summary) = review {
                        self.subscriptions.membership_review = Some((request, summary));
                        self.focus.focus(window, cx);
                    } else {
                        self.subscriptions
                            .send(&self.backend.subscriptions, request, After::Save);
                    }
                }
                Err(error) => self.subscriptions.error = Some(error),
            }
            cx.notify();
        }
    }
    pub(super) fn render_subscriptions(&mut self, cx: &mut Context<Self>) -> Div {
        let state = self.backend.subscriptions.state();
        let busy = self.subscriptions.pending.is_some() || self.subscriptions.rule_busy;
        let selected = self.subscriptions.selected().cloned();
        let mut body = div()
            .id("subs-content")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3();
        if self.subscriptions.notifications.is_some() {
            body = body.child(self.render_subscription_notifications(cx));
        } else if self.subscriptions.transfer.is_some() {
            body = body.child(self.render_subscription_transfer(cx));
        } else if self.subscriptions.bulk.is_some() {
            body = body.child(self.render_subscription_bulk(cx));
        } else if self.subscriptions.inspection.is_some() {
            body = body.child(self.render_subscription_rules(cx));
        } else if let Some((_, summary)) = &self.subscriptions.membership_review {
            body = body
                .child(crate::i18n::tr("Review collection membership change"))
                .child(div().text_sm().child(summary.clone()))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("subs-membership-back")
                                .label(crate::i18n::tr("Back to form"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.subscriptions.membership_review = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("subs-membership-confirm")
                                .label(crate::i18n::tr("Save reviewed membership"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some((request, _)) =
                                        this.subscriptions.membership_review.take()
                                    {
                                        this.subscriptions.send(
                                            &this.backend.subscriptions,
                                            request,
                                            After::Save,
                                        );
                                    }
                                    cx.notify();
                                })),
                        ),
                );
        } else if self.subscriptions.collections.is_some() {
            body = body.child(self.render_collections(cx));
        } else if self.subscriptions.remove {
            body=body.child(crate::i18n::tr("Remove this subscription from updates and group/all collections? Downloaded data is retained for direct-list rules. Removed entries can be restored from Show removed."))
                .child(Button::new("subs-remove-confirm").label(crate::i18n::tr("Remove subscription")).disabled(busy).on_click(cx.listener(|this,_,_,cx|{if let Some(row)=this.subscriptions.selected().cloned(){this.subscriptions.send(&this.backend.subscriptions,Request::Remove{id:row.id,revision:row.revision},After::Action);}cx.notify();})))
                .child(Button::new("subs-remove-cancel").label(crate::i18n::tr("Keep subscription")).disabled(busy).on_click(cx.listener(|this,_,_,cx|{this.subscriptions.remove=false;cx.notify();})));
        } else if let Some(text) = self.subscriptions.detail.clone() {
            body = body
                .child(
                    div()
                        .id("subs-preview-text")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .child(text.clone()),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("subs-preview-copy")
                                .label(crate::i18n::tr("Copy displayed text"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                                }),
                        )
                        .child(
                            Button::new("subs-preview-back")
                                .label(crate::i18n::tr("Back to subscriptions"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.subscriptions.detail = None;
                                    cx.notify();
                                })),
                        ),
                );
        } else if let Some(form) = &self.subscriptions.form {
            let settings = form.settings.is_some();
            let labels = if settings {
                vec![
                    "Default interval count",
                    "Default timeout count",
                    "Maximum list count",
                    "Retained lists budget (MiB)",
                    "HTTP user agent",
                ]
            } else {
                vec![
                    "Name",
                    "HTTP(S) URL",
                    "Filename",
                    "Groups (comma separated)",
                    "Interval count (empty inherits)",
                    "Timeout count (empty inherits)",
                    "Maximum count (empty inherits)",
                ]
            };
            let ids = if settings {
                vec![
                    "subs-default-interval",
                    "subs-default-timeout",
                    "subs-default-size",
                    "subs-default-disk",
                    "subs-default-agent",
                ]
            } else {
                vec![
                    "subs-name",
                    "subs-url",
                    "subs-filename",
                    "subs-groups",
                    "subs-interval",
                    "subs-timeout",
                    "subs-size",
                ]
            };
            let mut fields = div()
                .id("subs-form-fields")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap_2();
            for (index, ((input, label), id)) in form.fields.iter().zip(labels).zip(ids).enumerate()
            {
                let limit_index = if settings {
                    (index < 3).then_some(index)
                } else {
                    (4..7).contains(&index).then(|| index - 4)
                };
                fields = fields.child(
                    div()
                        .flex()
                        .gap_3()
                        .items_center()
                        .child(div().w(px(240.)).text_sm().child(crate::i18n::tr(label)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(input).id(id).disabled(busy)),
                        )
                        .when_some(limit_index, |v, index| {
                            v.child(
                                div().w(px(180.)).h_8().flex_shrink_0().child(
                                    Select::new(&form.limits.pickers[index])
                                        .id(if settings {
                                            [
                                                "subs-default-interval-unit",
                                                "subs-default-timeout-unit",
                                                "subs-default-size-unit",
                                            ][index]
                                        } else {
                                            [
                                                "subs-interval-unit",
                                                "subs-timeout-unit",
                                                "subs-size-unit",
                                            ][index]
                                        })
                                        .disabled(busy),
                                ),
                            )
                        }),
                );
            }
            if let Some(format) = &form.format {
                fields = fields
                    .child(
                        div()
                            .h_8()
                            .flex_shrink_0()
                            .child(Select::new(format).id("subs-format").disabled(busy)),
                    )
                    .child(
                        Checkbox::new("subs-enabled")
                            .label(crate::i18n::tr("Enabled for updates and collection rules"))
                            .checked(form.enabled)
                            .disabled(busy)
                            .on_change(cx.listener(|this, v: &bool, _, cx| {
                                if let Some(form) = &mut this.subscriptions.form {
                                    form.enabled = *v;
                                }
                                cx.notify();
                            })),
                    );
            }
            body = body
                .child(if settings {
                    crate::i18n::tr("Subscription defaults")
                } else if form.previous.is_some() {
                    crate::i18n::tr("Edit / restore subscription")
                } else {
                    crate::i18n::tr("Add subscription")
                })
                .child(fields)
                .when(!settings, |v| {
                    v.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(form.effective_label(cx)),
                    )
                })
                .when(!settings, |body| {
                    body.when_some(form.url_test.message.clone(), |v, message| {
                        v.child(
                            div()
                                .id("subs-url-test-result")
                                .max_h(px(96.))
                                .overflow_y_scroll()
                                .text_sm()
                                .text_color(if form.url_test.failed {
                                    cx.theme().warning
                                } else {
                                    cx.theme().foreground
                                })
                                .child(crate::i18n::presentation(&message)),
                        )
                    })
                })
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("subs-save")
                                .label(crate::i18n::tr("Save"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.subscription_save(window, cx)
                                })),
                        )
                        .child(
                            Button::new("subs-cancel")
                                .label(crate::i18n::tr("Cancel"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.subscriptions.form = None;
                                    this.subscriptions.error = None;
                                    this.subscriptions.input.focus_handle(cx).focus(window, cx);
                                    cx.notify();
                                })),
                        )
                        .when(!settings, |v| {
                            v.child(
                                Button::new("subs-test-url")
                                    .label(crate::i18n::tr("Test URL"))
                                    .disabled(busy || form.url_test.busy())
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.test_subscription_url(cx)
                                        }),
                                    ),
                            )
                            .child(
                                Button::new("subs-test-cancel")
                                    .label(crate::i18n::tr("Cancel URL test"))
                                    .disabled(!form.url_test.busy())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.subscriptions.cancel_url_test();
                                        cx.notify();
                                    })),
                            )
                        }),
                );
        } else {
            let mut toolbar = div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("subs-collections")
                        .label(crate::i18n::tr("Rule collections…"))
                        .disabled(busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_collections(window, cx)),
                        ),
                )
                .child(
                    Button::new("subs-add")
                        .label(crate::i18n::tr("Add subscription…"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.new_subscription(window, cx);
                        })),
                )
                .child(
                    Button::new("subs-edit")
                        .label(crate::i18n::tr("Edit / restore…"))
                        .disabled(busy || selected.is_none())
                        .on_click(cx.listener(|this, _, window, cx| {
                            let row = this.subscriptions.selected().cloned();
                            if let Some(row) = row {
                                this.subscriptions.form = Some(Form::spec(
                                    Some(&row),
                                    this.subscriptions
                                        .page
                                        .as_ref()
                                        .map(|p| p.settings.clone())
                                        .unwrap_or((0, Settings::default())),
                                    window,
                                    cx,
                                ));
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("subs-refresh")
                        .label(crate::i18n::tr("Refresh selected"))
                        .disabled(busy || selected.as_ref().is_none_or(|r| r.removed))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(id) = this.subscriptions.selected {
                                this.subscriptions.send(
                                    &this.backend.subscriptions,
                                    Request::Refresh { id, force: false },
                                    After::Action,
                                );
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("subs-force")
                        .label(crate::i18n::tr("Force refresh"))
                        .disabled(busy || selected.as_ref().is_none_or(|r| r.removed))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(id) = this.subscriptions.selected {
                                this.subscriptions.send(
                                    &this.backend.subscriptions,
                                    Request::Refresh { id, force: true },
                                    After::Action,
                                );
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("subs-remove")
                        .label(crate::i18n::tr("Remove…"))
                        .disabled(busy || selected.as_ref().is_none_or(|r| r.removed))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.subscriptions.remove = true;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("subs-notifications")
                        .label(crate::i18n::tr("Notifications…"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.subscriptions.send(
                                &this.backend.subscriptions,
                                Request::NotificationSettings,
                                After::NotificationsLoad,
                            );
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("subs-defaults")
                        .label(crate::i18n::tr("Defaults…"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.subscriptions.send(
                                &this.backend.subscriptions,
                                Request::Settings,
                                After::Settings,
                            );
                            cx.notify();
                        })),
                );
            toolbar = toolbar
                .child(
                    Button::new("subs-runtime")
                        .label(if state.automatic {
                            crate::i18n::tr("Pause automatic updates")
                        } else {
                            crate::i18n::tr("Start automatic updates")
                        })
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.subscriptions.send(
                                &this.backend.subscriptions,
                                Request::Runtime(!state.automatic),
                                After::Action,
                            );
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("subs-stop")
                        .label(crate::i18n::tr("Cancel queued/current download"))
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.subscriptions.send(
                                &this.backend.subscriptions,
                                Request::Cancel,
                                After::Action,
                            );
                            cx.notify();
                        })),
                );
            body = body
                .child(Input::new(&self.subscriptions.input).id("subs-search"))
                .child(toolbar);
            let mut sorting = div()
                .flex()
                .gap_2()
                .items_center()
                .child(crate::i18n::tr("Sort all matches:"));
            for (label, key) in [("Name", 0), ("Status", 1), ("Updated", 2), ("Size", 3)] {
                sorting = sorting.child(
                    Button::new(("subs-sort", key as usize))
                        .label(crate::i18n::tr(label))
                        .selected(matches!(
                            (self.subscriptions.query.sort, key),
                            (Sort::Name, 0)
                                | (Sort::Status, 1)
                                | (Sort::Updated, 2)
                                | (Sort::Size, 3)
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.subscriptions.query.sort = match key {
                                0 => Sort::Name,
                                1 => Sort::Status,
                                2 => Sort::Updated,
                                _ => Sort::Size,
                            };
                            this.subscriptions.query.offset = 0;
                            this.subscriptions.dirty = true;
                            cx.notify();
                        })),
                );
            }
            sorting = sorting
                .child(
                    Checkbox::new("subs-descending")
                        .label(crate::i18n::tr("Descending"))
                        .checked(self.subscriptions.query.descending)
                        .on_change(cx.listener(|this, v: &bool, _, cx| {
                            this.subscriptions.query.descending = *v;
                            this.subscriptions.query.offset = 0;
                            this.subscriptions.dirty = true;
                            cx.notify();
                        })),
                )
                .child(
                    Checkbox::new("subs-removed")
                        .label(crate::i18n::tr("Show removed"))
                        .checked(self.subscriptions.query.removed)
                        .on_change(cx.listener(|this, v: &bool, _, cx| {
                            this.subscriptions.marked.clear();
                            this.subscriptions.query.removed = *v;
                            this.subscriptions.query.offset = 0;
                            this.subscriptions.selected = None;
                            this.subscriptions.dirty = true;
                            cx.notify();
                        })),
                );
            body = body.child(sorting).child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .child(crate::i18n::tr_args(
                        "%{arg0} marked across pages",
                        &[("arg0", &(self.subscriptions.marked.len()).to_string())],
                    ))
                    .child(
                        Button::new("subs-mark-page")
                            .label(crate::i18n::tr("Mark page"))
                            .disabled(busy || self.subscriptions.query.removed)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(page) = &this.subscriptions.page {
                                    for row in &page.rows {
                                        if !row.removed
                                            && this.subscriptions.marked.len() < MAX_SUBSCRIPTIONS
                                        {
                                            this.subscriptions.marked.insert(row.id);
                                        }
                                    }
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("subs-clear-marks")
                            .label(crate::i18n::tr("Clear marks"))
                            .disabled(busy || self.subscriptions.marked.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.subscriptions.marked.clear();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("subs-transfer")
                            .label(crate::i18n::tr("Import / export…"))
                            .disabled(busy || self.subscriptions.query.removed)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_subscription_transfer(window, cx)
                            })),
                    )
                    .child(
                        Button::new("subs-bulk")
                            .label(crate::i18n::tr("Bulk edit…"))
                            .disabled(busy || self.subscriptions.query.removed)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_subscription_bulk(window, cx)
                            })),
                    ),
            );
            let mut rows = div()
                .id("subs-rows")
                .flex_1()
                .min_h(px(96.))
                .max_h(px(240.))
                .overflow_y_scroll()
                .flex()
                .flex_col();
            if let Some(page) = &self.subscriptions.page {
                if page.rows.is_empty() {
                    rows = rows.child(if self.subscriptions.query.search.is_empty() {
                        crate::i18n::tr("No subscriptions yet. Add a list URL to get started.")
                    } else {
                        crate::i18n::tr("No matching subscriptions")
                    });
                }
                for row in &page.rows {
                    let id = row.id;
                    rows = rows.child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Checkbox::new(("subs-mark", id))
                                    .checked(self.subscriptions.marked.contains(&id))
                                    .disabled(busy || row.removed)
                                    .on_change(cx.listener(move |this, v: &bool, _, cx| {
                                        if *v {
                                            if this.subscriptions.marked.len() < MAX_SUBSCRIPTIONS {
                                                this.subscriptions.marked.insert(id);
                                            }
                                        } else {
                                            this.subscriptions.marked.remove(&id);
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                ListItem::new(("subscription-row", id))
                                    .flex_1()
                                    .selected(self.subscriptions.selected == Some(id))
                                    .child(crate::i18n::tr_args(
                                        "%{arg0} · %{arg1} · %{arg2} · %{arg3} bytes · %{arg4}",
                                        &[
                                            ("arg0", &(row.spec.name).to_string()),
                                            ("arg1", (row.spec.format.key())),
                                            ("arg2", &(row.status).to_string()),
                                            ("arg3", &(row.bytes).to_string()),
                                            (
                                                "arg4",
                                                (if row.spec.enabled {
                                                    "enabled"
                                                } else {
                                                    "disabled"
                                                }),
                                            ),
                                        ],
                                    ))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.subscriptions.selected = Some(id);
                                        this.subscriptions.detail = None;
                                        cx.notify();
                                    })),
                            ),
                    );
                }
            } else {
                rows = rows.child(crate::i18n::tr("Loading subscriptions…"));
            }
            body = body.child(rows);
            if let Some(row) = selected {
                let target = self.backend.status().session;
                let node_label = target
                    .as_ref()
                    .map(|s| s.name.as_str())
                    .unwrap_or("none selected");
                let ready = target.as_ref().is_some_and(|s| {
                    s.notifications && self.backend.history.health.catalogues.ready(s.id)
                });
                body=body.child(div().text_sm().child(crate::i18n::tr_args("Local daemon target: %{node_label}. Choose a different target on the Nodes page.", &[("node_label", (node_label))])))
                    .child(div().flex().gap_2()
                        .child(Button::new("subs-rule-create").label(crate::i18n::tr("Review new list rule…")).disabled(busy||!ready||row.removed||row.publication.is_none()||self.rules.editor_window.is_some()).on_click(cx.listener(|this,_,window,cx|this.prepare_subscription_rules(true,0,window,cx))))
                        .child(Button::new("subs-rule-inspect").label(crate::i18n::tr("Inspect associated rules…")).disabled(busy||target.is_none()||row.publication.is_none()).on_click(cx.listener(|this,_,window,cx|this.prepare_subscription_rules(false,0,window,cx)))))
                    .when(row.publication.is_none(),|v|v.child(div().text_sm().child(crate::i18n::tr("Refresh this URL and format to prepare compatible daemon list data."))));
                body = body
                    .child(div().text_sm().child(crate::i18n::tr_args("%{arg0}\n%{arg1}\nChecked %{arg2} · updated %{arg3} · next %{arg4} · failures %{arg5}\n%{arg6}", &[("arg0", &(row.spec.url).to_string()), ("arg1", &(row.file
                                .clone()
                                .unwrap_or_else(|| "No downloaded data yet".into())).to_string()), ("arg2", &(row.checked).to_string()), ("arg3", &(row.updated).to_string()), ("arg4", &(row.next_due).to_string()), ("arg5", &(row.failures).to_string()), ("arg6", &(row.error).to_string())])))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("subs-preview")
                                    .label(crate::i18n::tr("Inspect downloaded text"))
                                    .disabled(busy || row.file.is_none())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(id) = this.subscriptions.selected {
                                            this.subscriptions.send(
                                                &this.backend.subscriptions,
                                                Request::Preview(id),
                                                After::Preview,
                                            );
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("subs-logs")
                                    .label(crate::i18n::tr("Status history"))
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(id) = this.subscriptions.selected {
                                            this.subscriptions.send(
                                                &this.backend.subscriptions,
                                                Request::Logs(id),
                                                After::Logs,
                                            );
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("subs-copy-path")
                                    .label(crate::i18n::tr("Copy list path"))
                                    .disabled(row.file.is_none())
                                    .on_click(move |_, _, cx| {
                                        if let Some(path) = &row.file {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                path.clone(),
                                            ));
                                        }
                                    }),
                            ),
                    );
            }
            let offset = self.subscriptions.query.offset;
            let total = self.subscriptions.page.as_ref().map_or(0, |p| p.total);
            body = body.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("subs-previous")
                            .label(crate::i18n::tr("Previous"))
                            .disabled(offset == 0 || busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.subscriptions.query.offset =
                                    this.subscriptions.query.offset.saturating_sub(PAGE);
                                this.subscriptions.dirty = true;
                                cx.notify();
                            })),
                    )
                    .child(crate::i18n::tr_args(
                        "%{arg0}–%{arg1} of %{total}",
                        &[
                            (
                                "arg0",
                                &(if total == 0 { 0 } else { offset + 1 }).to_string(),
                            ),
                            ("arg1", &((offset + PAGE).min(total)).to_string()),
                            ("total", &(total).to_string()),
                        ],
                    ))
                    .child(
                        Button::new("subs-next")
                            .label(crate::i18n::tr("Next"))
                            .disabled(offset + PAGE >= total || busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.subscriptions.query.offset += PAGE;
                                this.subscriptions.dirty = true;
                                cx.notify();
                            })),
                    ),
            );
        }
        div().flex_1().min_h_0().flex().flex_col().gap_3().p_6().child(crate::i18n::tr("Blocklist subscriptions"))
            .child(div().text_sm().child(if let Some(id)=state.active{crate::i18n::tr_args("Downloading subscription %{id} · %{arg0} bytes", &[("id", &format!("{}", id)), ("arg0", &format!("{}", state.downloaded.load(std::sync::atomic::Ordering::Relaxed)))])}else if state.automatic&&!state.local_ready{crate::i18n::tr("Automatic updates enabled; waiting for a local daemon")}else if state.automatic{crate::i18n::tr("Automatic updates enabled")}else{crate::i18n::tr("Automatic updates paused; manual refresh is available")}))
            .child(div().text_sm().child(crate::i18n::tr("Downloading lists does not change firewall rules. Previous valid data is kept when a download fails.")))
            .when(state.archived_metadata>0,|v|v.child(div().text_sm().child(crate::i18n::tr_args("%{arg0} older imported metadata records remain archived; reimport the original file to bind them to current identities.", &[("arg0", &(state.archived_metadata).to_string())]))))
            .when_some(state.error.clone().or_else(||self.subscriptions.error.clone()),|v,e|v.child(super::banner(e,cx)))
            .when_some(self.subscriptions.message.clone(),|v,m|v.child(div().text_sm().child(m))).child(body)
    }
}
