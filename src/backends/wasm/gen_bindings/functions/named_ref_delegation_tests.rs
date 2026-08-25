//! Free functions whose only non-delegatable params are non-opaque `&Named` / `&[Named]`
//! references must delegate to the core call, not fall through to `gen_wasm_unimplemented_body`.

use super::orchestration::gen_function_with_emitted_dtos;
use crate::backends::wasm::type_map::WasmMapper;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FieldDef, FunctionDef, ParamDef, PrimitiveType, TypeDef, TypeRef};
use ahash::AHashSet;
use std::collections::HashMap;

fn named_ref_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Named(type_name.to_string()),
        is_ref: true,
        ..ParamDef::default()
    }
}

fn slice_of_named_param(name: &str, type_name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named(type_name.to_string()))),
        is_ref: true,
        ..ParamDef::default()
    }
}

fn render(func: &FunctionDef) -> String {
    gen_function_with_emitted_dtos(
        func,
        &WasmMapper::new(HashMap::new(), "Wasm".to_string()),
        "sample_fixture",
        &AHashSet::new(),
        "Wasm",
        &AHashSet::new(),
        &ApiSurface::default(),
        &AHashSet::new(),
    )
}

/// Regression test for a shipped defect: a free function whose only non-delegatable params are
/// required non-opaque `&Named` references, returning a bare non-fallible primitive, fell through
/// to `gen_wasm_unimplemented_body` and emitted `compile_error!` into the consumer's default build
/// path. The NAPI backend — which drives the same `gen_named_let_bindings_no_promote` +
/// `gen_call_args_with_let_bindings` pair — delegates the identical IR correctly.
#[test]
fn required_named_ref_params_with_primitive_return_delegate_instead_of_compile_error() {
    let func = FunctionDef {
        name: "score_pair".to_string(),
        rust_path: "sample_fixture::score_pair".to_string(),
        params: vec![
            named_ref_param("query", "SampleVector"),
            named_ref_param("candidate", "SampleVector"),
        ],
        return_type: TypeRef::Primitive(PrimitiveType::F64),
        ..FunctionDef::default()
    };

    let out = render(&func);

    assert!(
        !out.contains("compile_error!"),
        "a required &Named param must not force compile_error! for a non-fallible return:\n{out}"
    );
    assert!(
        out.contains("let query_core: sample_fixture::SampleVector = query.into();"),
        "expected an owned core let-binding for the borrowed param:\n{out}"
    );
    assert!(
        out.contains("sample_fixture::score_pair(&query_core, &candidate_core)"),
        "expected delegation to the real core call:\n{out}"
    );
}

/// The `&[Named]` (slice) shape is rejected by the same `is_named_ref_param` guard. Its
/// let-binding (`Vec<_>` built with `map(Into::into)`) and call argument (`&{name}_core`, which
/// deref-coerces to `&[T]`) are already generated, so it must delegate too.
#[test]
fn named_ref_and_slice_params_with_vec_return_delegate_instead_of_compile_error() {
    let func = FunctionDef {
        name: "rank_candidates".to_string(),
        rust_path: "sample_fixture::rank_candidates".to_string(),
        params: vec![
            named_ref_param("query", "SampleVector"),
            slice_of_named_param("candidates", "SampleVector"),
        ],
        return_type: TypeRef::Vec(Box::new(TypeRef::Named("SampleMatch".to_string()))),
        ..FunctionDef::default()
    };

    let out = render(&func);

    assert!(
        !out.contains("compile_error!"),
        "a &[Named] slice param must not force compile_error! for a non-fallible return:\n{out}"
    );
    assert!(
        out.contains("let candidates_core: Vec<_> = candidates.into_iter().map(Into::into).collect();"),
        "expected an owned core Vec let-binding for the slice param:\n{out}"
    );
    assert!(
        out.contains("sample_fixture::rank_candidates(&query_core, &candidates_core)"),
        "expected delegation to the real core call:\n{out}"
    );
}

/// Negative control: a function the backend already supported must still be emitted the same way,
/// so the relaxed gate cannot be passing simply because it now accepts everything.
#[test]
fn plain_delegatable_free_function_is_still_emitted_normally() {
    let func = FunctionDef {
        name: "count_items".to_string(),
        rust_path: "sample_fixture::count_items".to_string(),
        params: vec![ParamDef {
            name: "label".to_string(),
            ty: TypeRef::String,
            is_ref: true,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Primitive(PrimitiveType::U32),
        ..FunctionDef::default()
    };

    let out = render(&func);

    assert!(
        !out.contains("compile_error!"),
        "a supported function must delegate:\n{out}"
    );
    assert!(
        out.contains("sample_fixture::count_items(&label)"),
        "expected the unchanged direct call args:\n{out}"
    );
}

fn wasm_crate_config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["wasm"]
[[crates]]
name = "sample-fixture"
sources = ["src/lib.rs"]
[crates.wasm]
"#,
    )
    .expect("valid fixture config");
    cfg.resolve().expect("resolvable fixture config").remove(0)
}

fn sample_vector_type() -> TypeDef {
    TypeDef {
        name: "SampleVector".to_string(),
        rust_path: "sample_fixture::SampleVector".to_string(),
        fields: vec![FieldDef {
            name: "dimension".to_string(),
            ty: TypeRef::Primitive(PrimitiveType::U32),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }
}

/// Artifact-level control: the same fixture rendered through the real `Backend::generate_bindings`
/// path must produce a whole `lib.rs` with no `compile_error!` anywhere in it.
#[test]
fn generated_lib_rs_carries_no_compile_error_for_named_ref_free_functions() {
    let api = ApiSurface {
        crate_name: "sample-fixture".to_string(),
        version: "0.1.0".to_string(),
        types: vec![sample_vector_type()],
        functions: vec![FunctionDef {
            name: "score_pair".to_string(),
            rust_path: "sample_fixture::score_pair".to_string(),
            params: vec![
                named_ref_param("query", "SampleVector"),
                named_ref_param("candidate", "SampleVector"),
            ],
            return_type: TypeRef::Primitive(PrimitiveType::F64),
            ..FunctionDef::default()
        }],
        ..ApiSurface::default()
    };

    let files = crate::backends::wasm::gen_bindings::WasmBackend
        .generate_bindings(&api, &wasm_crate_config())
        .expect("wasm bindings generate");
    let lib_rs = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .map(|f| f.content.clone())
        .expect("generated lib.rs");

    assert!(
        !lib_rs.contains("compile_error"),
        "generated lib.rs must not contain compile_error:\n{lib_rs}"
    );
    assert!(
        lib_rs.contains("sample_fixture::score_pair(&query_core, &candidate_core)"),
        "generated lib.rs must delegate to the core call:\n{lib_rs}"
    );
}

/// Negative control for the deliberate loud-failure design: a genuinely non-delegatable function
/// (a sanitized signature with a non-fallible return) must still emit `compile_error!` rather than
/// fabricate a value. The relaxation above must not swallow that path.
#[test]
fn sanitized_free_function_with_infallible_return_still_fails_loudly() {
    let func = FunctionDef {
        name: "describe_item".to_string(),
        rust_path: "sample_fixture::describe_item".to_string(),
        params: vec![named_ref_param("item", "SampleVector")],
        return_type: TypeRef::String,
        sanitized: true,
        ..FunctionDef::default()
    };

    let out = render(&func);

    assert!(
        out.contains("compile_error!"),
        "a sanitized non-fallible function must still fail the build loudly:\n{out}"
    );
}
