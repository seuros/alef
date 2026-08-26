//! Dart's consumption of the FFI result-presence channel, in the opt-in `dart.style = "ffi"` mode.
//!
//! The default `frb` style is genuinely not a consumer: flutter_rust_bridge generates Dart from
//! alef's Rust facade, so an `Option<i64>` stays a Rust `Option` and lowers to a real Dart `null`.
//! Only raw `dart:ffi` crosses the C ABI, where `None` and a legitimate `Some(0)` are the same
//! bits — and where the typedef used to declare `Pointer<Void>` against an `int64_t` return, so
//! the value was read at the wrong width before presence even came into it.
//!
//! These tests assert against the **rendered Dart**, never against a host-side predicate: a test
//! that compared Dart's own notion of eligibility with the FFI backend's would pass even if the
//! emitter never consulted either.

use alef::backends::dart::DartBackend;
use alef::backends::ffi::type_map::result_presence_companion_exists;
use alef::core::backend::Backend;
use alef::core::config::{ResolvedCrateConfig, new_config::NewAlefConfig};
use alef::core::ir::{ApiSurface, FunctionDef, PrimitiveType, TypeRef};

/// The substring every generated reference to a presence companion contains. ~keep
const COMPANION_MARKER: &str = "HasResult";

fn config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["dart"]

[[crates]]
name = "demo-crate"
sources = ["src/lib.rs"]

[crates.dart]
style = "ffi"
"#,
    )
    .expect("test config must parse");
    cfg.resolve().expect("test config must resolve").remove(0)
}

fn optional(inner: TypeRef) -> TypeRef {
    TypeRef::Optional(Box::new(inner))
}

fn i64_option() -> TypeRef {
    optional(TypeRef::Primitive(PrimitiveType::I64))
}

fn free_function(name: &str, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("demo::{name}"),
        return_type,
        ..Default::default()
    }
}

fn surface(functions: Vec<FunctionDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "demo".to_string(),
        version: "0.1.0".to_string(),
        functions,
        ..Default::default()
    }
}

/// The `_ffi.dart` implementation file — the only one that names a C symbol.
fn render(api: &ApiSurface) -> String {
    DartBackend
        .generate_bindings(api, &config())
        .expect("dart:ffi bindings")
        .into_iter()
        .find(|file| file.path.to_string_lossy().ends_with("_ffi.dart"))
        .expect("the ffi implementation file")
        .content
}

/// The width fix the presence gate stands on. A gate over a call that reads an `int64_t` as a
/// `Pointer<Void>` would guard nothing — and `Option<bool>` was worse, a 4-byte `i32` declared as
/// an 8-byte pointer. `gen_ffi` never parses the cbindgen header, so nothing downstream catches
/// this. ~keep
#[test]
fn should_declare_an_optional_scalar_return_at_the_width_the_ffi_crate_exports() {
    let generated = render(&surface(vec![
        free_function("port", i64_option()),
        free_function("enabled", optional(TypeRef::Primitive(PrimitiveType::Bool))),
        free_function("label", optional(TypeRef::String)),
    ]));

    assert!(
        generated.contains("typedef _portNative = Int64 Function();"),
        "`Option<i64>` crosses as int64_t; got:\n{generated}"
    );
    assert!(
        generated.contains("typedef _portDart = int Function();"),
        "the Dart callable side must match; got:\n{generated}"
    );
    assert!(
        generated.contains("typedef _enabledNative = Int32 Function();"),
        "`Option<bool>` crosses as int32_t, not a pointer; got:\n{generated}"
    );
    assert!(
        generated.contains("typedef _labelNative = Pointer<Char> Function();"),
        "`Option<String>` crosses as char*; got:\n{generated}"
    );
    assert!(
        !generated.contains("Pointer<Void> Function();"),
        "no optional return may fall back to a pointer of the wrong width; got:\n{generated}"
    );
}

#[test]
fn should_look_up_the_presence_companion_for_an_optional_scalar_return() {
    let generated = render(&surface(vec![free_function("port", i64_option())]));

    assert!(
        generated.contains("typedef _portHasResultNative = Int32 Function();"),
        "the companion always returns i32 regardless of the primary's shape; got:\n{generated}"
    );
    assert!(
        generated.contains("'demo_crate_port_has_result'"),
        "the companion must bind the FFI crate's exported symbol; got:\n{generated}"
    );
}

/// The value this whole channel exists for: `Some(0)` and `None` are the same bits at the C
/// boundary, and only the companion can tell them apart. ~keep
#[test]
fn should_report_absent_as_null_and_keep_a_zero_valued_result_present() {
    let generated = render(&surface(vec![free_function("port", i64_option())]));

    assert!(
        generated.contains("int? port() {"),
        "expected a nullable Dart return type; got:\n{generated}"
    );
    assert!(
        generated.contains("  if (_portHasResultFn() != 1) {\n    return null;\n  }\n"),
        "absence must come from the companion; got:\n{generated}"
    );
    assert!(
        generated.contains("  return _result;\n"),
        "a present result, zero included, must be returned unchanged; got:\n{generated}"
    );
}

/// The companion clears the FFI crate's last-error slot on entry. Calling it after the primary
/// would erase an error the primary had just recorded. ~keep
#[test]
fn should_call_the_companion_before_the_primary_lookup() {
    let generated = render(&surface(vec![free_function("port", i64_option())]));

    let body_at = generated.find("int? port() {").expect("wrapper body");
    let body = &generated[body_at..];
    let companion_at = body.find("_portHasResultFn(").expect("companion call");
    let primary_at = body.find("final _result = _portFn(").expect("primary call");
    assert!(
        companion_at < primary_at,
        "the presence gate must run before the primary call; got:\n{generated}"
    );
}

#[test]
fn should_check_the_error_slot_inside_the_gate_when_the_wrapper_is_fallible() {
    let mut func = free_function("port", i64_option());
    func.error_type = Some("demo::Error".to_string());

    let generated = render(&surface(vec![func]));

    assert!(
        generated.contains("  if (_portHasResultFn() != 1) {\n    _checkError();\n    return null;\n  }\n"),
        "a fallible wrapper must report the companion's own failure rather than absence; \
         got:\n{generated}"
    );
}

#[test]
fn should_free_a_string_parameter_on_the_gates_early_return() {
    let mut func = free_function("port", i64_option());
    func.params = vec![alef::core::ir::ParamDef {
        name: "name".to_string(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: alef::core::ir::CoreWrapper::None,
    }];

    let generated = render(&surface(vec![func]));

    assert!(
        generated.contains("    calloc.free(nameNative);\n    return null;\n"),
        "the gate returns early, so it must run the teardown the wrapper would have run after \
         the call; got:\n{generated}"
    );
}

#[test]
fn should_not_reference_a_companion_for_a_pointer_shaped_optional_return() {
    let generated = render(&surface(vec![free_function("label", optional(TypeRef::String))]));

    assert!(
        !generated.contains(COMPANION_MARKER),
        "`Option<String>` already carries a real null pointer; got:\n{generated}"
    );
    assert!(
        generated.contains("_result == nullptr ? null :"),
        "a pointer-shaped optional must test its own sentinel; got:\n{generated}"
    );
}

/// Dart's decision to look up `{fn}_has_result` must equal the FFI backend's decision to export
/// it, for every return shape. `lookupFunction` resolves eagerly at top-level initialization, so a
/// symbol the crate never exported throws when the library loads and takes the whole binding down.
/// This compares the rendered Dart against the authority rather than restating the rule. ~keep
#[test]
fn dart_ffi_references_a_companion_exactly_when_the_ffi_backend_exports_one() {
    let shapes: Vec<TypeRef> = vec![
        i64_option(),
        optional(TypeRef::Primitive(PrimitiveType::U64)),
        optional(TypeRef::Primitive(PrimitiveType::Bool)),
        optional(TypeRef::Primitive(PrimitiveType::F64)),
        optional(TypeRef::Duration),
        optional(TypeRef::String),
        optional(TypeRef::Path),
        optional(TypeRef::Json),
        optional(TypeRef::Bytes),
        optional(TypeRef::Vec(Box::new(TypeRef::String))),
        TypeRef::Primitive(PrimitiveType::I64),
        TypeRef::String,
        TypeRef::Unit,
    ];

    for shape in &shapes {
        let generated = render(&surface(vec![free_function("probe", shape.clone())]));
        assert_eq!(
            generated.contains(COMPANION_MARKER),
            result_presence_companion_exists(shape, None),
            "free function returning {shape:?} disagreed with the FFI backend; got:\n{generated}"
        );
    }
}
