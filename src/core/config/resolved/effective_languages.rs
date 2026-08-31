//! The set of languages a crate's coordinates are actually consumed by.

use super::ResolvedCrateConfig;
use crate::core::config::extras::Language;

impl ResolvedCrateConfig {
    /// Binding languages plus every language selected by full-suite or snippet e2e generation.
    ///
    /// ~keep Coordinate validation must key off this rather than `languages`: an e2e or snippet
    /// target can introduce a language that is absent from the binding list, and that target
    /// still consumes the crate's package coordinates. Validating only `languages` would leave
    /// those coordinates unchecked.
    #[must_use]
    pub(crate) fn effective_languages(&self) -> Vec<Language> {
        let mut languages = self.languages.clone();
        let Some(e2e) = self.e2e.as_ref() else {
            return languages;
        };

        let mut target_names = if e2e.languages.is_empty() {
            crate::e2e::default_e2e_languages(&self.languages)
        } else {
            e2e.languages.clone()
        };
        if let Some(snippets) = &e2e.snippets {
            target_names.extend(snippets.languages.iter().cloned());
        }

        for target_name in target_names {
            let Some(language) = Language::ALL
                .into_iter()
                .find(|language| language.to_string() == target_name)
            else {
                continue;
            };
            if !languages.contains(&language) {
                languages.push(language);
            }
        }
        languages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::NewAlefConfig;

    #[test]
    fn effective_languages_deduplicate_binding_e2e_and_snippet_selections() {
        let config: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["python", "dart"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.e2e]
languages = ["dart", "swift"]
[crates.e2e.call]
function = "test"
[crates.e2e.snippets]
output = "docs/snippets-generated"
languages = ["dart", "swift"]
"#,
        )
        .unwrap();
        let resolved = config.resolve().expect("valid coordinates must resolve");
        assert_eq!(
            resolved[0].effective_languages(),
            vec![Language::Python, Language::Dart, Language::Swift]
        );
    }
}
