use super::*;
use crate::core::config::HostCapsuleTypeConfig;
use std::collections::{HashMap, HashSet};

fn get_language_fn() -> FunctionDef {
    FunctionDef {
        name: "get_language".to_string(),
        rust_path: "sample_capsule::get_language".to_string(),
        original_rust_path: String::new(),
        params: vec![ParamDef {
            name: "name".to_string(),
            ty: TypeRef::String,
            optional: false,
            default: None,
            sanitized: false,
            typed_default: None,
            is_ref: true,
            is_mut: false,
            newtype_wrapper: None,
            original_type: None,
            map_is_ahash: false,
            map_key_is_cow: false,
            vec_inner_is_ref: false,
            map_is_btree: false,
            core_wrapper: crate::core::ir::CoreWrapper::None,
        }],
        return_type: TypeRef::Named("Language".to_string()),
        is_async: false,
        error_type: None,
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

#[test]
fn emit_function_constructs_host_language_for_capsule_return() {
    let f = get_language_fn();
    let mut capsule_types: HashMap<String, HostCapsuleTypeConfig> = HashMap::new();
    capsule_types.insert(
        "Language".to_string(),
        HostCapsuleTypeConfig {
            host_type: "?*const my_mod.Language".to_string(),
            package: "https://github.com/example/zig-my-lib".to_string(),
            package_version: String::new(),
            construct_expr: "my_mod.Language.fromRaw(@ptrCast({ptr}))".to_string(),
            ..Default::default()
        },
    );
    let mut out = String::new();
    emit_function(
        &f,
        "tsp",
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &capsule_types,
        &mut out,
    );
    assert!(
        out.contains("?*const my_mod.Language"),
        "capsule fn must return the host Language type. Got:\n{out}"
    );
    assert!(
        out.contains("my_mod.Language.fromRaw(@ptrCast(_result))"),
        "capsule fn must construct via fromRaw. Got:\n{out}"
    );
    assert!(
        out.contains("c.tsp_get_language("),
        "capsule fn must call the C symbol. Got:\n{out}"
    );
}

#[test]
fn emit_function_wraps_optional_capsule_return() {
    let mut f = get_language_fn();
    f.name = "find_language".to_string();
    f.return_type = TypeRef::Optional(Box::new(TypeRef::Named("Language".to_string())));
    let mut capsule_types: HashMap<String, HostCapsuleTypeConfig> = HashMap::new();
    capsule_types.insert(
        "Language".to_string(),
        HostCapsuleTypeConfig {
            host_type: "?*const my_mod.Language".to_string(),
            package: "https://github.com/example/zig-my-lib".to_string(),
            package_version: String::new(),
            construct_expr: "my_mod.Language.fromRaw(@ptrCast({ptr}))".to_string(),
            ..Default::default()
        },
    );
    let mut out = String::new();
    emit_function(
        &f,
        "tsp",
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &capsule_types,
        &mut out,
    );
    assert!(
        out.contains("?*const my_mod.Language"),
        "optional capsule fn must still return the host Language type. Got:\n{out}"
    );
    assert!(
        out.contains("if (_result == null) return null;"),
        "optional capsule fn must preserve a null result as `null` rather than an error. Got:\n{out}"
    );
    assert!(
        out.contains("my_mod.Language.fromRaw(@ptrCast(_result))"),
        "optional capsule fn must construct via fromRaw when non-null. Got:\n{out}"
    );
    assert!(
        out.contains("c.tsp_find_language("),
        "optional capsule fn must call the C symbol. Got:\n{out}"
    );
    assert!(
        !out.contains("return _result;"),
        "optional capsule return must not fall through to the generic bare-return path. Got:\n{out}"
    );
}

#[test]
fn emit_function_errors_when_construct_expr_empty() {
    let f = get_language_fn();
    let mut capsule_types: HashMap<String, HostCapsuleTypeConfig> = HashMap::new();
    capsule_types.insert(
        "Language".to_string(),
        HostCapsuleTypeConfig {
            host_type: "?*const my_mod.Language".to_string(),
            package: String::new(),
            package_version: String::new(),
            construct_expr: String::new(),
            ..Default::default()
        },
    );
    let mut out = String::new();
    emit_function(
        &f,
        "tsp",
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &capsule_types,
        &mut out,
    );
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
fn emit_function_errors_when_host_type_empty() {
    let f = get_language_fn();
    let mut capsule_types: HashMap<String, HostCapsuleTypeConfig> = HashMap::new();
    capsule_types.insert(
        "Language".to_string(),
        HostCapsuleTypeConfig {
            host_type: String::new(),
            package: String::new(),
            package_version: String::new(),
            construct_expr: "my_mod.Language.fromRaw(@ptrCast({ptr}))".to_string(),
            ..Default::default()
        },
    );
    let mut out = String::new();
    emit_function(
        &f,
        "tsp",
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &HashMap::new(),
        &capsule_types,
        &mut out,
    );
    assert!(
        out.contains("ALEF ERROR"),
        "empty host_type must produce an ALEF ERROR comment. Got:\n{out}"
    );
    assert!(
        out.contains("host_type"),
        "error must mention the missing field. Got:\n{out}"
    );
}
