//! Validates each fixture's effective `args` against the Rust function signature the
//! core IR declares for the call it resolves to.
//!
//! Every backend lowers a configured [`ArgMapping`] purely from its own `arg_type`
//! string and the fixture's `input` JSON — nothing checked the `name` on that mapping
//! against the actual Rust parameter it claims to fill, or that every required
//! parameter had an arg supplying it at all. A fixture naming a parameter the function
//! no longer takes, or omitting one it now requires, generates code that fails to
//! compile in every target language at once, with no diagnostic naming the fixture, the
//! arg, or the function responsible.
//!
//! This mirrors [`crate::e2e::validate_call_class`] and
//! [`crate::e2e::validate_call_result_type`] in shape, but resolves against
//! [`crate::e2e::codegen::call_ir::CallIr`] rather than re-deriving a lookup of its own —
//! that module already holds the "declared parameters for this call" answer every
//! backend's argument-type resolution consults (`TargetParams`), so this validator asks
//! the exact question codegen asks, not an approximation of it.
//!
//! ## Scope: what "the IR" licenses
//!
//! An unresolved call signature — no free function or IR-type method of that name, or
//! several same-named methods disagreeing (see `CallIr::signature`'s doc comment) —
//! licenses no claim at all, not "no function found." This is deliberate: a function
//! intentionally removed from the binding surface (`#[alef::skip]`, `#[alef::exclude]`,
//! a per-crate `exclude_functions` entry) is not necessarily removed from
//! `ApiSurface::functions` — see [`crate::core::ir::items::FunctionDef::binding_excluded`]
//! — but this validator does not attempt to independently re-derive every exclusion
//! surface itself (`[crates.exclude]`, per-language `exclude_functions`,
//! `[crates.skipped]`, per-crate `[workspace.crates."<name>"]` overrides — auditing which
//! of these actually reach `functions`/`type_defs` by the time they arrive here is
//! `alef-tasks#323`'s scope, not this one's). It scopes itself to two signals it *can*
//! confirm directly: an absent/ambiguous `CallIr::signature` (skip), and an explicit
//! `binding_excluded` flag on the resolved function/method (skip) — both licensing "say
//! nothing" rather than "say wrong."
//!
//! ## Severity
//!
//! Lands as [`Severity::Warning`]. See `crate::e2e::validate_call_module`'s "Severity"
//! section for the same rationale: promotion to `Error` is a decision for once a
//! consumer-fleet measurement shows the rule's finding rate is real bugs, not noise.

use super::validate::{Severity, ValidationError};
use crate::core::config::e2e::{ArgMapping, CallConfig, E2eConfig};
use crate::core::ir::{FunctionDef, ParamDef, TypeDef};
use crate::e2e::codegen::call_ir::CallIr;
use crate::e2e::fixture::Fixture;

/// Run [`validate_call_arg_signatures`] and log every diagnostic. See
/// `crate::e2e::validate_call_module::enforce_call_module_overrides`'s doc comment for
/// why this never bails today: every diagnostic here is [`Severity::Warning`].
pub fn enforce_call_arg_signatures(
    fixtures: &[Fixture],
    e2e_config: &E2eConfig,
    functions: &[FunctionDef],
    type_defs: &[TypeDef],
    languages: &[String],
) -> anyhow::Result<()> {
    let diagnostics = validate_call_arg_signatures(fixtures, e2e_config, functions, type_defs, languages);
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
        "e2e call argument signature validation failed: {}",
        errors
            .iter()
            .map(|diag| format!("{}: {}", diag.file, diag.message))
            .collect::<Vec<_>>()
            .join("; ")
    );
}

/// Validate every fixture's effective `args` (its own `args`, or its resolved call's
/// `args` when the fixture declares none — see [`Fixture::resolved_args`]) against the
/// Rust parameter list the core IR declares for the call it resolves to.
///
/// Skipped entirely when both `functions` and `type_defs` are empty, mirroring
/// `validate_call_class_overrides`'s "absent IR licenses no claim" rule: several
/// legitimate callers pass an empty IR, and validating fixture args against a
/// deliberately incomplete registry would manufacture false positives rather than catch
/// a real drift.
pub fn validate_call_arg_signatures(
    fixtures: &[Fixture],
    e2e_config: &E2eConfig,
    functions: &[FunctionDef],
    type_defs: &[TypeDef],
    languages: &[String],
) -> Vec<ValidationError> {
    if functions.is_empty() && type_defs.is_empty() {
        return Vec::new();
    }
    let ir = CallIr { functions, type_defs };

    let mut errors = Vec::new();
    for fixture in fixtures {
        let call_config = e2e_config.resolve_call_for_fixture(
            fixture.call.as_deref(),
            &fixture.id,
            &fixture.resolved_category(),
            &fixture.tags,
            &fixture.input,
        );
        let Some(lookup_name) = canonical_lookup_name(call_config, languages) else {
            continue;
        };
        let Some(signature) = ir.signature(&lookup_name) else {
            continue;
        };
        if binding_excluded(&lookup_name, functions, type_defs) {
            continue;
        }
        let args = fixture.resolved_args(call_config);
        check_unknown_args(fixture, &lookup_name, args, signature.params, &mut errors);
        check_missing_required_params(fixture, &lookup_name, args, signature.params, &mut errors);
    }
    errors
}

/// The language-neutral Rust identity a call's `args` are declared against:
/// `CallConfig::function` when set, otherwise the first configured language's resolved
/// name — see `CallConfig::core_lookup_name`'s doc comment for why a per-language
/// override still resolves to one Rust-side identity. Returns `None` when no language
/// names a function at all (a call that exists only to carry per-fixture assertions
/// against another call's result, for instance).
fn canonical_lookup_name<'a>(call: &'a CallConfig, languages: &[String]) -> Option<std::borrow::Cow<'a, str>> {
    languages.iter().find_map(|language| call.core_lookup_name(language))
}

/// Whether the resolved free function or IR-type method named `name` is explicitly
/// marked excluded from the binding surface. See the module doc comment's "Scope"
/// section for why this is the one exclusion signal this validator confirms directly
/// rather than re-deriving the full exclusion-surface audit `alef-tasks#323` owns.
fn binding_excluded(name: &str, functions: &[FunctionDef], type_defs: &[TypeDef]) -> bool {
    if let Some(function) = functions.iter().find(|function| function.name == name) {
        return function.binding_excluded;
    }
    type_defs
        .iter()
        .flat_map(|type_def| type_def.methods.iter())
        .filter(|method| method.name == name)
        .all(|method| method.binding_excluded)
}

fn check_unknown_args(
    fixture: &Fixture,
    lookup_name: &str,
    args: &[ArgMapping],
    params: &[ParamDef],
    errors: &mut Vec<ValidationError>,
) {
    for arg in args {
        if params.iter().any(|param| param.name == arg.name) {
            continue;
        }
        errors.push(ValidationError {
            file: fixture.source.clone(),
            message: format!(
                "fixture '{}' arg '{}' names a parameter '{}' does not declare (call resolves to Rust \
                 function/method '{lookup_name}')",
                fixture.id, arg.name, lookup_name
            ),
            severity: Severity::Warning,
        });
    }
}

fn check_missing_required_params(
    fixture: &Fixture,
    lookup_name: &str,
    args: &[ArgMapping],
    params: &[ParamDef],
    errors: &mut Vec<ValidationError>,
) {
    for param in params {
        if param.optional || param.default.is_some() {
            continue;
        }
        if args.iter().any(|arg| arg.name == param.name) {
            continue;
        }
        errors.push(ValidationError {
            file: fixture.source.clone(),
            message: format!(
                "fixture '{}' does not supply required parameter '{}' of '{lookup_name}' (no arg, and the \
                 parameter has no default)",
                fixture.id, param.name
            ),
            severity: Severity::Warning,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{MethodDef, TypeRef};

    fn param(name: &str) -> ParamDef {
        ParamDef {
            name: name.to_string(),
            ty: TypeRef::String,
            ..ParamDef::default()
        }
    }

    fn optional_param(name: &str) -> ParamDef {
        ParamDef {
            optional: true,
            ..param(name)
        }
    }

    fn function(name: &str, params: Vec<ParamDef>) -> FunctionDef {
        FunctionDef {
            name: name.to_string(),
            params,
            return_type: TypeRef::String,
            ..FunctionDef::default()
        }
    }

    fn arg(name: &str, optional: bool) -> ArgMapping {
        ArgMapping {
            name: name.to_string(),
            field: format!("input.{name}"),
            arg_type: "string".to_string(),
            optional,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: None,
        }
    }

    fn call_named(function: &str, args: Vec<ArgMapping>) -> CallConfig {
        CallConfig {
            function: function.to_string(),
            args,
            ..CallConfig::default()
        }
    }

    fn fixture_with_call(id: &str, call: Option<&str>) -> Fixture {
        Fixture {
            id: id.to_string(),
            call: call.map(str::to_string),
            source: format!("{id}.json"),
            ..Fixture::default()
        }
    }

    #[test]
    fn matching_args_pass() {
        let functions = vec![function("complete", vec![param("prompt"), optional_param("model")])];
        let e2e_config = E2eConfig {
            call: call_named("complete", vec![arg("prompt", false), arg("model", true)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["rust".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn an_arg_naming_a_removed_parameter_is_flagged() {
        let functions = vec![function("complete", vec![param("prompt")])];
        let e2e_config = E2eConfig {
            call: call_named("complete", vec![arg("prompt", false), arg("concurrency", true)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["rust".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert_eq!(errors[0].severity, Severity::Warning);
        assert!(
            errors[0].message.contains("fixture 'basic' arg 'concurrency'"),
            "got: {}",
            errors[0].message
        );
        assert!(errors[0].message.contains("'complete'"), "got: {}", errors[0].message);
    }

    #[test]
    fn a_missing_required_parameter_is_flagged() {
        let functions = vec![function("complete", vec![param("prompt"), param("config")])];
        let e2e_config = E2eConfig {
            call: call_named("complete", vec![arg("prompt", false)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["rust".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0]
                .message
                .contains("fixture 'basic' does not supply required parameter 'config'"),
            "got: {}",
            errors[0].message
        );
    }

    #[test]
    fn a_missing_optional_parameter_is_not_flagged() {
        let functions = vec![function("complete", vec![param("prompt"), optional_param("model")])];
        let e2e_config = E2eConfig {
            call: call_named("complete", vec![arg("prompt", false)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["rust".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn a_missing_parameter_with_a_declared_default_is_not_flagged() {
        let functions = vec![function(
            "complete",
            vec![
                param("prompt"),
                ParamDef {
                    default: Some("Config::default()".to_string()),
                    ..param("config")
                },
            ],
        )];
        let e2e_config = E2eConfig {
            call: call_named("complete", vec![arg("prompt", false)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["rust".to_string()]);

        assert_eq!(errors.len(), 0, "expected no errors, got: {errors:?}");
    }

    #[test]
    fn an_unresolvable_call_licenses_no_claim() {
        let functions = vec![function("complete", vec![param("prompt")])];
        let e2e_config = E2eConfig {
            call: call_named("mystery", vec![arg("wrong_name", false)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["rust".to_string()]);

        assert_eq!(
            errors.len(),
            0,
            "an unresolved call must not be claimed wrong: {errors:?}"
        );
    }

    #[test]
    fn an_empty_ir_skips_validation_entirely() {
        let e2e_config = E2eConfig {
            call: call_named("complete", vec![arg("wrong_name", false)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &[], &[], &["rust".to_string()]);

        assert_eq!(errors.len(), 0, "an absent IR must license no claim: {errors:?}");
    }

    #[test]
    fn a_binding_excluded_function_licenses_no_claim() {
        let functions = vec![FunctionDef {
            binding_excluded: true,
            ..function("complete", vec![param("prompt")])
        }];
        let e2e_config = E2eConfig {
            call: call_named("complete", vec![arg("wrong_name", false)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["rust".to_string()]);

        assert_eq!(
            errors.len(),
            0,
            "an excluded function must not be claimed wrong: {errors:?}"
        );
    }

    #[test]
    fn a_fixture_level_args_override_replaces_the_call_args_for_validation() {
        let functions = vec![function("complete", vec![param("prompt"), param("visitor")])];
        let e2e_config = E2eConfig {
            call: call_named("complete", vec![arg("prompt", false)]),
            ..E2eConfig::default()
        };
        let mut fixture = fixture_with_call("with_visitor", None);
        fixture.args = vec![arg("prompt", false), arg("visitor", false)];
        let fixtures = vec![fixture];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["rust".to_string()]);

        assert_eq!(errors.len(), 0, "fixture-level args must replace call args: {errors:?}");
    }

    #[test]
    fn a_method_declared_on_an_ir_type_resolves_too() {
        let type_defs = vec![TypeDef {
            name: "Client".to_string(),
            methods: vec![MethodDef {
                name: "chat".to_string(),
                params: vec![param("request")],
                return_type: TypeRef::String,
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }];
        let e2e_config = E2eConfig {
            call: call_named("chat", vec![arg("wrong_name", false), arg("request", false)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &[], &type_defs, &["rust".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(errors[0].message.contains("'chat'"), "got: {}", errors[0].message);
    }

    #[test]
    fn a_named_call_resolves_through_the_fixtures_call_field() {
        let functions = vec![function("embed", vec![param("text")])];
        let mut e2e_config = E2eConfig::default();
        e2e_config.calls.insert(
            "embed".to_string(),
            call_named("embed", vec![arg("wrong_name", false), arg("text", false)]),
        );
        let fixtures = vec![fixture_with_call("embed_basic", Some("embed"))];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["rust".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0].message.contains("fixture 'embed_basic'"),
            "got: {}",
            errors[0].message
        );
    }
}
