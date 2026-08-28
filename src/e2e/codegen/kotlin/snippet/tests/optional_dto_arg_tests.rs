//! Docs-snippet rendering for an optional DTO (`json_object`) argument the fixture omits.
//!
//! The emitter has three answers available for "the fixture supplied no value for this optional
//! argument", and only one of them is safe without knowing more than this module knows:
//!
//! - `null` — compiles for any parameter the generated Kotlin declares `T? = null`, which is
//!   exactly the set the core IR marks `Option<T>`. Both Kotlin emitters agree on that mapping
//!   (`kotlin_android`'s `facade_param`, and `object_wrapper::methods`' `if p.optional`), and
//!   `ParamOptionalityRule::DeclaredType` reads the same `ParamDef::optional` field they do, so
//!   the three cannot drift.
//! - `OptionsType()` — compiles only for a type in
//!   `backends::kotlin::default_constructible_type_names`.
//! - `OptionsType.builder().build()` — compiles only for a Java record that
//!   `backends::java::gen_bindings::types::builders::should_emit_builder` gave a builder factory.
//!
//! Neither of the latter two authorities is reachable from the argument builder, so emitting
//! those forms for a parameter that never needed an argument at all is a guess that fails in a
//! consumer's kotlinc run with `No value passed for parameter 'x'` or
//! `unresolved reference: builder`. These tests drive the real snippet entry point with a real
//! `FunctionDef` so the declared-optionality question is actually asked. ~keep

use super::*;
use crate::e2e::config::{ArgMapping, CallConfig, CallOverride};

fn dto_arg() -> ArgMapping {
    ArgMapping {
        name: "config".into(),
        field: "config".into(),
        arg_type: "json_object".into(),
        optional: true,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn source_arg() -> ArgMapping {
    ArgMapping {
        name: "source".into(),
        field: "source".into(),
        arg_type: "string".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

/// `extract_bytes(source: String, config: Option<ExtractionConfig>)` — the declared parameter is
/// genuinely `Option<T>`, so every Kotlin emitter gives it `= null`.
fn functions_with_optional_config() -> Vec<crate::core::ir::FunctionDef> {
    vec![crate::core::ir::FunctionDef {
        name: "extract_bytes".into(),
        params: vec![
            crate::core::ir::ParamDef {
                name: "source".into(),
                ty: crate::core::ir::TypeRef::String,
                optional: false,
                ..crate::core::ir::ParamDef::default()
            },
            crate::core::ir::ParamDef {
                name: "config".into(),
                ty: crate::core::ir::TypeRef::Named("ExtractionConfig".into()),
                optional: true,
                ..crate::core::ir::ParamDef::default()
            },
        ],
        return_type: crate::core::ir::TypeRef::String,
        ..crate::core::ir::FunctionDef::default()
    }]
}

fn omitted_config_call() -> CallConfig {
    let mut call = CallConfig {
        function: "extract_bytes".into(),
        result_var: "result".into(),
        args: vec![source_arg(), dto_arg()],
        ..CallConfig::default()
    };
    for language in ["kotlin", "kotlin_android"] {
        call.overrides.insert(
            language.into(),
            CallOverride {
                options_type: Some("ExtractionConfig".into()),
                ..CallOverride::default()
            },
        );
    }
    call
}

fn omitted_config_fixture() -> Fixture {
    Fixture {
        id: "extract_bytes_basic".into(),
        description: "Extract with the default configuration".into(),
        input: serde_json::json!({"source": "doc.pdf"}),
        ..Fixture::default()
    }
}

fn render(kotlin_android_style: bool, functions: &[crate::core::ir::FunctionDef]) -> String {
    render_snippet_body_with_ir(
        &omitted_config_fixture(),
        &E2eConfig {
            call: omitted_config_call(),
            ..E2eConfig::default()
        },
        &ResolvedCrateConfig {
            name: "sample_api".into(),
            ..ResolvedCrateConfig::default()
        },
        &[],
        &[],
        kotlin_android_style,
        functions,
    )
    .expect("snippet renders")
}

/// Regression: `ExtractionConfig()` requires every emitted constructor parameter to carry a
/// Kotlin default — the exact condition `default_constructible_type_names` was added to decide,
/// and which the argument builder cannot see. When the IR says the parameter is `Option<T>`,
/// `facade_param` has already declared it `config: ExtractionConfig? = null`, so `null` needs no
/// constructor to exist and is the value the signature itself defaults to.
#[test]
fn android_snippet_passes_null_for_an_omitted_argument_the_target_declares_optional() {
    let body = render(true, &functions_with_optional_config());

    assert!(
        body.contains("SampleApi.extractBytes(\"doc.pdf\", null)"),
        "a declared-optional DTO parameter must be filled with null, not a constructor: {body}"
    );
    assert!(
        !body.contains("ExtractionConfig()"),
        "must not guess that ExtractionConfig is default-constructible: {body}"
    );
}

/// The JVM half. Here the DTO is the Java record reached by typealias, and
/// `should_emit_builder` — not this module — decides whether `ExtractionConfig.builder()` exists
/// at all (`JavaBuilderMode::Never`, or `Auto` for a non-`has_serde` type under the field-count
/// thresholds, emits no builder factory). `null` is correct without asking, because
/// `object_wrapper::methods` declared the Kotlin wrapper's optional parameter `= null`.
#[test]
fn jvm_snippet_passes_null_rather_than_a_builder_the_backend_may_not_have_emitted() {
    let body = render(false, &functions_with_optional_config());

    assert!(
        body.contains("SampleApi.extractBytes(\"doc.pdf\", null)"),
        "a declared-optional DTO parameter must be filled with null, not a builder chain: {body}"
    );
    assert!(
        !body.contains("builder()"),
        "must not guess that a builder factory was emitted for ExtractionConfig: {body}"
    );
}

/// The control case, and the reason this is a narrowing rather than a revert of `7b1788ff3`.
///
/// With no IR to consult, `TargetParams::IrAbsent` licenses no claim about the target, so the
/// constructor fallback stays exactly as it was — this is the state
/// `tests/e2e_kotlin_android_optional_config_arg.rs` renders in, and it must not move.
#[test]
fn android_snippet_keeps_the_constructor_fallback_when_no_ir_is_in_scope() {
    let body = render(true, &[]);

    assert!(
        body.contains("SampleApi.extractBytes(\"doc.pdf\", ExtractionConfig())"),
        "without IR the pre-existing constructor fallback must be unchanged: {body}"
    );
}

/// The other control case: a fixture-optional argument the IR proves is *required* keeps
/// `f34fefff9`'s typed-default behaviour rather than falling into the new `null` branch.
#[test]
fn android_snippet_still_refuses_null_for_a_declared_required_argument() {
    let functions = vec![crate::core::ir::FunctionDef {
        name: "extract_bytes".into(),
        params: vec![
            crate::core::ir::ParamDef {
                name: "source".into(),
                ty: crate::core::ir::TypeRef::String,
                optional: false,
                ..crate::core::ir::ParamDef::default()
            },
            crate::core::ir::ParamDef {
                name: "config".into(),
                ty: crate::core::ir::TypeRef::Named("ExtractionConfig".into()),
                optional: false,
                ..crate::core::ir::ParamDef::default()
            },
        ],
        return_type: crate::core::ir::TypeRef::String,
        ..crate::core::ir::FunctionDef::default()
    }];
    let body = render(true, &functions);

    assert!(
        body.contains("SampleApi.extractBytes(\"doc.pdf\", ExtractionConfig())"),
        "a declared-required DTO parameter still needs a positional value: {body}"
    );
    assert!(
        !body.contains(", null)"),
        "a declared-required Kotlin parameter must never receive a bare null: {body}"
    );
}
