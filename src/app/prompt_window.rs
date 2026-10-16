//! Permission drafts never wait for telemetry, the main window, or storage.
use crate::{
    backend::Handle,
    config::Decision,
    prompts::choice::{self, Choice, Target},
    protocol::Connection,
};
use gpui::{prelude::*, *};
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath, Sizable,
    avatar::Avatar,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    radio::{Radio, RadioGroup},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    tab::{Tab, TabBar},
};
#[cfg(all(test, feature = "headless-tests"))]
use gpui_kit::test::TestSupportExt;
use std::time::Duration;
type Picker = Entity<SelectState<SearchableVec<SharedString>>>;

pub(super) fn open(
    backend: Handle,
    scale: f32,
    position_supported: bool,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let initial = backend
        .prompts
        .snapshot()
        .first()
        .map(|prompt| prompt.initial.clone())
        .unwrap_or_default();
    let bounds = super::placement::bounds(
        if position_supported {
            initial.position
        } else {
            Default::default()
        },
        initial.expanded,
        scale,
        cx,
    );
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(
                px(580.).min(bounds.size.width),
                px(440.).min(bounds.size.height),
            )),
            titlebar: Some(TitlebarOptions {
                title: Some("Rustleblower — connection permission".into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| PromptWindow::new(backend, window, cx)),
    )?;
    if position_supported {
        window.update(cx, |_, window, _| {
            window.on_next_frame(move |window, cx| {
                super::placement::request(window, initial.position, cx)
            });
        })?;
    }
    Ok(window)
}

pub(super) struct PromptWindow {
    backend: Handle,
    focus: FocusHandle,
    id: Option<u64>,
    choice: Choice,
    targets: Vec<Target>,
    target: Option<Picker>,
    duration: Option<Picker>,
    subscriptions: Vec<Subscription>,
    _keyboard: Subscription,
    expanded: bool,
    tab: usize,
    rows: Vec<(String, String)>,
    error: Option<String>,
    checksum: Option<super::prompt_checksums::Probe>,
    icon: super::app_icon::Probe,
}
impl PromptWindow {
    pub(super) fn new(backend: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.on_window_should_close(cx, |window, _| {
            window.minimize_window();
            false
        });
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this.update_in(cx, |_, _, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        // Tab and other Kit key bindings resolve as actions before element key
        // listeners. Observe input before dispatch, scoped to this exact window,
        // and leave propagation untouched so native control behavior is kept.
        let window_id = window.window_handle().window_id();
        let view = cx.weak_entity();
        let keyboard = cx.intercept_keystrokes(move |_, window, cx| {
            if window.window_handle().window_id() == window_id
                && let Some(view) = view.upgrade()
            {
                view.update(cx, |this, cx| {
                    if let Some(id) = this.id
                        && this.backend.prompts.review(id)
                    {
                        cx.notify();
                    }
                });
            }
        });
        Self {
            backend,
            focus,
            id: None,
            choice: Choice::default(),
            targets: Vec::new(),
            target: None,
            duration: None,
            subscriptions: Vec::new(),
            _keyboard: keyboard,
            expanded: false,
            tab: 0,
            rows: Vec::new(),
            error: None,
            checksum: None,
            icon: super::app_icon::Probe::new(""),
        }
    }
    fn picker(
        &mut self,
        labels: Vec<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
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
        self.subscriptions.push(cx.subscribe(
            &state,
            |_, _, _: &SelectEvent<SearchableVec<SharedString>>, cx| cx.notify(),
        ));
        state
    }
    fn sync(
        &mut self,
        id: Option<u64>,
        connection: Option<&Connection>,
        initial: Option<&crate::prompts::Initial>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.id == id {
            return;
        }
        self.id = id;
        self.choice = Choice::default();
        self.error = None;
        self.expanded = false;
        self.tab = 0;
        self.subscriptions.clear();
        self.target = None;
        self.duration = None;
        self.targets.clear();
        self.rows.clear();
        self.checksum = connection.and_then(super::prompt_checksums::Probe::new);
        self.icon = super::app_icon::Probe::new(connection.map_or("", |c| c.process_path.as_str()));
        if let Some(connection) = connection {
            if let Some(initial) = initial {
                self.choice = initial.choice.clone();
                self.expanded = initial.expanded;
                if self.expanded {
                    self.backend.prompts.review(id.unwrap());
                }
                if !initial.unavailable.is_empty() {
                    self.error = Some(format!(
                        "Requested default conditions were not reported: {}. Review the available match before answering.",
                        initial.unavailable.join(", ")
                    ));
                }
            }
            self.targets = choice::targets(connection);
            self.target = Some(
                self.picker(
                    self.targets
                        .iter()
                        .map(|target| target.label.clone().into())
                        .collect(),
                    window,
                    cx,
                ),
            );
            self.duration = Some(
                self.picker(
                    choice::DURATIONS
                        .iter()
                        .map(|(_, label)| (*label).into())
                        .collect(),
                    window,
                    cx,
                ),
            );
            if let Some(index) = self
                .targets
                .iter()
                .position(|target| target.key == self.choice.target)
            {
                self.target.as_ref().unwrap().update(cx, |picker, cx| {
                    picker.set_selected_index(Some(IndexPath::new(index)), window, cx)
                });
            }
            if let Some(index) = choice::DURATIONS
                .iter()
                .position(|(key, _)| *key == self.choice.duration)
            {
                self.duration.as_ref().unwrap().update(cx, |picker, cx| {
                    picker.set_selected_index(Some(IndexPath::new(index)), window, cx)
                });
            }
            self.rows = live_rows(connection);
        }
        window.resize(
            super::placement::bounds(
                crate::prompts::placement::Position::Center,
                self.expanded,
                window.scale_factor(),
                cx,
            )
            .size,
        );
        self.focus.focus(window, cx);
    }
    fn selected(&self, cx: &App) -> Choice {
        let mut selected = self.choice.clone();
        if let Some(label) = self
            .target
            .as_ref()
            .and_then(|state| state.read(cx).selected_value())
            && let Some(target) = self
                .targets
                .iter()
                .find(|target| target.label == label.as_ref())
        {
            selected.target = target.key.clone();
        }
        if let Some(label) = self
            .duration
            .as_ref()
            .and_then(|state| state.read(cx).selected_value())
            && let Some((duration, _)) = choice::DURATIONS
                .iter()
                .find(|(_, name)| *name == label.as_ref())
        {
            selected.duration = (*duration).into();
        }
        selected
    }
    fn apply(&mut self, id: u64, decision: Decision, cx: &mut Context<Self>) {
        self.apply_choice(id, decision, self.selected(cx), cx);
    }
    fn select_scope(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.targets.iter().position(|target| target.key == key) else {
            return;
        };
        self.choice = self.selected(cx);
        self.choice.target = key.into();
        // The simple choices describe the complete scope. Keeping hidden
        // address/port constraints here would contradict their visible labels.
        self.choice.constraints.clear();
        self.error = None;
        self.target.as_ref().unwrap().update(cx, |picker, cx| {
            picker.set_selected_index(Some(IndexPath::new(index)), window, cx)
        });
        cx.notify();
    }
    fn allow_for(&mut self, id: u64, duration: &str, cx: &mut Context<Self>) {
        let mut choice = self.selected(cx);
        choice.duration = duration.into();
        self.apply_choice(id, Decision::Allow, choice, cx);
    }
    fn apply_choice(
        &mut self,
        id: u64,
        decision: Decision,
        choice: Choice,
        cx: &mut Context<Self>,
    ) {
        if self.id != Some(id) {
            return;
        }
        match self.backend.prompts.choose(id, decision, &choice) {
            Ok(true) => self.error = None,
            Ok(false) => self.error = Some("This request expired or was already answered".into()),
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
}
fn live_rows(c: &Connection) -> Vec<(String, String)> {
    let mut rows = vec![
        (
            "Executable".into(),
            crate::history::bounded(&c.process_path, 4096),
        ),
        (
            "Working directory".into(),
            crate::history::bounded(&c.process_cwd, 4096),
        ),
        (
            "Source".into(),
            crate::history::details::endpoint(&c.src_ip, c.src_port),
        ),
        (
            "Destination".into(),
            crate::history::details::endpoint(&c.dst_ip, c.dst_port),
        ),
        ("Host".into(), crate::history::bounded(&c.dst_host, 1024)),
        ("Protocol".into(), crate::history::bounded(&c.protocol, 32)),
        ("User ID".into(), c.user_id.to_string()),
        ("Process ID".into(), c.process_id.to_string()),
    ];
    for (index, arg) in c.process_args.iter().enumerate() {
        rows.push((
            format!("Argument {index}"),
            crate::history::bounded(arg, 1024),
        ));
    }
    let mut checksums = c.process_checksums.iter().collect::<Vec<_>>();
    checksums.sort_unstable_by_key(|(key, _)| *key);
    for (key, value) in checksums.into_iter() {
        rows.push((
            crate::history::bounded(key, 128),
            crate::history::bounded(value, 128),
        ));
    }
    for item in c.process_tree.iter() {
        rows.push((
            format!("Ancestor PID {}", item.value),
            crate::history::bounded(&item.key, 4096),
        ));
    }
    let mut env = c.process_env.iter().collect::<Vec<_>>();
    env.sort_unstable_by_key(|(key, _)| *key);
    for (key, value) in env.into_iter() {
        rows.push((
            format!("Environment: {}", crate::history::bounded(key, 128)),
            crate::history::bounded(value, 2048),
        ));
    }
    rows.truncate(12_000);
    rows
}
impl Render for PromptWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prompts = self.backend.prompts.snapshot();
        let first = prompts.first();
        self.sync(
            first.map(|prompt| prompt.id),
            first.map(|prompt| prompt.connection.as_ref()),
            first.map(|prompt| &prompt.initial),
            window,
            cx,
        );
        let muted = cx.theme().muted_foreground;
        let mut body = div()
            .size_full()
            .track_focus(&self.focus)
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground);
        let Some(prompt) = first else {
            return body.child("No pending connections");
        };
        let con = &prompt.connection;
        self.icon.poll(cx);
        let id = prompt.id;
        if let Some(probe) = self.checksum.as_mut() {
            probe.poll(&self.backend, prompt.session, cx);
        }
        body = body.capture_any_mouse_down(cx.listener(move |this, _, _, cx| {
            if this.id == Some(id) && this.backend.prompts.review(id) {
                cx.notify();
            }
        }));
        let remaining = prompt
            .deadline
            .saturating_duration_since(std::time::Instant::now());
        let seconds = remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0);
        let selected = self.selected(cx);
        let app_name = std::path::Path::new(&con.process_path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .map(|name| crate::history::bounded(name, 96))
            .unwrap_or_else(|| "Unknown app".into());
        let destination_key = ["app-host", "app-address"]
            .into_iter()
            .find(|key| self.targets.iter().any(|target| target.key == *key));
        let destination = if destination_key == Some("app-host") {
            format!("hostname {}", crate::history::bounded(&con.dst_host, 96))
        } else {
            format!("IP {}", crate::history::bounded(&con.dst_ip, 64))
        };
        let simple_scope = if selected.constraints.is_empty() && selected.target == "process.path" {
            Some(0)
        } else if selected.constraints.is_empty()
            && destination_key == Some(selected.target.as_str())
        {
            Some(1)
        } else {
            None
        };
        let target_label = self
            .targets
            .iter()
            .find(|target| target.key == selected.target)
            .map(|target| target.label.as_str())
            .unwrap_or("Executable + destination");
        let preview = match simple_scope {
            Some(0) => format!("Applies to {app_name} at any destination and port."),
            Some(1) => format!("Applies to {app_name} only at {destination}, on any port."),
            _ => format!(
                "Custom match: {target_label} · {} extra restrictions. Review under Advanced.",
                selected.constraints.len()
            ),
        };
        let suffix = if selected.duration == "always" {
            "forever"
        } else {
            &selected.duration
        };
        body = body
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .flex_shrink_0()
                    .child(
                        Avatar::new()
                            .name(app_name.clone())
                            .with_size(px(40.))
                            .rounded_md()
                            .when_some(self.icon.image.clone(), |avatar, image| avatar.src(image)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xl()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Allow {app_name} to access the network?")),
                    ),
            )
            .child(div().text_sm().text_color(muted).child(format!(
                    "Node: {}",
                    self.backend
                        .session(prompt.session)
                        .map(|session| session.name)
                        .unwrap_or_else(|| "Disconnected".into())
                )))
            .child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .truncate()
                    .flex_shrink_0()
                    .child(if prompt.reviewing {
                        format!(
                            "{} pending · request limit {}s · {} once · Reviewing (paused)",
                            prompts.len(),
                            seconds,
                            prompt
                                .initial
                                .fallback
                                .unwrap_or(self.backend.timeout_action)
                                .action()
                        )
                    } else {
                        format!(
                            "{} pending · {} once in {}s if unanswered",
                            prompts.len(),
                            prompt
                                .initial
                                .fallback
                                .unwrap_or(self.backend.timeout_action)
                                .action(),
                            seconds
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(detail(
                        "App",
                        if con.process_path.is_empty() {
                            "Unknown executable".into()
                        } else {
                            crate::history::bounded(&con.process_path, 4096)
                        },
                        cx,
                    ))
                    .child(detail(
                        "To",
                        if con.dst_host.is_empty() {
                            crate::history::bounded(&con.dst_ip, 64)
                        } else {
                            crate::history::bounded(&con.dst_host, 1024)
                        },
                        cx,
                    )),
            )
            .when_some(self.error.clone(), |v, error| {
                v.child(super::banner(error, cx))
            });
        if let Some(probe) = self.checksum.as_ref()
            && (probe.found || probe.error.is_some())
        {
            let context = probe.context.clone();
            let session = prompt.session;
            body = body.child(div().flex().flex_col().gap_2()
                    .child(super::banner(if probe.found { "Reported executable checksums differ from saved rules. Review before trusting this binary.".into() } else { format!("Saved checksum check unavailable: {}. Permission actions remain available.", probe.error.as_deref().unwrap_or("unknown error")) }, cx))
                    .child(Button::new("prompt-checksums").label("Review saved checksums…").on_click(cx.listener(move |this, _, _, cx| {
                        if this.id != Some(id) { return; }
                        if let Err(error) = super::prompt_checksums::open(this.backend.clone(), id, session, context.clone(), cx) { this.error = Some(error.to_string()); }
                        cx.notify();
                    }))));
        }
        let has_app = self
            .targets
            .iter()
            .any(|target| target.key == "process.path");
        body = body.child(
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .gap_2()
                .child("Apply this decision to")
                .child(
                    RadioGroup::vertical("prompt-scope")
                        .w_full()
                        .flex_none()
                        .selected_index(Some(simple_scope.unwrap_or(2)))
                        .child(
                            Radio::new("app")
                                .label(format!("{app_name} — any destination"))
                                .disabled(!has_app),
                        )
                        .child(
                            Radio::new("destination")
                                .label(format!("{app_name} — only {destination} (any port)"))
                                .disabled(destination_key.is_none()),
                        )
                        .when(simple_scope.is_none(), |group| {
                            group.child(
                                Radio::new("custom").label("Custom rule — review in Advanced"),
                            )
                        })
                        .on_change(cx.listener(move |this, index: &usize, window, cx| {
                            if this.id != Some(id) {
                                return;
                            }
                            match *index {
                                0 => this.select_scope("process.path", window, cx),
                                1 => {
                                    if let Some(key) = destination_key {
                                        this.select_scope(key, window, cx);
                                    }
                                }
                                _ => {
                                    this.expanded = true;
                                    window.resize(
                                        super::placement::bounds(
                                            Default::default(),
                                            true,
                                            window.scale_factor(),
                                            cx,
                                        )
                                        .size,
                                    );
                                    cx.notify();
                                }
                            }
                        })),
                ),
        );
        if self.expanded {
            body = body.child(
                TabBar::new("prompt-tabs")
                    .selected_index(self.tab)
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.tab = *index;
                        cx.notify();
                    }))
                    .child(Tab::new().label("Advanced rules"))
                    .child(Tab::new().label("Connection details")),
            );
            if self.tab == 0 {
                #[cfg_attr(all(test, feature = "headless-tests"), allow(unused_mut))]
                let mut options=div().id("prompt-options-body").flex_1().min_h_0().overflow_y_scroll().flex().flex_col().gap_3()
                    .child(div().text_sm().child("Apply this decision for"))
                    .child(div().h_8().flex_shrink_0().child(Select::new(self.duration.as_ref().unwrap()).id("prompt-duration").accessibility_label("Rule duration").w_full()))
                    .child(div().text_sm().child("Rule scope"))
                    .child(div().h_8().flex_shrink_0().child(Select::new(self.target.as_ref().unwrap()).id("prompt-target").accessibility_label("Rule scope").w_full()))
                    .child(div().text_sm().child("Additional restrictions"))
                    .child(div().text_sm().text_color(muted).child("All selected conditions must match. Fields already required by the scope are omitted."));
                // Observe geometry for bounded wheel-input verification only.
                #[cfg(all(test, feature = "headless-tests"))]
                let mut options = options.test_support();
                let scope = self
                    .targets
                    .iter()
                    .find(|target| target.key == selected.target);
                for (index, target) in self
                    .targets
                    .iter()
                    .enumerate()
                    .filter(|(_, target)| target.key != "connection")
                {
                    let key = target.key.clone();
                    let included = scope.is_some_and(|scope| {
                        target
                            .operators
                            .iter()
                            .all(|op| scope.operators.contains(op))
                    });
                    if included {
                        continue;
                    }
                    let checked = selected.constraints.contains(&key);
                    options = options.child(
                        Checkbox::new(("prompt-constraint", index))
                            .label(if target.key == "process.path" {
                                format!("Require executable: {app_name}")
                            } else {
                                target.label.clone()
                            })
                            .checked(checked)
                            .disabled(!checked && selected.constraints.len() >= 16)
                            .on_change(cx.listener(move |this, value: &bool, _, cx| {
                                if *value {
                                    if !this.choice.constraints.contains(&key) {
                                        this.choice.constraints.push(key.clone());
                                    }
                                } else {
                                    this.choice.constraints.retain(|entry| entry != &key);
                                }
                                cx.notify();
                            })),
                    );
                }
                options = options.child(
                    div()
                        .flex()
                        .gap_4()
                        .child(
                            Checkbox::new("prompt-priority")
                                .label("Priority")
                                .checked(selected.priority)
                                .on_change(cx.listener(|this, value: &bool, _, cx| {
                                    this.choice.priority = *value;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Checkbox::new("prompt-logging")
                                .label("Log matching connections")
                                .checked(selected.logging)
                                .on_change(cx.listener(|this, value: &bool, _, cx| {
                                    this.choice.logging = *value;
                                    cx.notify();
                                })),
                        ),
                );
                body = body.child(options);
            } else {
                let rows = uniform_list(
                    "prompt-live-details",
                    self.rows.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| {
                                let (key, value) = &this.rows[index];
                                div()
                                    .w_full()
                                    .h(px(52.))
                                    .py_2()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .flex()
                                    .gap_3()
                                    .items_center()
                                    .child(
                                        div()
                                            .w(px(200.))
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .truncate()
                                            .child(key.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .truncate()
                                            .child(value.clone()),
                                    )
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .w_full()
                .flex_1()
                .min_h_0();
                body=body.child(div().text_xs().text_color(muted).child("Live request snapshot · long values may be shortened; environments are not saved in history."))
                    .child(rows).child(Button::new("prompt-copy-details").label("Copy displayed details").on_click(cx.listener(|this,_,_,cx|{cx.write_to_clipboard(ClipboardItem::new_string(this.rows.iter().map(|(key,value)|format!("{key}: {value}")).collect::<Vec<_>>().join("\n")));})));
            }
        } else {
            body = body.child(div().flex_1());
        }
        body.child(div().text_sm().text_color(muted).child(preview))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap_3()
                    .child(
                        Button::new("deny-once")
                            .label(format!("Block {suffix}"))
                            .danger()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.apply(id, Decision::Deny, cx)
                            })),
                    )
                    .child(
                        Button::new("allow-12h")
                            .label("Allow for 12 hours")
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.allow_for(id, "12h", cx)),
                            ),
                    )
                    .child(
                        Button::new("allow-forever")
                            .label("Allow forever")
                            .primary()
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.allow_for(id, "always", cx)),
                            ),
                    )
                    .when(self.expanded, |row| {
                        row.child(
                            Button::new("allow-selected")
                                .label(format!("Allow {suffix}"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.apply(id, Decision::Allow, cx)
                                })),
                        )
                        .child(
                            Button::new("reject-once")
                                .label(format!("Reject {suffix}"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.apply(id, Decision::Reject, cx)
                                })),
                        )
                    }),
            )
            .child(
                Button::new("prompt-more")
                    .label(if self.expanded {
                        "Hide details"
                    } else {
                        "Details and advanced rules…"
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.expanded = !this.expanded;
                        window.resize(
                            super::placement::bounds(
                                crate::prompts::placement::Position::Center,
                                this.expanded,
                                window.scale_factor(),
                                cx,
                            )
                            .size,
                        );
                        cx.notify();
                    })),
            )
    }
}
fn detail(label: &str, value: String, cx: &App) -> Div {
    div()
        .flex()
        .gap_3()
        .child(
            div()
                .w(px(90.))
                .flex_shrink_0()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_string()),
        )
        .child(div().flex_1().min_w_0().text_sm().truncate().child(value))
}
