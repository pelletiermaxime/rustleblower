//! Reviewable raw-file profiles; every filesystem and HTTP action uses the worker.
use crate::raw_downloads::{Command, Handle, Reply, Settings, Snapshot, Unit};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectState},
};
use std::{path::PathBuf, time::Duration};
type Picker = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<Unit>>>>;
const UNITS: [(Unit, &str); 4] = [
    (Unit::Seconds, "Seconds"),
    (Unit::Minutes, "Minutes"),
    (Unit::Hours, "Hours"),
    (Unit::Days, "Days"),
];
pub(super) struct View {
    handle: Handle,
    saved: Snapshot,
    draft: Settings,
    revision: Option<String>,
    profile: Entity<InputState>,
    intervals: Vec<Entity<InputState>>,
    units: Vec<Picker>,
    pending: bool,
    message: Option<String>,
    review: Option<Review>,
}
enum Review {
    Save(Settings),
    Run {
        group: String,
        revision: Option<String>,
    },
}
impl View {
    pub(super) fn new(handle: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = handle.snapshot();
        let draft = saved.settings.clone();
        let revision = saved.revision.clone();
        let profile = cx.new(|cx| {
            InputState::new(window, cx)
                .validate(|text, _| text.len() <= 4096 && !text.chars().any(char::is_control))
        });
        let mut this = Self {
            handle,
            saved,
            draft,
            revision,
            profile,
            intervals: Vec::new(),
            units: Vec::new(),
            pending: false,
            message: None,
            review: None,
        };
        this.editors(window, cx);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        let ready = this.saved.ready;
                        this.saved = this.handle.snapshot();
                        if !ready && this.saved.ready {
                            this.reset(window, cx);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        this
    }
    fn editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.intervals = self
            .draft
            .groups
            .iter()
            .map(|group| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(group.interval.clone())
                        .validate(|text, _| {
                            text.len() <= 16
                                && text.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                        })
                })
            })
            .collect();
        self.units = self
            .draft
            .groups
            .iter()
            .map(|group| {
                cx.new(|cx| {
                    SelectState::new(
                        SearchableVec::new(
                            UNITS
                                .iter()
                                .map(|(value, title)| {
                                    crate::i18n::LocalizedOption::new(*value, *title)
                                })
                                .collect::<Vec<_>>(),
                        ),
                        UNITS
                            .iter()
                            .position(|(value, _)| *value == group.units)
                            .map(IndexPath::new),
                        window,
                        cx,
                    )
                })
            })
            .collect();
    }
    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.draft = self.saved.settings.clone();
        self.revision = self.saved.revision.clone();
        self.review = None;
        self.editors(window, cx);
    }
    fn send(
        &mut self,
        command: Command,
        replace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending {
            return;
        }
        self.pending = true;
        self.review = None;
        let reply = self.handle.request(command);
        cx.spawn_in(window,async move|this,cx| {let result=reply.await; let _=this.update_in(cx,|this,window,cx| {
            this.pending=false;this.message=None;
            match result {
                Ok(Ok(Reply::State(state)))=>{this.saved=state;if replace {this.reset(window,cx);}},
                Ok(Ok(Reply::Profile(settings)))=>{this.draft=settings;this.review=None;this.editors(window,cx);this.message=Some(crate::i18n::tr("Profile staged with automatic downloads off. Review every destination before saving or running."));},
                Ok(Ok(Reply::Written(path)))=>this.message=Some(crate::i18n::tr_args("Profile exported to %{path}", &[("path", &path.display().to_string())])),
                Ok(Ok(Reply::Export(_)))=>{},
                Ok(Err(error))=>this.message=Some(error),Err(_)=>this.message=Some(crate::i18n::tr("Raw downloads are unavailable")),
            } cx.notify();
        });}).detach();
        cx.notify();
    }
    fn review_save(&mut self, cx: &mut Context<Self>) {
        let mut settings = self.draft.clone();
        for (index, group) in settings.groups.iter_mut().enumerate() {
            group.interval = self.intervals[index].read(cx).value().to_string();
            if let Some(unit) = self.units[index].read(cx).selected_value() {
                group.units = *unit;
            }
        }
        if let Err(error) = settings.validate() {
            self.message = Some(error.to_string());
        } else {
            self.review = Some(Review::Save(settings));
            self.message = None;
        }
        cx.notify();
    }
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.review.take() {
            Some(Review::Save(settings)) => self.send(
                Command::Save {
                    expected_revision: self.revision.clone(),
                    settings,
                },
                true,
                window,
                cx,
            ),
            Some(Review::Run { group, revision }) => self.send(
                Command::RunNow {
                    group,
                    expected_revision: revision,
                },
                false,
                window,
                cx,
            ),
            None => {}
        }
    }
}
impl Render for View {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.pending || !self.saved.ready;
        let content = div()
            .id("raw-downloads-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_5()
            .flex()
            .flex_col()
            .gap_3();
        #[cfg(feature = "headless-tests")]
        let content = content.test_support();
        let mut content = content;
        content=content.child(crate::i18n::tr("Raw URL-to-file downloads")).child(crate::i18n::tr("Import an OpenSnitch downloader profile to stage its URLs and exact file destinations. Saving enables only the reviewed schedule; manual runs use saved settings and require a separate review."))
          .child(Input::new(&self.profile).id("raw-profile-path").disabled(disabled))
          .child(div().flex().gap_2().flex_wrap()
            .child(Button::new("raw-import").label(crate::i18n::tr("Stage profile from file")).disabled(disabled || self.saved.busy).on_click(cx.listener(|this,_,window,cx|{let path=PathBuf::from(this.profile.read(cx).value().as_str());this.send(Command::ReadProfile{path},false,window,cx);})))
            .child(Button::new("raw-export").label(crate::i18n::tr("Export saved to new file")).disabled(disabled || self.saved.busy).on_click(cx.listener(|this,_,window,cx|{let path=PathBuf::from(this.profile.read(cx).value().as_str());this.send(Command::WriteProfile{path,expected_revision:this.saved.revision.clone()},false,window,cx);}))))
          .child(Checkbox::new("raw-enabled").label(crate::i18n::tr("Download automatically after each saved interval")).checked(self.draft.enabled).disabled(disabled || self.review.is_some()).on_change(cx.listener(|this,value:&bool,_,cx|{this.draft.enabled = *value;cx.notify();})));
        if self.draft.groups.is_empty() {
            content=content.child(crate::i18n::tr("No download groups configured. An imported profile starts with automatic downloads off."));
        }
        for (index, group) in self.draft.groups.iter().enumerate() {
            let name = group.name.clone();
            let mut card = div()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(group.name.clone())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(crate::i18n::tr("Repeat every"))
                        .child(
                            div().w_24().child(
                                Input::new(&self.intervals[index])
                                    .id(SharedString::from(format!("raw-interval-{index}")))
                                    .disabled(disabled || self.review.is_some()),
                            ),
                        )
                        .child(
                            div().w_32().h_8().flex_shrink_0().child(
                                Select::new(&self.units[index])
                                    .id(SharedString::from(format!("raw-unit-{index}")))
                                    .disabled(disabled || self.review.is_some()),
                            ),
                        ),
                );
            for resource in &group.urls {
                card = card.child(div().text_sm().child(format!(
                    "{} — {} → {}{}",
                    resource.name,
                    resource.remote,
                    resource.localfile.display(),
                    if resource.enabled {
                        String::new()
                    } else {
                        crate::i18n::tr(" (disabled)")
                    }
                )));
            }
            card = card.child(
                Button::new(SharedString::from(format!("raw-run-{index}")))
                    .label(crate::i18n::tr("Review saved run"))
                    .disabled(disabled || self.saved.busy || self.saved.revision != self.revision)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.review = Some(Review::Run {
                            group: name.clone(),
                            revision: this.saved.revision.clone(),
                        });
                        cx.notify();
                    })),
            );
            content = content.child(card);
        }
        if let Some(review) = &self.review {
            let groups = match review {
                Review::Save(settings) => settings.groups.clone(),
                Review::Run { group, .. } => self
                    .saved
                    .settings
                    .groups
                    .iter()
                    .filter(|g| &g.name == group)
                    .cloned()
                    .collect(),
            };
            let mut panel = div()
                .border_1()
                .border_color(cx.theme().warning)
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(crate::i18n::tr(match review {
                    Review::Save(settings) => {
                        if settings.enabled {
                            "Confirm saved automatic downloads and these exact destinations"
                        } else {
                            "Confirm saved profile with automatic downloads off"
                        }
                    }
                    Review::Run { .. } => {
                        "Confirm this manual run writes these saved destinations now"
                    }
                }));
            for group in groups {
                panel = panel.child(crate::i18n::tr_args(
                    "%{name} — every %{interval} %{unit}",
                    &[
                        ("name", &group.name),
                        ("interval", &group.interval),
                        ("unit", &crate::i18n::tr(group.units.label())),
                    ],
                ));
                for resource in group.urls.into_iter().filter(|r| r.enabled) {
                    panel = panel.child(format!(
                        "{} → {}",
                        resource.remote,
                        resource.localfile.display()
                    ));
                }
            }
            content = content.child(
                panel.child(
                    div()
                        .flex()
                        .gap_2()
                        .when(matches!(review, Review::Run { .. }), |view| {
                            view.child(
                                Button::new("raw-confirm")
                                    .primary()
                                    .label(crate::i18n::tr("Run now"))
                                    .disabled(disabled)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.confirm(window, cx)),
                                    ),
                            )
                        })
                        .child(
                            Button::new("raw-dismiss-review")
                                .label(crate::i18n::tr("Keep editing"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.review = None;
                                    cx.notify();
                                })),
                        ),
                ),
            );
        }
        for report in &self.saved.reports {
            content = content.child(format!(
                "{}: {}",
                report.name,
                crate::i18n::tr(report.status.label())
            ));
            for resource in &report.resources {
                content = content.child(div().text_sm().child(crate::i18n::tr_args(
                    "%{name}: %{status} (%{bytes} bytes)",
                    &[
                        ("name", &resource.name),
                        ("status", &crate::i18n::tr(resource.status.label())),
                        ("bytes", &resource.bytes.to_string()),
                    ],
                )));
            }
        }
        content = content.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("raw-reload")
                        .label(crate::i18n::tr("Reload saved"))
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.send(Command::Reload, true, window, cx)
                        })),
                )
                .child(
                    Button::new("raw-cancel")
                        .label(crate::i18n::tr("Cancel download"))
                        .disabled(disabled || !self.saved.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.send(Command::Cancel, false, window, cx)
                        })),
                ),
        );
        content = content
            .when_some(self.saved.warning.clone(), |container, text| {
                container.child(
                    div()
                        .text_color(cx.theme().danger)
                        .child(crate::i18n::presentation(&text)),
                )
            })
            .when_some(self.message.clone(), |container, text| {
                container.child(crate::i18n::presentation(&text))
            });
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .flex()
            .flex_col()
            .child(super::settings::heading("Raw downloads"))
            .child(content)
            .child(super::settings::footer(
                "raw-close",
                self.pending,
                Button::new(if matches!(self.review, Some(Review::Save(_))) {
                    "raw-confirm"
                } else {
                    "raw-save"
                })
                .disabled(
                    disabled || self.saved.busy || matches!(self.review, Some(Review::Run { .. })),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    if matches!(this.review, Some(Review::Save(_))) {
                        this.confirm(window, cx);
                    } else {
                        this.review_save(cx);
                    }
                })),
                cx,
            ))
    }
}
