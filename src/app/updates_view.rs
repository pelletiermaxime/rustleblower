//! Staged native release-check preferences; network checks use saved settings.
use crate::updates::{Command, Handle, Settings, State, Target, Unit};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, IndexPath,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use std::time::Duration;
type Picker<T> = Entity<SelectState<SearchableVec<crate::i18n::LocalizedOption<T>>>>;
const UNITS: [(&str, Unit); 4] = [
    ("Seconds", Unit::Seconds),
    ("Minutes", Unit::Minutes),
    ("Hours", Unit::Hours),
    ("Days", Unit::Days),
];
const TARGETS: [(&str, Target); 2] = [
    ("Rustleblower", Target::Application),
    ("Selected OpenSnitch daemon", Target::SelectedDaemon),
];

pub(super) struct View {
    handle: Handle,
    saved: State,
    draft: Settings,
    draft_revision: Option<String>,
    interval: Entity<InputState>,
    unit: Picker<Unit>,
    target: Picker<Target>,
    pending: bool,
    message: Option<String>,
}
fn picker<T: Clone + PartialEq + 'static>(
    options: &[(&str, T)],
    index: usize,
    window: &mut Window,
    cx: &mut Context<View>,
) -> Picker<T> {
    let state = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(
                options
                    .iter()
                    .map(|(label, value)| crate::i18n::LocalizedOption::new(value.clone(), *label))
                    .collect::<Vec<_>>(),
            ),
            Some(IndexPath::new(index)),
            window,
            cx,
        )
    });
    cx.subscribe(
        &state,
        |_, _, _: &SelectEvent<SearchableVec<crate::i18n::LocalizedOption<T>>>, cx| cx.notify(),
    )
    .detach();
    state
}
impl View {
    pub(super) fn new(handle: Handle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = handle.state();
        let draft = saved.settings.clone();
        let draft_revision = saved.revision.clone();
        let interval = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(draft.interval.to_string())
                .validate(|text, _| text.len() <= 8 && text.bytes().all(|b| b.is_ascii_digit()))
        });
        let unit = picker(
            &UNITS,
            UNITS
                .iter()
                .position(|(_, value)| *value == draft.unit)
                .unwrap_or(2),
            window,
            cx,
        );
        let target = picker(
            &TARGETS,
            TARGETS
                .iter()
                .position(|(_, value)| *value == draft.target)
                .unwrap_or(0),
            window,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        let was_ready = this.saved.ready;
                        this.saved = this.handle.state();
                        if !was_ready && this.saved.ready {
                            this.pending = false;
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
        let pending = !saved.ready;
        Self {
            handle,
            saved,
            draft,
            draft_revision,
            interval,
            unit,
            target,
            pending,
            message: None,
        }
    }
    fn send(
        &mut self,
        command: Command,
        replace_draft: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending {
            return;
        }
        self.pending = true;
        let reply = self.handle.request(command);
        cx.spawn_in(window, async move |this, cx| {
            let result = reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(Ok(state)) => {
                        this.saved = state;
                        this.message = None;
                        if replace_draft {
                            this.reset(window, cx);
                        }
                    }
                    Ok(Err(error)) => this.message = Some(error),
                    Err(_) => {
                        this.message = Some(crate::i18n::tr("Release checks are unavailable"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.draft = self.saved.settings.clone();
        self.draft_revision = self.saved.revision.clone();
        self.interval.update(cx, |input, cx| {
            input.set_value(self.draft.interval.to_string(), window, cx)
        });
        self.unit.update(cx, |select, cx| {
            select.set_selected_index(
                UNITS
                    .iter()
                    .position(|(_, value)| *value == self.draft.unit)
                    .map(IndexPath::new),
                window,
                cx,
            )
        });
        self.target.update(cx, |select, cx| {
            select.set_selected_index(
                TARGETS
                    .iter()
                    .position(|(_, value)| *value == self.draft.target)
                    .map(IndexPath::new),
                window,
                cx,
            )
        });
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut draft = self.draft.clone();
        let Some(interval) = self.interval.read(cx).value().parse::<u32>().ok() else {
            self.message = Some(crate::i18n::tr("Enter a whole interval count"));
            cx.notify();
            return;
        };
        draft.interval = interval;
        if let Some(value) = self.unit.read(cx).selected_value() {
            draft.unit = *value;
        }
        if let Some(value) = self.target.read(cx).selected_value() {
            draft.target = *value;
        }
        if let Err(error) = draft.duration() {
            self.message = Some(error.to_string());
            cx.notify();
            return;
        }
        self.send(
            Command::Save {
                expected_revision: self.draft_revision.clone(),
                settings: draft,
            },
            true,
            window,
            cx,
        );
    }
}
impl Render for View {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.pending;
        let scroll = div()
            .id("updates-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_5()
            .pb_5()
            .flex()
            .flex_col()
            .gap_4();
        #[cfg(feature = "headless-tests")]
        let scroll = scroll.test_support();
        div().size_full().flex().flex_col().bg(cx.theme().background).text_color(cx.theme().foreground)
            .child(super::settings::heading("Updates"))
            .child(scroll
                .child(div().child(crate::i18n::tr("Save preferences to apply automatic checks. Check now contacts the official release API using saved target and prerelease choices, even when automatic checks are off.")))
                .child(Checkbox::new("updates-enabled").label(crate::i18n::tr("Check automatically")).checked(self.draft.enabled).disabled(busy).on_change(cx.listener(|this, value: &bool, _, cx| { this.draft.enabled = *value; cx.notify(); })))
                .child(Checkbox::new("updates-on-start").label(crate::i18n::tr("Check when the application starts")).checked(self.draft.check_on_start).disabled(busy).on_change(cx.listener(|this, value: &bool, _, cx| { this.draft.check_on_start = *value; cx.notify(); })))
                .child(div().flex().items_center().gap_3().child(crate::i18n::tr("Repeat every")).child(div().w_24().child(Input::new(&self.interval).id("updates-interval").disabled(busy))).child(div().w_40().h_8().flex_shrink_0().child(Select::new(&self.unit).id("updates-unit").disabled(busy))))
                .child(div().child(crate::i18n::tr("Check for updates to")))
                .child(div().h_8().flex_shrink_0().child(Select::new(&self.target).id("updates-target").disabled(busy)))
                .child(Checkbox::new("updates-prereleases").label(crate::i18n::tr("Include prereleases")).checked(self.draft.include_prereleases).disabled(busy).on_change(cx.listener(|this, value: &bool, _, cx| { this.draft.include_prereleases = *value; cx.notify(); })))
                .child(div().border_t_1().border_color(cx.theme().border).pt_4().child(crate::i18n::presentation(&self.saved.outcome.label())))
                .child(div().child(crate::i18n::tr_args("Current: %{arg0}", &[("arg0", &(self.saved.current.as_ref().map(|v| format!("{} — {}", v.node, v.version)).unwrap_or_else(|| "selected daemon version unavailable".into())).to_string())])))
                .child(div().child(crate::i18n::tr_args("Latest: %{arg0}", &[("arg0", (self.saved.latest.as_deref().unwrap_or("not available")))])))
                .when_some(self.saved.warning.clone(), |container, warning| container.child(div().text_color(cx.theme().danger).child(crate::i18n::presentation(&warning))))
                .when_some(self.message.clone(), |container, message| container.child(div().text_color(cx.theme().danger).child(crate::i18n::presentation(&message))))
                .when_some(self.saved.release_url.clone(), |container, url| container.child(Button::new("updates-release").label(crate::i18n::tr("Open official release page")).on_click(move |_,_,cx| cx.open_url(&url))))
                .child(div().flex().flex_wrap().gap_2()
                .child(Button::new("updates-check").label(crate::i18n::tr("Check now (saved settings)")).disabled(busy || self.saved.busy).on_click(cx.listener(|this,_,window,cx| this.send(Command::CheckNow, false, window, cx))))
                .child(Button::new("updates-cancel").label(crate::i18n::tr("Cancel check")).disabled(busy || !self.saved.busy).on_click(cx.listener(|this,_,window,cx| this.send(Command::Cancel, false, window, cx))))
                .child(Button::new("updates-reload").label(crate::i18n::tr("Reload saved")).disabled(busy).on_click(cx.listener(|this,_,window,cx| this.send(Command::Reload, true, window, cx))))
)
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::i18n::tr("Rustleblower and OpenSnitch releases are checked separately. Install updates through your usual package source."))))
            .child(super::settings::footer("updates-close", busy,
                Button::new("updates-save").disabled(busy).on_click(cx.listener(|this,_,window,cx|this.save(window,cx))), cx))
    }
}
