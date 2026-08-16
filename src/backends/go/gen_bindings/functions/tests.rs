use super::*;
use crate::core::ir::{ParamDef, PrimitiveType, TypeRef};

fn make_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
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
        core_wrapper: crate::core::ir::CoreWrapper::None,
    }
}

#[test]
fn test_params_require_marshal_for_named_non_opaque() {
    let params = vec![make_param("options", TypeRef::Named("Config".to_string()))];
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    assert!(params_require_marshal(&params, &opaque));
}

#[test]
fn test_params_require_marshal_false_for_opaque() {
    let params = vec![make_param("client", TypeRef::Named("Client".to_string()))];
    let opaque: std::collections::HashSet<&str> = ["Client"].into();
    assert!(!params_require_marshal(&params, &opaque));
}

#[test]
fn test_is_bridge_param_matches_by_name() {
    let param = make_param("visitor", TypeRef::Named("VisitorHandle".to_string()));
    let bridge_names: HashSet<String> = ["visitor".to_string()].into();
    let aliases: HashSet<String> = HashSet::new();
    assert!(is_bridge_param(&param, &bridge_names, &aliases));
}

#[test]
fn test_params_require_marshal_for_vec() {
    let params = vec![make_param(
        "items",
        TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
    )];
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    assert!(params_require_marshal(&params, &opaque));
}

fn make_bytes_result_func(name: &str, with_bytes_param: bool) -> FunctionDef {
    let params = if with_bytes_param {
        vec![ParamDef {
            name: "data".to_string(),
            ty: TypeRef::Bytes,
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
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }]
    } else {
        vec![]
    };
    FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params,
        return_type: TypeRef::Bytes,
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn make_bytes_result_method(name: &str) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        doc: String::new(),
        params: vec![ParamDef {
            name: "data".to_string(),
            ty: TypeRef::Bytes,
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
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Bytes,
        is_static: false,
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
        receiver: None,
        sanitized: false,
        trait_source: None,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        has_default_impl: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

#[test]
fn test_is_bytes_result_func_detects_bytes_with_error() {
    let func = make_bytes_result_func("process_image", true);
    assert!(is_bytes_result_func(&func));
}

#[test]
fn test_is_bytes_result_func_detects_bytes_without_error() {
    let mut func = make_bytes_result_func("get_data", false);
    func.error_type = None;
    assert!(is_bytes_result_func(&func));
}

#[test]
fn test_gen_function_wrapper_named_handle_error_path_uses_zero_sentinel() {
    let mut func = make_bytes_result_func("load_config", true);
    func.return_type = TypeRef::Named("Config".to_string());
    let empty_refs = HashSet::new();
    let empty_strings = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "sample",
        &empty_refs,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    assert!(out.contains("if ptr != 0"));
    assert!(!out.contains("if ptr != nil"));
}

#[test]
fn test_is_bytes_result_func_false_for_string_with_error() {
    let func = FunctionDef {
        name: "get_text".to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::String,
        is_async: false,
        error_type: Some("SampleCrateError".to_string()),
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    assert!(!is_bytes_result_func(&func));
}

#[test]
fn test_is_bytes_result_method_detects_correctly() {
    let method = make_bytes_result_method("render_page");
    assert!(is_bytes_result_method(&method));
}

#[test]
fn test_gen_function_wrapper_bytes_result_emits_out_params() {
    let func = make_bytes_result_func("process_image", true);
    let opaque: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let bridge_names: HashSet<String> = HashSet::new();
    let bridge_aliases: HashSet<String> = HashSet::new();
    let value_only_types: HashSet<String> = HashSet::new();
    let enum_names: HashSet<String> = HashSet::new();
    let ffi_param_enum_names: HashSet<String> = HashSet::new();
    let reserved_type_names: HashSet<String> = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "krz",
        &opaque,
        &bridge_names,
        &bridge_aliases,
        &value_only_types,
        &enum_names,
        &ffi_param_enum_names,
        &reserved_type_names,
    );
    assert!(out.contains("([]byte, error)"), "missing bytes return type in:\n{out}");
    assert!(out.contains("var outPtr"), "missing outPtr in:\n{out}");
    assert!(out.contains("outLen"), "missing outLen in:\n{out}");
    assert!(out.contains("outCap"), "missing outCap in:\n{out}");
    assert!(out.contains("&outPtr"), "missing &outPtr in:\n{out}");
    assert!(out.contains("&outLen"), "missing &outLen in:\n{out}");
    assert!(out.contains("&outCap"), "missing &outCap in:\n{out}");
    assert!(out.contains("C.GoBytes"), "missing C.GoBytes in:\n{out}");
    assert!(out.contains("krz_free_bytes"), "missing krz_free_bytes in:\n{out}");
}

#[test]
fn test_gen_function_wrapper_infallible_bytes_uses_owned_buffer_abi() {
    let mut func = make_bytes_result_func("read_file", false);
    func.error_type = None;
    let empty_refs = HashSet::new();
    let empty_strings = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "sample",
        &empty_refs,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    assert!(out.contains("&outPtr, &outLen, &outCap"));
    assert!(out.contains("C.GoBytes"));
    assert!(!out.contains("unmarshalBytes"));
}

fn make_capsule_func(name: &str, fallible: bool) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params: vec![make_param("name", TypeRef::String)],
        return_type: TypeRef::Named("Language".to_string()),
        is_async: false,
        error_type: if fallible {
            Some("SampleCrateError".to_string())
        } else {
            None
        },
        doc: String::new(),
        cfg: None,
        sanitized: false,
        return_sanitized: false,
        returns_ref: false,
        returns_cow: false,
        return_newtype_wrapper: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    }
}

fn capsule_cfg() -> crate::core::config::HostCapsuleTypeConfig {
    crate::core::config::HostCapsuleTypeConfig {
        host_type: "*my_pkg.Language".to_string(),
        package: "github.com/example/go-my-lib".to_string(),
        package_version: "v1.0.0".to_string(),
        construct_expr: "my_pkg.NewLanguage(unsafe.Pointer({ptr}))".to_string(),
        ..Default::default()
    }
}

#[test]
fn test_capsule_fallible_returns_error_tuple_and_checks_last_error() {
    let func = make_capsule_func("get_language", true);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &capsule_cfg(), &empty_s);
    assert!(
        out.contains("(*my_pkg.Language, error)"),
        "fallible capsule must return (host, error):\n{out}"
    );
    assert!(
        out.contains("lastError()"),
        "fallible capsule must check lastError():\n{out}"
    );
    assert!(
        out.contains("return nil, err"),
        "fallible capsule must propagate the error:\n{out}"
    );
}

#[test]
fn test_capsule_infallible_returns_bare_host_type() {
    let func = make_capsule_func("builtin_language", false);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &capsule_cfg(), &empty_s);
    assert!(
        !out.contains(", error)"),
        "infallible capsule must not return an error:\n{out}"
    );
    assert!(
        !out.contains("lastError()"),
        "infallible capsule must not check lastError():\n{out}"
    );
}

#[test]
fn test_capsule_errors_when_construct_expr_empty() {
    let func = make_capsule_func("get_language", false);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let cfg = crate::core::config::HostCapsuleTypeConfig {
        host_type: "*my_pkg.Language".to_string(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: String::new(),
        ..Default::default()
    };
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &cfg, &empty_s);
    assert!(
        out.contains("ALEF ERROR"),
        "empty construct_expr must produce an ALEF ERROR comment. Got:\n{out}"
    );
    assert!(
        out.contains("construct_expr"),
        "error must mention the missing field. Got:\n{out}"
    );
}

#[test]
fn test_capsule_errors_when_host_type_empty() {
    let func = make_capsule_func("get_language", false);
    let empty: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let empty_s: std::collections::HashSet<String> = std::collections::HashSet::new();
    let cfg = crate::core::config::HostCapsuleTypeConfig {
        host_type: String::new(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: "my_pkg.NewLanguage(unsafe.Pointer({ptr}))".to_string(),
        ..Default::default()
    };
    let out = gen_capsule_function_wrapper(&func, "krz", &empty, &empty_s, &empty_s, &cfg, &empty_s);
    assert!(
        out.contains("ALEF ERROR"),
        "empty host_type must produce an ALEF ERROR comment. Got:\n{out}"
    );
    assert!(
        out.contains("host_type"),
        "error must mention the missing field. Got:\n{out}"
    );
}

/// Regression test for the Go backend emitting opaque-pointer idioms for values that cross
/// the FFI boundary as alef's scalar generational `AlefHandle`, per
/// `backends::ffi::type_map::{c_param_optional, c_return_optional}` (both map every
/// `TypeRef::Named` to `AlefHandle` unconditionally). Every `TypeRef::Named` param or return
/// value is a `uint64_t` handle in the emitted C header, never a `T*` pointer, so the Go side
/// must declare locals as the scalar `C.<PREFIX>AlefHandle` type and compare them to `0`, not
/// `nil`. Prior to the fix, this emitted `var cOptions *C.HTMConversionOptions` and
/// `cOptions == nil` / `ptr == nil`, which fails to compile under cgo.
#[test]
fn gen_convert_with_visitor_wrapper_uses_scalar_handle_not_opaque_pointer() {
    let func = FunctionDef {
        name: "convert".to_string(),
        params: vec![
            make_param("html", TypeRef::String),
            make_param("options", TypeRef::Named("ConversionOptions".to_string())),
        ],
        return_type: TypeRef::Named("ConversionResult".to_string()),
        error_type: Some("ConversionError".to_string()),
        ..Default::default()
    };
    let opaque_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let value_only_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    let bridge_cfg = TraitBridgeConfig {
        trait_name: "HtmlVisitor".to_string(),
        type_alias: Some("VisitorHandle".to_string()),
        param_name: Some("visitor".to_string()),
        options_type: Some("ConversionOptions".to_string()),
        ..Default::default()
    };
    let reserved_type_names: HashSet<String> = HashSet::new();

    let out = gen_convert_with_visitor_wrapper(
        &func,
        "htm",
        &opaque_names,
        &value_only_types,
        &bridge_cfg,
        &reserved_type_names,
    );

    // Positive sanity check: prove this slice actually covers the generated function body,
    // so the negative assertions below are not vacuously true.
    assert!(
        out.contains("func Convert(") && out.contains("C.htm_convert("),
        "expected a full Convert wrapper body with an htm_convert FFI call, got:\n{out}"
    );

    assert!(
        out.contains("var cOptions C.HTMAlefHandle"),
        "cOptions must be declared as the scalar AlefHandle type, got:\n{out}"
    );
    assert!(
        !out.contains("*C.HTMConversionOptions"),
        "cOptions must not be declared as an opaque pointer to the options struct, got:\n{out}"
    );
    assert!(
        !out.contains("cOptions == nil"),
        "handle comparisons must use == 0, not nil, got:\n{out}"
    );
    assert!(
        out.contains("cOptions == 0"),
        "expected a scalar handle nil-check for cOptions, got:\n{out}"
    );
    assert!(
        !out.contains("ptr == nil"),
        "the convert result handle must not be compared to nil, got:\n{out}"
    );
    assert!(
        out.contains("ptr == 0"),
        "expected a scalar handle nil-check for the convert result, got:\n{out}"
    );
}

/// `Optional<Bytes>` shares one C signature with bare `Bytes`
/// (`(args…, uint8_t **out_ptr, uintptr_t *out_len, uintptr_t *out_cap) -> int32_t`), with
/// absence carried by `*out_ptr == NULL`. Modelling it as a direct `*mut u8` return links
/// fine against that signature and only misreads at run time, so the Go wrapper must go
/// through the byte-buffer out-param path exactly as bare `Bytes` does.
#[test]
fn optional_bytes_return_reads_the_three_out_params_and_maps_null_to_nil() {
    let mut func = make_bytes_result_func("thumbnail", true);
    func.return_type = TypeRef::Optional(Box::new(TypeRef::Bytes));

    assert!(
        is_bytes_result_func(&func),
        "Optional<Bytes> must use the same owned-byte-buffer ABI as bare Bytes"
    );

    let empty_refs = HashSet::new();
    let empty_strings = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "krz",
        &empty_refs,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    // Positive control: the fixture must actually render a wrapper body for this function,
    // so the ABI assertions below cannot pass over empty output. ~keep
    assert!(
        out.contains("func Thumbnail(") && out.contains("C.krz_thumbnail("),
        "fixture must emit a real wrapper calling the C symbol, got:\n{out}"
    );

    assert!(out.contains("([]byte, error)"), "missing bytes return type in:\n{out}");
    assert!(out.contains("var outPtr"), "missing outPtr declaration in:\n{out}");
    assert!(out.contains("var outLen, outCap"), "missing outLen/outCap in:\n{out}");
    assert!(
        out.contains("&outPtr, &outLen, &outCap"),
        "the C call must pass all three out-params in:\n{out}"
    );
    assert!(
        out.contains("if outPtr == nil"),
        "absence is carried by a NULL out_ptr and must map to Go's nil slice, got:\n{out}"
    );
    assert!(out.contains("C.GoBytes"), "missing C.GoBytes in:\n{out}");
    assert!(out.contains("krz_free_bytes"), "missing krz_free_bytes in:\n{out}");
    assert!(
        !out.contains("ptr :="),
        "Optional<Bytes> must not fall through to the direct-pointer return path in:\n{out}"
    );
}

/// Control for the fix above: widening the predicate must not make every optional return
/// take the byte-buffer path. `Optional<String>` is a nullable `*mut c_char` on the C side
/// and must keep the direct-pointer shape — without this a predicate that answered `true`
/// for any `Optional` would still pass the test above.
#[test]
fn optional_string_return_keeps_the_direct_pointer_shape() {
    let mut func = make_bytes_result_func("caption", false);
    func.return_type = TypeRef::Optional(Box::new(TypeRef::String));

    assert!(
        !is_bytes_result_func(&func),
        "only Bytes / Optional<Bytes> use the owned-byte-buffer ABI"
    );

    let empty_refs = HashSet::new();
    let empty_strings = HashSet::new();
    let out = gen_function_wrapper(
        &func,
        "krz",
        &empty_refs,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
        &empty_strings,
    );

    assert!(
        out.contains("func Caption(") && out.contains("C.krz_caption("),
        "fixture must emit a real wrapper calling the C symbol, got:\n{out}"
    );
    assert!(
        !out.contains("outPtr"),
        "no byte out-params for Optional<String> in:\n{out}"
    );
    assert!(
        !out.contains("outCap"),
        "no byte out-params for Optional<String> in:\n{out}"
    );
    assert!(
        !out.contains("([]byte, error)"),
        "Optional<String> must not be typed as []byte in:\n{out}"
    );
}
