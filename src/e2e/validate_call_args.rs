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
//! confirm directly: an absent/ambiguous `CallIr::signature` (skip), and
//! [`crate::e2e::codegen::call_ir::binding_excluded_for_language`] agreeing across every
//! resolved language that the call is excluded (skip) — both licensing "say nothing"
//! rather than "say wrong."
//!
//! `binding_excluded` is *not* a blanket skip, and asking it per-language matters: it marks
//! a symbol hidden from other-language bindings generated from IR, not from the Rust source
//! itself, and `src/e2e/codegen/rust/` emits real, positionally-bound calls against
//! `binding_excluded` methods regardless — a `DefaultClient` trait excluded from every
//! non-Rust language via `[crates.exclude].types` still gets a real `client.chat(req)` in the
//! generated Rust e2e suite. Skipping this check whenever the flag was set (regardless of
//! which languages this run actually resolves) left that Rust call's argument names
//! unchecked by construction. This validator now asks
//! [`crate::e2e::codegen::call_ir::binding_excluded_for_language`] the same question the
//! generator side already answers, instead of re-deriving a language-blind copy of it.
//!
//! ### Adapter-handled methods (`alef-tasks#361`)
//!
//! One more resolution the `binding_excluded_for_language` skip does not cover: `CallIr::signature`
//! prefers a visible free function unconditionally over a same-named method, but a free function
//! can itself be excluded from `ApiSurface.functions` on its own account (its own `#[alef::skip]`,
//! or a crate-wide `exclude.functions` entry matching its bare name) -- invisible to that lookup,
//! not merely deprioritized. When the only thing left to resolve against is a method every
//! `[[crates.adapters]]` entry has already claimed (`MethodDef::binding_exclusion_reason` starting
//! with [`crate::core::ir::ADAPTER_HANDLED_REASON_PREFIX`]), that method's own raw signature is not
//! trustworthy as the call's shape: an adapter reroutes the call for every binding, Rust's own e2e
//! suite included, and the shape actually configured under `[e2e.calls.*]` is frequently a
//! differently-parametered sibling free function written specifically to give that config a
//! Rust-side calling convention -- exactly the free function this validator can no longer see once
//! it has been excluded. Measured on a real consumer: a genuine `Handle::stream(&self, req)` /
//! `stream(handle, url)` name collision resolved to the method and hard-failed generation on
//! unmodified, previously-generating source, even though the configured `args` matched the (now
//! invisible) free function the real generator has always called. This is the same "ambiguous
//! name -> skip" convention `CallIr::signature` already applies when several same-named methods
//! disagree, extended to the one collision shape a functions-first priority rule cannot detect on
//! its own -- see [`crate::e2e::codegen::call_ir::resolves_only_via_adapter_handled_method`]'s doc
//! comment for why this reads the extractor's own marker rather than re-deriving the answer, and
//! why it does not weaken the ordinary (non-adapter) `binding_excluded` case `alef-tasks#350`
//! fixed.
//!
//! ## Severity
//!
//! Lands as [`Severity::Error`] (`alef-tasks#335`): unlike `validate_call_module`'s two
//! checks, both rules here derive from an IR-verified fact -- the exact `ParamDef` list
//! `CallIr::signature` resolves for the call, the same lookup every backend's own
//! argument-type resolution consults -- not a naming-convention heuristic. A finding is
//! never a guess about what the generator *might* do; it is the same answer the
//! generator itself is about to act on. Promotion followed a fleet survey across every
//! consumer repo pinning the next alef release: 88 real `[e2e.call]`/`[e2e.calls.*]`
//! call sites, zero of which this check would have flagged, plus a live sabotage
//! check -- deliberately breaking `check_unknown_args`/`check_missing_required_params`
//! and confirming the regression tests below fail -- to rule out the check being
//! vacuous (see `alef-tasks#350`, which fixed the two vacuity defects this rule
//! previously had: a `binding_excluded` skip that hid a call the generator still
//! emits, and an `.all()` over zero languages that returned `true` by construction).

use super::validate::{Severity, ValidationError};
use crate::core::config::e2e::{ArgMapping, CallConfig, E2eConfig};
use crate::core::ir::{FunctionDef, ParamDef, TypeDef};
use crate::e2e::codegen::call_ir::{CallIr, binding_excluded_for_language, resolves_only_via_adapter_handled_method};
use crate::e2e::fixture::Fixture;

/// Run [`validate_call_arg_signatures`] and log every diagnostic, then bail with every
/// [`Severity::Error`] diagnostic's message when any fired -- see this module's doc
/// comment's "Severity" section for why every diagnostic here is `Error`.
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
        if resolves_only_via_adapter_handled_method(&lookup_name, ir) {
            continue;
        }
        if languages
            .iter()
            .all(|language| binding_excluded_for_language(&lookup_name, language, ir))
        {
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
            severity: Severity::Error,
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
            severity: Severity::Error,
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
        assert_eq!(errors[0].severity, Severity::Error);
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

    /// Regression coverage for the defect this validator used to have: a `binding_excluded`
    /// function/method still gets a real, positionally-bound call from `src/e2e/codegen/rust/`
    /// (Rust never excludes), so a run that resolves `"rust"` must still catch a wrong arg name
    /// on it. Before `binding_excluded_for_language`'s language-aware check, the old blanket
    /// `binding_excluded(...)` skip made this case structurally undetectable — this exact
    /// shape (a `DefaultClient` trait method excluded from every non-Rust language, bound
    /// positionally in the generated Rust e2e suite) was reported as measured, not inferred.
    #[test]
    fn a_binding_excluded_function_is_still_validated_when_rust_is_resolved() {
        let functions = vec![FunctionDef {
            binding_excluded: true,
            ..function("complete", vec![param("prompt")])
        }];
        let e2e_config = E2eConfig {
            call: call_named("complete", vec![arg("prompt", false), arg("wrong_name", false)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["rust".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0].message.contains("fixture 'basic' arg 'wrong_name'"),
            "got: {}",
            errors[0].message
        );
    }

    /// The flip side: when no resolved language in this run is Rust, and every one of them
    /// treats the call as `binding_excluded`, no generator anywhere emits a real call against
    /// it — the check still licenses no claim in that case.
    #[test]
    fn a_binding_excluded_function_licenses_no_claim_when_no_resolved_language_renders_it() {
        let functions = vec![FunctionDef {
            binding_excluded: true,
            ..function("complete", vec![param("prompt")])
        }];
        let e2e_config = E2eConfig {
            call: call_named("complete", vec![arg("wrong_name", false)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["python".to_string()]);

        assert_eq!(
            errors.len(),
            0,
            "no resolved language renders an excluded call, so it must not be claimed wrong: {errors:?}"
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

    /// Builds a `Handle::stream` method flagged the way `pipeline::extract`'s
    /// `mark_adapter_handled_methods` flags a real `[[crates.adapters]]` target.
    fn adapter_handled_method(name: &str, params: Vec<ParamDef>) -> MethodDef {
        MethodDef {
            name: name.to_string(),
            params,
            return_type: TypeRef::String,
            binding_excluded: true,
            binding_exclusion_reason: Some(format!(
                "{} entry `{name}`",
                crate::core::ir::ADAPTER_HANDLED_REASON_PREFIX
            )),
            ..MethodDef::default()
        }
    }

    /// `alef-tasks#361`: reproduces the release-blocking regression measured on a real consumer.
    /// A method (`Handle::stream(&self, req)`) is bound via `[[crates.adapters]]`, so
    /// `pipeline::extract` flags it adapter-handled; the sibling free function
    /// (`stream(handle, url)`) written to mirror that call's convention for the polyglot e2e
    /// surface has, independently, been excluded from `ApiSurface.functions` (its own
    /// `#[alef::skip]`, or a bare-name `exclude.functions` entry) -- so `functions` is empty of
    /// "stream" here, exactly as it would be by the time this validator runs. Before this fix,
    /// `CallIr::signature`'s method fallback resolved with full confidence to the method's
    /// `req` parameter, and the free-function-shaped `args` this config declares (matching what
    /// the real generator has always emitted) hard-failed with "does not supply required
    /// parameter 'req'" on unmodified, previously-generating source.
    #[test]
    fn an_adapter_handled_method_with_no_visible_free_function_licenses_no_claim() {
        let type_defs = vec![TypeDef {
            name: "Handle".to_string(),
            methods: vec![adapter_handled_method("stream", vec![param("req")])],
            ..TypeDef::default()
        }];
        let e2e_config = E2eConfig {
            call: call_named("stream", vec![arg("handle", false), arg("url", false)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        // "rust" is deliberately included: `binding_excluded_for_language` always answers `false`
        // for "rust" (it carves Rust out unconditionally), so if "rust" is in the resolved set,
        // that pre-existing skip alone can never fire here -- only
        // `resolves_only_via_adapter_handled_method` can produce zero errors below. Asserting
        // against "python" alone would let the older, unrelated skip mask this fix's absence.
        let errors = validate_call_arg_signatures(
            &fixtures,
            &e2e_config,
            &[],
            &type_defs,
            &["python".to_string(), "rust".to_string()],
        );

        assert_eq!(
            errors.len(),
            0,
            "an adapter-handled method's own signature must not be asserted against a call whose \
             free-function sibling is invisible here: {errors:?}"
        );
    }

    /// The positive twin: when the free function IS visible in `functions` (unexcluded), it still
    /// wins per `CallIr::signature`'s own priority, and a genuinely wrong arg list against *that*
    /// signature must still be caught -- the adapter-handled method sharing its name must not make
    /// this check vacuous once the collision is resolvable.
    #[test]
    fn a_visible_free_function_still_validates_even_with_an_adapter_handled_namesake_method() {
        let functions = vec![function("stream", vec![param("handle"), param("url")])];
        let type_defs = vec![TypeDef {
            name: "Handle".to_string(),
            methods: vec![adapter_handled_method("stream", vec![param("req")])],
            ..TypeDef::default()
        }];
        let e2e_config = E2eConfig {
            call: call_named(
                "stream",
                vec![arg("handle", false), arg("url", false), arg("wrong_name", false)],
            ),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors =
            validate_call_arg_signatures(&fixtures, &e2e_config, &functions, &type_defs, &["python".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0].message.contains("arg 'wrong_name'"),
            "got: {}",
            errors[0].message
        );
    }

    /// `alef-tasks#350`'s fix must not regress: an *ordinary* `binding_excluded` method (no
    /// `[[crates.adapters]]` marker in its exclusion reason -- e.g. a plain `#[alef::skip]` for a
    /// cross-language reason) still gets a real, positionally-bound call from
    /// `src/e2e/codegen/rust/`, so a wrong arg name on it must still be caught when "rust" is
    /// resolved. Only the adapter-marked reason licenses skipping.
    #[test]
    fn a_plain_binding_excluded_method_without_an_adapter_reason_still_validates_for_rust() {
        let type_defs = vec![TypeDef {
            name: "Handle".to_string(),
            methods: vec![MethodDef {
                name: "stream".to_string(),
                params: vec![param("req")],
                return_type: TypeRef::String,
                binding_excluded: true,
                binding_exclusion_reason: Some("source binding exclusion".to_string()),
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }];
        let e2e_config = E2eConfig {
            call: call_named("stream", vec![arg("req", false), arg("wrong_name", false)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let errors = validate_call_arg_signatures(&fixtures, &e2e_config, &[], &type_defs, &["rust".to_string()]);

        assert_eq!(errors.len(), 1, "expected exactly one error, got: {errors:?}");
        assert!(
            errors[0].message.contains("arg 'wrong_name'"),
            "got: {}",
            errors[0].message
        );
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

    /// `alef-tasks#335`: this check is now `Severity::Error`, so a genuinely broken fixture
    /// must abort generation through [`enforce_call_arg_signatures`], not merely log. Sabotage
    /// coverage for the promotion: revert the severity literals to `Warning` and this test is
    /// the one that stops failing, proving it is load-bearing rather than vacuous.
    #[test]
    fn enforce_bails_when_an_arg_names_a_removed_parameter() {
        let functions = vec![function("complete", vec![param("prompt")])];
        let e2e_config = E2eConfig {
            call: call_named("complete", vec![arg("prompt", false), arg("concurrency", true)]),
            ..E2eConfig::default()
        };
        let fixtures = vec![fixture_with_call("basic", None)];

        let result = enforce_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["rust".to_string()]);

        let err = result.expect_err("a removed-parameter arg must abort generation");
        assert!(
            err.to_string().contains("fixture 'basic' arg 'concurrency'"),
            "got: {err}"
        );
    }

    /// The positive twin, built from the exact shapes the `alef-tasks#335` consumer-fleet
    /// survey found live across every repo pinning the next alef release: a required
    /// parameter supplied, an optional one omitted, and a `binding_excluded` call resolved
    /// only for a non-rust language. None of these may abort generation.
    #[test]
    fn enforce_does_not_bail_on_the_legitimate_patterns_the_fleet_survey_found() {
        let functions = vec![
            function("complete", vec![param("prompt"), optional_param("model")]),
            FunctionDef {
                binding_excluded: true,
                ..function("chat", vec![param("request")])
            },
        ];
        let e2e_config = {
            let mut config = E2eConfig {
                call: call_named("complete", vec![arg("prompt", false)]),
                ..E2eConfig::default()
            };
            config
                .calls
                .insert("chat".to_string(), call_named("chat", vec![arg("request", false)]));
            config
        };
        let fixtures = vec![
            fixture_with_call("basic", None),
            fixture_with_call("chat_basic", Some("chat")),
        ];

        let result = enforce_call_arg_signatures(&fixtures, &e2e_config, &functions, &[], &["python".to_string()]);

        assert!(result.is_ok(), "expected Ok(()), got: {result:?}");
    }
}
