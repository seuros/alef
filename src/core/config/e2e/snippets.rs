use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SnippetConfig {
    pub output: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub capabilities: SnippetCapabilities,
}

impl SnippetConfig {
    pub fn languages_or<'a>(&'a self, fallback: &'a [String]) -> &'a [String] {
        if self.languages.is_empty() {
            fallback
        } else {
            &self.languages
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_snippet_languages_override_e2e_targets() {
        let fallback = vec!["python".to_string(), "java".to_string()];
        let mut config = SnippetConfig {
            output: "docs/snippets-generated".into(),
            ..SnippetConfig::default()
        };
        assert_eq!(config.languages_or(&fallback), fallback);

        config.languages = vec!["python".into()];
        assert_eq!(config.languages_or(&fallback), ["python"]);
    }
}
