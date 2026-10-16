use crate::{
    backend::Handle,
    config::Decision,
    preferences::{CONSTRAINTS, FALLBACKS, Popup, TARGETS},
    prompts::choice::DURATIONS,
    prompts::placement::{POSITIONS, Position},
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
};
type Picker = Entity<SelectState<SearchableVec<SharedString>>>;

pub(super) fn open(backend: Handle, cx: &mut App) -> gpui::Result<AnyWindowHandle> {
    let (handle, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(760.), px(700.)),
                cx,
            ))),
            window_min_size: Some(size(px(640.), px(560.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Rustleblower — popup preferences".into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| Preferences::new(backend, window, cx)),
    )?;
    Ok(handle)
}
struct Preferences {
    backend: Handle,
    draft: Popup,
    duration: Picker,
    target: Picker,
    fallback: Picker,
    position: Picker,
    timeout: Entity<InputState>,
    tab: usize,
    _subscriptions: Vec<Subscription>,
    busy: bool,
    message: Option<String>,
}
impl Preferences {
    fn new(backend: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = backend.preferences.snapshot();
        let duration = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    DURATIONS
                        .iter()
                        .map(|(_, label)| SharedString::from(*label))
                        .collect::<Vec<_>>(),
                ),
                Some(IndexPath::new(
                    DURATIONS
                        .iter()
                        .position(|(key, _)| *key == draft.duration)
                        .unwrap_or(0),
                )),
                window,
                cx,
            )
            .searchable(true)
        });
        let target = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    TARGETS
                        .iter()
                        .map(|(_, label)| SharedString::from(*label))
                        .collect::<Vec<_>>(),
                ),
                Some(IndexPath::new(
                    TARGETS
                        .iter()
                        .position(|(key, _)| *key == draft.target)
                        .unwrap_or(0),
                )),
                window,
                cx,
            )
            .searchable(true)
        });
        let fallback = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    FALLBACKS
                        .iter()
                        .map(|(_, label)| SharedString::from(*label))
                        .collect::<Vec<_>>(),
                ),
                Some(IndexPath::new(
                    FALLBACKS
                        .iter()
                        .position(|(key, _)| {
                            *key == draft.fallback.map(Decision::action).unwrap_or("inherit")
                        })
                        .unwrap_or(0),
                )),
                window,
                cx,
            )
            .searchable(true)
        });
        let timeout = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(
                    draft
                        .timeout
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                )
                .placeholder("1–110; blank uses launch setting")
                .validate(|text, _| {
                    text.len() <= 3 && text.bytes().all(|byte| byte.is_ascii_digit())
                })
        });
        let position = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    POSITIONS
                        .iter()
                        .map(|(_, label)| SharedString::from(*label))
                        .collect::<Vec<_>>(),
                ),
                Some(IndexPath::new(
                    POSITIONS
                        .iter()
                        .position(|(key, _)| *key == draft.position.key())
                        .unwrap_or(0),
                )),
                window,
                cx,
            )
            .searchable(true)
        });
        let subscriptions = [&duration, &target, &fallback, &position]
            .into_iter()
            .map(|picker| {
                cx.subscribe(
                    picker,
                    |_, _, _: &SelectEvent<SearchableVec<SharedString>>, cx| cx.notify(),
                )
            })
            .collect();
        Self {
            backend,
            draft,
            duration,
            target,
            fallback,
            position,
            timeout,
            tab: 0,
            _subscriptions: subscriptions,
            busy: false,
            message: None,
        }
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let mut draft = self.draft.clone();
        if let Some(label) = self.position.read(cx).selected_value()
            && let Some((key, _)) = POSITIONS.iter().find(|(_, name)| *name == label.as_ref())
        {
            draft.position = Position::parse(key).unwrap_or_default();
        }
        let seconds = self.timeout.read(cx).value();
        draft.timeout = if seconds.trim().is_empty() {
            None
        } else {
            match seconds.trim().parse::<u64>() {
                Ok(seconds) => Some(seconds),
                Err(_) => {
                    self.message=Some("Enter a timeout between 1 and 110 seconds, or leave it blank to use the launch setting.".into());
                    cx.notify();
                    return;
                }
            }
        };
        draft.fallback = self
            .fallback
            .read(cx)
            .selected_value()
            .and_then(|label| FALLBACKS.iter().find(|(_, name)| *name == label.as_ref()))
            .and_then(|(key, _)| {
                [Decision::Allow, Decision::Deny, Decision::Reject]
                    .into_iter()
                    .find(|decision| decision.action() == *key)
            });
        if let Some(label) = self.duration.read(cx).selected_value()
            && let Some((key, _)) = DURATIONS.iter().find(|(_, name)| *name == label.as_ref())
        {
            draft.duration = (*key).into();
        }
        if let Some(label) = self.target.read(cx).selected_value()
            && let Some((key, _)) = TARGETS.iter().find(|(_, name)| *name == label.as_ref())
        {
            draft.target = (*key).into();
        }
        let store = self.backend.preferences.clone();
        if let Err(error) = draft.validate() {
            self.message = Some(error.to_string());
            cx.notify();
            return;
        }
        let save = cx.background_executor().spawn(async move {
            store
                .save(draft)
                .map_err(|error| format!("Cannot save popup preferences: {error:#}"))
        });
        self.busy = true;
        self.message = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = save.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                this.message = Some(match result {
                    Ok(()) => "Saved. Permission defaults apply to new requests; desktop notification settings apply immediately.".into(),
                    Err(error) => error,
                });
                cx.notify();
            });
        })
        .detach();
    }
}
impl Render for Preferences {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let placement_supported = super::placement::supported(window);
        let form = if self.tab == 0 {
            div().flex().flex_col().gap_4()
            .child(div().text_sm().child("These UI defaults apply to new requests. They do not change daemon configuration or existing rules."))
            .child(Checkbox::new("preferences-popups").label("Show permission popups").checked(self.draft.popups).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.draft.popups = *value;cx.notify();})))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Disabled popups immediately use the configured one-time fallback. Pending requests keep their captured settings."))
            .child(Checkbox::new("preferences-notifications").label("Desktop notifications for daemon alerts").checked(self.draft.desktop_notifications).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.draft.desktop_notifications = *value;cx.notify();})))
            .child(div().text_sm().child("Default duration"))
            .child(div().h_8().flex_shrink_0().child(Select::new(&self.duration).id("preferences-duration").w_full().disabled(self.busy)))
            .child(div().text_sm().child("Default match target"))
            .child(div().h_8().flex_shrink_0().child(Select::new(&self.target).id("preferences-target").w_full().disabled(self.busy)))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Unavailable targets fall back to executable + destination. Review broad or lasting defaults before answering."))
            .child(div().flex().gap_4()
                .child(Checkbox::new("preferences-priority").label("Priority").checked(self.draft.priority).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.draft.priority = *value;cx.notify();})))
                .child(Checkbox::new("preferences-logging").label("Log matching connections").checked(self.draft.logging).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.draft.logging = *value;cx.notify();}))))
        } else if self.tab == 2 {
            div().flex().flex_col().gap_4()
                .child(div().text_sm().child("Initial popup position (X11 primary display)"))
                .child(div().h_8().flex_shrink_0().child(Select::new(&self.position).id("preferences-position").w_full().disabled(self.busy || !placement_supported)))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(if placement_supported {"Popups open within the primary display’s available workspace. Your window manager may adjust the initial position. Expanding or manually moving a popup does not re-anchor it."}else{"This compositor controls window placement. Saved corner preferences apply when running on X11."}))
        } else {
            let mut form = div()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    Checkbox::new("preferences-expanded")
                        .label("Open rule options on new requests")
                        .checked(self.draft.show_advanced)
                        .disabled(self.busy)
                        .on_change(cx.listener(|this, value: &bool, _, cx| {
                            this.draft.show_advanced = *value;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .text_sm()
                        .child("Always add these reported fields with AND"),
                );
            for (key, label) in CONSTRAINTS {
                let key = key.to_string();
                let checked = self.draft.constraints.contains(&key);
                form = form.child(
                    Checkbox::new(SharedString::from(format!("preferences-condition-{key}")))
                        .label(*label)
                        .checked(checked)
                        .disabled(self.busy)
                        .on_change(cx.listener(move |this, value: &bool, _, cx| {
                            if *value {
                                if !this.draft.constraints.contains(&key) {
                                    this.draft.constraints.push(key.clone());
                                }
                            } else {
                                this.draft.constraints.retain(|field| field != &key);
                            }
                            cx.notify();
                        })),
                );
            }
            form.child(div().text_sm().text_color(cx.theme().muted_foreground).child("Unavailable requested fields open the options with a warning. Checksums require daemon monitoring; reported MD5 is preferred, then SHA1."))
                .child(div().text_sm().child("Wait before applying the fallback (seconds)"))
                .child(div().h_8().flex_shrink_0().child(Input::new(&self.timeout).id("preferences-timeout").disabled(self.busy)))
                .child(div().text_sm().child("Unanswered, disabled, overloaded or disconnected requests"))
                .child(div().h_8().flex_shrink_0().child(Select::new(&self.fallback).id("preferences-fallback").w_full().disabled(self.busy)))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(format!("Launch setting: {} once after {} seconds. A saved override applies only to new requests. The daemon's RPC deadline can be shorter.",self.backend.timeout_action.action(),self.backend.prompt_timeout)))
        };
        div()
            .size_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().text_xl().child("Popup preferences"))
            .child(
                TabBar::new("preferences-tabs")
                    .selected_index(self.tab)
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.tab = *index;
                        cx.notify();
                    }))
                    .child(Tab::new().label("Popup defaults"))
                    .child(Tab::new().label("Conditions & timeout"))
                    .child(Tab::new().label("Placement")),
            )
            .child(
                div()
                    .id("popup-preferences-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(form),
            )
            .when_some(self.message.clone(), |view, message| {
                view.child(div().text_sm().child(message))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_3()
                    .child(
                        Button::new("preferences-close")
                            .label("Close")
                            .disabled(self.busy)
                            .on_click(|_, window, _| window.remove_window()),
                    )
                    .child(
                        Button::new("preferences-save")
                            .label(if self.busy {
                                "Saving…"
                            } else {
                                "Save preferences"
                            })
                            .primary()
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}
