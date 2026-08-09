use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateReturnForm {
    #[default]
    Dict,
    BareString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureEnv {
    #[serde(default)]
    pub api_key_var: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupCall {
    pub call: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureDocs {
    pub topic: String,
    #[serde(default)]
    pub stem: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub side_effects: SideEffectClass,
    #[serde(default)]
    pub coverage_exceptions: BTreeMap<String, SnippetCoverageException>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnippetCoverageException {
    pub reason: String,
    pub documentation: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SideEffectClass {
    #[default]
    #[serde(alias = "none", alias = "local")]
    Safe,
    Network,
    Process,
    Install,
    Server,
}

#[cfg(test)]
mod tests {
    use super::SideEffectClass;

    #[test]
    fn side_effects_round_trip_without_collapsing_classes() {
        for class in [
            SideEffectClass::Safe,
            SideEffectClass::Network,
            SideEffectClass::Process,
            SideEffectClass::Install,
            SideEffectClass::Server,
        ] {
            let encoded = serde_json::to_string(&class).unwrap();
            assert_eq!(serde_json::from_str::<SideEffectClass>(&encoded).unwrap(), class);
        }
    }

    #[test]
    fn legacy_safe_aliases_remain_accepted_but_external_mutation_is_rejected() {
        assert_eq!(
            serde_json::from_str::<SideEffectClass>(r#""none""#).unwrap(),
            SideEffectClass::Safe
        );
        assert_eq!(
            serde_json::from_str::<SideEffectClass>(r#""local""#).unwrap(),
            SideEffectClass::Safe
        );
        assert!(serde_json::from_str::<SideEffectClass>(r#""external_mutation""#).is_err());
    }
}
