//! alef #102 follow-up: two Magnus trait-bridge generators each hand-recomputed a subset of
//! `has_error` (just `func.error_type.is_some()`), independent of a `?` their own body emits
//! unconditionally elsewhere. A function with no declared error type then got a bare-`T`
//! signature wrapped around a body containing a `?`, which rustc rejects with E0277.
//!
//! - `gen_options_field_bridge_function` always parses its `args: &[magnus::Value]` through
//!   `scan_args::<...>(args)?` — that `?` fires regardless of `func.error_type`.
//! - `gen_bridge_function` emits `serde_json::from_str(..)?` / `.transpose()?` for any non-opaque
//!   `Named`/`Optional<Named>` parameter that isn't a configured "default type" — gated on
//!   parameter shape, never on `func.error_type`.

use alef::backends::magnus::trait_bridge::{gen_bridge_function, gen_options_field_bridge_function};
use alef::codegen::type_mapper::IdentityMapper;
use alef::core::config::{BridgeBinding, TraitBridgeConfig};
use alef::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef};

fn function_param_bridge_config() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "Visitor".to_string(),
        type_alias: Some("VisitorHandle".to_string()),
        bind_via: BridgeBinding::FunctionParam,
        ..Default::default()
    }
}

fn options_field_bridge_config() -> TraitBridgeConfig {
    TraitBridgeConfig {
        trait_name: "Visitor".to_string(),
        type_alias: Some("VisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        bind_via: BridgeBinding::OptionsField,
        options_type: Some("RenderOptions".to_string()),
        options_field: Some("visitor".to_string()),
        ..Default::default()
    }
}

/// A bridge-param function with no declared error type, whose one non-bridge param
/// (`options: RenderOptions`) is a non-opaque `Named` type that isn't a configured "default
/// type" — so `gen_bridge_function` must emit a fallible `serde_json::from_str(..)?` deser
/// binding for it, independent of `error_type`.
fn render_document_function() -> FunctionDef {
    FunctionDef {
        name: "render_document".to_string(),
        rust_path: "sample_core::render_document".to_string(),
        params: vec![
            ParamDef {
                name: "visitor".to_string(),
                ty: TypeRef::Named("VisitorHandle".to_string()),
                ..ParamDef::default()
            },
            ParamDef {
                name: "options".to_string(),
                ty: TypeRef::Named("RenderOptions".to_string()),
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::String,
        error_type: None,
        ..FunctionDef::default()
    }
}

#[test]
fn bridge_function_without_error_type_stays_result_shaped_when_a_param_needs_fallible_deser() {
    let code = gen_bridge_function(
        &ApiSurface::default(),
        &render_document_function(),
        0,
        &function_param_bridge_config(),
        &IdentityMapper,
        &ahash::AHashSet::new(),
        &std::collections::HashSet::new(),
        "sample_core",
    );

    assert!(
        code.contains("-> Result<"),
        "a param needing fallible deser must force a Result-shaped signature even with no \
         declared error type, got: {code}"
    );
    assert!(
        code.contains("serde_json::from_str(&options)") && code.contains('?'),
        "the deser binding for a non-default-type param must stay fallible, got: {code}"
    );
    assert!(
        code.contains("Ok("),
        "the tail expression must be Ok(..)-wrapped to fit the Result-shaped signature, got: {code}"
    );
}

/// The same function, but `options` becomes an opaque type (skips deser entirely) and there's no
/// other fallible source — the signature must stay bare (not our concern, but pins the "should
/// still special-case the no-fallibility path" behavior so a future change doesn't regress it).
#[test]
fn bridge_function_without_error_type_and_without_fallible_params_stays_bare() {
    let mut func = render_document_function();
    func.params[1].ty = TypeRef::Primitive(alef::core::ir::PrimitiveType::U32);
    func.params[1].name = "count".to_string();

    let code = gen_bridge_function(
        &ApiSurface::default(),
        &func,
        0,
        &function_param_bridge_config(),
        &IdentityMapper,
        &ahash::AHashSet::new(),
        &std::collections::HashSet::new(),
        "sample_core",
    );

    assert!(
        !code.contains("-> Result<"),
        "no error type and no fallible param must keep a bare return, got: {code}"
    );
    assert!(
        !code.contains('?'),
        "a bare-return body must not contain a `?`, got: {code}"
    );
}

#[test]
fn options_field_bridge_function_without_error_type_stays_result_shaped() {
    let mut func = render_document_function();
    func.name = "render_document_via_options".to_string();
    func.params = vec![ParamDef {
        name: "options".to_string(),
        ty: TypeRef::Named("RenderOptions".to_string()),
        ..ParamDef::default()
    }];

    let code = gen_options_field_bridge_function(
        &ApiSurface::default(),
        &func,
        0,
        &options_field_bridge_config(),
        &IdentityMapper,
        &ahash::AHashSet::new(),
        "sample_core",
    );

    assert!(
        code.contains("-> Result<"),
        "this generator always emits an unconditional `scan_args::<...>(args)?`, so it must stay \
         Result-shaped even with no declared error type, got: {code}"
    );
    assert!(
        code.contains(">(args)?;"),
        "the scan_args parse must stay fallible, got: {code}"
    );
    assert!(
        code.contains("Ok("),
        "the tail expression must be Ok(..)-wrapped to fit the Result-shaped signature, got: {code}"
    );
}
