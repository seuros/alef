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
//!   names a *class* instead of a *package* (e.g. `"io.sample.Sample"`, copied from a
//!   `class` override one line up) produces `import io.sample.Sample.*;`, which does not
//!   compile — every emitted snippet, not the config.
//! - `src/e2e/codegen/go/snippet.rs` (per-fixture snippets, which may resolve to any named
//!   `[e2e.calls.*]` call) resolves, in order: the resolved call's own `overrides.go.module`,
//!   then the *base* `[e2e.call]`'s `overrides.go.module` (a named call with no go override of
//!   its own still inherits the base call's), then `[go].module`, then the resolved call's own
//!   base `module` field. `src/e2e/codegen/go.rs` (the package-level `go.mod` resolution)
//!   follows the same order but only ever resolves against the base call, so its first two
//!   rungs collapse into one. Either way the winner is spliced verbatim into a Go `import`
//!   path. A bare word (no `.`, no `/`) is never a real import path for a project's own
//!   generated package — only the standard library spells import paths that way, and this
//!   field never names the standard library. See [`effective_go_module`]'s doc comment for why
//!   this check must consult the base call's override too, not just the call being validated.
//!
//! Both failure modes were reported together from one consumer: `module` set to a Java
//! class instead of a package, and to a bare word for Go, producing ~38 broken generated
//! snippets (19 java + 19 go) that a single config-time diagnostic would have caught.
//! See `crate::e2e::validate_call_class` for the sibling check this mirrors, and its doc
//! comment for why an absent/inert value licenses no claim.
//!
//! ## Severity
//!
//! The two checks split (`alef-tasks#335`), because the evidence for each differs:
//!
//! - The go check ([`check_go_module`]) is [`Severity::Error`]. A Go import path that is a
//!   bare word (no `.`, no `/`) is not a style guess — Go's own module resolution can never
//!   treat it as anything but a standard-library package, and this field never names one.
//!   The fleet survey backing the promotion found 42 live `overrides.go.module`/`module`
//!   resolutions across every consumer repo pinning the next alef release, every one a real
//!   `github.com/...` path, zero of which this check would have flagged.
//! - The java check ([`check_java_module`]) stays [`Severity::Warning`]. "Last segment is
//!   uppercase" is a strong Java convention, not something javac enforces — an uppercase
//!   package segment compiles fine, just against convention — so it remains a heuristic.
//!   More importantly, the same fleet survey found **zero** consumers currently setting
//!   `overrides.java.module` at all (every one uses `overrides.<lang>.class` instead, which
//!   `validate_call_class` already checks). "Zero false positives out of zero exercised
//!   values" is not evidence the rule is safe to hard-error on — it is exactly the vacuous-
//!   survey shape this promotion decision was supposed to rule out, so this check waits for
//!   a live positive data point before promotion, per the original bar below.
//!
//! (Original bar, still governing the java check: a rule that only ever fires on real
//! consumer fleets can be promoted once it is shown to have zero false positives against
//! every consumer's current, working config — not zero opportunities to fire at all.)

use super::diagnostic_log::{DiagnosticLog, unreported};
use super::validate::{Severity, ValidationError};
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::{CallConfig, E2eConfig};

const CONFIG_FILE_LABEL: &str = "alef.toml";

/// Run [`validate_call_module_overrides`], log every diagnostic `log` has not already reported,
/// then bail with every
/// [`Severity::Error`] diagnostic's message when any fired.
///
/// A genuinely broken go `module` now aborts generation ([`check_go_module`] is `Error`);
/// a suspicious java `module` still only logs ([`check_java_module`] stays `Warning`) — see
/// the module doc comment's "Severity" section for why the two checks carry different
/// severities today.
pub fn enforce_call_module_overrides(
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    languages: &[String],
    log: &DiagnosticLog,
) -> anyhow::Result<()> {
    let diagnostics = validate_call_module_overrides(e2e_config, config, languages);
    for diag in unreported(&diagnostics, log) {
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
///
/// Deliberately a no-op when neither `"java"` nor `"go"` is in `languages`: this check exists
/// to catch a value that will not compile in *generated* java/go code, and a run that resolves
/// neither language generates none of it -- the value licenses no claim about correctness the
/// same way an absent/ambiguous IR licenses none in `validate_call_args`. Silence here means
/// "nothing to check this run," not "checked and clean" -- a consumer relying on this check
/// still needs at least one java or go run (e.g. in CI) to ever see the diagnostic at all.
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
            check_go_module(config_key, call, &e2e_config.call, config, &mut errors);
        }
    }
    errors
}

/// The java e2e generator only ever reads `overrides.java.module`
/// (`src/e2e/codegen/java/snippet.rs`'s `package_name`) — `src/e2e/codegen/java.rs` (the
/// package-level generator) never reads the base `module` field for java at all, so a base
/// value licenses no claim about what java actually emits. An absent override falls back to
/// `config.java_package()`, which is always a config-derived, well-formed package name, never
/// free text this check needs to validate.
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
/// regression (`"io.sample.Sample"`): a correctly-cased package prefix with a class name
/// appended at the end, copy-pasted from a `class` override one line up.
fn java_module_looks_like_a_class(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    let last_segment = trimmed.rsplit('.').next().unwrap_or(trimmed);
    last_segment.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

/// The go e2e generator's own resolution order for the module/import path, which differs by
/// which generator layer resolves it:
///
/// - `src/e2e/codegen/go/snippet.rs` (per-fixture snippets, which may resolve to any named
///   `[e2e.calls.*]` call): the resolved call's own `overrides.go.module`, then the *base*
///   `[e2e.call]`'s `overrides.go.module` (a named call with no go override of its own still
///   inherits the base call's — see `render_snippet_body`'s `module` binding), then
///   `[go].module`, then the resolved call's own base `module` field.
/// - `src/e2e/codegen/go.rs` (the package-level `go.mod`/import-path resolution): the same
///   order, but it only ever resolves against the base call (`e2e_config.call`) — there is no
///   "resolved call" at that layer, so the first two rungs collapse into one.
///
/// This function always takes both `call` (the source being validated) and `base_call`
/// (`e2e_config.call`) so it can reproduce the snippet generator's four-rung order exactly for
/// every named call. For the `[e2e.call]` source itself, `call` and `base_call` are the same
/// reference, which collapses the second rung for free and reproduces `go.rs`'s three-rung
/// order without a second code path.
///
/// `[go].module` is itself a distinct, already-validated field (a real go.mod module path, not
/// e2e config) — when it is set, it always wins over either call's base `module` field, so
/// neither licenses a claim once it is set. Returns the winning value together with a
/// description of where it lives, for the diagnostic message; `None` when nothing go actually
/// consumes was configured.
fn effective_go_module(
    call: &CallConfig,
    base_call: &CallConfig,
    config: &ResolvedCrateConfig,
) -> Option<(String, GoModuleSource)> {
    if let Some(value) = call.overrides.get("go").and_then(|o| o.module.clone()) {
        return Some((value, GoModuleSource::OwnOverride));
    }
    if let Some(value) = base_call.overrides.get("go").and_then(|o| o.module.clone()) {
        return Some((value, GoModuleSource::BaseOverride));
    }
    if config.go.as_ref().and_then(|go| go.module.as_ref()).is_some() {
        return None;
    }
    let base = call.module.trim();
    if base.is_empty() {
        None
    } else {
        Some((base.to_string(), GoModuleSource::OwnModuleField))
    }
}

/// Where [`effective_go_module`]'s winning value actually lives in `alef.toml`, so
/// [`check_go_module`] can name it accurately — the winning rung is not always a field on the
/// call being validated (see [`GoModuleSource::BaseOverride`]).
enum GoModuleSource {
    /// `<config_key>.overrides.go.module`, on the call being validated itself.
    OwnOverride,
    /// `[e2e.call].overrides.go.module` — the *base* call's override, inherited because the
    /// call being validated declares no go override of its own.
    BaseOverride,
    /// `<config_key>.module`, on the call being validated itself.
    OwnModuleField,
}

fn check_go_module(
    config_key: &str,
    call: &CallConfig,
    base_call: &CallConfig,
    config: &ResolvedCrateConfig,
    errors: &mut Vec<ValidationError>,
) {
    let Some((value, source)) = effective_go_module(call, base_call, config) else {
        return;
    };
    if !go_module_is_a_bare_word(&value) {
        return;
    }
    let location = match source {
        GoModuleSource::OwnOverride => format!("{config_key}.overrides.go.module"),
        GoModuleSource::BaseOverride => {
            format!(
                "[e2e.call].overrides.go.module (inherited by {config_key}, which declares no go override of its own)"
            )
        }
        GoModuleSource::OwnModuleField => format!("{config_key}.module"),
    };
    errors.push(ValidationError {
        file: CONFIG_FILE_LABEL.to_string(),
        message: format!(
            "{location} = \"{value}\" is a bare word, not a resolvable Go import path (no \".\" \
             or \"/\") — the go e2e generator imports this value verbatim as the crate's own package \
             path; set `[go] module = \"github.com/<org>/<repo>\"` or `overrides.go.module` to a real \
             import path"
        ),
        severity: Severity::Error,
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
            call: call_with_java_override("io.sample.Sample"),
            ..E2eConfig::default()
        };

        let errors = validate_call_module_overrides(&e2e_config, &config, &["java".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Warning);
        assert!(
            errors[0]
                .message
                .contains("[e2e.call].overrides.java.module = \"io.sample.Sample\""),
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
            call: call_with_java_override("io.sample.Sample"),
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
        assert_eq!(errors[0].severity, Severity::Error);
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
            .insert("summarize".to_string(), call_with_java_override("io.sample.Sample"));

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

    /// Regression coverage for the precedence bug: `src/e2e/codegen/go/snippet.rs` falls back
    /// to the *base* `[e2e.call]`'s go override for a named call that declares none of its
    /// own -- the resolved snippet for `summarize` really does import a bare word here. The
    /// check must catch it even though the bad value lives on a different config key than the
    /// one being reported for.
    #[test]
    fn a_named_call_with_no_go_override_inherits_the_bad_base_override() {
        let config = make_config("sample-widget-rs");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.overrides.insert(
            "go".to_string(),
            CallOverride {
                module: Some("widget".to_string()),
                ..CallOverride::default()
            },
        );
        e2e_config.calls.insert("summarize".to_string(), CallConfig::default());

        let errors = validate_call_module_overrides(&e2e_config, &config, &["go".to_string()]);

        assert_eq!(
            errors.len(),
            2,
            "expected the base call and its inheritor both flagged: {errors:?}"
        );
        let summarize_error = errors
            .iter()
            .find(|e| e.message.contains("inherited by [e2e.calls.summarize]"))
            .unwrap_or_else(|| panic!("no error named the inherited base override: {errors:?}"));
        assert!(
            summarize_error
                .message
                .starts_with("[e2e.call].overrides.go.module (inherited by [e2e.calls.summarize]"),
            "got: {}",
            summarize_error.message
        );
        assert!(
            summarize_error.message.contains("= \"widget\""),
            "got: {}",
            summarize_error.message
        );
    }

    /// The positive twin: a named call inheriting a *valid* base go override must not be
    /// flagged, proving the inheritance rung itself (not just "any base value") is what the
    /// generator consults.
    #[test]
    fn a_named_call_inheriting_a_valid_base_go_override_passes() {
        let config = make_config("sample-widget-rs");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.overrides.insert(
            "go".to_string(),
            CallOverride {
                module: Some("github.com/example/sample-widget".to_string()),
                ..CallOverride::default()
            },
        );
        e2e_config.calls.insert("summarize".to_string(), CallConfig::default());

        let errors = validate_call_module_overrides(&e2e_config, &config, &["go".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    /// A named call's *own* go override always wins over the base call's, matching the
    /// generator's first rung -- inheriting the base value is strictly a fallback for when the
    /// named call declares nothing itself.
    #[test]
    fn a_named_calls_own_go_override_wins_over_the_base_calls() {
        let config = make_config("sample-widget-rs");
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.overrides.insert(
            "go".to_string(),
            CallOverride {
                module: Some("widget".to_string()),
                ..CallOverride::default()
            },
        );
        e2e_config.calls.insert(
            "summarize".to_string(),
            call_with_go_override("github.com/example/sample-widget"),
        );

        let errors = validate_call_module_overrides(&e2e_config, &config, &["go".to_string()]);

        assert_eq!(
            errors.len(),
            1,
            "only the base call's own bad override should be flagged: {errors:?}"
        );
        assert!(
            errors[0].message.starts_with("[e2e.call].overrides.go.module"),
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

    /// `alef-tasks#335`: [`check_go_module`] is now `Severity::Error`, so a bare-word go
    /// import path must abort generation through [`enforce_call_module_overrides`], not
    /// merely log. Sabotage coverage for the promotion: revert `check_go_module`'s severity
    /// literal to `Warning` and this is the test that stops failing.
    #[test]
    fn enforce_bails_on_a_bare_word_go_module() {
        let config = make_config("sample-widget-rs");
        let e2e_config = E2eConfig {
            call: call_with_go_override("widget"),
            ..E2eConfig::default()
        };

        let result = enforce_call_module_overrides(&e2e_config, &config, &["go".to_string()], &DiagnosticLog::new());

        let err = result.expect_err("a bare-word go module must abort generation");
        assert!(
            err.to_string().contains("not a resolvable Go import path"),
            "got: {err}"
        );
    }

    /// The positive twin, built from the exact shape the `alef-tasks#335` consumer-fleet
    /// survey found live across every repo pinning the next alef release: a real
    /// `github.com/...` go import path. This may not abort generation.
    #[test]
    fn enforce_does_not_bail_on_a_real_go_import_path() {
        let config = make_config("sample-widget-rs");
        let e2e_config = E2eConfig {
            call: call_with_go_override("github.com/example/sample-widget"),
            ..E2eConfig::default()
        };

        let result = enforce_call_module_overrides(&e2e_config, &config, &["go".to_string()], &DiagnosticLog::new());

        assert!(result.is_ok(), "expected Ok(()), got: {result:?}");
    }

    /// The java check stays `Warning`: even a value [`check_java_module`] flags must not
    /// abort generation, since the fleet survey backing this promotion found no live
    /// `overrides.java.module` value to validate the heuristic against (see the module doc
    /// comment's "Severity" section).
    #[test]
    fn enforce_does_not_bail_on_a_flagged_java_module() {
        let config = make_config("sample_crate");
        let e2e_config = E2eConfig {
            call: call_with_java_override("io.sample.Sample"),
            ..E2eConfig::default()
        };

        let result = enforce_call_module_overrides(&e2e_config, &config, &["java".to_string()], &DiagnosticLog::new());

        assert!(
            result.is_ok(),
            "a flagged java module is still only a warning: {result:?}"
        );
    }
}
