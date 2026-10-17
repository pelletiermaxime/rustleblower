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
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
};
type Picker = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<String>>>>;

#[cfg(test)]
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
                title: Some(crate::i18n::tr("Rustleblower — popup preferences").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| Preferences::new(backend, window, cx)),
    )?;
    crate::i18n::register_title(handle, "Rustleblower — popup preferences", cx);
    Ok(handle)
}
pub(super) struct Preferences {
    backend: Handle,
    draft: Popup,
    duration: Picker,
    target: Picker,
    fallback: Picker,
    position: Picker,
    timeout: Entity<InputState>,
    missed_template: Entity<InputState>,
    tab: usize,
    _subscriptions: Vec<Subscription>,
    busy: bool,
    message: Option<String>,
}
impl Preferences {
    pub(super) fn new(backend: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let draft = backend.preferences.snapshot();
        let missed_template = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(crate::desktop_notifications::missed::encode_input(
                    &draft.missed_popup_template,
                ))
                .validate(|text, _| {
                    text.len() <= crate::desktop_notifications::missed::MAX_TEMPLATE * 2
                        && !text.chars().any(char::is_control)
                })
        });
        let duration = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    DURATIONS
                        .iter()
                        .map(|(key, label)| {
                            crate::i18n::LocalizedOption::new((*key).to_owned(), *label)
                        })
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
                        .map(|(key, label)| {
                            crate::i18n::LocalizedOption::new((*key).to_owned(), *label)
                        })
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
                        .map(|(key, label)| {
                            crate::i18n::LocalizedOption::new((*key).to_owned(), *label)
                        })
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
                .placeholder(crate::i18n::placeholder(
                    "1–110; blank uses launch setting",
                    window,
                    cx,
                ))
                .validate(|text, _| {
                    text.len() <= 3 && text.bytes().all(|byte| byte.is_ascii_digit())
                })
        });
        let position = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    POSITIONS
                        .iter()
                        .map(|(key, label)| {
                            crate::i18n::LocalizedOption::new((*key).to_owned(), *label)
                        })
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
        let mut subscriptions = [&duration, &target, &fallback, &position]
            .into_iter()
            .map(|picker| {
                cx.subscribe(
                    picker,
                    |_,
                     _,
                     _: &SelectEvent<SearchableVec<crate::i18n::LocalizedOption<String>>>,
                     cx| cx.notify(),
                )
            })
            .collect::<Vec<_>>();
        subscriptions.push(cx.subscribe(&missed_template, |_, _, _: &InputEvent, cx| cx.notify()));
        Self {
            backend,
            draft,
            duration,
            target,
            fallback,
            position,
            timeout,
            missed_template,
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
        draft.missed_popup_template = match crate::desktop_notifications::missed::decode_input(
            &self.missed_template.read(cx).value(),
        ) {
            Ok(template) => template,
            Err(error) => {
                self.message = Some(error.into());
                cx.notify();
                return;
            }
        };
        if let Some(key) = self.position.read(cx).selected_value() {
            draft.position = Position::parse(key).unwrap_or_default();
        }
        let seconds = self.timeout.read(cx).value();
        draft.timeout = if seconds.trim().is_empty() {
            None
        } else {
            match seconds.trim().parse::<u64>() {
                Ok(seconds) => Some(seconds),
                Err(_) => {
                    self.message = Some(crate::i18n::tr(
                        "Enter a timeout between 1 and 110 seconds, or leave it blank to use the launch setting.",
                    ));
                    cx.notify();
                    return;
                }
            }
        };
        draft.fallback = self.fallback.read(cx).selected_value().and_then(|key| {
            [Decision::Allow, Decision::Deny, Decision::Reject]
                .into_iter()
                .find(|decision| decision.action() == key)
        });
        if let Some(key) = self.duration.read(cx).selected_value() {
            draft.duration = key.clone();
        }
        if let Some(key) = self.target.read(cx).selected_value() {
            draft.target = key.clone();
        }
        let store = self.backend.preferences.clone();
        if let Err(error) = draft.validate() {
            self.message = Some(error.to_string());
            cx.notify();
            return;
        }
        let save = cx.background_executor().spawn(async move {
            store.save(draft).map_err(|error| {
                crate::i18n::tr_args(
                    "Cannot save popup preferences: %{error}",
                    &[("error", &format!("{:#}", error))],
                )
            })
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
            .child(div().text_sm().child(crate::i18n::tr("These UI defaults apply to new requests. They do not change daemon configuration or existing rules.")))
            .child(Checkbox::new("preferences-popups").label(crate::i18n::tr("Show permission popups")).checked(self.draft.popups).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.draft.popups = *value;cx.notify();})))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::tr("Disabled popups immediately use the configured one-time fallback. Pending requests keep their captured settings.")))
            .child(Checkbox::new("preferences-notifications").label(crate::i18n::tr("Desktop notifications")).checked(self.draft.desktop_notifications).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.draft.desktop_notifications = *value;cx.notify();})))
            .child(div().text_sm().child(crate::i18n::tr("Default duration")))
            .child(div().h_8().flex_shrink_0().child(Select::new(&self.duration).id("preferences-duration").w_full().disabled(self.busy)))
            .child(div().text_sm().child(crate::i18n::tr("Default match target")))
            .child(div().h_8().flex_shrink_0().child(Select::new(&self.target).id("preferences-target").w_full().disabled(self.busy)))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::tr("Unavailable targets fall back to executable + destination. Review broad or lasting defaults before answering.")))
            .child(div().flex().gap_4()
                .child(Checkbox::new("preferences-priority").label(crate::i18n::tr("Priority")).checked(self.draft.priority).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.draft.priority = *value;cx.notify();})))
                .child(Checkbox::new("preferences-logging").label(crate::i18n::tr("Log matching connections")).checked(self.draft.logging).disabled(self.busy).on_change(cx.listener(|this,value:&bool,_,cx|{this.draft.logging = *value;cx.notify();}))))
        } else if self.tab == 3 {
            let input = Input::new(&self.missed_template)
                .id("preferences-missed-template")
                .disabled(self.busy);
            let example = crate::protocol::Connection {
                process_path: "/usr/bin/example".into(),
                process_args: vec!["example".into(), "--connect".into()],
                dst_host: "example.org".into(),
                dst_ip: "192.0.2.42".into(),
                dst_port: 443,
                protocol: "tcp".into(),
                process_cwd: "/home/example".into(),
                user_id: 1000,
                process_id: 123,
                ..Default::default()
            };
            let action = self
                .fallback
                .read(cx)
                .selected_value()
                .map(String::as_str)
                .filter(|key| *key != "inherit")
                .unwrap_or(self.backend.timeout_action.action());
            let decoded = crate::desktop_notifications::missed::decode_input(
                &self.missed_template.read(cx).value(),
            );
            let preview = decoded
                .as_deref()
                .map(|template| {
                    crate::desktop_notifications::missed::render(
                        template,
                        &example,
                        &crate::protocol::Rule {
                            action: action.into(),
                            ..Default::default()
                        },
                        "",
                        "example-host",
                    )
                })
                .unwrap_or_else(|error| error.to_string());
            div().flex().flex_col().gap_3()
                .child(Checkbox::new("preferences-missed-notifications").label(crate::i18n::tr("Notify after an unanswered permission popup"))
                    .checked(self.draft.missed_popup_notifications).disabled(self.busy)
                    .on_change(cx.listener(|this, value: &bool, _, cx| { this.draft.missed_popup_notifications = *value; cx.notify(); })))
                .child(div().text_sm().child(crate::i18n::tr("When a popup times out, the notice explains the fallback action. Open Application rules from the notice to allow or block an app manually. Desktop notifications must also be enabled in Popup defaults.")))
                .child(crate::i18n::tr("Message template (use \\n for a new line and \\\\ for a backslash)"))
                .child(input)
                .child(div().text_sm().child(crate::i18n::tr("Fields: %rule.action%, %conn.process%, %conn.process_args%, %conn.process_cwd%, %conn.dsthost%, %conn.dstip%, %conn.dstport%, %conn.srcip%, %conn.srcport%, %conn.proto%, %conn.pid%, %conn.uid%, %node.addr%, %node.hostname%. Unrecognized fields remain as written.")))
                .child(Button::new("preferences-missed-reset").label(crate::i18n::tr("Use default message")).disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.missed_template.update(cx, |state, cx| state.set_value(crate::desktop_notifications::missed::encode_input(crate::desktop_notifications::missed::DEFAULT_TEMPLATE), window, cx));
                        cx.notify();
                    })))
                .child(crate::i18n::tr("Example preview"))
                .child(div().id("preferences-missed-preview").text_sm().child(preview))
        } else if self.tab == 2 {
            div().flex().flex_col().gap_4()
                .child(div().text_sm().child(crate::i18n::tr("Initial popup position (X11 primary display)")))
                .child(div().h_8().flex_shrink_0().child(Select::new(&self.position).id("preferences-position").w_full().disabled(self.busy || !placement_supported)))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(if placement_supported {crate::i18n::tr("Popups open within the primary display’s available workspace. Your window manager may adjust the initial position. Expanding or manually moving a popup does not re-anchor it.")}else{crate::i18n::tr("This compositor controls window placement. Saved corner preferences apply when running on X11.")}))
        } else {
            let mut form = div()
                .flex()
                .flex_col()
                .child(
                    Checkbox::new("preferences-expanded")
                        .label(crate::i18n::tr("Open rule options on new requests"))
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
                        .child(crate::i18n::tr("Always add these reported fields with AND")),
                );
            for (key, label) in CONSTRAINTS {
                let key = key.to_string();
                let checked = self.draft.constraints.contains(&key);
                form = form.child(
                    Checkbox::new(SharedString::from(format!("preferences-condition-{key}")))
                        .label(crate::i18n::tr(label))
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
            form.child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::tr("Unavailable requested fields open the options with a warning. Checksums require daemon monitoring; reported MD5 is preferred, then SHA1.")))
                .child(div().text_sm().child(crate::i18n::tr("Wait before applying the fallback (seconds)")))
                .child(div().h_8().flex_shrink_0().child(Input::new(&self.timeout).id("preferences-timeout").disabled(self.busy)))
                .child(div().text_sm().child(crate::i18n::tr("Unanswered, disabled, overloaded or disconnected requests")))
                .child(div().h_8().flex_shrink_0().child(Select::new(&self.fallback).id("preferences-fallback").w_full().disabled(self.busy)))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::tr_args("Launch setting: %{arg0} once after %{arg1} seconds. A saved override applies only to new requests. The daemon's RPC deadline can be shorter.", &[("arg0", (self.backend.timeout_action.action())), ("arg1", &(self.backend.prompt_timeout).to_string())])))
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(super::settings::heading("Popup preferences"))
            .child(
                div().px_5().pb_4().child(
                    TabBar::new("preferences-tabs")
                        .selected_index(self.tab)
                        .on_click(cx.listener(|this, index: &usize, _, cx| {
                            this.tab = *index;
                            cx.notify();
                        }))
                        .child(Tab::new().label(crate::i18n::tr("Popup defaults")))
                        .child(Tab::new().label(crate::i18n::tr("Conditions & timeout")))
                        .child(Tab::new().label(crate::i18n::tr("Placement")))
                        .child(Tab::new().label(crate::i18n::tr("Missed popups"))),
                ),
            )
            .child(
                div()
                    .id("popup-preferences-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_5()
                    .pb_5()
                    .child(form),
            )
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
                "preferences-close",
                self.busy,
                Button::new("preferences-save")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                cx,
            ))
    }
}
