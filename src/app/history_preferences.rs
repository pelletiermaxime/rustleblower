//! Retention review and display refresh controls; all persistence is off GPUI.
use crate::history::{
    History,
    storage::{self, Applied, Preview, Snapshot},
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::Button,
    input::{Input, InputState},
    tab::{Tab, TabBar},
};
actions!(rustleblower_history, [ReviewRetention, ApplyRetention]);
pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-enter", ReviewRetention, Some("HistoryPreferences")),
        KeyBinding::new(
            "ctrl-shift-enter",
            ApplyRetention,
            Some("HistoryPreferences"),
        ),
    ]);
}
pub(super) struct Preferences {
    history: History,
    base: Snapshot,
    days: Entity<InputState>,
    mib: Entity<InputState>,
    cleanup: Entity<InputState>,
    refresh: Entity<InputState>,
    tab: usize,
    busy: bool,
    review: Option<Preview>,
    message: Option<String>,
    error: Option<String>,
    focus: FocusHandle,
}
fn number(
    value: u32,
    max: usize,
    window: &mut Window,
    cx: &mut Context<Preferences>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value.to_string())
            .validate(move |v, _| v.len() <= max && v.bytes().all(|b| b.is_ascii_digit()))
    })
}
impl Preferences {
    pub(super) fn new(history: History, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let base = history.storage.snapshot();
        let config = base.config;
        let seconds = history.settings.snapshot().refresh_seconds;
        let days = number(config.days, 3, window, cx);
        let mib = number(config.mib, 4, window, cx);
        let cleanup = number(config.cleanup_minutes, 2, window, cx);
        let refresh = number(seconds.into(), 2, window, cx);
        let focus = cx.focus_handle();
        days.focus_handle(cx).focus(window, cx);
        Self {
            history,
            base,
            days,
            mib,
            cleanup,
            refresh,
            tab: 0,
            busy: false,
            review: None,
            message: None,
            error: None,
            focus,
        }
    }
    fn candidate(&self, cx: &App) -> anyhow::Result<storage::Config> {
        use anyhow::Context;
        let config = storage::Config {
            days: self
                .days
                .read(cx)
                .value()
                .parse()
                .context("Enter retention days")?,
            mib: self
                .mib
                .read(cx)
                .value()
                .parse()
                .context("Enter the database target in MiB")?,
            cleanup_minutes: self
                .cleanup
                .read(cx)
                .value()
                .parse()
                .context("Enter the idle cleanup interval")?,
        };
        config.validate()?;
        Ok(config)
    }
    fn review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.review.is_some() {
            return;
        }
        self.message = None;
        let candidate = match self.candidate(cx) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let Some(reply) = self.history.preview_storage(candidate, self.base.revision) else {
            self.error = Some(crate::i18n::tr(
                "History worker is busy; try reviewing again",
            ));
            cx.notify();
            return;
        };
        self.busy = true;
        self.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = reply
                .await
                .unwrap_or_else(|_| Err("History worker stopped before review".into()));
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(review) => {
                        this.review = Some(review);
                        this.focus.focus(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(review) = &self.review else {
            return;
        };
        let Some(reply) = self
            .history
            .apply_storage(review.candidate, review.previous.revision)
        else {
            self.error = Some(crate::i18n::tr(
                "History worker is busy; try applying again",
            ));
            cx.notify();
            return;
        };
        self.busy = true;
        self.error = None;
        cx.spawn_in(window,async move|this,cx|{let result=reply.await.unwrap_or_else(|_|Err("History worker stopped before saving; reopen preferences to check the saved values".into()));let _=this.update_in(cx,|this,_,cx|{this.busy=false;match result{Ok(report)=>{this.base=report.snapshot;this.review=None;this.message=Some(Self::feedback(&report));},Err(error)=>{this.error=Some(error);this.review=None;}}cx.notify();});}).detach();
        cx.notify();
    }
    fn feedback(report: &Applied) -> String {
        let mut message = crate::i18n::tr_args(
            "Retention saved. Removed %{arg0} event(s), %{arg1} alert(s), %{arg2} counter(s).",
            &[
                ("arg0", &format!("{}", report.removed_events)),
                ("arg1", &format!("{}", report.removed_alerts)),
                ("arg2", &format!("{}", report.removed_counters)),
            ],
        );
        if report.disk_verified {
            message.push_str(&crate::i18n::tr_args(
                " Database allocation: %{arg0} MiB.",
                &[(
                    "arg0",
                    &format!("{:.1}", report.disk.allocated as f64 / 1048576.),
                )],
            ));
            if report.disk.pending() {
                message
                    .push_str(" File compaction is pending; the target is not a filesystem quota.");
            }
        }
        if let Some(warning) = &report.warning {
            message.push(' ');
            message.push_str(warning);
        }
        message
    }
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.base = self.history.storage.snapshot();
        let config = self.base.config;
        for (input, value) in [
            (&self.days, config.days),
            (&self.mib, config.mib),
            (&self.cleanup, config.cleanup_minutes),
            (
                &self.refresh,
                u32::from(self.history.settings.snapshot().refresh_seconds),
            ),
        ] {
            input.update(cx, |input, cx| {
                input.set_value(value.to_string(), window, cx)
            });
        }
        self.review = None;
        self.error = None;
        self.message = Some(crate::i18n::tr("Loaded saved settings."));
        cx.notify();
    }
    fn save_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let seconds = match self.refresh.read(cx).value().parse::<u16>() {
            Ok(value) if value <= 60 => value,
            _ => {
                self.error = Some(crate::i18n::tr(
                    "Refresh interval must be 0 (manual) or 1–60 seconds",
                ));
                cx.notify();
                return;
            }
        };
        let store = self.history.settings.clone();
        self.busy = true;
        self.error = None;
        let task = cx
            .background_executor()
            .spawn(async move { store.save_refresh(seconds).map_err(|e| e.to_string()) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.message = Some(if seconds == 0 {
                            "Manual refresh saved. History continues to be recorded.".into()
                        } else {
                            crate::i18n::tr_args(
                                "Display refresh saved: every %{seconds} second(s).",
                                &[("seconds", &format!("{}", seconds))],
                            )
                        })
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
impl Focusable for Preferences {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Preferences {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = div().flex().flex_col().gap_3();
        if self.tab == 0 {
            body=body.child(div().text_sm().child(crate::i18n::tr_args("Saved: %{arg0} days · %{arg1} MiB target · idle cleanup every %{arg2} minute(s)", &[("arg0", &(self.base.config.days).to_string()), ("arg1", &(self.base.config.mib).to_string()), ("arg2", &(self.base.config.cleanup_minutes).to_string())])))
   .child(div().text_sm().child(crate::i18n::tr("Retention days (1–365)")))
   .child(div().h_8().flex_shrink_0().child(Input::new(&self.days).id("history-days").disabled(self.busy||self.review.is_some())))
   .child(div().text_sm().child(crate::i18n::tr("Database target in MiB (16–4096)")))
   .child(div().h_8().flex_shrink_0().child(Input::new(&self.mib).id("history-mib").disabled(self.busy||self.review.is_some())))
   .child(div().text_sm().child(crate::i18n::tr("Idle cleanup interval in minutes (1–60)")))
   .child(div().h_8().flex_shrink_0().child(Input::new(&self.cleanup).id("history-cleanup").disabled(self.busy||self.review.is_some())))
   .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::tr("Age applies to retained events, daemon alerts and statistics. Size limits are checked during writes; idle cleanup also removes expired data while traffic is quiet. Smaller limits can remove older data. Existing files may take time to compact.")));
            if let Some(review) = &self.review {
                body=div().flex().flex_col().gap_3().child(div().text_lg().child(crate::i18n::tr("Review retention changes"))).child(div().text_sm().child(crate::i18n::tr_args("Review: %{arg0} → %{arg1} days; %{arg2} → %{arg3} MiB; %{arg4} → %{arg5} minutes. %{arg6} event(s), %{arg7} alert(s), %{arg8} counter(s) are already expired at these settings. Size limits may remove additional oldest history. Removal cannot be undone.", &[("arg0", &(review.previous.config.days).to_string()), ("arg1", &(review.candidate.days).to_string()), ("arg2", &(review.previous.config.mib).to_string()), ("arg3", &(review.candidate.mib).to_string()), ("arg4", &(review.previous.config.cleanup_minutes).to_string()), ("arg5", &(review.candidate.cleanup_minutes).to_string()), ("arg6", &(review.expired_events).to_string()), ("arg7", &(review.expired_alerts).to_string()), ("arg8", &(review.expired_counters).to_string())])))
   .child(div().text_sm().child(crate::i18n::tr_args("Database currently allocated: %{arg0} MiB; reusable pages: %{arg1} MiB. Cached application rules and node settings are preserved. Other cached data may keep the file above a smaller target.", &[("arg0", &format!("{:.1}", review.disk.allocated as f64/1048576.)), ("arg1", &format!("{:.1}", review.disk.reusable as f64/1048576.))])))
   .child(Button::new("history-retention-back").label(crate::i18n::tr("Back without applying")).disabled(self.busy).on_click(cx.listener(|this,_,_,cx|{this.review=None;cx.notify();})));
            }
        } else {
            body=body.child(div().text_sm().child(crate::i18n::tr("History display refresh in seconds (0 = manual, 1–60 = automatic)")))
   .child(div().h_8().flex_shrink_0().child(Input::new(&self.refresh).id("history-refresh-seconds").disabled(self.busy)))
   .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::tr("Manual mode freezes automatic display updates. Recording, permission prompts, searching, paging and Refresh now remain available. This does not change the daemon's reporting interval.")))
;
        }
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("history-preferences-reload")
                        .label(crate::i18n::tr("Reload"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                )
                .child(
                    Button::new("history-preferences-copy-path")
                        .label(crate::i18n::tr("Copy database path"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                this.history.database.display().to_string(),
                            ))
                        })),
                ),
        );
        let save = Button::new(if self.tab == 1 {
            "history-refresh-save"
        } else if self.review.is_some() {
            "history-retention-apply"
        } else {
            "history-retention-review"
        })
        .disabled(self.busy)
        .on_click(cx.listener(|this, _, window, cx| {
            if this.tab == 1 {
                this.save_refresh(window, cx);
            } else if this.review.is_some() {
                this.apply(window, cx);
            } else {
                this.review(window, cx);
            }
        }));
        div()
            .key_context("HistoryPreferences")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ReviewRetention, window, cx| this.review(window, cx)))
            .on_action(cx.listener(|this, _: &ApplyRetention, window, cx| this.apply(window, cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(super::settings::heading("History"))
            .child(
                div()
                    .px_5()
                    .pb_3()
                    .text_xs()
                    .truncate()
                    .child(crate::i18n::tr_args(
                        "Database: %{arg0}",
                        &[("arg0", &self.history.database.display().to_string())],
                    )),
            )
            .child(
                div().px_5().pb_4().child(
                    TabBar::new("history-preferences-tabs")
                        .selected_index(self.tab)
                        .on_click(cx.listener(|this, index: &usize, _, cx| {
                            this.tab = *index;
                            cx.notify();
                        }))
                        .child(Tab::new().label(crate::i18n::tr("Storage & retention")))
                        .child(Tab::new().label(crate::i18n::tr("Display refresh"))),
                ),
            )
            .child(
                div()
                    .id("history-preferences-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_5()
                    .pb_5()
                    .child(body),
            )
            .when(self.busy, |view| {
                view.child(div().px_5().py_2().text_sm().child(crate::i18n::tr(
                    "Working in the background… Permission prompts remain available.",
                )))
            })
            .when_some(self.error.clone(), |view, error| {
                view.child(
                    div()
                        .px_5()
                        .py_2()
                        .text_sm()
                        .child(crate::i18n::presentation(&error)),
                )
            })
            .when_some(self.message.clone(), |view, message| {
                view.child(
                    div()
                        .px_5()
                        .py_2()
                        .text_sm()
                        .child(crate::i18n::presentation(&message)),
                )
            })
            .child(super::settings::footer(
                "history-preferences-close",
                self.busy,
                save,
                cx,
            ))
    }
}
