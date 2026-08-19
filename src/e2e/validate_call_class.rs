//! Validates `[e2e.call(s).*.overrides.<lang>] class` against the classes a backend
//! actually emits.
//!
//! `class` names the host-language type generated tests and snippets call methods on
//! (a crate facade, a struct/enum wrapper, or a trait bridge). Nothing checked this
//! value against the backend's own naming before it reached the emitter, so a typo or
//! a stale rename silently produced calls against a class that does not exist —
//! surfacing only as a wall of compile errors in generated code, never at config time.
//! See `crate::e2e::validate` for the sibling checks this mirrors (unknown call
//! references, field-classification-vs-IR mismatches).

use super::validate::{Severity, ValidationError};
use crate::codegen::naming::{self, PublicIdentifierKind};
use crate::core::config::e2e::{CallConfig, E2eConfig};
use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{EnumDef, TypeDef};

const CONFIG_FILE_LABEL: &str = "alef.toml";

/// Languages whose e2e generators read `CallOverride::class`. Kept as an explicit list
/// rather than derived from `Language::ALL` because most languages (python, node, go,
/// rust, csharp, ...) never consult this field — see the backend generators under
/// `src/e2e/codegen/`.
const CLASS_CONSUMING_LANGUAGES: &[(&str, Language)] = &[
    ("java", Language::Java),
    ("kotlin", Language::Kotlin),
    ("kotlin_android", Language::KotlinAndroid),
    ("php", Language::Php),
    ("ruby", Language::Ruby),
    ("dart", Language::Dart),
];

fn naming_language_for(lang: &str) -> Option<Language> {
    CLASS_CONSUMING_LANGUAGES
        .iter()
        .find(|(name, _)| *name == lang)
        .map(|(_, language)| *language)
}

/// Validate every `class` override in `e2e_config` (the default `[e2e.call]` and every
/// named `[e2e.calls.*]`) against the set of host-language class names the target
/// backend will actually emit for this crate.
///
/// Skipped entirely when both `type_defs` and `enums` are empty: several legitimate
/// callers (unit tests, snippet entry points, generation paths that fall back to
/// explicit call-override mappings — see `crate::e2e::generate_e2e`'s doc comment) pass
/// an empty IR, and validating against a deliberately incomplete candidate set would
/// manufacture false positives rather than catch a real typo. Mirrors the same
/// "absent IR licenses no claim" rule `validate::validate_field_classifications` uses.
pub fn validate_call_class_overrides(
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    languages: &[String],
) -> Vec<ValidationError> {
    if type_defs.is_empty() && enums.is_empty() {
        return Vec::new();
    }

    let mut errors = Vec::new();
    let mut sources: Vec<(String, &CallConfig)> = vec![("[e2e.call]".to_string(), &e2e_config.call)];
    let mut named_calls: Vec<(&String, &CallConfig)> = e2e_config.calls.iter().collect();
    named_calls.sort_by_key(|(name, _)| (*name).clone());
    for (name, call) in named_calls {
        sources.push((format!("[e2e.calls.{name}]"), call));
    }

    for (config_key, call) in sources {
        let mut override_langs: Vec<&String> = call.overrides.keys().collect();
        override_langs.sort();
        for lang in override_langs {
            if !languages.iter().any(|resolved| resolved == lang) {
                continue;
            }
            let Some(naming_lang) = naming_language_for(lang) else {
                continue;
            };
            let Some(class_value) = call.overrides.get(lang).and_then(|o| o.class.as_ref()) else {
                continue;
            };
            check_class_override(
                &config_key,
                lang,
                naming_lang,
                class_value,
                config,
                type_defs,
                enums,
                &mut errors,
            );
        }
    }
    errors
}

#[allow(clippy::too_many_arguments)]
fn check_class_override(
    config_key: &str,
    lang: &str,
    naming_lang: Language,
    class_value: &str,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    errors: &mut Vec<ValidationError>,
) {
    let candidates = emitted_class_names(lang, naming_lang, config, type_defs, enums);
    let simple_name = simple_class_name(class_value);
    if candidates.iter().any(|candidate| candidate == simple_name) {
        return;
    }

    let suggestion = closest_candidates(simple_name, &candidates);
    let suggestion_text = if suggestion.is_empty() {
        String::new()
    } else {
        format!(" (did you mean {}?)", suggestion.join(" or "))
    };
    errors.push(ValidationError {
        file: CONFIG_FILE_LABEL.to_string(),
        message: format!(
            "{config_key}.overrides.{lang}.class = \"{class_value}\" does not match any class the {lang} backend \
             emits for crate '{}'{suggestion_text}",
            config.name
        ),
        severity: Severity::Error,
    });
}

/// The host-language class names the `lang` backend actually emits for this crate: the
/// crate facade, every struct/enum wrapper, and every active trait bridge.
fn emitted_class_names(
    lang: &str,
    naming_lang: Language,
    config: &ResolvedCrateConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Vec<String> {
    let mut names = vec![naming::to_class_name(&config.name)];
    for type_def in type_defs {
        names.push(naming::public_host_identifier(
            naming_lang,
            PublicIdentifierKind::Type,
            &type_def.name,
        ));
    }
    for enum_def in enums {
        names.push(naming::public_host_identifier(
            naming_lang,
            PublicIdentifierKind::Type,
            &enum_def.name,
        ));
    }
    for bridge in &config.trait_bridges {
        if bridge.is_active_for(lang) {
            names.push(format!("{}Bridge", bridge.trait_name));
        }
    }
    names.sort();
    names.dedup();
    names
}

/// The trailing class name of a possibly-qualified override value. Java/Kotlin/Dart use
/// `.`-separated packages, Ruby uses `::`-nested modules, PHP uses `\`-namespaces —
/// candidates are always bare names, so a qualified override is compared on its last
/// segment, mirroring how `src/e2e/codegen/java/snippet.rs` resolves an FQN override
/// down to a simple name for import handling.
fn simple_class_name(raw: &str) -> &str {
    let after_dot = raw.rsplit('.').next().unwrap_or(raw);
    let after_namespace = after_dot.rsplit("::").next().unwrap_or(after_dot);
    after_namespace.rsplit('\\').next().unwrap_or(after_namespace)
}

/// Up to two candidates closest to `value` by Levenshtein edit distance, ascending.
fn closest_candidates(value: &str, candidates: &[String]) -> Vec<String> {
    let mut scored: Vec<(usize, &String)> = candidates
        .iter()
        .map(|candidate| (levenshtein_distance(value, candidate), candidate))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(2)
        .map(|(_, candidate)| format!("\"{candidate}\""))
        .collect()
}

/// Classic Wagner-Fischer edit distance. No external dependency, small inputs (class
/// names), so the O(n*m) table is negligible.
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, &character_a) in a.iter().enumerate() {
        let mut previous_diagonal = row[0];
        row[0] = i + 1;
        for (j, &character_b) in b.iter().enumerate() {
            let above = row[j + 1];
            let cost = usize::from(character_a != character_b);
            let substitution = previous_diagonal + cost;
            let insertion = row[j] + 1;
            let deletion = above + 1;
            previous_diagonal = above;
            row[j + 1] = substitution.min(insertion).min(deletion);
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::CallOverride;
    use crate::core::config::trait_bridge::TraitBridgeConfig;
    use crate::core::ir::FieldDef;

    fn make_config(crate_name: &str) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: crate_name.to_string(),
            ..ResolvedCrateConfig::default()
        }
    }

    fn make_type(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            fields: vec![FieldDef::default()],
            ..TypeDef::default()
        }
    }

    fn make_e2e_config(class: &str, lang: &str) -> E2eConfig {
        let mut call = CallConfig::default();
        let override_config = CallOverride {
            class: Some(class.to_string()),
            ..CallOverride::default()
        };
        call.overrides.insert(lang.to_string(), override_config);
        E2eConfig {
            call,
            ..E2eConfig::default()
        }
    }

    #[test]
    fn a_class_override_matching_an_emitted_struct_passes() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let e2e_config = make_e2e_config("DocumentApi", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn a_misspelled_class_override_fails_with_the_offending_language_and_value() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let e2e_config = make_e2e_config("DocumentAppi", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Error);
        assert_eq!(errors[0].file, "alef.toml");
        assert!(
            errors[0]
                .message
                .contains("[e2e.call].overrides.java.class = \"DocumentAppi\""),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0]
                .message
                .contains("java backend emits for crate 'sample_crate'"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn the_suggestion_names_the_closest_candidate() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let e2e_config = make_e2e_config("DocumentAppi", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0].message.contains("did you mean \"DocumentApi\""),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn an_absent_override_is_a_no_op() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let e2e_config = E2eConfig::default();

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn a_fully_qualified_class_override_is_compared_on_its_simple_name() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("SampleService")];
        let e2e_config = make_e2e_config("dev.example.SampleService", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn a_crate_facade_override_passes_with_no_ir_types() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("SampleCrate", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn an_active_trait_bridge_class_override_passes() {
        let mut config = make_config("sample_crate");
        config.trait_bridges = vec![TraitBridgeConfig {
            trait_name: "Validator".to_string(),
            ..TraitBridgeConfig::default()
        }];
        let type_defs = vec![make_type("Placeholder")];
        let e2e_config = make_e2e_config("ValidatorBridge", "kotlin_android");

        let errors =
            validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["kotlin_android".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn an_empty_ir_skips_validation_entirely() {
        let config = make_config("sample_crate");
        let e2e_config = make_e2e_config("TotallyWrongClassName", "java");

        let errors = validate_call_class_overrides(&e2e_config, &config, &[], &[], &["java".to_string()]);

        assert_eq!(errors.len(), 0, "an absent IR must license no claim: {errors:?}");
    }

    #[test]
    fn a_language_that_does_not_consume_class_is_never_checked() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let e2e_config = make_e2e_config("TotallyWrongClassName", "python");

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["python".to_string()]);

        assert_eq!(errors.len(), 0, "python does not consume `class`: {errors:?}");
    }

    #[test]
    fn a_named_call_override_names_its_own_config_key() {
        let config = make_config("sample_crate");
        let type_defs = vec![make_type("DocumentApi")];
        let mut e2e_config = E2eConfig::default();
        let mut call = CallConfig::default();
        let override_config = CallOverride {
            class: Some("NotARealClass".to_string()),
            ..CallOverride::default()
        };
        call.overrides.insert("java".to_string(), override_config);
        e2e_config.calls.insert("summarize".to_string(), call);

        let errors = validate_call_class_overrides(&e2e_config, &config, &type_defs, &[], &["java".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0]
                .message
                .starts_with("[e2e.calls.summarize].overrides.java.class"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn levenshtein_distance_matches_known_values() {
        assert_eq!(levenshtein_distance("DocumentApi", "DocumentApi"), 0);
        assert_eq!(levenshtein_distance("DocumentAppi", "DocumentApi"), 1);
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    }
}
