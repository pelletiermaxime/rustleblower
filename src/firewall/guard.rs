//! Count containers before allocating typed JSON vectors, including empty messages.
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::fmt;

#[derive(Clone, Copy)]
enum Array {
    Groups,
    Chains,
    Rules,
    Expressions,
    Values,
}
#[derive(Default)]
struct Budget {
    groups: usize,
    chains: usize,
    rules: usize,
    expressions: usize,
    values: usize,
}
impl Budget {
    fn count<E: de::Error>(&mut self, array: Array) -> Result<(), E> {
        let (count, max) = match array {
            Array::Groups => (&mut self.groups, 256),
            Array::Chains => (&mut self.chains, super::MAX_CHAINS),
            Array::Rules => (&mut self.rules, super::MAX_RULES),
            Array::Expressions => (&mut self.expressions, super::MAX_EXPRESSIONS),
            Array::Values => (&mut self.values, super::MAX_VALUES),
        };
        *count += 1;
        if *count > max {
            return Err(E::custom("Firewall JSON exceeds its structural limits"));
        }
        Ok(())
    }
}
struct Seed<'a> {
    budget: &'a mut Budget,
    item: Option<Array>,
    array: Option<Array>,
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        if let Some(item) = self.item {
            self.budget.count::<D::Error>(item)?;
        }
        de.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded firewall JSON")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<String>()? {
            let array = match key.as_str() {
                "SystemRules" => Some(Array::Groups),
                "Chains" => Some(Array::Chains),
                "Rules" => Some(Array::Rules),
                "Expressions" => Some(Array::Expressions),
                "Values" => Some(Array::Values),
                "Enabled" | "Version" | "Rule" | "Name" | "Table" | "Family" | "Priority"
                | "Type" | "Hook" | "Policy" | "Chain" | "UUID" | "Position" | "Description"
                | "Parameters" | "Target" | "TargetParameters" | "Statement" | "Op" | "Key"
                | "Value" => None,
                _ => {
                    return Err(de::Error::custom(
                        "Unknown firewall field; it cannot be preserved by this wire format",
                    ));
                }
            };
            map.next_value_seed(Seed {
                budget: self.budget,
                item: None,
                array,
            })?;
        }
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let item = self
            .array
            .ok_or_else(|| de::Error::custom("Unexpected firewall array"))?;
        while seq
            .next_element_seed(Seed {
                budget: self.budget,
                item: Some(item),
                array: None,
            })?
            .is_some()
        {}
        Ok(())
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}
pub(super) fn check(text: &str) -> Result<(), String> {
    let mut budget = Budget::default();
    let mut de = serde_json::Deserializer::from_str(text);
    Seed {
        budget: &mut budget,
        item: None,
        array: None,
    }
    .deserialize(&mut de)
    .map_err(|e| e.to_string())?;
    de.end().map_err(|e| e.to_string())
}
