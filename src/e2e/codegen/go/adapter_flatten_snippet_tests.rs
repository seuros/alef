//! Prove `render_snippet_body` asks `adapter_target_params::flattened_stream_params` the same
//! question `gen_adapter_wrapper` answers, instead of re-deriving it.

use crate::core::config::{AdapterConfig, AdapterParam, AdapterPattern, ResolvedCrateConfig};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, ParamDef, TypeDef, TypeRef};
use crate::e2e::config::{ArgMapping, E2eConfig};
use crate::e2e::fixture::Fixture;

use super::snippet::render_snippet_body;

fn fixture(input: serde_json::Value) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "send_mode",
        "description": "Send with a mode",
        "input": input,
    }))
    .expect("Fixture's #[serde(default)] fills in every field this test does not set")
}

fn mode_arg() -> ArgMapping {
    ArgMapping {
        name: "mode".into(),
        field: "input.mode".into(),
        arg_type: "string".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn e2e_config(function: &str) -> E2eConfig {
    let mut e2e = E2eConfig::default();
    e2e.call.function = function.into();
    e2e.call.module = "example.com/sample".into();
    e2e.call.result_var = "result".into();
    e2e.call.returns_result = true;
    e2e.call.args = vec![mode_arg()];
    e2e
}

fn streaming_adapter(name: &str, params: Vec<AdapterParam>) -> AdapterConfig {
    AdapterConfig {
        name: name.to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: format!("sample::Engine::{name}"),
        params,
        returns: None,
        error_type: None,
        owner_type: Some("EngineHandle".to_string()),
        item_type: Some("sample::Event".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: Some("sample::SampleRequest".to_string()),
        skip_languages: Vec::new(),
    }
}

fn request_type_with_mode_field(mode_type: TypeRef) -> TypeDef {
    TypeDef {
        name: "SampleRequest".into(),
        fields: vec![FieldDef {
            name: "mode".into(),
            ty: mode_type,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }
}

fn sample_mode_enum() -> EnumDef {
    EnumDef {
        name: "SampleMode".into(),
        rust_path: "samplelib::SampleMode".into(),
        variants: vec![
            EnumVariant {
                name: "Fast".into(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Careful".into(),
                ..EnumVariant::default()
            },
        ],
        serde_rename_all: Some("snake_case".into()),
        ..EnumDef::default()
    }
}

/// The wrapping request DTO's own extracted Rust signature -- what a normal (non-adapter-aware)
/// `target_params` resolution would answer with for this call name, if it weren't intercepted.
/// Included so the fix's test does not merely benefit from an absent/unresolvable IR (which
/// would also fall back to a raw literal for an unrelated reason) but from a *present, resolved,
/// wrong* signature that `flattened_stream_params` must actually out-rank. ~keep
fn wrapping_request_function(name: &str) -> FunctionDef {
    FunctionDef {
        name: name.into(),
        params: vec![ParamDef {
            name: "request".into(),
            ty: TypeRef::Named("SampleRequest".into()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("SampleResponse".into()),
        ..FunctionDef::default()
    }
}

fn call_line(body: &str) -> &str {
    body.lines()
        .find(|line| line.trim_start().starts_with("result, err :="))
        .expect("a fallible call always binds `result, err :=`")
}

/// The fix: a flattening streaming adapter's docs snippet must render the flattened field's
/// *own* type (the enum), not the wrapping request DTO `target_params` would otherwise resolve
/// to -- which cannot name a Go enum constant, so without this fix rendering falls back to a raw
/// fixture literal the real (decomposed) Go parameter does not accept.
#[test]
fn flattening_adapter_renders_the_flattened_fields_enum_constant() {
    let config = ResolvedCrateConfig {
        adapters: vec![streaming_adapter(
            "send",
            vec![AdapterParam {
                name: "request".into(),
                ty: "SampleRequest".into(),
                optional: false,
            }],
        )],
        ..ResolvedCrateConfig::default()
    };
    let type_defs = [request_type_with_mode_field(TypeRef::Named("SampleMode".into()))];
    let enums = [sample_mode_enum()];
    let functions = [wrapping_request_function("send")];

    let body = render_snippet_body(
        &fixture(serde_json::json!({"mode": "careful"})),
        &e2e_config("send"),
        &config,
        &type_defs,
        &enums,
        &functions,
    )
    .expect("snippet renders");

    assert_eq!(call_line(&body), "\tresult, err := pkg.Send(pkg.SampleModeCareful)");
}

/// Control: a flattening adapter whose flattened field is a plain scalar must render byte-
/// identically to a call with no adapter involvement at all -- `TypeRef::String` names no IR
/// type either way, so the synthetic parameter this fix introduces changes nothing here.
#[test]
fn flattening_adapter_with_a_scalar_field_is_unchanged() {
    let config = ResolvedCrateConfig {
        adapters: vec![streaming_adapter(
            "send",
            vec![AdapterParam {
                name: "request".into(),
                ty: "SampleRequest".into(),
                optional: false,
            }],
        )],
        ..ResolvedCrateConfig::default()
    };
    let type_defs = [request_type_with_mode_field(TypeRef::String)];

    let body = render_snippet_body(
        &fixture(serde_json::json!({"mode": "careful"})),
        &e2e_config("send"),
        &config,
        &type_defs,
        &[],
        &[],
    )
    .expect("snippet renders");

    assert_eq!(call_line(&body), "\tresult, err := pkg.Send(`careful`)");
}

/// Else branch, at the e2e boundary: a two-param (non-flattening) adapter contributes no
/// synthetic parameter, so rendering falls back to `recipe.target_params` exactly as it did
/// before this fix.
#[test]
fn non_flattening_two_param_adapter_falls_back_unchanged() {
    let config = ResolvedCrateConfig {
        adapters: vec![streaming_adapter(
            "send",
            vec![
                AdapterParam {
                    name: "mode".into(),
                    ty: "String".into(),
                    optional: false,
                },
                AdapterParam {
                    name: "depth".into(),
                    ty: "u32".into(),
                    optional: false,
                },
            ],
        )],
        ..ResolvedCrateConfig::default()
    };
    let type_defs = [request_type_with_mode_field(TypeRef::Named("SampleMode".into()))];
    let enums = [sample_mode_enum()];

    let body = render_snippet_body(
        &fixture(serde_json::json!({"mode": "careful"})),
        &e2e_config("send"),
        &config,
        &type_defs,
        &enums,
        &[],
    )
    .expect("snippet renders");

    // No `functions` entry names `send`, so `recipe.target_params` is `Unresolvable` -- the
    // same raw-literal outcome as before this fix existed.
    assert_eq!(call_line(&body), "\tresult, err := pkg.Send(`careful`)");
}

/// Non-streaming control: an adapter list that does not match this call's name at all must
/// leave rendering untouched -- lookup by name is the only thing `flattened_stream_params`
/// contributes.
#[test]
fn call_matching_no_adapter_is_unchanged() {
    let config = ResolvedCrateConfig {
        adapters: vec![streaming_adapter(
            "unrelated_stream",
            vec![AdapterParam {
                name: "request".into(),
                ty: "SampleRequest".into(),
                optional: false,
            }],
        )],
        ..ResolvedCrateConfig::default()
    };
    let type_defs = [request_type_with_mode_field(TypeRef::Named("SampleMode".into()))];
    let enums = [sample_mode_enum()];
    let functions = [wrapping_request_function("send")];

    let body = render_snippet_body(
        &fixture(serde_json::json!({"mode": "careful"})),
        &e2e_config("send"),
        &config,
        &type_defs,
        &enums,
        &functions,
    )
    .expect("snippet renders");

    // `send`'s real IR signature takes the whole `SampleRequest`, and the fixture value is a
    // bare string, not an object, so the typed-literal renderer still refuses to spell it as
    // that struct: the raw literal survives exactly as it does with an absent adapter list.
    assert_eq!(call_line(&body), "\tresult, err := pkg.Send(`careful`)");
}
