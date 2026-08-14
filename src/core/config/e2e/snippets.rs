use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
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

    /// Regression test for the misplaced-key defect: a consumer that writes
    /// `[e2e].fields_optional` / `fields_array` / `fields_enum` / `result_fields` /
    /// `fields_method_calls` one level too deep, under `[crates.e2e.snippets]`,
    /// used to have all five keys silently discarded by serde because
    /// `SnippetConfig` had no `deny_unknown_fields`. `FieldResolver::is_optional()`
    /// (and friends) then returned `false`/empty unconditionally for every field,
    /// in every fixture, in every language — with no error anywhere.
    ///
    /// Without `#[serde(deny_unknown_fields)]` on `SnippetConfig` this test fails:
    /// `toml::from_str` returns `Ok(..)` and the misplaced keys vanish silently.
    #[test]
    fn misplaced_e2e_fields_under_snippets_table_is_rejected_not_silently_dropped() {
        // Carry exactly one misplaced key. toml reports only the first unknown
        // field it reaches, so a fixture with several would pin the assertion to
        // whichever the deserializer happens to visit first.
        let toml_str = r#"
            output = "docs/snippets-generated"
            fields_optional = ["metadata", "content"]
        "#;
        let err = toml::from_str::<SnippetConfig>(toml_str).expect_err(
            "a SnippetConfig table carrying e2e-level field-classification keys must be \
             rejected, not silently accepted with those keys discarded",
        );
        let message = err.to_string();
        assert!(
            message.contains("fields_optional"),
            "error must name the offending key so the misplacement is discoverable: {message}"
        );
        assert!(
            message.contains("unknown field"),
            "error must be serde's unknown-field diagnostic, not an unrelated parse failure: {message}"
        );
    }

    /// Companion happy-path check: a `SnippetConfig` table containing only its
    /// real fields (`output`, `languages`, `capabilities`) still deserializes
    /// cleanly under `deny_unknown_fields` — the fix must not reject legitimate
    /// configs.
    #[test]
    fn well_formed_snippet_config_still_deserializes_under_deny_unknown_fields() {
        let toml_str = r#"
            output = "docs/snippets-generated"
            languages = ["python", "java"]

            [capabilities]
            all = ["run"]
            python = ["compile"]
        "#;
        let config: SnippetConfig = toml::from_str(toml_str).expect("well-formed config must still parse");
        assert_eq!(config.output, "docs/snippets-generated");
        assert_eq!(config.languages, vec!["python", "java"]);
        assert_eq!(config.capabilities.for_language("python"), {
            let mut expected = BTreeSet::new();
            expected.insert("run".to_string());
            expected.insert("compile".to_string());
            expected
        });
    }
}
