// Separate file rather than adding to `regressions.rs` (already over the 1,000-line
// file-modularization cap) so this method-parameter enum coverage does not push it further
// past the limit. ~keep
use super::super::FfiBackend;
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

/// A data-carrying enum (`Verdict`) reached ONLY through a method parameter — no free
/// function ever takes or returns it. `lib_rs.rs`'s `enum_pointer_param` set used to be built
/// exclusively from `api.functions[].params`, so an enum that only ever crosses through
/// `TypeDef::methods[].params` was invisible to it and got no `<prefix>_verdict_from_json`,
/// even though the C# backend (and any other C-ABI consumer) declares the DllImport for it
/// unconditionally whenever the enum is `has_serde` and reaches any parameter position. That
/// mismatch is a live `EntryPointNotFoundException`. `has_serde: true` makes `Verdict` eligible
/// for the JSON companion; the `Rejected { reason }` variant makes it data-carrying, so it
/// crosses the ABI as `AlefHandle`, not a scalar `int32_t` (fieldless `is_copy` enums are
/// intentionally exempt from `_from_json` and must stay that way). ~keep
fn method_param_only_enum_api() -> ApiSurface {
    let verdict = EnumDef {
        name: "Verdict".to_string(),
        rust_path: "my_lib::Verdict".to_string(),
        variants: vec![
            EnumVariant {
                name: "Approved".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Rejected".to_string(),
                fields: vec![visitor_result_string_field("reason")],
                ..EnumVariant::default()
            },
        ],
        has_serde: true,
        ..EnumDef::default()
    };
    let reviewer = TypeDef {
        name: "Reviewer".to_string(),
        rust_path: "my_lib::Reviewer".to_string(),
        methods: vec![MethodDef {
            name: "explain".to_string(),
            params: vec![ParamDef {
                name: "verdict".to_string(),
                ty: TypeRef::Named("Verdict".to_string()),
                ..ParamDef::default()
            }],
            return_type: TypeRef::String,
            receiver: Some(ReceiverKind::Ref),
            ..MethodDef::default()
        }],
        ..TypeDef::default()
    };
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        enums: vec![verdict],
        types: vec![reviewer],
        ..ApiSurface::default()
    }
}

/// Regression for the method-parameter blind spot in `enum_pointer_param`: a data-carrying
/// enum reached only via `TypeDef::methods[].params` must still get `<prefix>_verdict_from_json`
/// (and its paired `_free`), matching what the C# backend declares for any `has_serde`
/// parameter-position enum regardless of whether it arrived through a free function or a
/// method.
#[test]
fn enum_reached_only_via_method_parameter_still_gets_from_json() {
    let api = method_param_only_enum_api();
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content
            .contains("fn my_lib_verdict_from_json(json: *const c_char) -> AlefHandle"),
        "a data-carrying enum reached only through a method parameter must still get \
         `_from_json`, got:\n{}",
        lib.content
    );
    assert!(
        lib.content.contains("fn my_lib_verdict_free(handle: AlefHandle)"),
        "`_from_json`'s paired `_free` must also be emitted, got:\n{}",
        lib.content
    );

    let explain = lib
        .content
        .split("fn my_lib_reviewer_explain")
        .nth(1)
        .expect("explain wrapper must be emitted");
    assert!(
        explain.contains("verdict: AlefHandle"),
        "the method parameter itself must already take the scalar handle, got:\n{explain}"
    );

    syn::parse_file(&lib.content).expect("method-parameter enum wiring must parse as valid Rust");
}
