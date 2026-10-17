use super::*;
#[derive(Default)]
pub(super) struct UrlTest {
    target: Option<(String, Format)>,
    ticket: Option<subscriptions::probe::Ticket>,
    pub message: Option<String>,
    pub failed: bool,
}
impl UrlTest {
    pub fn busy(&self) -> bool {
        self.ticket.is_some()
    }
    fn target(form: &Form, cx: &App) -> Result<(String, Format), String> {
        let label = form
            .format
            .as_ref()
            .ok_or("Open a subscription before testing")?
            .read(cx)
            .selected_value()
            .map(ToString::to_string)
            .unwrap_or_default();
        let key = FORMATS
            .iter()
            .find(|(_, value)| *value == label)
            .map(|(key, _)| *key)
            .ok_or("Choose a format")?;
        Ok((
            form.fields[1].read(cx).value().trim().to_owned(),
            Format::parse(key)?,
        ))
    }
    fn refresh(&mut self, target: (String, Format)) -> bool {
        if self.target.as_ref() == Some(&target) {
            return false;
        }
        self.ticket = None;
        self.message = None;
        self.failed = false;
        self.target = Some(target);
        true
    }
    pub fn cancel(&mut self) {
        if self.ticket.take().is_some() {
            self.message = Some(crate::i18n::tr("URL test cancelled"));
            self.failed = false;
        }
    }
    pub fn poll(form: &mut Form, cx: &App) -> bool {
        let Ok(target) = Self::target(form, cx) else {
            return false;
        };
        let mut changed = form.url_test.refresh(target);
        if let Some(result) = form.url_test.ticket.as_mut().and_then(|t| t.try_result()) {
            form.url_test.ticket = None;
            changed = true;
            match result {
                Ok(report) => {
                    form.url_test.failed = report.hosts_like == Some(false);
                    form.url_test.message = Some(report.message());
                }
                Err(error) => {
                    form.url_test.failed = true;
                    form.url_test.message = Some(error);
                }
            }
        }
        changed
    }
}
impl SubscriptionsView {
    pub(in crate::app) fn cancel_url_test(&mut self) {
        if let Some(form) = &mut self.form {
            form.url_test.cancel();
        }
    }
    #[cfg(all(test, feature = "headless-tests"))]
    pub(in crate::app) fn url_test_state(&self) -> (bool, Option<&str>, bool) {
        self.form
            .as_ref()
            .map(|f| {
                (
                    f.url_test.busy(),
                    f.url_test.message.as_deref(),
                    f.url_test.failed,
                )
            })
            .unwrap_or((false, None, false))
    }
}
impl Dashboard {
    pub(super) fn test_subscription_url(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &mut self.subscriptions.form else {
            return;
        };
        let target = UrlTest::target(form, cx);
        match target {
            Ok(target) => {
                form.url_test.refresh(target.clone());
                match self.backend.subscriptions.test_url(target.0, target.1) {
                    Ok(ticket) => {
                        form.url_test.ticket = Some(ticket);
                        form.url_test.message = Some(crate::i18n::tr(
                            "Testing URL… No subscription or list data will be saved.",
                        ));
                        form.url_test.failed = false;
                    }
                    Err(error) => {
                        form.url_test.message = Some(error);
                        form.url_test.failed = true;
                    }
                }
            }
            Err(error) => {
                form.url_test.message = Some(error);
                form.url_test.failed = true;
            }
        }
        cx.notify();
    }
}
