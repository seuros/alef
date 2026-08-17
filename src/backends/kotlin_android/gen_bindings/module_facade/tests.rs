use super::*;
use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

fn make_get_language_fn() -> FunctionDef {
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
fn capsule_wrapper_errors_when_construct_expr_empty() {
    let func = make_get_language_fn();
    let cfg = HostCapsuleTypeConfig {
        host_type: "com.example.Language".to_string(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: String::new(),
        ..Default::default()
    };
    let mut body = String::new();
    emit_capsule_function_wrapper(&mut body, &func, "SampleBridge", &cfg);
    assert!(
        body.contains("ALEF ERROR"),
        "empty construct_expr must produce ALEF ERROR. Got:\n{body}"
    );
    assert!(
        body.contains("construct_expr"),
        "error must name the missing field. Got:\n{body}"
    );
}

#[test]
fn capsule_wrapper_errors_when_host_type_empty() {
    let func = make_get_language_fn();
    let cfg = HostCapsuleTypeConfig {
        host_type: String::new(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: "com.example.Language({ptr})".to_string(),
        ..Default::default()
    };
    let mut body = String::new();
    emit_capsule_function_wrapper(&mut body, &func, "SampleBridge", &cfg);
    assert!(
        body.contains("ALEF ERROR"),
        "empty host_type must produce ALEF ERROR. Got:\n{body}"
    );
    assert!(
        body.contains("host_type"),
        "error must name the missing field. Got:\n{body}"
    );
}

#[test]
fn capsule_wrapper_constructs_host_language_without_alef_close_path() {
    let func = make_get_language_fn();
    let cfg = HostCapsuleTypeConfig {
        host_type: "dev.runtime.Language".into(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: "dev.runtime.Language({ptr})".into(),
        ..Default::default()
    };
    let mut body = String::new();
    emit_capsule_function_wrapper(&mut body, &func, "SampleBridge", &cfg);

    assert!(body.contains("return dev.runtime.Language(capsulePtr)"), "{body}");
    assert!(
        !body.contains("nativeFreeLanguage"),
        "capsule ownership belongs to the host runtime: {body}"
    );
}

#[test]
fn opaque_handle_header_clears_ownership_before_idempotent_free() {
    let rendered = crate::backends::kotlin_android::template_env::render(
        "handle_wrapper_header.jinja",
        minijinja::context! {
            class_name => "ResourceHandle",
            bridge_name => "SampleBridge",
            free_name => "nativeFreeResourceHandle",
        },
    );

    assert!(rendered.contains("@Synchronized get()"), "{rendered}");
    assert!(
        rendered.contains("check(nativeHandle != 0L) { \"ResourceHandle is closed\" }"),
        "{rendered}"
    );
    assert!(rendered.contains("if (ownedHandle == 0L) return"), "{rendered}");
    let clear = rendered.find("nativeHandle = 0L").expect("ownership clear");
    let free = rendered
        .find("nativeFreeResourceHandle(ownedHandle)")
        .expect("native free");
    assert!(clear < free, "ownership must clear before native free: {rendered}");
}

#[test]
fn capsule_type_gets_no_handle_wrapper_class_or_free_call() {
    let language_type = crate::core::ir::TypeDef {
        name: "Language".to_string(),
        rust_path: "sample_capsule::Language".to_string(),
        is_opaque: true,
        is_return_type: true,
        ..Default::default()
    };
    let api = crate::core::ir::ApiSurface {
        types: vec![language_type],
        functions: vec![make_get_language_fn()],
        ..Default::default()
    };
    let capsule_config = HostCapsuleTypeConfig {
        host_type: "dev.runtime.Language".to_string(),
        package: String::new(),
        package_version: String::new(),
        construct_expr: "dev.runtime.Language({ptr})".to_string(),
        ..Default::default()
    };
    let config = crate::core::config::ResolvedCrateConfig {
        kotlin_android: Some(crate::core::config::KotlinAndroidConfig {
            capsule_types: std::collections::HashMap::from([("Language".to_string(), capsule_config)]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let visible_functions: Vec<&crate::core::ir::FunctionDef> = api.functions.iter().collect();
    let mut files = Vec::new();

    super::handle_wrappers::emit_handle_wrappers(
        &api,
        &config,
        std::path::Path::new("src/main/kotlin"),
        "dev.sample",
        &mut files,
        "SampleBridge",
        &visible_functions,
    );

    assert!(
        files.is_empty(),
        "a capsule-typed return must not get a handle-wrapper .kt file: {files:?}"
    );
}

#[test]
fn android_trait_bridge_lifecycle_functions_are_managed_by_bridge_object() {
    let config = crate::core::config::ResolvedCrateConfig {
        trait_bridges: vec![crate::core::config::TraitBridgeConfig {
            trait_name: "DocumentExtractor".to_string(),
            register_fn: Some("register_document_extractor".to_string()),
            unregister_fn: Some("unregister_document_extractor".to_string()),
            clear_fn: Some("clear_document_extractors".to_string()),
            ..crate::core::config::TraitBridgeConfig::default()
        }],
        ..crate::core::config::ResolvedCrateConfig::default()
    };

    assert!(trait_bridge_manages_android_function(
        "register_document_extractor",
        &config
    ));
    assert!(trait_bridge_manages_android_function(
        "unregister_document_extractor",
        &config
    ));
    assert!(trait_bridge_manages_android_function(
        "clear_document_extractors",
        &config
    ));
    assert!(!trait_bridge_manages_android_function(
        "list_document_extractors",
        &config
    ));
}
