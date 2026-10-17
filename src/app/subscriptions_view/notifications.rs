use super::*;
use crate::subscriptions::notifications::{Config, MAX_MESSAGE};

pub(super) struct Notifications {
    revision: u64,
    enabled: bool,
    success: Entity<InputState>,
    error: Entity<InputState>,
}
impl Notifications {
    pub(super) fn new(
        revision: u64,
        config: Config,
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> Self {
        let success = field(&config.success_message, MAX_MESSAGE, window, cx);
        let error = field(&config.error_message, MAX_MESSAGE, window, cx);
        Self {
            revision,
            enabled: config.enabled,
            success,
            error,
        }
    }
    fn config(&self, cx: &App) -> Config {
        Config {
            enabled: self.enabled,
            success_message: self.success.read(cx).value().to_string(),
            error_message: self.error.read(cx).value().to_string(),
        }
    }
}
impl Dashboard {
    pub(super) fn save_subscription_notifications(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.subscriptions.notifications {
            let config = view.config(cx);
            match config.validate() {
                Ok(()) => self.subscriptions.send(
                    &self.backend.subscriptions,
                    Request::SaveNotificationSettings {
                        revision: view.revision,
                        config,
                    },
                    After::NotificationsSave,
                ),
                Err(error) => self.subscriptions.error = Some(error),
            }
        }
        cx.notify();
    }
    pub(super) fn render_subscription_notifications(&mut self, cx: &mut Context<Self>) -> Div {
        let view = self.subscriptions.notifications.as_ref().unwrap();
        let busy = self.subscriptions.pending.is_some();
        div().flex().flex_col().gap_3()
            .child(crate::i18n::tr("Subscription desktop notifications"))
            .child(div().text_sm().child(crate::i18n::tr("Show successful checks and download errors, including manual refreshes. Cancelled downloads and URL tests stay quiet. Desktop notifications must also be enabled in Popup preferences.")))
            .child(Checkbox::new("subs-notify-enabled").label(crate::i18n::tr("Notify when subscription checks finish")).checked(view.enabled).disabled(busy).on_click(cx.listener(|this, checked: &bool, _, cx| {
                if let Some(view) = &mut this.subscriptions.notifications { view.enabled = *checked; }
                cx.notify();
            })))
            .child(crate::i18n::tr("Success message (empty uses the subscription name and status)"))
            .child(Input::new(&view.success).id("subs-notify-success").disabled(busy || !view.enabled))
            .child(crate::i18n::tr("Failure message (empty uses the subscription name and error)"))
            .child(Input::new(&view.error).id("subs-notify-error").disabled(busy || !view.enabled))
            .child(div().text_sm().child(crate::i18n::tr("Turning this off stops subscription notices and clears the saved custom messages.")))
            .child(div().flex().gap_2()
                .child(Button::new("subs-notify-save").label(crate::i18n::tr("Save notification settings")).disabled(busy).on_click(cx.listener(|this, _, _, cx| {
                    this.save_subscription_notifications(cx);
                })))
                .child(Button::new("subs-notify-reload").label(crate::i18n::tr("Reload saved settings")).disabled(busy).on_click(cx.listener(|this, _, _, cx| {
                    this.subscriptions.send(&this.backend.subscriptions, Request::NotificationSettings, After::NotificationsLoad);
                    cx.notify();
                })))
                .child(Button::new("subs-notify-cancel").label(crate::i18n::tr("Back")).disabled(busy).on_click(cx.listener(|this, _, window, cx| {
                    this.subscriptions.notifications = None;
                    this.subscriptions.error = None;
                    this.subscriptions.input.focus_handle(cx).focus(window, cx);
                    cx.notify();
                }))))
    }
}
