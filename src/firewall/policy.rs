//! Built-in policy drafts and acknowledged, bounded full-reload sequences.
use super::{
    document::{Chain, Document, Expression, Rule, Statement, Value},
    fingerprint, validate,
};
use crate::{
    protocol,
    rules::{Control, FirewallLease},
};
use prost::Message;
use std::sync::{Arc, Mutex};

// These are protocol-visible identities used by OpenSnitch's built-in profiles.
// Retaining them lets either UI recognize and remove the same profile rules.
const INPUT_LOOPBACK: &str = "profile-drop-inbound-2d7e6fe4-c21d-11ec-99a6-3c970e298b0c";
const INPUT_STATE: &str = "profile-drop-inbound-e1fc1a1c-c21c-11ec-9a2a-3c970e298b0c";
const OUTPUT_STATE: &str = "profile-drop-outbound-e1fc1a1c-c21c-11ec-9a2a-3c970e298b0c";

#[derive(Clone, Copy)]
pub enum Policy {
    Accept,
    Drop,
}
fn rule(id: &str, description: &str, name: &str, values: &[(&str, &str)]) -> Rule {
    Rule {
        uuid: id.into(),
        description: description.into(),
        enabled: true,
        target: "accept".into(),
        expressions: vec![Expression {
            statement: Some(Statement {
                name: name.into(),
                values: values
                    .iter()
                    .map(|(key, value)| Value {
                        key: (*key).into(),
                        value: (*value).into(),
                    })
                    .collect(),
                ..Default::default()
            }),
        }],
        ..Default::default()
    }
}
/// Edit the selected chain, preserving its actual table/name/family/priority.
/// No configuration is sent here; the common review owns every daemon write.
pub fn preset(
    document: &mut Document,
    group: usize,
    index: usize,
    policy: Policy,
) -> Result<(), String> {
    document.edit(|doc| {
        let chain=doc.chain_mut(group,index)?;
        let (ids,rules)=if chain.hook.eq_ignore_ascii_case("input")&&chain.kind.eq_ignore_ascii_case("filter") {
            (vec![INPUT_LOOPBACK,INPUT_STATE],vec![rule(INPUT_LOOPBACK,"Allow loopback input","iifname",&[("lo","")]),rule(INPUT_STATE,"Allow established and related input","ct",&[("state","related"),("state","established")])])
        } else if chain.hook.eq_ignore_ascii_case("output")&&chain.kind.eq_ignore_ascii_case("mangle") {
            (vec![OUTPUT_STATE],vec![rule(OUTPUT_STATE,"Allow established and related output","ct",&[("state","established,related")])])
        } else if chain.hook.eq_ignore_ascii_case("forward")&&chain.kind.eq_ignore_ascii_case("mangle") {
            (Vec::new(),Vec::new())
        } else {return Err("Built-in policy presets require an input/filter, output/mangle or forward/mangle chain".into());};
        match policy {
            Policy::Accept=>{chain.policy="accept".into();chain.rules.retain(|r|!ids.contains(&r.uuid.as_str()));},
            Policy::Drop=>{
                chain.policy="drop".into();
                for rule in rules {if !chain.rules.iter().any(|r|r.uuid==rule.uuid){chain.rules.push(rule);}}
            },
        }
        Ok(())
    })
}
fn prepare_policies(chain: &mut Chain) -> Result<(), String> {
    if chain.hook.is_empty() {
        return Ok(());
    }
    if chain.policy.eq_ignore_ascii_case("drop") {
        chain.policy = "accept".into();
    } else if !chain.policy.is_empty() && !chain.policy.eq_ignore_ascii_case("accept") {
        return Err(
            "A base chain has an unknown policy; review it before disabling the firewall".into(),
        );
    }
    Ok(())
}
pub fn enabled(document: &mut Document, enabled: bool) -> Result<(), String> {
    document.edit(|doc| {
        if !enabled {
            for chain in doc.system_rules.iter_mut().flat_map(|g| &mut g.chains) {
                prepare_policies(chain)?;
            }
        }
        doc.enabled = enabled;
        Ok(())
    })
}
struct Step {
    config: protocol::SysFirewall,
    label: &'static str,
}
pub struct Plan {
    expected: [u8; 32],
    steps: Vec<Step>,
    summary: Vec<String>,
}
impl Plan {
    pub fn new(
        expected: [u8; 32],
        before: &protocol::SysFirewall,
        after: protocol::SysFirewall,
    ) -> Result<Self, String> {
        super::admit(before)?;
        validate(&after)?;
        if before.version != 1 || fingerprint(&before.encode_to_vec()) != expected {
            return Err(
                "The reviewed source cannot be preserved by the supported firewall wire format"
                    .into(),
            );
        }
        if before == &after {
            return Err("No firewall changes to review".into());
        }
        let mut steps = Vec::with_capacity(2);
        if before.enabled && !after.enabled {
            let mut checked = Document::from(after.clone());
            enabled(&mut checked, false)?;
            if protocol::SysFirewall::from(checked) != after {
                return Err("Disabling needs accept policies first. Use Enable system firewall to prepare the transition, or review all base-chain policies explicitly".into());
            }
            let mut preparation = Document::from(before.clone());
            enabled(&mut preparation, false)?;
            preparation.enabled = true;
            let preparation = protocol::SysFirewall::from(preparation);
            if &preparation != before {
                steps.push(Step {
                    config: preparation,
                    label: "Prepare accept policies while the system firewall is enabled",
                });
            }
        }
        steps.push(Step {
            label: if before.enabled && !after.enabled {
                "Save the disabled system-firewall configuration"
            } else if !before.enabled && after.enabled {
                "Enable the reviewed system-firewall configuration"
            } else {
                "Reload the reviewed system-firewall configuration"
            },
            config: after,
        });
        let mut summary = Vec::new();
        let mut previous = before;
        let mut total = 0;
        let mut shown = 0;
        for (index, step) in steps.iter().enumerate() {
            summary.push(format!(
                "Step {}/{}: {}",
                index + 1,
                steps.len(),
                step.label
            ));
            let (lines, count) = Document::from(previous.clone())
                .differences(&Document::from(step.config.clone()))?;
            total += count;
            for line in lines.into_iter().take(200 - shown) {
                summary.push(line);
                shown += 1;
            }
            previous = &step.config;
        }
        if total > shown {
            summary.push(format!("{} more changed fields; inspect the complete draft. Review values above 512 bytes are shortened.",total-shown));
        }
        Ok(Self {
            expected,
            steps,
            summary,
        })
    }
    pub fn summary(&self) -> &[String] {
        &self.summary
    }
    pub fn configs(&self) -> impl Iterator<Item = &protocol::SysFirewall> {
        self.steps.iter().map(|s| &s.config)
    }
    pub fn begin(self, control: &Arc<Control>, session: u64) -> Result<Operation, String> {
        let lease = control.reserve_firewall(session, self.expected)?;
        let progress = Arc::new(Mutex::new(Progress {
            completed: 0,
            total: self.steps.len(),
            label: self.steps[0].label.into(),
            outcome: None,
        }));
        Ok(Operation {
            plan: self,
            lease,
            progress,
        })
    }
}
#[derive(Clone)]
pub struct Progress {
    pub completed: usize,
    pub total: usize,
    pub label: String,
    pub outcome: Option<Result<usize, String>>,
}
pub struct Operation {
    plan: Plan,
    lease: FirewallLease,
    progress: Arc<Mutex<Progress>>,
}
impl Operation {
    pub fn progress(&self) -> Arc<Mutex<Progress>> {
        self.progress.clone()
    }
    /// Run on an independent executor and detach it from window ownership.
    /// The shared command path includes both daemon ACK and durable cache save.
    pub async fn run(self) {
        let Self {
            plan,
            lease,
            progress,
        } = self;
        let mut expected = plan.expected;
        let mut completed = 0;
        let total = plan.steps.len();
        let mut outcome = Ok(total);
        for step in plan.steps {
            progress.lock().unwrap().label = step.label.into();
            let next = fingerprint(&step.config.encode_to_vec());
            let result = match super::Change::new(expected, step.config)
                .and_then(|change| lease.submit(change))
            {
                Ok(pending) => pending.await.unwrap_or_else(|_| {
                    Err("The command stopped before confirmation; reconnect to verify".into())
                }),
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                outcome = Err(format!(
                    "Stopped with {completed}/{total} confirmed cache updates. {error} Earlier accepted stages remain in place."
                ));
                break;
            }
            completed += 1;
            progress.lock().unwrap().completed = completed;
            expected = next;
        }
        // Release before publishing completion, so a reopened window can retry.
        drop(lease);
        progress.lock().unwrap().outcome = Some(outcome);
    }
}
