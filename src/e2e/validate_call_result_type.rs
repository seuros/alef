//! Validates `[e2e.call(s).*.overrides.<lang>] result_type` against the struct/enum type
//! names the core IR actually declares for this crate.
//!
//! `result_type` names the IR type a call's result binds to. Unlike `class` (a
//! host-language identifier, produced through `src/codegen/naming.rs`), `result_type` is
//! a language-agnostic IR type name: `src/e2e/codegen/c.rs::resolve_call_info` reads its
//! own override directly and bakes the value into literal C accessor/free symbols;
//! `src/e2e/codegen/kotlin/test_method.rs` reads `kotlin`/`kotlin_android`'s own override
//! to auto-detect enum-typed result fields; and `src/e2e/codegen/dart.rs`,
//! `src/e2e/codegen/swift/values.rs`, and `src/e2e/codegen/php/types.rs` all fall back
//! through the `c`, `csharp`, `java`, `kotlin`, `go`, `php` overrides looking for the
//! first non-empty value, because (per `swift/values.rs`'s own doc comment) "these
//! overrides are language-agnostic IR type names ... shared across every binding."
//!
//! Nothing checked this value against the IR before it reached the emitter, so a typo
//! was trusted blindly: for the `c` generator specifically it either silently disabled
//! nested-field verification (see `warn_if_result_type_override_disables_verification`)
//! or, when the call also failed to resolve any other way, surfaced late as a wall of
//! "call did not resolve to a core IR function..." warnings/errors naming `result_type`
//! as the fix (`unresolved_result_type_name`, `ResultTypeName::require`) -- the exact
//! failure this validator now catches at config time instead.
//!
//! See `crate::e2e::validate_call_class` for the sibling check this mirrors, and its doc
//! comment for why an empty IR licenses no claim either way.

use super::validate::{Severity, ValidationError};
use super::validate_call_class::closest_candidates;
use crate::core::config::e2e::{CallConfig, E2eConfig};
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::codegen::c::is_primitive_c_type;

const CONFIG_FILE_LABEL: &str = "alef.toml";

/// Languages whose e2e generators read `CallOverride::result_type`, directly or by the
/// cross-language fallback convention documented on this module. Kept as an explicit
/// list for the same reason `validate_call_class::CLASS_CONSUMING_LANGUAGES` is: most
/// languages never consult this field.
const RESULT_TYPE_CONSUMING_LANGUAGES: &[&str] = &["c", "csharp", "java", "kotlin", "kotlin_android", "go", "php"];

/// Languages that read `result_type` from ANOTHER language's override section rather
/// than only their own -- `dart_call_result_type`, `swift_call_result_type`, and
/// `derive_root_type` all walk the `c`/`csharp`/`java`/`kotlin`/`go`/`php` overrides
/// looking for the first non-empty value. An override set under any of
/// `RESULT_TYPE_CONSUMING_LANGUAGES` is therefore live whenever one of these three is
/// being generated, even when the language owning the override section is not.
const CROSS_READING_LANGUAGES: &[&str] = &["dart", "swift", "php"];

/// True when a `result_type` override authored under `lang` will actually be read by
/// some active generator, given the resolved language set for this run.
fn result_type_override_is_active(lang: &str, languages: &[String]) -> bool {
    if !RESULT_TYPE_CONSUMING_LANGUAGES.contains(&lang) {
        return false;
    }
    languages.iter().any(|resolved| resolved == lang)
        || languages
            .iter()
            .any(|resolved| CROSS_READING_LANGUAGES.contains(&resolved.as_str()))
}

/// Run [`validate_call_result_type_overrides`], log every diagnostic (warnings included,
/// matching `validate_call_class`'s sibling wiring in `crate::e2e::generate_e2e`), and turn
/// any `Severity::Error` into a generation-aborting error naming every offending config key.
///
/// Kept here rather than inlined at the `generate_e2e` call site so that call site stays a
/// single line -- `src/e2e/mod.rs` sits right at this repo's 1,000-line file cap, and the
/// log-filter-bail glue is boilerplate this module owns anyway.
pub fn enforce_call_result_type_overrides(
    e2e_config: &E2eConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    languages: &[String],
) -> anyhow::Result<()> {
    let diagnostics = validate_call_result_type_overrides(e2e_config, type_defs, enums, languages);
    for diag in &diagnostics {
        tracing::warn!("{}: {}", diag.file, diag.message);
    }
    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|diag| diag.severity == Severity::Error)
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "e2e call result_type override validation failed: {}",
        errors
            .iter()
            .map(|diag| format!("{}: {}", diag.file, diag.message))
            .collect::<Vec<_>>()
            .join("; ")
    );
}

/// Validate every `result_type` override in `e2e_config` (the default `[e2e.call]` and
/// every named `[e2e.calls.*]`) against the struct/enum type names the core IR declares
/// for this crate.
///
/// Skipped entirely when both `type_defs` and `enums` are empty, mirroring
/// `validate_call_class::validate_call_class_overrides`: several legitimate callers pass
/// an empty IR (see `crate::e2e::generate_e2e`'s doc comment), and validating against a
/// deliberately incomplete candidate set would manufacture false positives.
pub fn validate_call_result_type_overrides(
    e2e_config: &E2eConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    languages: &[String],
) -> Vec<ValidationError> {
    if type_defs.is_empty() && enums.is_empty() {
        return Vec::new();
    }

    let candidates = emitted_result_type_names(type_defs, enums);

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
            if !result_type_override_is_active(lang, languages) {
                continue;
            }
            let Some(override_config) = call.overrides.get(lang) else {
                continue;
            };
            let Some(result_type_value) = override_config.result_type.as_ref() else {
                continue;
            };
            check_result_type_override(
                &config_key,
                lang,
                result_type_value,
                &candidates,
                override_config.raw_c_result_type.as_deref(),
                &mut errors,
            );
        }
    }
    errors
}

/// The struct/enum type names the core IR declares for this crate -- the same set
/// `result_type` is documented to name (`CallOverride::result_type`'s doc: "The
/// PascalCase name of the result struct, without the prefix").
fn emitted_result_type_names(type_defs: &[TypeDef], enums: &[EnumDef]) -> Vec<String> {
    let mut names: Vec<String> = type_defs.iter().map(|type_def| type_def.name.clone()).collect();
    names.extend(enums.iter().map(|enum_def| enum_def.name.clone()));
    names.sort();
    names.dedup();
    names
}

/// A C scalar/pointer spelling (`char*`, `int32_t`, `uintptr_t`, ...) typed into
/// `result_type` where an IR struct/enum name belongs -- the same predicate
/// `src/e2e/codegen/c.rs::warn_if_result_type_override_disables_verification` uses,
/// reusing its primitive-type list (`is_primitive_c_type`) rather than duplicating it so
/// the two classifications cannot drift apart.
fn is_primitive_or_pointer_c_spelling(value: &str) -> bool {
    is_primitive_c_type(value) || value.ends_with('*')
}

fn check_result_type_override(
    config_key: &str,
    lang: &str,
    result_type_value: &str,
    candidates: &[String],
    raw_c_result_type: Option<&str>,
    errors: &mut Vec<ValidationError>,
) {
    if candidates.iter().any(|candidate| candidate == result_type_value) {
        return;
    }

    // A primitive/pointer C spelling is a DIFFERENT problem from an unknown type name: the
    // author reached for `result_type` when `raw_c_result_type` / `result_is_bytes` /
    // `result_is_simple` was the right field, not a typo of a real IR type. Reporting it as
    // "does not match any type" would send the fix in the wrong direction, so it gets its own
    // message and, matching `c.rs`'s own `tracing::warn!` severity for the same condition, stays
    // a warning rather than a hard error: the call may still render correctly when
    // `raw_c_result_type` is also set, unlike a genuinely unresolvable type name.
    if is_primitive_or_pointer_c_spelling(result_type_value) {
        // Only `raw_c_result_type` on the SAME `c` override actually changes what the C
        // generator does with a primitive/pointer `result_type`: `resolve_call_info` in
        // `src/e2e/codegen/c.rs` reads `raw_c_result_type` and, when it is set, takes the
        // dedicated "Raw C result type path" in `src/e2e/codegen/c/test_function.rs` --
        // which returns before ever reaching the legacy path that resolves
        // `result_type_name` against the IR (`result_type_name.require()` /
        // `ensure_leaf_field_exists`). So once `raw_c_result_type` is set, the primitive
        // `result_type` spelling is never dereferenced as an IR type name at all.
        //
        // `result_is_bytes` / `result_is_simple` do NOT license this the same way: the C
        // generator only ever reads them from `call_declares_non_struct_result`, which is
        // reached solely through `unresolved_result_type_name` -- the path taken when NO
        // `result_type` override is set. An explicit `result_type` override short-circuits
        // `resolve_call_info`'s `.or_else()` chain before either flag is ever consulted, so
        // setting them alongside a primitive `result_type` changes nothing about how that
        // primitive spelling is used.
        //
        // `raw_c_result_type` is also only ever read from the `c` override section itself
        // (`resolve_call_info` is always called with `lang == "c"`), so it licenses the
        // override only when it is set on that same `c` override, not on some other
        // language's override section that happens to also carry a `result_type` value.
        if lang == "c" && raw_c_result_type.is_some() {
            return;
        }
        errors.push(ValidationError {
            file: CONFIG_FILE_LABEL.to_string(),
            message: format!(
                "{config_key}.overrides.{lang}.result_type = \"{result_type_value}\" is a primitive/pointer C \
                 spelling, not an IR struct/enum name -- this disables nested-field verification for the call \
                 because no IR type will ever match it; if the result genuinely carries no named fields, declare \
                 that with `result_is_bytes` / `result_is_simple` instead, or set `raw_c_result_type` on the \
                 call's `c` override to the C spelling the export actually returns"
            ),
            severity: Severity::Warning,
        });
        return;
    }

    let suggestion = closest_candidates(result_type_value, candidates);
    let suggestion_text = if suggestion.is_empty() {
        String::new()
    } else {
        format!(" (did you mean {}?)", suggestion.join(" or "))
    };
    errors.push(ValidationError {
        file: CONFIG_FILE_LABEL.to_string(),
        message: format!(
            "{config_key}.overrides.{lang}.result_type = \"{result_type_value}\" does not match any struct or \
             enum type the core IR declares for this crate{suggestion_text}"
        ),
        severity: Severity::Error,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::CallOverride;
    use crate::core::ir::FieldDef;

    fn make_type(name: &str) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            fields: vec![FieldDef::default()],
            ..TypeDef::default()
        }
    }

    fn make_enum(name: &str) -> EnumDef {
        EnumDef {
            name: name.to_string(),
            ..EnumDef::default()
        }
    }

    fn make_e2e_config(result_type: &str, lang: &str) -> E2eConfig {
        let mut call = CallConfig::default();
        let override_config = CallOverride {
            result_type: Some(result_type.to_string()),
            ..CallOverride::default()
        };
        call.overrides.insert(lang.to_string(), override_config);
        E2eConfig {
            call,
            ..E2eConfig::default()
        }
    }

    #[test]
    fn a_result_type_override_matching_an_emitted_struct_passes() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let e2e_config = make_e2e_config("ChatCompletionResponse", "c");

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["c".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn a_result_type_override_matching_an_emitted_enum_passes() {
        let enums = vec![make_enum("VisitResult")];
        let e2e_config = make_e2e_config("VisitResult", "c");

        let errors = validate_call_result_type_overrides(&e2e_config, &[], &enums, &["c".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn a_misspelled_result_type_override_fails_with_the_offending_language_and_value() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let e2e_config = make_e2e_config("ChatCompletionRespnse", "c");

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["c".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Error);
        assert_eq!(errors[0].file, "alef.toml");
        assert!(
            errors[0]
                .message
                .contains("[e2e.call].overrides.c.result_type = \"ChatCompletionRespnse\""),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains("does not match any struct or enum type"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn the_suggestion_names_the_closest_candidate() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let e2e_config = make_e2e_config("ChatCompletionRespnse", "c");

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["c".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0].message.contains("did you mean \"ChatCompletionResponse\""),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn an_absent_override_is_a_no_op() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let e2e_config = E2eConfig::default();

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["c".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn an_empty_ir_skips_validation_entirely() {
        let e2e_config = make_e2e_config("TotallyWrongTypeName", "c");

        let errors = validate_call_result_type_overrides(&e2e_config, &[], &[], &["c".to_string()]);

        assert_eq!(errors.len(), 0, "an absent IR must license no claim: {errors:?}");
    }

    #[test]
    fn a_language_that_does_not_consume_result_type_is_never_checked() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let e2e_config = make_e2e_config("TotallyWrongTypeName", "rust");

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["rust".to_string()]);

        assert_eq!(errors.len(), 0, "rust does not consume result_type: {errors:?}");
    }

    #[test]
    fn a_primitive_c_spelling_is_reported_separately_from_an_unknown_type() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let e2e_config = make_e2e_config("uintptr_t", "c");

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["c".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Warning);
        assert!(
            !errors[0].message.contains("does not match any struct or enum type"),
            "a primitive spelling must not be reported as an unknown type: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains("primitive/pointer C spelling"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn a_pointer_spelling_is_reported_separately_from_an_unknown_type() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let e2e_config = make_e2e_config("char*", "c");

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["c".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Warning);
        assert!(
            errors[0].message.contains("primitive/pointer C spelling"),
            "got: {}",
            errors[0].message
        );
    }

    /// The documented remedy for a primitive/pointer `result_type` spelling: `resolve_call_info`
    /// in `src/e2e/codegen/c.rs` reads `raw_c_result_type` off the SAME `c` override and, when
    /// set, takes the dedicated raw-result-type path in
    /// `src/e2e/codegen/c/test_function.rs` that returns before `result_type_name` is ever
    /// resolved against the IR. A call that already sets both `result_type` and
    /// `raw_c_result_type` to the same C spelling has applied the fix -- this must produce no
    /// diagnostic at all, not merely a downgraded one.
    #[test]
    fn a_primitive_result_type_with_a_matching_raw_c_result_type_produces_no_diagnostic() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let mut call = CallConfig::default();
        let override_config = CallOverride {
            result_type: Some("char*".to_string()),
            raw_c_result_type: Some("char*".to_string()),
            ..CallOverride::default()
        };
        call.overrides.insert("c".to_string(), override_config);
        let e2e_config = E2eConfig {
            call,
            ..E2eConfig::default()
        };

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["c".to_string()]);

        assert_eq!(
            errors.len(),
            0,
            "raw_c_result_type is the documented remedy and must fully suppress the warning: {errors:?}"
        );
    }

    /// The same primitive `result_type` spelling WITHOUT `raw_c_result_type` set must still
    /// warn -- proves the suppression above is conditioned on the remedy actually being present,
    /// not on the primitive spelling alone.
    #[test]
    fn a_primitive_result_type_without_raw_c_result_type_still_warns() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let e2e_config = make_e2e_config("char*", "c");

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["c".to_string()]);

        assert_eq!(
            errors.len(),
            1,
            "without raw_c_result_type the primitive spelling must still be reported: {errors:?}"
        );
        assert_eq!(errors[0].severity, Severity::Warning);
    }

    /// `result_is_bytes` / `result_is_simple` are the remedy for a call that names NO
    /// `result_type` at all (`unresolved_result_type_name` / `call_declares_non_struct_result`).
    /// They are never consulted once `result_type` is explicitly set -- `resolve_call_info`'s
    /// `.or_else()` chain short-circuits before either flag is read -- so setting them alongside
    /// a primitive `result_type` must NOT suppress the diagnostic.
    #[test]
    fn result_is_bytes_does_not_license_a_primitive_result_type_override() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let mut call = CallConfig::default();
        call.result_is_bytes = true;
        let override_config = CallOverride {
            result_type: Some("char*".to_string()),
            ..CallOverride::default()
        };
        call.overrides.insert("c".to_string(), override_config);
        let e2e_config = E2eConfig {
            call,
            ..E2eConfig::default()
        };

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["c".to_string()]);

        assert_eq!(
            errors.len(),
            1,
            "result_is_bytes is not read once result_type is set, so it must not suppress: {errors:?}"
        );
        assert_eq!(errors[0].severity, Severity::Warning);
    }

    /// `raw_c_result_type` only takes effect on the `c` override section -- the C generator
    /// always reads `call.overrides.get("c")`. Setting it on a different language's override
    /// section must not license a primitive `result_type` value authored under that same
    /// (non-`c`) section.
    #[test]
    fn raw_c_result_type_on_a_different_language_override_does_not_license_it() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let mut call = CallConfig::default();
        let override_config = CallOverride {
            result_type: Some("char*".to_string()),
            raw_c_result_type: Some("char*".to_string()),
            ..CallOverride::default()
        };
        call.overrides.insert("csharp".to_string(), override_config);
        let e2e_config = E2eConfig {
            call,
            ..E2eConfig::default()
        };

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["csharp".to_string()]);

        assert_eq!(
            errors.len(),
            1,
            "raw_c_result_type is only read from the c override section: {errors:?}"
        );
        assert_eq!(errors[0].severity, Severity::Warning);
    }

    #[test]
    fn a_named_call_override_names_its_own_config_key() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let mut e2e_config = E2eConfig::default();
        let mut call = CallConfig::default();
        let override_config = CallOverride {
            result_type: Some("NotARealType".to_string()),
            ..CallOverride::default()
        };
        call.overrides.insert("c".to_string(), override_config);
        e2e_config.calls.insert("chat".to_string(), call);

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["c".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0]
                .message
                .starts_with("[e2e.calls.chat].overrides.c.result_type"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn a_csharp_override_is_checked_when_dart_is_generated() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let e2e_config = make_e2e_config("TotallyWrongTypeName", "csharp");

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["dart".to_string()]);

        assert_eq!(
            errors.len(),
            1,
            "a csharp override read by dart's cross-language fallback must be checked: {errors:?}"
        );
    }

    #[test]
    fn a_csharp_override_is_a_no_op_when_only_csharp_is_out_of_scope() {
        let type_defs = vec![make_type("ChatCompletionResponse")];
        let e2e_config = make_e2e_config("TotallyWrongTypeName", "csharp");

        let errors = validate_call_result_type_overrides(&e2e_config, &type_defs, &[], &["python".to_string()]);

        assert_eq!(
            errors.len(),
            0,
            "no active generator reads a csharp result_type override here: {errors:?}"
        );
    }
}
