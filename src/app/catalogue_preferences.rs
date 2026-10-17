//! Staged preferences for local rule catalogue visibility and cache freshness.
use crate::rules::catalogue_policy::{Command, Handle, PRESETS, Preset, Settings, Snapshot};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath,
    button::Button,
    checkbox::Checkbox,
    select::{SearchableVec, Select, SelectState},
};
use std::{sync::Arc, time::Duration};
type Picker = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<Preset>>>>;
const LABELS: [&str; 8] = [
    "Once",
    "30 seconds",
    "5 minutes",
    "15 minutes",
    "30 minutes",
    "1 hour",
    "12 hours",
    "Until restart",
];
pub(super) struct View {
    handle: Handle,
    saved: Arc<Snapshot>,
    draft: Settings,
    revision: Option<String>,
    threshold: Picker,
    pending: bool,
    review: Option<Settings>,
    message: Option<String>,
}
impl View {
    pub(super) fn new(handle: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = handle.snapshot();
        let draft = saved.settings;
        let revision = saved.revision.clone();
        let threshold = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    PRESETS
                        .iter()
                        .zip(LABELS)
                        .map(|(value, label)| crate::i18n::LocalizedOption::new(*value, label))
                        .collect::<Vec<_>>(),
                ),
                Some(IndexPath::new(draft.threshold.index())),
                window,
                cx,
            )
        });
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
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
        Self {
            handle,
            saved,
            draft,
            revision,
            threshold,
            pending: false,
            review: None,
            message: None,
        }
    }
    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.draft = self.saved.settings;
        self.revision = self.saved.revision.clone();
        self.review = None;
        let index = self.draft.threshold.index();
        self.threshold.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::new(index)), window, cx)
        });
    }
    fn send(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.pending = true;
        self.review = None;
        let reply = self.handle.request(command);
        cx.spawn_in(window, async move |this, cx| {
            let result = reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(Ok(saved)) => {
                        this.saved = Arc::new(saved);
                        this.message = None;
                        this.reset(window, cx);
                    }
                    Ok(Err(error)) => this.message = Some(error),
                    Err(_) => {
                        this.message =
                            Some(crate::i18n::tr("Catalogue preferences are unavailable"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
impl Render for View {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.pending || !self.saved.ready;
        let editing = busy || self.review.is_some();
        let content = div()
            .id("catalogue-preferences-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_5()
            .flex()
            .flex_col()
            .gap_4();
        #[cfg(feature = "headless-tests")]
        let content = content.test_support();
        let mut content=content
      .child(crate::i18n::tr("Choose which temporary rules appear in the local catalogue. Cached expiry is shown separately from the daemon's enabled report."))
      .child(Checkbox::new("catalogue-ignore").label(crate::i18n::tr("Ignore temporary presets shorter than the selected threshold")).checked(self.draft.ignore_temporary).disabled(editing).on_change(cx.listener(|this,value:&bool,_,cx|{this.draft.ignore_temporary = *value;cx.notify();})))
      .child(div().child(crate::i18n::tr("Keep this preset and longer presets")))
      .child(div().h_8().flex_shrink_0().child(Select::new(&self.threshold).id("catalogue-threshold").disabled(editing)))
      .child(crate::i18n::tr("Always and custom duration values remain visible. Selecting Once ignores no presets. Saving preferences sends no rule changes to a daemon."));
        if let Some(review) = self.review {
            let ignored = PRESETS
                .iter()
                .filter(|p| !review.includes(p.duration()))
                .map(|p| p.duration())
                .collect::<Vec<_>>()
                .join(", ");
            let kept = PRESETS
                .iter()
                .filter(|p| review.includes(p.duration()))
                .map(|p| p.duration())
                .collect::<Vec<_>>()
                .join(", ");
            content = content.child(
                div()
                    .border_1()
                    .border_color(cx.theme().border)
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(crate::i18n::tr("Review local catalogue preferences"))
                    .child(crate::i18n::tr_args(
                        "Ignored preset durations: %{arg0}",
                        &[("arg0", if ignored.is_empty() { "—" } else { &ignored })],
                    ))
                    .child(crate::i18n::tr_args(
                        "Kept preset durations: %{arg0}",
                        &[("arg0", &kept)],
                    ))
                    .child(
                        Button::new("catalogue-keep-editing")
                            .label(crate::i18n::tr("Keep editing"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.review = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        content = content.child(
            div().flex().child(
                Button::new("catalogue-reload")
                    .label(crate::i18n::tr("Reload"))
                    .disabled(busy)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.send(Command::Reload, window, cx)),
                    ),
            ),
        );
        content = content
            .when_some(self.saved.warning.clone(), |container, text| {
                container.child(div().text_color(cx.theme().danger).child(text))
            })
            .when_some(self.message.clone(), |container, text| {
                container.child(div().text_color(cx.theme().danger).child(text))
            });
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .flex()
            .flex_col()
            .child(super::settings::heading("Rule catalogue"))
            .child(content)
            .child(super::settings::footer(
                "catalogue-close",
                busy,
                Button::new(if self.review.is_some() {
                    "catalogue-confirm"
                } else {
                    "catalogue-review"
                })
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(settings) = this.review {
                        this.send(
                            Command::Save {
                                expected_revision: this.revision.clone(),
                                settings,
                            },
                            window,
                            cx,
                        );
                    } else {
                        let mut settings = this.draft;
                        if let Some(value) = this.threshold.read(cx).selected_value() {
                            settings.threshold = *value;
                        }
                        this.review = Some(settings);
                        this.message = None;
                        cx.notify();
                    }
                })),
                cx,
            ))
    }
}
