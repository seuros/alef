//! Canonical registry of fixture assertion types, and the generation-time gate
//! that rejects any type the target backend cannot render.
//!
//! Before this gate existed an assertion type a backend did not recognise was
//! handled three incompatible ways: the `java`, `php` and `zig` JSON-result
//! templates end their `{% elif %}` chain with no `{% else %}`, so the assertion
//! rendered to the empty string and the generated test passed while checking
//! nothing; `dart` wrote an unregistered `// skipped:` comment; the remaining
//! backends panicked. Only the last is a real failure, and none of the three
//! names the fixture the bad assertion came from. ~keep

use anyhow::{Result, bail};

use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::FixtureGroup;

/// Every assertion `type` alef's fixture schema accepts.
///
/// This is the Rust mirror of the `type` enum in `src/e2e/schema/fixture.schema.json`;
/// [`tests::known_assertion_types_mirror_the_fixture_schema`] fails if the two drift. It
/// is also the default backend support set, so a backend added later inherits the gate at
/// full strength instead of being silently exempt from it. ~keep
pub const KNOWN_ASSERTION_TYPES: &[&str] = &[
    "contains",
    "contains_all",
    "contains_any",
    "count_equals",
    "count_min",
    "ends_with",
    "equals",
    "error",
    "greater_than",
    "greater_than_or_equal",
    "is_empty",
    "is_false",
    "is_true",
    "less_than",
    "less_than_or_equal",
    "matches_regex",
    "max_length",
    "method_result",
    "min_length",
    "not_contains",
    "not_empty",
    "not_equals",
    "not_error",
    "starts_with",
];

/// Schema-legal assertion types a specific backend has no dispatch arm for, keyed by
/// [`super::E2eCodegen::language_name`].
///
/// Each entry records a gap that already failed before this table existed — as a panic
/// (`rust`, `brew`) or as an empty render (everything else on `not_equals`) — so listing
/// it here only changes *how* generation fails, never *whether* it does. A key that
/// matches no generator would silently disable its own row, so
/// [`tests::every_unsupported_key_names_a_real_generator`] pins the keys to
/// [`super::all_generators`]. ~keep
pub const BACKEND_UNSUPPORTED_ASSERTION_TYPES: &[(&str, &[&str])] = &[
    // ~keep `not_equals` is in the fixture schema but only the Dart backend renders it
    // (`dart/assertions.rs:264`); no other backend has an arm for it at all.
    ("brew", &["contains_any", "not_equals", "starts_with"]),
    ("c", &["not_equals"]),
    ("csharp", &["not_equals"]),
    ("elixir", &["not_equals"]),
    ("gleam", &["not_equals"]),
    ("go", &["not_equals"]),
    ("homebrew", &["not_equals"]),
    ("java", &["not_equals"]),
    ("kotlin", &["not_equals"]),
    ("kotlin_android", &["not_equals"]),
    ("node", &["not_equals"]),
    ("php", &["not_equals"]),
    ("php_ext", &["not_equals"]),
    ("python", &["not_equals"]),
    ("r", &["not_equals"]),
    ("ruby", &["not_equals"]),
    ("rust", &["matches_regex", "not_equals"]),
    ("swift", &["not_equals"]),
    ("wasm", &["not_equals"]),
    ("zig", &["not_equals"]),
];

/// The assertion types `language`'s backend can render.
pub fn supported_assertion_types(language: &str) -> Vec<&'static str> {
    let unsupported = BACKEND_UNSUPPORTED_ASSERTION_TYPES
        .iter()
        .find(|(name, _)| *name == language)
        .map(|(_, types)| *types)
        .unwrap_or(&[]);
    KNOWN_ASSERTION_TYPES
        .iter()
        .copied()
        .filter(|known| !unsupported.contains(known))
        .collect()
}

/// Fail generation when any fixture `language` will actually emit declares an assertion
/// type that backend cannot render.
///
/// Fixtures the backend would not emit at all are skipped, so `skip.languages` stays the
/// documented escape hatch for a fixture that exercises a type one backend lacks. ~keep
pub fn ensure_supported_assertion_types(
    groups: &[FixtureGroup],
    e2e_config: &E2eConfig,
    language: &str,
    supported: &[&str],
) -> Result<()> {
    let mut rejected: Vec<String> = Vec::new();
    for group in groups {
        for fixture in &group.fixtures {
            if !super::fixture_inclusion(fixture, language, e2e_config).is_included() {
                continue;
            }
            for assertion in &fixture.assertions {
                let assertion_type = assertion.assertion_type.as_str();
                if supported.contains(&assertion_type) {
                    continue;
                }
                let reason = if KNOWN_ASSERTION_TYPES.contains(&assertion_type) {
                    "not rendered by this backend"
                } else {
                    "not a known assertion type"
                };
                rejected.push(format!(
                    "fixture '{}' ({}) declares assertion type '{}' -- {reason}",
                    fixture.id, fixture.source, assertion_type
                ));
            }
        }
    }
    if rejected.is_empty() {
        return Ok(());
    }
    rejected.sort();
    rejected.dedup();
    bail!(
        "e2e backend '{language}' cannot render {} assertion(s): {}. Supported types for '{language}': {}",
        rejected.len(),
        rejected.join("; "),
        supported.join(", ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_assertion_types() -> Vec<String> {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schema/fixture.schema.json")).expect("fixture schema parses as JSON");
        let values = schema["$defs"]["assertion"]["properties"]["type"]["enum"]
            .as_array()
            .expect("assertion `type` declares an enum");
        let mut types: Vec<String> = values
            .iter()
            .map(|value| value.as_str().expect("enum entries are strings").to_string())
            .collect();
        types.sort();
        types
    }

    #[test]
    fn known_assertion_types_mirror_the_fixture_schema() {
        let mut known: Vec<String> = KNOWN_ASSERTION_TYPES.iter().map(|t| (*t).to_string()).collect();
        known.sort();
        assert_eq!(
            known,
            schema_assertion_types(),
            "KNOWN_ASSERTION_TYPES and the `type` enum in src/e2e/schema/fixture.schema.json must agree"
        );
    }

    #[test]
    fn known_assertion_types_are_sorted_and_unique() {
        let mut sorted: Vec<&str> = KNOWN_ASSERTION_TYPES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted,
            KNOWN_ASSERTION_TYPES.to_vec(),
            "keep the list sorted and duplicate-free"
        );
    }

    #[test]
    fn every_unsupported_key_names_a_real_generator() {
        let languages: Vec<&str> = super::super::all_generators()
            .iter()
            .map(|generator| generator.language_name())
            .collect();
        for (language, types) in BACKEND_UNSUPPORTED_ASSERTION_TYPES {
            assert!(
                languages.contains(language),
                "'{language}' is not a generator language name, so its row would never apply"
            );
            for assertion_type in *types {
                assert!(
                    KNOWN_ASSERTION_TYPES.contains(assertion_type),
                    "'{language}' excludes '{assertion_type}', which is not a known assertion type"
                );
            }
        }
    }

    #[test]
    fn not_equals_is_supported_by_dart_alone() {
        let dart_only: Vec<&str> = super::super::all_generators()
            .iter()
            .map(|generator| generator.language_name())
            .filter(|language| supported_assertion_types(language).contains(&"not_equals"))
            .collect();
        assert_eq!(
            dart_only,
            vec!["dart"],
            "only the Dart backend has a `not_equals` arm; a new supporter must add one"
        );
    }
}
