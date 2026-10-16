//! Independent JSON interchange for the pinned wire format, including uint64 strings.
use super::{MAX_JSON, validate};
use crate::protocol;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase", deny_unknown_fields)]
pub struct Document {
    pub enabled: bool,
    pub version: u32,
    #[serde(deserialize_with = "null_vec")]
    pub system_rules: Vec<Group>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase", deny_unknown_fields)]
pub struct Group {
    pub rule: Option<Rule>,
    #[serde(deserialize_with = "null_vec")]
    pub chains: Vec<Chain>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase", deny_unknown_fields)]
pub struct Chain {
    pub name: String,
    pub table: String,
    pub family: String,
    pub priority: String,
    #[serde(rename = "Type")]
    pub kind: String,
    pub hook: String,
    pub policy: String,
    #[serde(deserialize_with = "null_vec")]
    pub rules: Vec<Rule>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase", deny_unknown_fields)]
pub struct Rule {
    pub table: String,
    pub chain: String,
    #[serde(rename = "UUID")]
    pub uuid: String,
    pub enabled: bool,
    #[serde(with = "position")]
    pub position: u64,
    pub description: String,
    pub parameters: String,
    #[serde(deserialize_with = "null_vec")]
    pub expressions: Vec<Expression>,
    pub target: String,
    pub target_parameters: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase", deny_unknown_fields)]
pub struct Expression {
    pub statement: Option<Statement>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase", deny_unknown_fields)]
pub struct Statement {
    pub op: String,
    pub name: String,
    #[serde(deserialize_with = "null_vec")]
    pub values: Vec<Value>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase", deny_unknown_fields)]
pub struct Value {
    pub key: String,
    pub value: String,
}

#[derive(Clone)]
pub enum Command {
    Policy {
        group: usize,
        chain: usize,
        drop: bool,
    },
    Enabled(bool),
    Chain {
        group: usize,
        index: Option<usize>,
        fields: Chain,
    },
    Rule {
        group: usize,
        chain: usize,
        index: Option<usize>,
        rule: Rule,
    },
    DeleteChain {
        group: usize,
        chain: usize,
    },
    DeleteRule {
        group: usize,
        chain: usize,
        rule: usize,
    },
    ToggleRule {
        group: usize,
        chain: usize,
        rule: usize,
    },
    MoveRule {
        group: usize,
        chain: usize,
        from: usize,
        to: usize,
    },
}
fn null_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}
mod position {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Position {
            Text(String),
            Number(u64),
        }
        match Position::deserialize(deserializer)? {
            Position::Text(s) => s.parse().map_err(D::Error::custom),
            Position::Number(n) => Ok(n),
        }
    }
}

// Explicit conversions preserve every field in the supported protobuf messages.
macro_rules! plain {
    ($local:ty, $wire:ty, {$($name:ident),*}) => {
        impl From<$wire> for $local {
            fn from(v: $wire) -> Self {Self {$($name:v.$name,)*}}
        }
        impl From<$local> for $wire {
            fn from(v: $local) -> Self {Self {$($name:v.$name,)*}}
        }
    };
}
plain!(Value, protocol::StatementValues, {key,value});
impl From<protocol::Statement> for Statement {
    fn from(v: protocol::Statement) -> Self {
        Self {
            op: v.op,
            name: v.name,
            values: v.values.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<Statement> for protocol::Statement {
    fn from(v: Statement) -> Self {
        Self {
            op: v.op,
            name: v.name,
            values: v.values.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<protocol::Expressions> for Expression {
    fn from(v: protocol::Expressions) -> Self {
        Self {
            statement: v.statement.map(Into::into),
        }
    }
}
impl From<Expression> for protocol::Expressions {
    fn from(v: Expression) -> Self {
        Self {
            statement: v.statement.map(Into::into),
        }
    }
}
impl From<protocol::FwRule> for Rule {
    fn from(v: protocol::FwRule) -> Self {
        Self {
            table: v.table,
            chain: v.chain,
            uuid: v.uuid,
            enabled: v.enabled,
            position: v.position,
            description: v.description,
            parameters: v.parameters,
            expressions: v.expressions.into_iter().map(Into::into).collect(),
            target: v.target,
            target_parameters: v.target_parameters,
        }
    }
}
impl From<Rule> for protocol::FwRule {
    fn from(v: Rule) -> Self {
        Self {
            table: v.table,
            chain: v.chain,
            uuid: v.uuid,
            enabled: v.enabled,
            position: v.position,
            description: v.description,
            parameters: v.parameters,
            expressions: v.expressions.into_iter().map(Into::into).collect(),
            target: v.target,
            target_parameters: v.target_parameters,
        }
    }
}
impl From<protocol::FwChain> for Chain {
    fn from(v: protocol::FwChain) -> Self {
        Self {
            name: v.name,
            table: v.table,
            family: v.family,
            priority: v.priority,
            kind: v.r#type,
            hook: v.hook,
            policy: v.policy,
            rules: v.rules.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<Chain> for protocol::FwChain {
    fn from(v: Chain) -> Self {
        Self {
            name: v.name,
            table: v.table,
            family: v.family,
            priority: v.priority,
            r#type: v.kind,
            hook: v.hook,
            policy: v.policy,
            rules: v.rules.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<protocol::FwChains> for Group {
    fn from(v: protocol::FwChains) -> Self {
        Self {
            rule: v.rule.map(Into::into),
            chains: v.chains.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<Group> for protocol::FwChains {
    fn from(v: Group) -> Self {
        Self {
            rule: v.rule.map(Into::into),
            chains: v.chains.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<protocol::SysFirewall> for Document {
    fn from(v: protocol::SysFirewall) -> Self {
        Self {
            enabled: v.enabled,
            version: v.version,
            system_rules: v.system_rules.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<Document> for protocol::SysFirewall {
    fn from(v: Document) -> Self {
        Self {
            enabled: v.enabled,
            version: v.version,
            system_rules: v.system_rules.into_iter().map(Into::into).collect(),
        }
    }
}
impl Document {
    pub fn command(&mut self, command: Command) -> Result<(), String> {
        if let Command::Policy { group, chain, drop } = command {
            return super::policy::preset(
                self,
                group,
                chain,
                if drop {
                    super::policy::Policy::Drop
                } else {
                    super::policy::Policy::Accept
                },
            );
        }
        if let Command::Enabled(value) = command {
            return super::policy::enabled(self, value);
        }
        if let Command::MoveRule {
            group,
            chain,
            from,
            to,
        } = command
        {
            return self.move_rule(group, chain, from, to);
        }
        self.edit(move |doc| {
            match command {
                Command::Chain {
                    group,
                    index,
                    mut fields,
                } => {
                    if group == doc.system_rules.len() && index.is_none() {
                        doc.system_rules.push(Group::default());
                    }
                    let chains = &mut doc
                        .system_rules
                        .get_mut(group)
                        .ok_or("Select a current group")?
                        .chains;
                    if let Some(index) = index {
                        let old = chains.get_mut(index).ok_or("Select a current chain")?;
                        fields.rules = std::mem::take(&mut old.rules);
                        *old = fields;
                    } else {
                        chains.push(fields);
                    }
                }
                Command::Rule {
                    group: target_group,
                    chain: target_chain,
                    index,
                    rule,
                } => {
                    if rule.enabled&&rule.expressions.is_empty(){return Err("Enabled rules need at least one statement; add a condition or disable the rule".into());}
                    if !rule.uuid.is_empty()
                        && doc.system_rules.iter().enumerate().any(|(g, group)| {
                            group.chains.iter().enumerate().any(|(c, chain)| {
                                chain.rules.iter().enumerate().any(|(r, other)| {
                                    other.uuid == rule.uuid
                                        && !(g == target_group
                                            && c == target_chain
                                            && Some(r) == index)
                                })
                            })
                        })
                    {
                        return Err("This rule ID already exists; choose a different ID".into());
                    }
                    let rules = &mut doc.chain_mut(target_group, target_chain)?.rules;
                    if let Some(index) = index {
                        *rules.get_mut(index).ok_or("Select a current rule")? = rule;
                    } else {
                        rules.push(rule);
                    }
                }
                Command::DeleteChain { group, chain } => {
                    let chains = &mut doc
                        .system_rules
                        .get_mut(group)
                        .ok_or("Select a current group")?
                        .chains;
                    if chain >= chains.len() {
                        return Err("Select a current chain".into());
                    }
                    chains.remove(chain);
                }
                Command::DeleteRule { group, chain, rule } => {
                    let rules = &mut doc.chain_mut(group, chain)?.rules;
                    if rule >= rules.len() {
                        return Err("Select a current rule".into());
                    }
                    rules.remove(rule);
                }
                Command::ToggleRule { group, chain, rule } => {
                    let rule = doc
                        .chain_mut(group, chain)?
                        .rules
                        .get_mut(rule)
                        .ok_or("Select a current rule")?;
                    if !rule.enabled&&rule.expressions.is_empty(){return Err("Add a statement before enabling this rule; the daemon ignores empty expressions".into());}
                    rule.enabled = !rule.enabled;
                }
                Command::MoveRule { .. } | Command::Policy{..} | Command::Enabled(_) => unreachable!(),
            }
            Ok(())
        })
    }
    pub fn differences(&self, after: &Self) -> Result<(Vec<String>, usize), String> {
        fn walk(
            before: Option<&serde_json::Value>,
            after: Option<&serde_json::Value>,
            path: &str,
            lines: &mut Vec<String>,
            total: &mut usize,
        ) {
            use serde_json::Value;
            if before == after {
                return;
            }
            match (before, after) {
                (Some(Value::Object(a)), Some(Value::Object(b))) => {
                    let keys = a
                        .keys()
                        .chain(b.keys())
                        .collect::<std::collections::BTreeSet<_>>();
                    for key in keys {
                        walk(
                            a.get(key),
                            b.get(key),
                            &format!("{path}/{key}"),
                            lines,
                            total,
                        );
                    }
                }
                (Some(Value::Array(a)), Some(Value::Array(b))) => {
                    for i in 0..a.len().max(b.len()) {
                        walk(a.get(i), b.get(i), &format!("{path}/{i}"), lines, total);
                    }
                }
                (None, Some(Value::Object(b))) if !b.is_empty() => {
                    for (key, value) in b {
                        walk(None, Some(value), &format!("{path}/{key}"), lines, total);
                    }
                }
                (Some(Value::Object(a)), None) if !a.is_empty() => {
                    for (key, value) in a {
                        walk(Some(value), None, &format!("{path}/{key}"), lines, total);
                    }
                }
                (None, Some(Value::Array(b))) if !b.is_empty() => {
                    for (index, value) in b.iter().enumerate() {
                        walk(None, Some(value), &format!("{path}/{index}"), lines, total);
                    }
                }
                (Some(Value::Array(a)), None) if !a.is_empty() => {
                    for (index, value) in a.iter().enumerate() {
                        walk(Some(value), None, &format!("{path}/{index}"), lines, total);
                    }
                }
                _ => {
                    *total += 1;
                    if lines.len() < 200 {
                        let display = |v: Option<&Value>| {
                            crate::history::bounded(
                                &v.map(ToString::to_string)
                                    .unwrap_or_else(|| "absent".into()),
                                512,
                            )
                        };
                        lines.push(format!("{path}: {} → {}", display(before), display(after)));
                    }
                }
            }
        }
        let before = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let after = serde_json::to_value(after).map_err(|e| e.to_string())?;
        let (mut lines, mut total) = (Vec::new(), 0);
        walk(Some(&before), Some(&after), "", &mut lines, &mut total);
        Ok((lines, total))
    }
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_JSON {
            return Err("Firewall JSON exceeds 1 MiB".into());
        }
        super::guard::check(text)?;
        let document: Self =
            serde_json::from_str(text).map_err(|e| format!("Invalid firewall JSON: {e}"))?;
        validate(&document.clone().into())?;
        Ok(document)
    }
    pub fn json(&self) -> Result<String, String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        if text.len() > MAX_JSON {
            return Err("Pretty firewall JSON exceeds 1 MiB".into());
        }
        Ok(text)
    }
    pub fn chain(&self, group: usize, chain: usize) -> Option<&Chain> {
        self.system_rules.get(group)?.chains.get(chain)
    }
    pub fn chain_mut(&mut self, group: usize, chain: usize) -> Result<&mut Chain, String> {
        self.system_rules
            .get_mut(group)
            .and_then(|g| g.chains.get_mut(chain))
            .ok_or_else(|| "Select a current chain".into())
    }
    /// Atomic draft mutation: failed validation leaves the document unchanged.
    pub fn edit(
        &mut self,
        edit: impl FnOnce(&mut Self) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut next = self.clone();
        edit(&mut next)?;
        validate(&next.clone().into())?;
        *self = next;
        Ok(())
    }
    pub fn move_rule(
        &mut self,
        group: usize,
        chain: usize,
        from: usize,
        to: usize,
    ) -> Result<(), String> {
        self.edit(|doc| {
            let rules = &mut doc.chain_mut(group, chain)?.rules;
            if from >= rules.len() || to >= rules.len() {
                return Err("Rule order changed; choose a current row".into());
            }
            if from == to {
                return Ok(());
            }
            let rule = rules.remove(from);
            rules.insert(to, rule);
            // The daemon reloads this array in reverse, inserting at the head.
            // Position is passed through to nftables, not an array row number.
            Ok(())
        })
    }
    pub fn apply_profile(&mut self, profile: Chain) -> Result<(), String> {
        self.edit(|doc| {
            let chain = doc
                .system_rules
                .iter_mut()
                .flat_map(|g| &mut g.chains)
                .find(|c| c.matches_profile(&profile))
                .ok_or("No reported chain matches this profile's table/family/type/hook")?;
            chain.policy = profile.policy;
            for rule in profile.rules {
                if !chain.rules.iter().any(|r| {
                    (!rule.uuid.is_empty() && r.uuid == rule.uuid) || r.same_content(&rule)
                }) {
                    if rule.enabled && rule.expressions.is_empty() {
                        return Err("An enabled profile rule needs at least one statement".into());
                    }
                    chain.rules.push(rule);
                }
            }
            Ok(())
        })
    }
    pub fn remove_profile(&mut self, profile: &Chain) -> Result<(), String> {
        if profile.rules.iter().any(|r| r.uuid.is_empty()) {
            return Err("Profile removal requires stable nonempty rule IDs".into());
        }
        self.edit(|doc| {
            let chain = doc
                .system_rules
                .iter_mut()
                .flat_map(|g| &mut g.chains)
                .find(|c| c.matches_profile(profile))
                .ok_or("No reported chain matches this profile's table/family/type/hook")?;
            // Removing a profile never invents a replacement policy. The chain
            // policy remains visible in the common full-document review.
            chain
                .rules
                .retain(|rule| !profile.rules.iter().any(|r| r.uuid == rule.uuid));
            Ok(())
        })
    }
}
impl Chain {
    fn matches_profile(&self, profile: &Self) -> bool {
        self.table.eq_ignore_ascii_case(&profile.table)
            && self.family.eq_ignore_ascii_case(&profile.family)
            && self.kind.eq_ignore_ascii_case(&profile.kind)
            && self.hook.eq_ignore_ascii_case(&profile.hook)
    }
}
impl Rule {
    fn same_content(&self, other: &Self) -> bool {
        // The upstream UI compares complete rule messages after clearing UUID.
        // Compare borrowed fields so large values aren't cloned per candidate.
        self.table == other.table
            && self.chain == other.chain
            && self.enabled == other.enabled
            && self.position == other.position
            && self.description == other.description
            && self.parameters == other.parameters
            && self.expressions == other.expressions
            && self.target == other.target
            && self.target_parameters == other.target_parameters
    }
}
