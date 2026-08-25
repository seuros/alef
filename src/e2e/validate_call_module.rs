//! Validates `[e2e.call(s).*] module` — and its `overrides.java` / `overrides.go` forms —
//! against the shape each backend actually consumes it as: a Java package name, or a Go
//! import path.
//!
//! `module` is a single, heavily overloaded field: nearly every e2e backend
//! (`src/e2e/codegen/<lang>.rs`) reads it as a fallback for a completely different
//! per-language concept — a Python/Ruby/Elixir module name, a C# namespace, an npm
//! package name, a PHP namespace, a Homebrew binary name. Two backends specifically
//! read it as something structurally checkable:
//!
//! - `src/e2e/codegen/java/snippet.rs` reads `overrides.java.module` (never the base
//!   field — see `effective_go_module`'s sibling reasoning inline below for why the base
//!   is skipped for java) and splices it verbatim into `import {value}.*;`. A value that
//!   names a *class* instead of a *package* (e.g. `"io.xberg.Xberg"`, copied from a
//!   `class` override one line up) produces `import io.xberg.Xberg.*;`, which does not
//!   compile — every emitted snippet, not the config.
//! - `src/e2e/codegen/go.rs` / `src/e2e/codegen/go/snippet.rs` resolve, in order,
//!   `overrides.go.module`, then `[go].module`, then the base `module` field, and splice
//!   the winner verbatim into a Go `import` path. A bare word (no `.`, no `/`) is never a
//!   real import path for a project's own generated package — only the standard library
//!   spells import paths that way, and this field never names the standard library.
//!
//! Both failure modes were reported together from one consumer: `module` set to a Java
//! class instead of a package, and to a bare word for Go, producing ~38 broken generated
//! snippets (19 java + 19 go) that a single config-time diagnostic would have caught.
//! See `crate::e2e::validate_call_class` for the sibling check this mirrors, and its doc
//! comment for why an absent/inert value licenses no claim.
//!
//! ## Severity
//!
//! Both checks land as [`Severity::Warning`], not [`Severity::Error`]: the structural
//! rules below (uppercase last segment, no `.`/`/`) are heuristics, not IR-verified
//! facts the way `validate_call_class` and `validate_call_result_type` are, and a
//! rule that only ever fires on real consumer fleets can be promoted later once it is
//! shown to have zero false positives against every consumer's current, working config.

use super::validate::{Severity, ValidationError};
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::{CallConfig, E2eConfig};

const CONFIG_FILE_LABEL: &str = "alef.toml";

/// Run [`validate_call_module_overrides`] and log every diagnostic.
///
/// Unlike [`crate::e2e::validate_call_class::enforce_call_class_overrides`], this never
/// turns a diagnostic into a generation-aborting error: every diagnostic
/// [`validate_call_module_overrides`] produces is [`Severity::Warning`] today (see the
/// module doc comment's "Severity" section), so there is nothing to bail on. Kept as an
/// `enforce_*` function with the same shape as its two siblings anyway, so promoting this
/// check to hard-error status later (once a consumer-fleet measurement shows zero false
/// positives) is a one-line severity change here, not a call-site rewrite.
pub fn enforce_call_module_overrides(
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    languages: &[String],
) -> anyhow::Result<()> {
    let diagnostics = validate_call_module_overrides(e2e_config, config, languages);
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
        "e2e call module validation failed: {}",
        errors
            .iter()
            .map(|diag| format!("{}: {}", diag.file, diag.message))
            .collect::<Vec<_>>()
            .join("; ")
    );
}

/// Validate every `module` value in `e2e_config` (the default `[e2e.call]` and every
/// named `[e2e.calls.*]`) that java or go codegen will actually consume, given the
/// resolved `languages` for this run.
pub fn validate_call_module_overrides(
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    languages: &[String],
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let java_active = languages.iter().any(|lang| lang == "java");
    let go_active = languages.iter().any(|lang| lang == "go");
    if !java_active && !go_active {
        return errors;
    }

    let mut sources: Vec<(String, &CallConfig)> = vec![("[e2e.call]".to_string(), &e2e_config.call)];
    let mut named_calls: Vec<(&String, &CallConfig)> = e2e_config.calls.iter().collect();
    named_calls.sort_by_key(|(name, _)| (*name).clone());
    for (name, call) in named_calls {
        sources.push((format!("[e2e.calls.{name}]"), call));
    }

    for (config_key, call) in &sources {
        if java_active {
            check_java_module(config_key, call, &mut errors);
        }
        if go_active {
            check_go_module(config_key, call, config, &mut errors);
        }
    }
    errors
}

/// The java e2e generator only ever reads `overrides.java.module`
/// (`src/e2e/codegen/java/snippet.rs`'s `package_name`) — the base `module` field is
/// computed in `src/e2e/codegen/java.rs` but never used (bound to `_module_path`), so a
/// base value licenses no claim about what java actually emits. An absent override falls
/// back to `config.java_package()`, which is always a config-derived, well-formed
/// package name, never free text this check needs to validate.
fn check_java_module(config_key: &str, call: &CallConfig, errors: &mut Vec<ValidationError>) {
    let Some(value) = call.overrides.get("java").and_then(|o| o.module.as_deref()) else {
        return;
    };
    if !java_module_looks_like_a_class(value) {
        return;
    }
    errors.push(ValidationError {
        file: CONFIG_FILE_LABEL.to_string(),
        message: format!(
            "{config_key}.overrides.java.module = \"{value}\" looks like a Java class, not a package \
             (its last segment starts with an uppercase letter) — the java e2e generator emits \
             `import {value}.*;`, which will not compile against a class"
        ),
        severity: Severity::Warning,
    });
}

/// A java package segment starts with a lowercase letter (or `_`/`$`) by strong,
/// near-universal Java convention; a class name starts with an uppercase letter by the
/// same convention. This checks only the last dot-segment, mirroring the exact reported
/// regression (`"io.xberg.Xberg"`): a correctly-cased package prefix with a class name
/// appended at the end, copy-pasted from a `class` override one line up.
fn java_module_looks_like_a_class(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    let last_segment = trimmed.rsplit('.').next().unwrap_or(trimmed);
    last_segment.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

/// The go e2e generator's own resolution order for the module/import path
/// (`src/e2e/codegen/go.rs`, `src/e2e/codegen/go/snippet.rs`): `overrides.go.module`
/// first, then `[go].module`, then the base `module` field.
///
/// `[go].module` is itself a distinct, already-validated field (a real go.mod module
/// path, not e2e config) — when it is set, it always wins over the base `module` field,
/// so the base value never reaches the go backend and licenses no claim regardless of
/// what it looks like. Returns the winning value together with the config key it lives
/// under, for the diagnostic message; `None` when nothing go actually consumes was
/// configured.
fn effective_go_module(call: &CallConfig, config: &ResolvedCrateConfig) -> Option<(String, &'static str)> {
    if let Some(value) = call.overrides.get("go").and_then(|o| o.module.clone()) {
        return Some((value, ".overrides.go.module"));
    }
    if config.go.as_ref().and_then(|go| go.module.as_ref()).is_some() {
        return None;
    }
    let base = call.module.trim();
    if base.is_empty() {
        None
    } else {
        Some((base.to_string(), ".module"))
    }
}

fn check_go_module(
    config_key: &str,
    call: &CallConfig,
    config: &ResolvedCrateConfig,
    errors: &mut Vec<ValidationError>,
) {
    let Some((value, field)) = effective_go_module(call, config) else {
        return;
    };
    if !go_module_is_a_bare_word(&value) {
        return;
    }
    errors.push(ValidationError {
        file: CONFIG_FILE_LABEL.to_string(),
        message: format!(
            "{config_key}{field} = \"{value}\" is a bare word, not a resolvable Go import path (no \".\" \
             or \"/\") — the go e2e generator imports this value verbatim as the crate's own package \
             path; set `[go] module = \"github.com/<org>/<repo>\"` or `overrides.go.module` to a real \
             import path"
        ),
        severity: Severity::Warning,
    });
}

/// A real Go import path always carries either a domain (`.`) or a path separator (`/`)
/// — a bare identifier with neither can only ever resolve as a standard-library package,
/// which this field never names (it always names the current crate's own generated
/// package).
fn go_module_is_a_bare_word(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && !trimmed.contains('.') && !trimmed.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::CallOverride;

    fn make_config(crate_name: &str) -> ResolvedCrateConfig {
        ResolvedCrateConfig {
            name: crate_name.to_string(),
            ..ResolvedCrateConfig::default()
        }
    }

    /// Resolve a crate config from a literal `alef.toml` snippet — used for cases that
    /// need `[go] module` set, since `GoConfig` has no `Default` impl.
    fn resolved_one(toml: &str) -> ResolvedCrateConfig {
        use crate::core::config::new_config::NewAlefConfig;
        let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    fn call_with_java_override(module: &str) -> CallConfig {
        let mut call = CallConfig::default();
        call.overrides.insert(
            "java".to_string(),
            CallOverride {
                module: Some(module.to_string()),
                ..CallOverride::default()
            },
        );
        call
    }

    fn call_with_go_override(module: &str) -> CallConfig {
        let mut call = CallConfig::default();
        call.overrides.insert(
            "go".to_string(),
            CallOverride {
                module: Some(module.to_string()),
                ..CallOverride::default()
            },
        );
        call
    }

    #[test]
    fn a_java_package_override_passes() {
        let config = make_config("sample_crate");
        let e2e_config = E2eConfig {
            call: call_with_java_override("io.example.widget"),
            ..E2eConfig::default()
        };

        let errors = validate_call_module_overrides(&e2e_config, &config, &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn a_java_override_naming_a_class_fails() {
        let config = make_config("sample_crate");
        let e2e_config = E2eConfig {
            call: call_with_java_override("io.xberg.Xberg"),
            ..E2eConfig::default()
        };

        let errors = validate_call_module_overrides(&e2e_config, &config, &["java".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Warning);
        assert!(
            errors[0]
                .message
                .contains("[e2e.call].overrides.java.module = \"io.xberg.Xberg\""),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains("looks like a Java class, not a package"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn an_absent_java_override_is_a_no_op_regardless_of_the_base_module() {
        let config = make_config("sample_crate");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.module = "TotallyWrongBaseValue".to_string();

        let errors = validate_call_module_overrides(&e2e_config, &config, &["java".to_string()]);

        assert_eq!(errors.len(), 0, "java never reads the base `module` field: {errors:?}");
    }

    #[test]
    fn a_single_segment_lowercase_java_module_passes() {
        let config = make_config("sample_crate");
        let e2e_config = E2eConfig {
            call: call_with_java_override("widget"),
            ..E2eConfig::default()
        };

        let errors = validate_call_module_overrides(&e2e_config, &config, &["java".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn java_is_not_checked_when_java_is_not_an_active_language() {
        let config = make_config("sample_crate");
        let e2e_config = E2eConfig {
            call: call_with_java_override("io.xberg.Xberg"),
            ..E2eConfig::default()
        };

        let errors = validate_call_module_overrides(&e2e_config, &config, &["python".to_string()]);

        assert_eq!(errors.len(), 0, "java is not active: {errors:?}");
    }

    #[test]
    fn a_resolvable_go_module_override_passes() {
        let config = make_config("sample-widget-rs");
        let e2e_config = E2eConfig {
            call: call_with_go_override("github.com/example/sample-widget"),
            ..E2eConfig::default()
        };

        let errors = validate_call_module_overrides(&e2e_config, &config, &["go".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn a_bare_word_go_module_override_fails() {
        let config = make_config("sample-widget-rs");
        let e2e_config = E2eConfig {
            call: call_with_go_override("widget"),
            ..E2eConfig::default()
        };

        let errors = validate_call_module_overrides(&e2e_config, &config, &["go".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Warning);
        assert!(
            errors[0]
                .message
                .contains("[e2e.call].overrides.go.module = \"widget\""),
            "got: {}",
            errors[0].message
        );
        assert!(
            errors[0].message.contains("not a resolvable Go import path"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn a_bare_word_base_module_fails_when_go_module_config_is_absent() {
        let config = make_config("sample-widget-rs");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.module = "widget".to_string();

        let errors = validate_call_module_overrides(&e2e_config, &config, &["go".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0].message.contains("[e2e.call].module = \"widget\""),
            "got: {}",
            errors[0].message
        );
    }

    fn config_with_go_module(crate_name: &str, go_module: &str) -> ResolvedCrateConfig {
        resolved_one(&format!(
            r#"
[workspace]
languages = ["go"]

[[crates]]
name = "{crate_name}"
sources = ["src/lib.rs"]

[crates.go]
module = "{go_module}"
"#
        ))
    }

    #[test]
    fn a_bare_word_base_module_is_a_no_op_when_go_module_config_is_set() {
        let config = config_with_go_module("sample-widget-rs", "github.com/example/sample-widget");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.module = "widget".to_string();

        let errors = validate_call_module_overrides(&e2e_config, &config, &["go".to_string()]);

        assert_eq!(
            errors.len(),
            0,
            "`[go] module` wins over the base field, so the base value is inert: {errors:?}"
        );
    }

    #[test]
    fn a_bare_word_go_override_still_fails_even_when_go_module_config_is_set() {
        let config = config_with_go_module("sample-widget-rs", "github.com/example/sample-widget");
        let e2e_config = E2eConfig {
            call: call_with_go_override("widget"),
            ..E2eConfig::default()
        };

        let errors = validate_call_module_overrides(&e2e_config, &config, &["go".to_string()]);

        assert_eq!(
            errors.len(),
            1,
            "an explicit override still wins over `[go] module` and is still checked: {errors:?}"
        );
    }

    #[test]
    fn go_is_not_checked_when_go_is_not_an_active_language() {
        let config = make_config("sample-widget-rs");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.module = "widget".to_string();

        let errors = validate_call_module_overrides(&e2e_config, &config, &["python".to_string()]);

        assert_eq!(errors.len(), 0, "go is not active: {errors:?}");
    }

    #[test]
    fn a_named_call_override_names_its_own_config_key() {
        let config = make_config("sample_crate");
        let mut e2e_config = E2eConfig::default();
        e2e_config
            .calls
            .insert("summarize".to_string(), call_with_java_override("io.xberg.Xberg"));

        let errors = validate_call_module_overrides(&e2e_config, &config, &["java".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0]
                .message
                .starts_with("[e2e.calls.summarize].overrides.java.module"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn an_empty_module_value_is_a_no_op() {
        let config = make_config("sample_crate");
        let e2e_config = E2eConfig::default();

        let errors = validate_call_module_overrides(&e2e_config, &config, &["java".to_string(), "go".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }
}
