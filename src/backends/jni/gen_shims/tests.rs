#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jni_return_type_unit() {
        assert_eq!(jni_return_type(&TypeRef::Unit), "()");
    }

    #[test]
    fn jni_return_type_i64() {
        assert_eq!(jni_return_type(&TypeRef::Primitive(PrimitiveType::I64)), "jlong");
    }

    #[test]
    fn jni_return_type_string() {
        assert_eq!(jni_return_type(&TypeRef::String), "jstring");
    }

    #[test]
    fn jni_return_type_vec_u8() {
        assert_eq!(
            jni_return_type(&TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U8)))),
            "jbyteArray"
        );
    }

    #[test]
    fn type_ref_to_core_path_uses_btree_for_btree_map() {
        let map = TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String));
        assert_eq!(
            type_ref_to_core_path_with_btree(&map, "core_crate", true),
            "std::collections::BTreeMap<String, String>"
        );
        assert_eq!(
            type_ref_to_core_path_with_btree(&map, "core_crate", false),
            "std::collections::HashMap<String, String>"
        );
    }

    #[test]
    fn bytes_call_arg_optional_ref_uses_as_deref() {
        assert_eq!(
            bytes_call_arg("document_bytes", true, true),
            "document_bytes.as_deref()"
        );
        assert_eq!(bytes_call_arg("document_bytes", true, false), "document_bytes");
        assert_eq!(bytes_call_arg("document_bytes", false, true), "&document_bytes");
        assert_eq!(bytes_call_arg("document_bytes", false, false), "document_bytes");
    }

    fn btree_fixture_config() -> crate::core::config::ResolvedCrateConfig {
        use crate::core::config::NewAlefConfig;
        let raw: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
"#,
        )
        .unwrap();
        raw.resolve().unwrap().remove(0)
    }

    fn api_with_functions(functions: Vec<crate::core::ir::FunctionDef>) -> crate::core::ir::ApiSurface {
        crate::core::ir::ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![],
            functions,
            enums: vec![],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    /// `analyze_document(..., document_bytes: Option<&[u8]>)` must pass
    /// `document_bytes.as_deref()` (Option<Vec<u8>> -> Option<&[u8]>), not the owned
    /// `Option<Vec<u8>>` which fails with E0308.
    #[test]
    fn optional_byte_slice_param_uses_as_deref_at_call_site() {
        let func = crate::core::ir::FunctionDef {
            name: "analyze_document".into(),
            rust_path: "demo::analyze_document".into(),
            params: vec![crate::core::ir::ParamDef {
                name: "document_bytes".into(),
                ty: TypeRef::Bytes,
                optional: true,
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::String,
            error_type: Some("DemoError".into()),
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_functions(vec![func]), &btree_fixture_config());
        assert!(
            content.contains("document_bytes.as_deref()"),
            "optional &[u8] param must be passed via .as_deref(): {content}"
        );
        assert!(
            content.contains("core_crate::analyze_document(document_bytes.as_deref())"),
            "call site must pass document_bytes.as_deref(): {content}"
        );
    }

    /// `resolve(..., context: &BTreeMap<String, String>)` must deserialize into a
    /// `BTreeMap` (not `HashMap`) so the `&context` argument matches the core's
    /// `&BTreeMap<String, String>` slot (E0308 otherwise).
    #[test]
    fn btree_map_param_deserializes_into_btreemap() {
        let func = crate::core::ir::FunctionDef {
            name: "resolve".into(),
            rust_path: "demo::resolve".into(),
            params: vec![crate::core::ir::ParamDef {
                name: "context".into(),
                ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                optional: false,
                is_ref: true,
                map_is_btree: true,
                ..Default::default()
            }],
            return_type: TypeRef::String,
            error_type: Some("DemoError".into()),
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_functions(vec![func]), &btree_fixture_config());
        assert!(
            content.contains("let context: std::collections::BTreeMap<String, String>"),
            "BTreeMap param must deserialize into BTreeMap: {content}"
        );
        assert!(
            !content.contains("let context: std::collections::HashMap<String, String>"),
            "BTreeMap param must NOT deserialize into HashMap: {content}"
        );
        assert!(
            content.contains("core_crate::resolve(&context)"),
            "call site must pass &context: {content}"
        );
    }

    /// A free function resolved into a *sibling* workspace crate (rust_path
    /// `<sibling_crate>::<fn>`, where the sibling crate is not the umbrella crate)
    /// must be reached through the umbrella facade by item path
    /// (`core_crate::<fn>`), mirroring how opaque types are referenced. Prefixing
    /// the origin crate — `core_crate::<sibling_crate>::<fn>` — does not resolve
    /// (E0433: cannot find `<sibling_crate>` in `core_crate`).
    #[test]
    fn sibling_crate_function_is_reached_through_umbrella_facade() {
        let func = crate::core::ir::FunctionDef {
            name: "schema_query_only".into(),
            rust_path: "demo_graphql::schema_query_only".into(),
            params: vec![],
            return_type: TypeRef::String,
            error_type: None,
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_functions(vec![func]), &btree_fixture_config());
        assert!(
            content.contains("core_crate::schema_query_only("),
            "sibling-crate fn must be called as core_crate::schema_query_only(): {content}"
        );
        assert!(
            !content.contains("core_crate::demo_graphql::"),
            "sibling-crate fn must NOT be prefixed with the origin crate: {content}"
        );
    }

    /// The generated `throw_jni_error` helper must use `env.throw_new(...).is_err()`
    /// and fall back to `java/lang/RuntimeException` rather than silently discarding
    /// a failed throw (which would leave the Kotlin caller with no exception pending
    /// and a null/zero sentinel that looks like a valid return value).
    #[test]
    fn throw_jni_error_has_runtime_exception_fallback() {
        use crate::core::config::NewAlefConfig;
        let raw: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
"#,
        )
        .unwrap();
        let config = raw.resolve().unwrap().remove(0);
        let api = crate::core::ir::ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let content = emit_lib_rs(&api, &config);
        assert!(
            !content.contains("let _ = env.throw_new(ERROR_CLASS"),
            "throw_jni_error must not discard the throw_new result: {content}"
        );
        assert!(
            content.contains("if env.throw_new(&class_jni, &msg_jni).is_err()"),
            "throw_jni_error must check throw_new result: {content}"
        );
        assert!(
            content.contains("jni::strings::JNIString::from(ERROR_CLASS)"),
            "throw_jni_error must wrap ERROR_CLASS in JNIString::from: {content}"
        );
        assert!(
            content.contains("java/lang/RuntimeException"),
            "throw_jni_error must fall back to RuntimeException: {content}"
        );
    }

    /// Build an `ApiSurface` whose single opaque client type carries `methods`,
    /// so `emit_lib_rs` routes them through `emit_method_shim` (the request-map
    /// multi-param path) rather than the per-param free-function path.
    fn api_with_client_methods(methods: Vec<crate::core::ir::MethodDef>) -> crate::core::ir::ApiSurface {
        let client = crate::core::ir::TypeDef {
            name: "Loader".into(),
            rust_path: "demo::Loader".into(),
            is_opaque: true,
            methods,
            ..Default::default()
        };
        crate::core::ir::ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![client],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        }
    }

    /// Multi-param method `parse_preset(path: &str, raw: &[u8])` is decoded from the
    /// request map. The `&[u8]` param must bind `let raw: Vec<u8>` (not the generic
    /// `serde_json::Value` catch-all) and be passed as `&raw` so `&Vec<u8>` derefs to
    /// `&[u8]` (E0308 otherwise: `expected &[u8], found &Value`).
    #[test]
    fn request_map_byte_slice_param_binds_vec_u8_not_json_value() {
        let method = crate::core::ir::MethodDef {
            name: "parse_preset".into(),
            params: vec![
                crate::core::ir::ParamDef {
                    name: "path".into(),
                    ty: TypeRef::String,
                    is_ref: true,
                    ..Default::default()
                },
                crate::core::ir::ParamDef {
                    name: "raw".into(),
                    ty: TypeRef::Bytes,
                    is_ref: true,
                    ..Default::default()
                },
            ],
            return_type: TypeRef::Named("Preset".into()),
            error_type: Some("LoadError".into()),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_client_methods(vec![method]), &btree_fixture_config());
        assert!(
            content.contains("let raw: Vec<u8> = match req_map.get(\"raw\")"),
            "request-map &[u8] param must bind Vec<u8>: {content}"
        );
        assert!(
            !content.contains("let raw: serde_json::Value"),
            "request-map &[u8] param must NOT bind serde_json::Value: {content}"
        );
        assert!(
            content.contains("client.parse_preset(&path, &raw)"),
            "call site must pass &path and &raw: {content}"
        );
    }

    /// Multi-param method `load_at(path: &Path, raw: &[u8])`: a `&Path` param in the
    /// request-map path must deserialize as `String` then convert to `PathBuf` (so
    /// `&path` derefs `&PathBuf` → `&Path`), never bind the `serde_json::Value`
    /// catch-all (E0277: `Value` does not impl `AsRef<Path>`).
    #[test]
    fn request_map_path_param_binds_pathbuf_not_json_value() {
        let method = crate::core::ir::MethodDef {
            name: "load_at".into(),
            params: vec![
                crate::core::ir::ParamDef {
                    name: "path".into(),
                    ty: TypeRef::Path,
                    is_ref: true,
                    ..Default::default()
                },
                crate::core::ir::ParamDef {
                    name: "raw".into(),
                    ty: TypeRef::Bytes,
                    is_ref: true,
                    ..Default::default()
                },
            ],
            return_type: TypeRef::Named("Preset".into()),
            error_type: Some("LoadError".into()),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            ..Default::default()
        };
        let content = emit_lib_rs(&api_with_client_methods(vec![method]), &btree_fixture_config());
        assert!(
            content.contains("let path = std::path::PathBuf::from(path);"),
            "request-map &Path param must convert to PathBuf: {content}"
        );
        assert!(
            !content.contains("let path: serde_json::Value"),
            "request-map &Path param must NOT bind serde_json::Value: {content}"
        );
        assert!(
            content.contains("client.load_at(&path, &raw)"),
            "call site must pass &path and &raw: {content}"
        );
    }

    /// A client type listed in `[crates.kotlin_android].exclude_types` (or the shared
    /// `[crates.ffi].exclude_types`) must not have any JNI shims emitted. The
    /// kotlin_android binding backend already drops the Kotlin class via
    /// `effective_exclude_types`; without the matching filter here the JNI side emits
    /// orphan `#[no_mangle]` shims and re-exposes a type every other FFI-derived
    /// binding hides (e.g. the test-only client). The exclusion must be *targeted*:
    /// a sibling client that is not excluded keeps its shims.
    #[test]
    fn excluded_client_type_emits_no_shims_but_keeps_others() {
        use crate::core::config::NewAlefConfig;
        let raw: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["kotlin_android", "jni"]

[[crates]]
name = "demo"
sources = ["src/lib.rs"]

[crates.kotlin_android]
package = "dev.sample_crate"
namespace = "dev.sample_crate"
exclude_types = ["Loader"]
"#,
        )
        .unwrap();
        let config = raw.resolve().unwrap().remove(0);
        let method = |name: &str| crate::core::ir::MethodDef {
            name: name.into(),
            params: vec![crate::core::ir::ParamDef {
                name: "path".into(),
                ty: TypeRef::String,
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::String,
            error_type: Some("LoadError".into()),
            receiver: Some(crate::core::ir::ReceiverKind::Ref),
            ..Default::default()
        };
        let client = |name: &str, m: crate::core::ir::MethodDef| crate::core::ir::TypeDef {
            name: name.into(),
            rust_path: format!("demo::{name}"),
            is_opaque: true,
            methods: vec![m],
            ..Default::default()
        };
        let api = crate::core::ir::ApiSurface {
            crate_name: "demo".into(),
            version: "0.1.0".into(),
            types: vec![
                client("Loader", method("excluded_call")),
                client("Keeper", method("kept_call")),
            ],
            functions: vec![],
            enums: vec![],
            errors: vec![],
            excluded_type_paths: Default::default(),
            excluded_trait_names: ::std::collections::HashSet::new(),
            services: vec![],
            handler_contracts: vec![],
            unsupported_public_items: Vec::new(),
        };
        let content = emit_lib_rs(&api, &config);
        assert!(
            !content.contains("excluded_call"),
            "excluded client type must not emit method shims: {content}"
        );
        assert!(
            !content.contains("FreeLoader") && !content.contains("nativeFreeLoader"),
            "excluded client type must not emit a destructor shim: {content}"
        );
        assert!(
            content.contains("client.kept_call"),
            "a non-excluded sibling client must keep its shims: {content}"
        );
    }
}
