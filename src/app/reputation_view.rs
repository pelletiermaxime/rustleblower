//! Native settings and explicitly captured reputation inspections. Secrets are
//! file paths only; HTTP/persistence remain on the independent worker.
use crate::reputation::{
    Capture, Command, Handle, Query, Settings, Snapshot, Status, Subject, Ticket, Verdict,
    WarningAction,
};
use gpui::{prelude::*, *};
#[cfg(feature = "headless-tests")]
use gpui_kit::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable,
    button::Button,
    checkbox::Checkbox,
    input::{Input, InputState},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
static INSPECTION: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy)]
struct OpenInspection {
    window: AnyWindowHandle,
    capture: Option<Capture>,
}
impl Global for OpenInspection {}
pub(super) fn open(
    handle: Handle,
    initial: Option<Query>,
    cx: &mut App,
) -> gpui::Result<AnyWindowHandle> {
    let capture = initial.as_ref().map(|query| query.capture);
    if let Some(open) = cx.try_global::<OpenInspection>().copied() {
        let same = initial.is_none() || capture == open.capture;
        if same
            && open
                .window
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return Ok(open.window);
        }
        if !same {
            let _ = open
                .window
                .update(cx, |_, window, _| window.remove_window());
        }
    }
    let (window, _) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(860.), px(740.)),
                cx,
            ))),
            window_min_size: Some(size(px(640.), px(560.))),
            titlebar: Some(TitlebarOptions {
                title: Some(crate::i18n::tr("Rustleblower — reputation").into()),
                ..Default::default()
            }),
            app_id: Some("dev.rustleblower.Rustleblower".into()),
            icon: Some(super::brand_icons::window_icon()),
            ..Default::default()
        },
        cx,
        move |window, cx| cx.new(|cx| View::new(handle, initial, window, cx)),
    )?;
    crate::i18n::register_title(window, "Rustleblower — reputation", cx);
    cx.set_global(OpenInspection { window, capture });
    Ok(window)
}
pub(super) struct View {
    handle: Handle,
    saved: Snapshot,
    draft: Settings,
    draft_revision: Option<String>,
    key: Entity<InputState>,
    timeout: Entity<InputState>,
    budget: Entity<InputState>,
    threshold: Entity<InputState>,
    excluded_domains: Vec<Entity<InputState>>,
    excluded_ips: Vec<Entity<InputState>>,
    query: Entity<InputState>,
    initial: Option<Query>,
    ticket: Option<Ticket>,
    details: Option<usize>,
    tab: usize,
    pending: bool,
    message: Option<String>,
}
#[derive(Clone, Copy)]
enum Toggle {
    Enabled,
    Domains,
    Ips,
    Hashes,
    Suspicious,
    Community,
    Reputation,
    Recommend,
}
fn input(
    value: String,
    max: usize,
    window: &mut Window,
    cx: &mut Context<View>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value)
            .validate(move |text, _| text.len() <= max && !text.chars().any(char::is_control))
    })
}
fn exclusions(
    value: &[String],
    window: &mut Window,
    cx: &mut Context<View>,
) -> Vec<Entity<InputState>> {
    value
        .iter()
        .map(|text| input(text.clone(), 128, window, cx))
        .collect()
}
impl View {
    pub(super) fn new(
        handle: Handle,
        initial: Option<Query>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let saved = handle.snapshot();
        let draft = saved.settings.clone();
        let draft_revision = saved.revision.clone();
        let pending = !saved.ready;
        let key = input(
            draft
                .api_key_file
                .as_ref()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .into(),
            4096,
            window,
            cx,
        );
        let timeout = input(draft.timeout_seconds.to_string(), 2, window, cx);
        let budget = input(draft.request_budget.to_string(), 4, window, cx);
        let threshold = input(draft.minimum_threshold.to_string(), 7, window, cx);
        let excluded_domains = exclusions(&draft.excluded_domains, window, cx);
        let excluded_ips = exclusions(&draft.excluded_ips, window, cx);
        let query = input(String::new(), 254, window, cx);
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
                            this.pending = false;
                            this.reset(window, cx);
                            this.start_initial();
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
        let tab = usize::from(initial.is_some());
        let mut view = Self {
            handle,
            saved,
            draft,
            draft_revision,
            key,
            timeout,
            budget,
            threshold,
            excluded_domains,
            excluded_ips,
            query,
            initial,
            ticket: None,
            details: None,
            tab,
            pending,
            message: None,
        };
        if !pending {
            view.start_initial();
        }
        view
    }
    fn start_initial(&mut self) {
        if self.ticket.is_none()
            && let Some(query) = &self.initial
        {
            self.ticket = Some(self.handle.lookup(query.clone()));
        }
    }
    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.draft = self.saved.settings.clone();
        self.draft_revision = self.saved.revision.clone();
        for (input, value) in [
            (
                &self.key,
                self.draft
                    .api_key_file
                    .as_ref()
                    .and_then(|p| p.to_str())
                    .unwrap_or("")
                    .to_string(),
            ),
            (&self.timeout, self.draft.timeout_seconds.to_string()),
            (&self.budget, self.draft.request_budget.to_string()),
            (&self.threshold, self.draft.minimum_threshold.to_string()),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.excluded_domains = exclusions(&self.draft.excluded_domains, window, cx);
        self.excluded_ips = exclusions(&self.draft.excluded_ips, window, cx);
    }
    fn send(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
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
                    Ok(Ok(snapshot)) => {
                        this.saved = snapshot;
                        this.reset(window, cx);
                        this.message = None;
                    }
                    Ok(Err(message)) => this.message = Some(message),
                    Err(_) => {
                        this.message = Some(crate::i18n::tr("Reputation checks are unavailable"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut draft = self.draft.clone();
        let key = self.key.read(cx).value().to_string();
        draft.api_key_file = (!key.is_empty()).then(|| PathBuf::from(key));
        let values = (
            self.timeout.read(cx).value().parse::<u8>(),
            self.budget.read(cx).value().parse::<u32>(),
            self.threshold.read(cx).value().parse::<u32>(),
        );
        let (Ok(timeout), Ok(budget), Ok(threshold)) = values else {
            self.message = Some(crate::i18n::tr(
                "Enter whole numbers for timeout, request budget and warning threshold",
            ));
            cx.notify();
            return;
        };
        draft.timeout_seconds = timeout;
        draft.request_budget = budget;
        draft.minimum_threshold = threshold;
        draft.excluded_domains = self
            .excluded_domains
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .filter(|text| !text.is_empty())
            .collect();
        draft.excluded_ips = self
            .excluded_ips
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .filter(|text| !text.is_empty())
            .collect();
        if let Err(error) = draft.validate() {
            self.message = Some(error.to_string());
            cx.notify();
            return;
        }
        self.send(
            Command::Save {
                expected_revision: self.draft_revision.clone(),
                settings: draft,
            },
            window,
            cx,
        );
    }
    fn inspect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.query.read(cx).value().to_string();
        let subject = if let Ok(ip) = text.parse() {
            Ok(Subject::Ip(ip))
        } else if [32, 40, 64].contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit())
        {
            Subject::hash(&text)
        } else {
            Subject::domain(&text)
        };
        match subject {
            Ok(subject) => {
                self.initial = None;
                let capture = Capture {
                    session: 0,
                    object_id: INSPECTION.fetch_add(1, Ordering::Relaxed),
                };
                if let Some(open) = cx.try_global::<OpenInspection>().copied()
                    && open.window == window.window_handle()
                {
                    cx.set_global(OpenInspection {
                        capture: Some(capture),
                        ..open
                    });
                }
                self.ticket = Some(self.handle.lookup(Query {
                    capture,
                    subjects: vec![subject],
                }));
                self.details = None;
                self.message = None;
            }
            Err(error) => self.message = Some(error.to_string()),
        }
        cx.notify();
    }
    fn toggle(
        &self,
        id: &'static str,
        label: &'static str,
        value: bool,
        field: Toggle,
        cx: &Context<Self>,
    ) -> Checkbox {
        Checkbox::new(id)
            .label(crate::i18n::tr(label))
            .checked(value)
            .disabled(self.pending)
            .on_change(cx.listener(move |this, value: &bool, _, cx| {
                match field {
                    Toggle::Enabled => this.draft.enabled = *value,
                    Toggle::Domains => this.draft.domains = *value,
                    Toggle::Ips => this.draft.ips = *value,
                    Toggle::Hashes => this.draft.hashes = *value,
                    Toggle::Suspicious => this.draft.use_suspicious = *value,
                    Toggle::Community => this.draft.use_community_score = *value,
                    Toggle::Reputation => this.draft.use_reputation = *value,
                    Toggle::Recommend => {
                        this.draft.warning_action = if *value {
                            WarningAction::RecommendReject
                        } else {
                            WarningAction::None
                        }
                    }
                }
                cx.notify();
            }))
    }
    fn exclusion_fields(&self, domains: bool, cx: &mut Context<Self>) -> Div {
        let fields = if domains {
            &self.excluded_domains
        } else {
            &self.excluded_ips
        };
        let category = if domains { "domains" } else { "ips" };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(fields.iter().enumerate().map(|(index, input)| {
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(input)
                                .id(SharedString::from(format!(
                                    "rep-excluded-{category}-{index}"
                                )))
                                .disabled(self.pending),
                        ),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "rep-excluded-{category}-remove-{index}"
                        )))
                        .label(crate::i18n::tr("Remove"))
                        .disabled(self.pending)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let fields = if domains {
                                &mut this.excluded_domains
                            } else {
                                &mut this.excluded_ips
                            };
                            if index < fields.len() {
                                fields.remove(index);
                            }
                            cx.notify();
                        })),
                    )
            }))
            .child(
                Button::new(SharedString::from(format!("rep-excluded-{category}-add")))
                    .label(crate::i18n::tr("Add exclusion fragment"))
                    .disabled(self.pending || fields.len() >= 32)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let field = input(String::new(), 128, window, cx);
                        let fields = if domains {
                            &mut this.excluded_domains
                        } else {
                            &mut this.excluded_ips
                        };
                        if fields.len() < 32 {
                            fields.push(field);
                        }
                        cx.notify();
                    })),
            )
    }
    fn settings(&self, cx: &mut Context<Self>) -> Div {
        div().flex().flex_col().gap_4()
            .child(div().child(crate::i18n::tr("Enabling VirusTotal sends the selected domain, IP address or reported file hash to VirusTotal. File contents are never uploaded. Save applies these choices to new checks.")))
            .child(self.toggle("rep-enabled","Enable reputation checks",self.draft.enabled,Toggle::Enabled,cx))
            .child(div().child(crate::i18n::tr("API key file path (owned by you, permissions 0600; no symlink)")))
            .child(Input::new(&self.key).id("rep-key-file").disabled(self.pending))
            .child(div().child(crate::i18n::tr_args("Key file: %{arg0} · Requests this session: %{arg1} / %{arg2}", &[("arg0", &crate::i18n::tr(&format!("{:?}", self.saved.key_status))), ("arg1", &(self.saved.dispatched).to_string()), ("arg2", &(self.saved.settings.request_budget).to_string())])))
            .child(div().flex().gap_3().flex_wrap().child(div().w(px(144.)).child(crate::i18n::tr("Timeout (seconds)")).child(Input::new(&self.timeout).id("rep-timeout").disabled(self.pending))).child(div().w(px(176.)).child(crate::i18n::tr("Session request budget")).child(Input::new(&self.budget).id("rep-budget").disabled(self.pending))).child(div().w(px(176.)).child(crate::i18n::tr("Warning threshold")).child(Input::new(&self.threshold).id("rep-threshold").disabled(self.pending))))
            .child(self.toggle("rep-domains","Check domains",self.draft.domains,Toggle::Domains,cx))
            .child(self.toggle("rep-ips","Check destination IP addresses",self.draft.ips,Toggle::Ips,cx))
            .child(self.toggle("rep-hashes","Check reported executable hashes",self.draft.hashes,Toggle::Hashes,cx))
            .child(self.toggle("rep-suspicious","Add suspicious engine results to the warning score",self.draft.use_suspicious,Toggle::Suspicious,cx))
            .child(self.toggle("rep-community","Add malicious community votes minus harmless votes",self.draft.use_community_score,Toggle::Community,cx))
            .child(self.toggle("rep-reputation","Subtract community reputation from the warning score",self.draft.use_reputation,Toggle::Reputation,cx))
            .child(self.toggle("rep-recommend","Recommend Reject for a warning on an untouched popup",self.draft.warning_action==WarningAction::RecommendReject,Toggle::Recommend,cx))
            .child(div().text_sm().child(crate::i18n::tr("A recommendation never changes your choice or the captured timeout fallback. A report without warnings does not prove that a destination or executable is safe.")))
            .child(div().child(crate::i18n::tr("Excluded domain fragments")))
            .child(self.exclusion_fields(true,cx))
            .child(div().child(crate::i18n::tr("Excluded IP fragments")))
            .child(self.exclusion_fields(false,cx))
            .child(div().text_sm().child(crate::i18n::tr("Local/private/multicast destinations and VirusTotal itself are excluded from automatic connection checks. Each category supports up to 32 fragments.")))
    }
    fn results(&self, cx: &mut Context<Self>) -> Div {
        let mut content=div().flex().flex_col().gap_4().child(div().child(crate::i18n::tr("Manual checks use saved settings. Enter a domain, IP address or reported MD5/SHA1/SHA256 hash. No binary is read or uploaded.")))
            .child(Input::new(&self.query).id("rep-query").disabled(self.pending))
            .child(Button::new("rep-check").label(crate::i18n::tr("Check entered value")).disabled(self.pending).on_click(cx.listener(|this,_,window,cx|this.inspect(window,cx))));
        if let Some(query) = &self.initial {
            content = content
                .child(div().child(crate::i18n::tr("Captured connection subjects:")))
                .children(query.subjects.iter().map(|subject| {
                    div()
                        .text_sm()
                        .child(format!("{}: {}", subject.category(), subject.value()))
                }))
                .child(
                    Button::new("rep-captured-check")
                        .label(crate::i18n::tr("Check captured subjects again"))
                        .disabled(self.pending)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ticket = None;
                            this.start_initial();
                            this.details = None;
                            cx.notify();
                        })),
                );
        }
        if let Some(ticket) = &self.ticket {
            let result = ticket.snapshot();
            content = content.child(
                div()
                    .text_lg()
                    .child(crate::i18n::tr(result.status.label())),
            );
            for (index, report) in result.reports.into_iter().enumerate() {
                let color = if report.verdict == Verdict::Warning {
                    cx.theme().danger
                } else {
                    cx.theme().foreground
                };
                let url = report.details_url.clone();
                let detail = report.details.clone();
                content=content.child(div().border_1().border_color(cx.theme().border).rounded_md().p_3().flex().flex_col().gap_2()
                    .child(div().text_color(color).child(format!("{} — {}",report.subject.value(),crate::i18n::tr(report.status.label()))))
                    .child(div().child(crate::i18n::tr(match report.verdict{Verdict::Unknown=>"Unknown — no safety conclusion",Verdict::NotFlagged=>"Not flagged at the configured threshold",Verdict::Warning=>"Warning at the configured threshold"})))
                    .child(div().text_sm().child(crate::i18n::tr_args("Engines: %{arg0} malicious · %{arg1} suspicious · %{arg2} harmless · %{arg3} undetected", &[("arg0", &(report.counts.malicious).to_string()), ("arg1", &(report.counts.suspicious).to_string()), ("arg2", &(report.counts.harmless).to_string()), ("arg3", &(report.counts.undetected).to_string())])))
                    .when_some(report.score,|div,score|div.child(crate::i18n::tr_args("Configured score: %{score}", &[("score", &(score).to_string())])))
                    .child(div().flex().flex_wrap().gap_2()
                        .child(Button::new(SharedString::from(format!("rep-official-{index}"))).label(crate::i18n::tr("Open VirusTotal report")).on_click(move|_,_,cx|cx.open_url(&url)))
                        .child(Button::new(SharedString::from(format!("rep-details-{index}"))).label(crate::i18n::tr("Show report excerpt")).disabled(detail.is_empty()).on_click(cx.listener(move|this,_,_,cx|{this.details=if this.details==Some(index){None}else{Some(index)};cx.notify();})))
                        .child(Button::new(SharedString::from(format!("rep-copy-{index}"))).label(crate::i18n::tr("Copy report excerpt")).disabled(detail.is_empty()).on_click(move|_,_,cx|cx.write_to_clipboard(ClipboardItem::new_string(detail.clone())))))
                    .when(self.details==Some(index),|container|container.child(div().text_sm().child(report.details.clone())).when(report.details_truncated,|container|container.child(crate::i18n::tr("Report excerpt shortened; open VirusTotal for the full report.")))));
            }
        } else {
            content = content.child(crate::i18n::tr("No reputation check requested"));
        }
        content
    }
}
impl Render for View {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if self.tab == 0 {
            self.settings(cx)
        } else {
            self.results(cx)
        };
        let scroll = div()
            .id("rep-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_5()
            .child(content)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("rep-reload")
                            .label(crate::i18n::tr("Reload saved"))
                            .disabled(self.pending)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.send(Command::Reload, window, cx)
                            })),
                    )
                    .child(
                        Button::new("rep-cancel")
                            .label(crate::i18n::tr("Cancel check"))
                            .disabled(self.ticket.as_ref().is_none_or(|t| {
                                !matches!(t.snapshot().status, Status::Queued | Status::Checking)
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(ticket) = &this.ticket {
                                    ticket.cancel();
                                }
                                cx.notify();
                            })),
                    ),
            );
        #[cfg(feature = "headless-tests")]
        let scroll = scroll.test_support();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .gap_3()
                    .px_5()
                    .py_4()
                    .child(
                        div()
                            .text_xl()
                            .flex_1()
                            .child(crate::i18n::tr("VirusTotal reputation")),
                    )
                    .child(
                        Button::new("rep-tab-settings")
                            .label(crate::i18n::tr("Settings"))
                            .selected(self.tab == 0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab = 0;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("rep-tab-results")
                            .label(crate::i18n::tr("Results"))
                            .selected(self.tab == 1)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab = 1;
                                cx.notify();
                            })),
                    ),
            )
            .child(scroll)
            .when_some(
                self.saved.warning.clone().or_else(|| self.message.clone()),
                |container, message| {
                    container.child(
                        div()
                            .px_4()
                            .text_color(cx.theme().danger)
                            .child(crate::i18n::presentation(&message)),
                    )
                },
            )
            .child(super::settings::footer(
                "rep-close",
                self.pending,
                Button::new("rep-save")
                    .disabled(self.pending)
                    .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                cx,
            ))
    }
}

#[cfg(all(test, feature = "headless-tests"))]
pub(super) fn test_result(window: &Window, cx: &App) -> Option<crate::reputation::ResultSet> {
    let view = gpui_kit::base::Root::read(window, cx).view().clone();
    if let Ok(view) = view.clone().downcast::<View>() {
        return view.read(cx).test_result();
    }
    view.downcast::<super::settings::Settings>()
        .ok()?
        .read(cx)
        .test_reputation_result(cx)
}

#[cfg(all(test, feature = "headless-tests"))]
impl View {
    pub(super) fn test_result(&self) -> Option<crate::reputation::ResultSet> {
        self.ticket.as_ref().map(Ticket::snapshot)
    }
}
