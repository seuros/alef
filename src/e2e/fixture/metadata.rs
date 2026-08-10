use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::core::config::e2e::ArgMapping;

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
    pub paths: BTreeMap<String, String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub presentation: Option<FixtureDocsPresentation>,
    #[serde(default)]
    pub side_effects: SideEffectClass,
    #[serde(default)]
    pub coverage_exceptions: BTreeMap<String, SnippetCoverageException>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureDocsPresentation {
    #[serde(default)]
    pub call: Option<String>,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub args: Option<Vec<ArgMapping>>,
    #[serde(default)]
    pub files: Vec<FixtureDocsFileInput>,
    #[serde(default)]
    pub operations: Vec<FixtureDocsOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixtureDocsFileInput {
    pub field: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FixtureDocsOperation {
    Show {
        path: String,
    },
    Iterate {
        path: String,
        item: String,
        #[serde(default)]
        fields: Vec<String>,
        #[serde(default)]
        display: bool,
        #[serde(default)]
        optional: bool,
    },
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
    use super::{FixtureDocs, FixtureDocsOperation, SideEffectClass};

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
    fn docs_paths_deserialize_as_target_specific_relative_paths() {
        let docs: FixtureDocs = serde_json::from_value(serde_json::json!({
            "topic": "fallback",
            "paths": {
                "node": "config/example.md",
                "wasm": "browser/example.md"
            }
        }))
        .expect("fixture docs deserialize");

        assert_eq!(docs.paths["node"], "config/example.md");
        assert_eq!(docs.paths["wasm"], "browser/example.md");
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

    #[test]
    fn structured_presentation_deserializes_without_language_code() {
        let docs: FixtureDocs = serde_json::from_value(serde_json::json!({
            "topic": "configuration",
            "presentation": {
                "call": "process_file",
                "input": {"source": "guide.txt"},
                "args": [{"name": "source", "field": "source", "type": "string"}],
                "files": [{"field": "/source", "path": "examples/guide.txt"}],
                "operations": [
                    {"op": "show", "path": "summary"},
                    {"op": "iterate", "path": "items", "item": "item", "fields": ["text"], "optional": true}
                ]
            }
        }))
        .expect("fixture docs presentation deserialize");

        let presentation = docs.presentation.expect("presentation");
        assert_eq!(presentation.input, Some(serde_json::json!({"source": "guide.txt"})));
        assert_eq!(presentation.call.as_deref(), Some("process_file"));
        assert_eq!(presentation.files[0].field, "/source");
        assert!(matches!(presentation.operations[0], FixtureDocsOperation::Show { .. }));
        assert!(matches!(
            presentation.operations[1],
            FixtureDocsOperation::Iterate { optional: true, .. }
        ));
    }
}
