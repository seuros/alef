use super::*;
use crate::core::ir::ParamDef;

fn get_language_fn() -> FunctionDef {
    FunctionDef {
        name: "get_language".to_string(),
        rust_path: "sample::get_language".to_string(),
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

fn make_cfg(host_type: &str, construct_expr: &str) -> HostCapsuleTypeConfig {
    HostCapsuleTypeConfig {
        host_type: host_type.to_string(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: construct_expr.to_string(),
        ..Default::default()
    }
}

#[test]
fn capsule_method_emits_configured_host_type_and_construct_expr() {
    let func = get_language_fn();
    let cfg = make_cfg(
        "io.github.example.jtreesitter.Language",
        "new io.github.example.jtreesitter.Language({ptr})",
    );
    let mut out = String::new();
    gen_capsule_function_method(
        &mut out,
        &func,
        "tsp",
        "LanguagePack",
        &AHashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &cfg,
    );
    assert!(
        out.contains("io.github.example.jtreesitter.Language"),
        "must use configured host_type. Got:\n{out}"
    );
    assert!(
        out.contains("new io.github.example.jtreesitter.Language(resultPtr)"),
        "must use configured construct_expr with ptr substituted. Got:\n{out}"
    );
}

#[test]
fn capsule_method_registers_named_temporary_before_invocation() {
    let mut func = get_language_fn();
    func.params = vec![ParamDef {
        name: "config".to_string(),
        ty: TypeRef::Named("Config".to_string()),
        ..Default::default()
    }];
    let cfg = make_cfg("Language", "new Language({ptr})");
    let mut out = String::new();

    gen_capsule_function_method(
        &mut out,
        &func,
        "tsp",
        "LanguagePack",
        &AHashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &cfg,
    );

    let registration = out
        .find("nativeResources.register(cconfig, handle -> NativeLib.TSP_CONFIG_FREE.invoke(handle))")
        .expect("temporary registration");
    let invocation = out
        .find("NativeLib.TSP_GET_LANGUAGE.invoke")
        .expect("capsule invocation");
    assert!(out.contains("var nativeResources = new NativeResources()"), "{out}");
    assert!(registration < invocation, "{out}");
}

#[test]
fn capsule_method_errors_when_host_type_empty() {
    let func = get_language_fn();
    let cfg = make_cfg("", "new MyLanguage({ptr})");
    let mut out = String::new();
    gen_capsule_function_method(
        &mut out,
        &func,
        "tsp",
        "LanguagePack",
        &AHashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &cfg,
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

#[test]
fn capsule_method_errors_when_construct_expr_empty() {
    let func = get_language_fn();
    let cfg = make_cfg("io.github.example.jtreesitter.Language", "");
    let mut out = String::new();
    gen_capsule_function_method(
        &mut out,
        &func,
        "tsp",
        "LanguagePack",
        &AHashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &cfg,
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
