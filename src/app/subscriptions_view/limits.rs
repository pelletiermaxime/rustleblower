use super::*;
use crate::subscriptions::limits::{Override, Unit, Units};
pub(super) struct Controls {
    pub pickers: [Picker; 3],
    pub defaults: (u64, Settings),
}
fn picker(
    unit: Option<Unit>,
    size: bool,
    inherit: bool,
    window: &mut Window,
    cx: &mut Context<Dashboard>,
) -> Picker {
    let mut labels = Vec::<SharedString>::new();
    if inherit {
        labels.push("Inherit default".into());
    }
    labels.extend(
        if size { Unit::SIZE } else { Unit::TIME }
            .iter()
            .map(|u| SharedString::from(u.label())),
    );
    let index = unit
        .and_then(|u| labels.iter().position(|label| label.as_ref() == u.label()))
        .unwrap_or(0);
    let picker = cx.new(|cx| {
        SelectState::new(
            SearchableVec::new(labels),
            Some(IndexPath::new(index)),
            window,
            cx,
        )
        .searchable(true)
    });
    cx.subscribe(
        &picker,
        |_, _, _: &SelectEvent<SearchableVec<SharedString>>, cx| cx.notify(),
    )
    .detach();
    picker
}
impl Controls {
    pub fn subscription(
        spec: &Spec,
        defaults: (u64, Settings),
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> (Self, [String; 3]) {
        let fields = [spec.interval, spec.timeout, spec.max_bytes];
        let mut values = Vec::new();
        let mut pickers = Vec::new();
        for (i, value) in fields.into_iter().enumerate() {
            let (text, unit) = value.map(|v| v.display(i == 2)).unwrap_or_default();
            values.push(text);
            pickers.push(picker(unit, i == 2, true, window, cx));
        }
        (
            Self {
                pickers: pickers.try_into().ok().unwrap(),
                defaults,
            },
            values.try_into().ok().unwrap(),
        )
    }
    pub fn settings(
        defaults: (u64, Settings),
        window: &mut Window,
        cx: &mut Context<Dashboard>,
    ) -> (Self, [String; 3]) {
        let s = &defaults.1;
        let units = s.units();
        let units = [units.interval, units.timeout, units.size];
        let values = [u64::from(s.interval), u64::from(s.timeout), s.max_bytes]
            .into_iter()
            .zip(units)
            .map(|(v, u)| (v / u.factor()).to_string())
            .collect::<Vec<_>>();
        let pickers = units
            .into_iter()
            .enumerate()
            .map(|(i, unit)| picker(Some(unit), i == 2, false, window, cx))
            .collect::<Vec<_>>();
        (
            Self {
                pickers: pickers.try_into().ok().unwrap(),
                defaults,
            },
            values.try_into().ok().unwrap(),
        )
    }
    fn unit(&self, index: usize, cx: &App) -> Result<Option<Unit>, String> {
        let label = self.pickers[index]
            .read(cx)
            .selected_value()
            .map(ToString::to_string)
            .unwrap_or_default();
        if label == "Inherit default" {
            Ok(None)
        } else {
            Unit::parse(&label, index == 2).map(Some)
        }
    }
    pub fn overrides(
        &self,
        values: [Option<u64>; 3],
        cx: &App,
    ) -> Result<[Option<Override>; 3], String> {
        let mut out = [None; 3];
        for (i, count) in values.into_iter().enumerate() {
            out[i] = Override::parts(count, self.unit(i, cx)?);
            if let Some(value) = out[i] {
                value.validate(i == 2)?;
            }
        }
        Ok(out)
    }
    pub fn set_defaults(
        &self,
        values: [u64; 3],
        settings: &mut Settings,
        cx: &App,
    ) -> Result<(), String> {
        let units = [
            self.unit(0, cx)?.ok_or("Choose default units")?,
            self.unit(1, cx)?.ok_or("Choose default units")?,
            self.unit(2, cx)?.ok_or("Choose default units")?,
        ];
        let converted = values
            .into_iter()
            .zip(units)
            .map(|(n, u)| n.checked_mul(u.factor()).ok_or("Limit conversion overflow"))
            .collect::<Result<Vec<_>, _>>()?;
        settings.interval = u32::try_from(converted[0]).map_err(|_| "Interval is too large")?;
        settings.timeout = u32::try_from(converted[1]).map_err(|_| "Timeout is too large")?;
        settings.max_bytes = converted[2];
        settings.units = Some(Units {
            interval: units[0],
            timeout: units[1],
            size: units[2],
        });
        Ok(())
    }
}
impl Form {
    pub(super) fn limit_counts(&self, cx: &App) -> Result<[Option<u64>; 3], String> {
        let offset = if self.settings.is_some() { 0 } else { 4 };
        let mut values = [None; 3];
        for (i, v) in values.iter_mut().enumerate() {
            let text = self.fields[offset + i].read(cx).value();
            if !text.trim().is_empty() {
                *v = Some(
                    text.trim()
                        .parse()
                        .map_err(|_| "Enter whole nonnegative limit counts")?,
                );
            }
        }
        Ok(values)
    }
    pub fn effective_label(&self, cx: &App) -> String {
        let values = (|| -> Result<_, String> {
            let [interval, timeout, max_bytes] =
                self.limits.overrides(self.limit_counts(cx)?, cx)?;
            effective(
                &Spec {
                    interval,
                    timeout,
                    max_bytes,
                    ..Default::default()
                },
                &self.limits.defaults.1,
            )
        })();
        match values {
            Ok((interval, timeout, size)) => format!(
                "Effective with loaded defaults: {interval} seconds between checks · {timeout} seconds timeout · {size} bytes maximum. Empty count and Inherit default unit are independent choices."
            ),
            Err(e) => format!("Limits need attention: {e}"),
        }
    }
}

#[cfg(all(test, feature = "headless-tests"))]
impl SubscriptionsView {
    pub(in crate::app) fn limit_state(
        &self,
        cx: &App,
    ) -> Option<(Vec<String>, Vec<String>, String)> {
        let form = self.form.as_ref()?;
        let offset = if form.settings.is_some() { 0 } else { 4 };
        Some((
            (offset..offset + 3)
                .map(|i| form.fields[i].read(cx).value().to_string())
                .collect(),
            form.limits
                .pickers
                .iter()
                .map(|p| {
                    p.read(cx)
                        .selected_value()
                        .map(ToString::to_string)
                        .unwrap_or_default()
                })
                .collect(),
            form.effective_label(cx),
        ))
    }
}
