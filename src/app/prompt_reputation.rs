//! A best-effort, captured permission lookup. Rendering never waits for I/O and
//! no report owns or resolves a permission request.
use crate::{
    config::Decision,
    prompts::Prompt,
    reputation::{Capture, Handle, Query, ResultSet, Status, Ticket, Verdict},
};
use gpui::{prelude::*, *};
use gpui_kit::component::{ActiveTheme, Disableable, button::Button};

#[derive(Default)]
pub(super) struct Probe {
    capture: Option<Capture>,
    revision: Option<String>,
    ticket: Option<Ticket>,
    result: Option<ResultSet>,
    emphasized: bool,
}
impl Probe {
    pub(super) fn reset(&mut self) {
        // Dropping the ticket cancels queued/active work for the old capture.
        *self = Self::default();
    }
    pub(super) fn poll(&mut self, handle: &Handle, prompt: &Prompt) {
        let capture = Capture {
            session: prompt.session,
            object_id: prompt.id,
        };
        let saved = handle.snapshot();
        if self.capture != Some(capture) || self.revision != saved.revision {
            self.reset();
            self.capture = Some(capture);
            self.revision = saved.revision.clone();
        }
        if !saved.ready || !saved.settings.enabled {
            self.ticket = None;
            self.result = None;
            self.emphasized = false;
            return;
        }
        if prompt.reviewing {
            self.emphasized = false;
        }
        if self.ticket.is_none() && self.result.is_none() {
            self.ticket = Some(handle.lookup_connection(capture, &prompt.connection));
        }
        if self
            .result
            .as_ref()
            .is_some_and(|r| !matches!(r.status, Status::Queued | Status::Checking))
        {
            return;
        }
        let Some(ticket) = &self.ticket else { return };
        let result = ticket.snapshot();
        if result.capture != capture || result.settings_revision != self.revision {
            self.reset();
            return;
        }
        // A late result cannot change the emphasis once the user has begun
        // reviewing. It never changes the selected duration/scope or fallback.
        if !prompt.reviewing
            && result.reports.iter().any(|r| {
                r.verdict == Verdict::Warning && r.suggested_default == Some(Decision::Reject)
            })
        {
            self.emphasized = true;
        }
        self.result = Some(result);
    }
    pub(super) fn emphasized(&self) -> bool {
        self.emphasized
    }
    pub(super) fn visible(&self) -> bool {
        self.result.is_some()
    }
    pub(super) fn warning(&self) -> bool {
        self.result
            .as_ref()
            .is_some_and(|r| r.reports.iter().any(|r| r.verdict == Verdict::Warning))
    }
    pub(super) fn query(&self, expected: Capture) -> Option<Query> {
        let result = self.result.as_ref()?;
        (self.capture == Some(expected)
            && result.capture == expected
            && !result.reports.is_empty()
            && !matches!(result.status, Status::Queued | Status::Checking))
        .then(|| Query {
            capture: expected,
            subjects: result.reports.iter().map(|r| r.subject.clone()).collect(),
        })
    }
    pub(super) fn render(&self, cx: &mut Context<super::prompt_window::PromptWindow>) -> Div {
        let Some(result) = &self.result else {
            return div();
        };
        let capture = result.capture;
        let mut body = div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .text_sm()
            .child(crate::i18n::tr_args(
                "VirusTotal: %{arg0}",
                &[("arg0", &crate::i18n::tr(result.status.label()))],
            ));
        if self.warning() {
            body = body
                .p_2()
                .bg(cx.theme().warning.opacity(0.12))
                .text_color(cx.theme().warning)
                .child(crate::i18n::tr(
                    "Potential risk reported. Review the evidence before allowing this connection.",
                ));
        }
        for report in &result.reports {
            body = body.child(div().truncate().child(crate::i18n::tr_args(
                "%{arg0} · %{arg1} · %{arg2} malicious, %{arg3} suspicious",
                &[
                    ("arg0", &(report.subject.value()).to_string()),
                    ("arg1", &crate::i18n::tr(report.status.label())),
                    ("arg2", &(report.counts.malicious).to_string()),
                    ("arg3", &(report.counts.suspicious).to_string()),
                ],
            )));
        }
        body.child(
            Button::new("prompt-reputation-details")
                .label(crate::i18n::tr("View reputation reports…"))
                .disabled(self.query(capture).is_none())
                .on_click(cx.listener(move |this, _, _, cx| this.open_reputation(capture, cx))),
        )
    }
}
