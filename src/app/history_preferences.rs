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
pub(super) fn open(history: History, cx: &mut App) -> gpui::Result<AnyWindowHandle> {
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(800.), px(700.)),
                cx,
            ))),
            window_min_size: Some(size(px(680.), px(560.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Rustleblower — history preferences".into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| Preferences::new(history, window, cx)),
    )?;
    Ok(handle)
}
struct Preferences {
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
    fn new(history: History, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            self.error = Some("History worker is busy; try reviewing again".into());
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
            self.error = Some("History worker is busy; try applying again".into());
            cx.notify();
            return;
        };
        self.busy = true;
        self.error = None;
        cx.spawn_in(window,async move|this,cx|{let result=reply.await.unwrap_or_else(|_|Err("History worker stopped before saving; reopen preferences to check the saved values".into()));let _=this.update_in(cx,|this,_,cx|{this.busy=false;match result{Ok(report)=>{this.base=report.snapshot;this.review=None;this.message=Some(Self::feedback(&report));},Err(error)=>{this.error=Some(error);this.review=None;}}cx.notify();});}).detach();
        cx.notify();
    }
    fn feedback(report: &Applied) -> String {
        let mut message = format!(
            "Retention saved. Removed {} event(s), {} alert(s), {} counter(s).",
            report.removed_events, report.removed_alerts, report.removed_counters
        );
        if report.disk_verified {
            message.push_str(&format!(
                " Database allocation: {:.1} MiB.",
                report.disk.allocated as f64 / 1048576.
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
        self.message = Some("Loaded saved settings.".into());
        cx.notify();
    }
    fn save_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let seconds = match self.refresh.read(cx).value().parse::<u16>() {
            Ok(value) if value <= 60 => value,
            _ => {
                self.error = Some("Refresh interval must be 0 (manual) or 1–60 seconds".into());
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
                            format!("Display refresh saved: every {seconds} second(s).")
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
            body=body.child(div().text_sm().child(format!("Saved: {} days · {} MiB target · idle cleanup every {} minute(s)",self.base.config.days,self.base.config.mib,self.base.config.cleanup_minutes)))
   .child(div().text_sm().child("Retention days (1–365)"))
   .child(div().h_8().flex_shrink_0().child(Input::new(&self.days).id("history-days").disabled(self.busy||self.review.is_some())))
   .child(div().text_sm().child("Database target in MiB (16–4096)"))
   .child(div().h_8().flex_shrink_0().child(Input::new(&self.mib).id("history-mib").disabled(self.busy||self.review.is_some())))
   .child(div().text_sm().child("Idle cleanup interval in minutes (1–60)"))
   .child(div().h_8().flex_shrink_0().child(Input::new(&self.cleanup).id("history-cleanup").disabled(self.busy||self.review.is_some())))
   .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Age applies to retained events, daemon alerts and statistics. Size limits are checked during writes; idle cleanup also removes expired data while traffic is quiet. Smaller limits can remove older data. Existing files may take time to compact."));
            if let Some(review) = &self.review {
                body=div().flex().flex_col().gap_3().child(div().text_lg().child("Review retention changes")).child(div().text_sm().child(format!("Review: {} → {} days; {} → {} MiB; {} → {} minutes. {} event(s), {} alert(s), {} counter(s) are already expired at these settings. Size limits may remove additional oldest history. Removal cannot be undone.",review.previous.config.days,review.candidate.days,review.previous.config.mib,review.candidate.mib,review.previous.config.cleanup_minutes,review.candidate.cleanup_minutes,review.expired_events,review.expired_alerts,review.expired_counters)))
   .child(div().text_sm().child(format!("Database currently allocated: {:.1} MiB; reusable pages: {:.1} MiB. Cached application rules and node settings are preserved. Other cached data may keep the file above a smaller target.",review.disk.allocated as f64/1048576.,review.disk.reusable as f64/1048576.)))
   .child(div().flex().gap_2().child(Button::new("history-retention-back").label("Back without applying").disabled(self.busy).on_click(cx.listener(|this,_,_,cx|{this.review=None;cx.notify();}))).child(Button::new("history-retention-apply").label("Apply retention and cleanup").disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.apply(window,cx)))));
            } else {
                body = body.child(
                    Button::new("history-retention-review")
                        .label("Review retention changes")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.review(window, cx))),
                );
            }
        } else {
            body=body.child(div().text_sm().child("History display refresh in seconds (0 = manual, 1–60 = automatic)"))
   .child(div().h_8().flex_shrink_0().child(Input::new(&self.refresh).id("history-refresh-seconds").disabled(self.busy)))
   .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Manual mode freezes automatic display updates. Recording, permission prompts, searching, paging and Refresh now remain available. This does not change the daemon's reporting interval."))
   .child(Button::new("history-refresh-save").label("Save display refresh").disabled(self.busy).on_click(cx.listener(|this,_,window,cx|this.save_refresh(window,cx))));
        }
        div()
            .key_context("HistoryPreferences")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ReviewRetention, window, cx| this.review(window, cx)))
            .on_action(cx.listener(|this, _: &ApplyRetention, window, cx| this.apply(window, cx)))
            .size_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_xl().child("History preferences"))
            .child(
                div()
                    .text_xs()
                    .truncate()
                    .child(format!("Database: {}", self.history.database.display())),
            )
            .child(
                TabBar::new("history-preferences-tabs")
                    .selected_index(self.tab)
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.tab = *index;
                        cx.notify();
                    }))
                    .child(Tab::new().label("Storage & retention"))
                    .child(Tab::new().label("Display refresh")),
            )
            .child(
                div()
                    .id("history-preferences-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .when(self.busy, |view| {
                view.child(
                    div()
                        .text_sm()
                        .child("Working in the background… Permission prompts remain available."),
                )
            })
            .when_some(self.error.clone(), |view, error| {
                view.child(div().text_sm().child(error))
            })
            .when_some(self.message.clone(), |view, message| {
                view.child(div().text_sm().child(message))
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("history-preferences-reload")
                            .label("Reload saved values")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                    )
                    .child(
                        Button::new("history-preferences-copy-path")
                            .label("Copy database path")
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    this.history.database.display().to_string(),
                                ))
                            })),
                    )
                    .child(
                        Button::new("history-preferences-close")
                            .label("Close")
                            .disabled(self.busy)
                            .on_click(|_, window, _| window.remove_window()),
                    ),
            )
    }
}
