use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SnippetConfig {
    pub output: String,
    #[serde(default)]
    pub capabilities: SnippetCapabilities,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SnippetCapabilities {
    #[serde(default)]
    pub all: BTreeSet<String>,
    #[serde(flatten)]
    pub languages: BTreeMap<String, BTreeSet<String>>,
}

impl SnippetCapabilities {
    pub fn for_language(&self, language: &str) -> BTreeSet<String> {
        let mut values = self.all.clone();
        if let Some(language_values) = self.languages.get(language) {
            values.extend(language_values.iter().cloned());
        }
        values
    }
}
