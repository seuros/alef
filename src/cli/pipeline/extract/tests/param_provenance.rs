//! Provenance coverage for `sanitize_param`, the parameter-side sibling of `sanitize_field`
//! (both private to `crate::cli::pipeline::extract::sanitizer`).
//!
//! Task #396 follow-up: only field rewrites populated `original_type` before the partner change
//! in `sanitizer.rs`. Parameter rewrites left it `None` even though several backends (`dart`,
//! `wasm`, `magnus`, `php`, `ffi`, `swift`, and the shared
//! `codegen::generators::binding_helpers` call-site builders) already gate reconstruction logic
//! on `param.original_type.is_some()` combined with `param.sanitized` -- those readers were
//! silently inert for every parameter.

use crate::cli::pipeline::extract::sanitizer::sanitize_unknown_types;
use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, ParamDef, TypeDef, TypeRef};

fn surface_with_method_param(param_ty: TypeRef) -> ApiSurface {
    ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![TypeDef {
            name: "Client".to_string(),
            rust_path: "sample::Client".to_string(),
            methods: vec![MethodDef {
                name: "configure".to_string(),
                params: vec![ParamDef {
                    name: "options".to_string(),
                    ty: param_ty,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Unit,
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    }
}

fn surface_with_function_param(param_ty: TypeRef) -> ApiSurface {
    ApiSurface {
        crate_name: "sample".to_string(),
        functions: vec![FunctionDef {
            name: "configure".to_string(),
            rust_path: "sample::configure".to_string(),
            params: vec![ParamDef {
                name: "options".to_string(),
                ty: param_ty,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    }
}

#[test]
fn a_method_param_rewritten_to_a_placeholder_records_its_original_type() {
    let mut api = surface_with_method_param(TypeRef::Named("ExternalOptions".to_string()));

    sanitize_unknown_types(&mut api);

    let param = &api.types[0].methods[0].params[0];
    assert!(
        param.sanitized,
        "an unknown named param type must still be flagged as a lossy rewrite"
    );
    assert_eq!(
        param.original_type.as_deref(),
        Some("ExternalOptions"),
        "the diagnostic needs the original Rust type name, not just the String placeholder"
    );
}

#[test]
fn a_function_param_rewritten_to_a_placeholder_records_its_original_type() {
    let mut api = surface_with_function_param(TypeRef::Named("ExternalOptions".to_string()));

    sanitize_unknown_types(&mut api);

    let param = &api.functions[0].params[0];
    assert!(
        param.sanitized,
        "an unknown named param type must still be flagged as a lossy rewrite"
    );
    assert_eq!(
        param.original_type.as_deref(),
        Some("ExternalOptions"),
        "the diagnostic needs the original Rust type name, not just the String placeholder"
    );
}

/// `original_type.is_some()` is not the same question as "was this rewritten" -- a lossless
/// fixed-array lowering also records `original_type` without setting `sanitized`. One function
/// carries a param of each shape so a reader can tell them apart from the recorded fields alone,
/// not from which placeholder the type happened to land on. ~keep
#[test]
fn a_lossy_and_a_lossless_param_rewrite_stay_distinguishable_by_the_sanitized_flag() {
    let mut api = ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![TypeDef {
            name: "Point".to_string(),
            rust_path: "sample::Point".to_string(),
            ..TypeDef::default()
        }],
        functions: vec![FunctionDef {
            name: "configure".to_string(),
            rust_path: "sample::configure".to_string(),
            params: vec![
                ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("ExternalOptions".to_string()),
                    ..ParamDef::default()
                },
                ParamDef {
                    name: "corners".to_string(),
                    ty: TypeRef::Named("[Point ; 4]".to_string()),
                    ..ParamDef::default()
                },
            ],
            return_type: TypeRef::Unit,
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };

    sanitize_unknown_types(&mut api);

    let lossy = &api.functions[0].params[0];
    let lossless = &api.functions[0].params[1];

    assert!(lossy.sanitized, "the unknown-type param is a lossy rewrite");
    assert_eq!(lossy.original_type.as_deref(), Some("ExternalOptions"));

    assert!(
        !lossless.sanitized,
        "lowering a fixed array of a known type to Vec<T> is lossless, so sanitized must stay false"
    );
    assert_eq!(
        lossless.original_type.as_deref(),
        Some("[Point ; 4]"),
        "the declared length is still erased by Vec<Point>, so it must be recorded even though this is lossless"
    );

    assert_ne!(
        lossy.sanitized, lossless.sanitized,
        "original_type.is_some() alone cannot distinguish these two params -- sanitized is the fact that does"
    );
}
