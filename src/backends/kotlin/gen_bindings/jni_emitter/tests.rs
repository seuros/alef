// `#[cfg(test)]` module is the final item in the flattened module (the other `include!`d files
// contribute production items, which must not follow a test module — `clippy::items_after_test_module`).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{KotlinAndroidConfig, TraitBridgeConfig};

    #[test]
    fn jni_bridge_object_treats_android_trait_lifecycle_functions_as_managed() {
        let config = ResolvedCrateConfig {
            kotlin_android: Some(KotlinAndroidConfig::default()),
            trait_bridges: vec![TraitBridgeConfig {
                trait_name: "Renderer".to_string(),
                register_fn: Some("register_renderer".to_string()),
                unregister_fn: Some("unregister_renderer".to_string()),
                clear_fn: Some("clear_renderers".to_string()),
                ..TraitBridgeConfig::default()
            }],
            ..ResolvedCrateConfig::default()
        };

        assert!(trait_bridge_manages_jni_function("register_renderer", &config));
        assert!(trait_bridge_manages_jni_function("unregister_renderer", &config));
        assert!(trait_bridge_manages_jni_function("clear_renderers", &config));
        assert!(!trait_bridge_manages_jni_function("list_renderers", &config));
    }

    /// Regression: the `close()` free-fn call must pascal-case an acronym owner
    /// (e.g. `GraphQLRouteConfig` -> `nativeFreeGraphQlRouteConfig`) so it resolves the
    /// bridge external-fun declaration and the Rust JNI export, both of which pascal-case
    /// the owner. Using the class name verbatim produced `nativeFreeGraphQLRouteConfig`,
    /// an unresolved reference that failed `compileReleaseKotlin`.
    #[test]
    fn jni_client_close_pascal_cases_acronym_owner_free_name() {
        use crate::core::ir::{MethodDef, TypeDef, TypeRef};

        let mut api = ApiSurface::default();
        api.types.push(TypeDef {
            name: "GraphQLRouteConfig".to_owned(),
            rust_path: "my_crate::GraphQLRouteConfig".to_owned(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "path".to_owned(),
                is_static: false,
                return_type: TypeRef::String,
                ..MethodDef::default()
            }],
            ..TypeDef::default()
        });

        let config = ResolvedCrateConfig {
            kotlin_android: Some(KotlinAndroidConfig::default()),
            ..ResolvedCrateConfig::default()
        };

        let file = emit_jni_client_class(&api, &config, Some("dev.example")).expect("client class must be emitted");

        assert!(
            file.content.contains("nativeFreeGraphQlRouteConfig"),
            "close() must call the pascal-cased free name matching the bridge decl + Rust export; got:\n{}",
            file.content
        );
        assert!(
            !file.content.contains("nativeFreeGraphQLRouteConfig"),
            "close() must not emit the verbatim (miscased) acronym free name; got:\n{}",
            file.content
        );
    }

    #[test]
    fn jni_client_serializes_native_calls_and_makes_close_idempotent() {
        use crate::core::ir::{MethodDef, TypeDef, TypeRef};

        let api = ApiSurface {
            types: vec![TypeDef {
                name: "ClientHandle".to_owned(),
                rust_path: "sample::ClientHandle".to_owned(),
                is_opaque: true,
                methods: vec![MethodDef {
                    name: "status".to_owned(),
                    is_static: false,
                    return_type: TypeRef::String,
                    ..MethodDef::default()
                }],
                ..TypeDef::default()
            }],
            ..ApiSurface::default()
        };
        let config = ResolvedCrateConfig {
            kotlin_android: Some(KotlinAndroidConfig::default()),
            ..ResolvedCrateConfig::default()
        };

        let file = emit_jni_client_class(&api, &config, Some("dev.example")).expect("client class must be emitted");

        assert!(file.content.contains("private val handleLock = Any()"));
        assert!(
            file.content
                .contains("check(nativeHandle != 0L) { \"ClientHandle is closed\" }")
        );
        assert!(file.content.contains("withHandle { handle ->"));
        assert!(file.content.contains("nativeClientHandleStatus(handle)"));
        assert!(file.content.contains("override fun close() = synchronized(handleLock)"));
        assert!(file.content.contains("if (nativeHandle != 0L)"));
        assert!(file.content.contains("nativeHandle = 0L"));
    }
}
