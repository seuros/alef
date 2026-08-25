use super::binding_file::{is_ffi_enum_type, strip_trailing_whitespace};
use super::constructors::gen_go_opaque_constructor;
use super::*;
use crate::core::config::NewAlefConfig;

fn resolved_one(toml: &str) -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(toml).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn make_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "test"
[crates.go]
module = "github.com/test/test-lib"
"#,
    )
}

#[test]
fn test_package_name_extracts_last_segment() {
    assert_eq!(GoBackend::package_name("github.com/org/my-lib"), "mylib");
    assert_eq!(GoBackend::package_name("binding"), "binding");
}

#[test]
fn test_strip_trailing_whitespace_normalizes_lines() {
    let input = "line one   \nline two\n";
    let result = strip_trailing_whitespace(input);
    assert_eq!(result, "line one\nline two\n");
}

#[test]
fn test_is_ffi_enum_type_returns_true_for_known_enum() {
    let mut enum_names = HashSet::new();
    enum_names.insert("Status".to_string());
    assert!(is_ffi_enum_type("Status", &enum_names));
    assert!(!is_ffi_enum_type("Config", &enum_names));
}

#[test]
fn test_generate_bindings_produces_binding_go_file() {
    use crate::core::ir::ApiSurface;
    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    assert!(!files.is_empty());
    assert!(files[0].path.to_string_lossy().contains("binding.go"));

    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");
    let pkg_line = binding
        .content
        .lines()
        .find(|l| l.starts_with("package "))
        .expect("binding.go declares a package");
    let embed = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("embed_ffi.go"))
        .expect("embed_ffi.go present");
    assert!(
        !embed.content.contains("package samplepack"),
        "embed_ffi.go must not hardcode the samplepack package name"
    );
    assert!(
        embed.content.contains(pkg_line),
        "embed_ffi.go package must match binding.go ({pkg_line})"
    );
}

/// Regression test for the dropped `exclude_functions` config key on `[crates.go]`: today
/// the Go backend only honours `[crates.ffi].exclude_functions`, which would also strip the
/// function's C symbol from every other binding. A per-language `[crates.go].exclude_functions`
/// must hide a function from Go's generated `binding.go` while leaving the FFI-level list (and
/// hence the C ABI, and other bindings) untouched — mirrors `CSharpConfig::exclude_functions`.
#[test]
fn test_generate_bindings_unions_go_exclude_functions_with_ffi_exclude_functions() {
    use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};

    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "test"
[crates.go]
module = "github.com/test/test-lib"
exclude_functions = ["embed_sparse_async"]
"#,
    );

    let make_fn = |name: &str| FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Unit,
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![make_fn("embed_sparse_async"), make_fn("other_func")],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        !binding.content.contains("EmbedSparseAsync"),
        "GoConfig::exclude_functions must drop the function from binding.go:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("OtherFunc"),
        "a function not named in exclude_functions must still be generated:\n{}",
        binding.content
    );
}

/// `GoConfig::exclude_functions` must UNION with `[crates.ffi].exclude_functions`, not
/// replace it: a function named only at the FFI level must still be dropped from Go's
/// `binding.go`, alongside a function named only at the Go level, while a function named in
/// neither list survives.
#[test]
fn test_generate_bindings_go_exclude_functions_unions_rather_than_replaces_ffi_list() {
    use crate::core::ir::{ApiSurface, FunctionDef, TypeRef};

    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "test"
exclude_functions = ["ffi_only_excluded"]
[crates.go]
module = "github.com/test/test-lib"
exclude_functions = ["go_only_excluded"]
"#,
    );

    let make_fn = |name: &str| FunctionDef {
        name: name.to_string(),
        rust_path: String::new(),
        original_rust_path: String::new(),
        params: vec![],
        return_type: TypeRef::Unit,
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
    };

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![],
        functions: vec![
            make_fn("ffi_only_excluded"),
            make_fn("go_only_excluded"),
            make_fn("kept_everywhere"),
        ],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        !binding.content.contains("FfiOnlyExcluded"),
        "a function excluded only at the FFI level must still be dropped from Go:\n{}",
        binding.content
    );
    assert!(
        !binding.content.contains("GoOnlyExcluded"),
        "a function excluded only at the Go level must be dropped from Go:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("KeptEverywhere"),
        "a function excluded in neither list must survive:\n{}",
        binding.content
    );
}

#[test]
fn test_generate_bindings_emits_cmd_setup_and_native_setup_sentinel() {
    use crate::core::ir::ApiSurface;
    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.0.0-rc.38".to_string(),
        types: vec![],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };
    let backend = GoBackend;
    let files = backend.generate_bindings(&api, &config).unwrap();

    assert!(
        !files
            .iter()
            .any(|f| f.path.to_string_lossy().ends_with("cmd/download_ffi/main.go")),
        "the old cmd/download_ffi tool must no longer be emitted"
    );

    let setup = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("cmd/setup/main.go"))
        .expect("cmd/setup/main.go must be generated");
    assert!(
        setup.content.contains(r#"moduleVersion     = "1.0.0-rc.38""#),
        "cmd/setup/main.go must embed the crate version:\n{}",
        setup.content
    );
    assert!(
        setup.content.contains(r#"versionIdent      = "1_0_0_rc_38""#),
        "cmd/setup must embed the version-matched sentinel identifier:\n{}",
        setup.content
    );
    assert!(
        setup.content.contains("RequireNativeSetup_%s"),
        "cmd/setup's shim writer must build the RequireNativeSetup_<versionIdent> reference:\n{}",
        setup.content
    );

    let native_setup = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("native_setup.go"))
        .expect("native_setup.go must be generated");
    assert!(
        native_setup
            .content
            .contains(r#"const RequireNativeSetup_1_0_0_rc_38 = "1.0.0-rc.38""#),
        "native_setup.go must declare the version-skew sentinel:\n{}",
        native_setup.content
    );
}

#[test]
fn test_gen_go_opaque_constructor_emits_new_function() {
    use crate::core::config::workspace::{ClientConstructorConfig, ConstructorParam};
    use crate::core::ir::TypeDef;

    let typ = TypeDef {
        name: "TestClient".to_string(),
        rust_path: "test_lib::TestClient".to_string(),
        original_rust_path: "test_lib::TestClient".to_string(),
        fields: vec![],
        methods: vec![],
        is_opaque: true,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_conversion: Default::default(),
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    let ctor = ClientConstructorConfig {
        params: vec![ConstructorParam {
            name: "api_key".to_string(),
            ty: "*const std::ffi::c_char".to_string(),
        }],
        body: "TestClient::new(api_key)".to_string(),
        error_type: None,
    };
    let output = gen_go_opaque_constructor(&typ, "test", &ctor);
    assert!(
        output.contains("func NewTestClient("),
        "should contain func NewTestClient"
    );
    assert!(output.contains("api_key string"), "should contain api_key string param");
    assert!(
        output.contains("C.CString(api_key)"),
        "should use C.CString for c_char param"
    );
    assert!(
        output.contains("C.free(unsafe.Pointer("),
        "should defer-free the C string"
    );
    assert!(
        output.contains("C.test_test_client_new("),
        "should call FFI constructor"
    );
    assert!(output.contains("return nil, fmt.Errorf"), "should return error on nil");
    assert!(
        output.contains("return &TestClient{ptr:"),
        "should return handle on success"
    );
}

fn capsule_config() -> ResolvedCrateConfig {
    resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "sample-capsule"
sources = ["src/lib.rs"]
[crates.ffi]
prefix = "tsp"
[crates.ffi.capsule_types.Language]
into_raw_type = "my_crate::ffi::MyLang"
c_return_type = "MyLang"
[crates.go]
module = "github.com/test/sample-capsule"
[crates.go.capsule_types.Language]
host_type = "*my_pkg.Language"
package = "github.com/example/go-my-lib"
package_version = "v1.0.0"
construct_expr = "my_pkg.NewLanguage(unsafe.Pointer({ptr}))"
pointer_ownership = "borrowed_static"
abi_compatible = true
host_destructor = "none"
"#,
    )
}

fn capsule_api() -> crate::core::ir::ApiSurface {
    use crate::core::ir::*;
    ApiSurface {
        crate_name: "sample-capsule".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "Language".to_string(),
            rust_path: "sample_capsule::Language".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: true,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: true,
            serde_rename_all: None,
            has_serde: false,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: "A grammar.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![FunctionDef {
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
            doc: "Look up a grammar.".to_string(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    }
}

#[test]
fn capsule_function_constructs_host_language_and_imports_package() {
    let config = capsule_config();
    let api = capsule_api();
    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        binding
            .content
            .contains("func GetLanguage(name string) *my_pkg.Language"),
        "capsule wrapper must return host *my_pkg.Language. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("my_pkg.NewLanguage(unsafe.Pointer(cLang))"),
        "capsule wrapper must construct via my_pkg.NewLanguage. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("github.com/example/go-my-lib"),
        "binding.go must import the configured capsule package"
    );
}

/// A free function whose Go PascalCase name collides with a struct type of the same name
/// (e.g. Rust's `fn model_info(...)` and `struct ModelInfo`) must not produce two `ModelInfo`
/// package-level declarations. The type keeps the plain name; the function is renamed
/// `GetModelInfo`. A non-colliding function in the same package is unaffected.
#[test]
fn free_function_colliding_with_type_name_is_renamed_get_prefixed() {
    use crate::core::ir::*;

    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "ModelInfo".to_string(),
            rust_path: "test_lib::ModelInfo".to_string(),
            original_rust_path: String::new(),
            fields: vec![],
            methods: vec![],
            is_opaque: false,
            is_clone: false,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: true,
            serde_rename_all: None,
            has_serde: true,
            serde_container_default: false,
            serde_container_conversion: Default::default(),
            super_traits: vec![],
            doc: "Model metadata.".to_string(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
        }],
        functions: vec![
            FunctionDef {
                name: "model_info".to_string(),
                rust_path: "test_lib::model_info".to_string(),
                original_rust_path: String::new(),
                params: vec![ParamDef {
                    name: "model".to_string(),
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
                return_type: TypeRef::Optional(Box::new(TypeRef::Named("ModelInfo".to_string()))),
                is_async: false,
                error_type: None,
                doc: "Look up model metadata by name.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            FunctionDef {
                name: "list_models".to_string(),
                rust_path: "test_lib::list_models".to_string(),
                original_rust_path: String::new(),
                params: vec![],
                return_type: TypeRef::String,
                is_async: false,
                error_type: None,
                doc: "List known model names.".to_string(),
                cfg: None,
                sanitized: false,
                return_sanitized: false,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
        ],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        binding.content.contains("type ModelInfo struct"),
        "struct type must keep its plain name. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("func GetModelInfo(model string)"),
        "colliding free function must be renamed to GetModelInfo. Got:\n{}",
        binding.content
    );
    assert!(
        !binding.content.contains("func ModelInfo("),
        "colliding free function must not keep the bare type name. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("func ListModels()"),
        "non-colliding function must be unaffected. Got:\n{}",
        binding.content
    );
}

/// Go rejects a struct that carries both a field and a method named `Providers`
/// (`field and method with the same name`). A core type with a public `providers` field and
/// an inherent `providers()` method feeds both the struct emitter and the method-wrapper
/// emitter, so the wrapper must be dropped and the field kept.
#[test]
fn generate_bindings_skips_method_wrapper_when_struct_field_has_same_name() {
    use crate::core::ir::{ApiSurface, FieldDef, MethodDef, ReceiverKind, TypeDef, TypeRef};

    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "LlmConfig".to_string(),
            rust_path: "test_lib::LlmConfig".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "providers".to_string(),
                ty: TypeRef::String,
                optional: true,
                ..Default::default()
            }],
            methods: vec![MethodDef {
                name: "providers".to_string(),
                return_type: TypeRef::String,
                receiver: Some(ReceiverKind::Ref),
                cfg: None,
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        binding.content.contains("Providers "),
        "the struct field must still be emitted. Got:\n{}",
        binding.content
    );
    let wrappers = binding.content.matches("func (r *LlmConfig) Providers(").count();
    assert_eq!(
        wrappers, 0,
        "the same-named method wrapper must be skipped, found {wrappers} in:\n{}",
        binding.content
    );
}

/// Regression (Defect 1): a `Duration`-typed struct field pulls in the package-level
/// `DurationMillis` wire helper (and, transitively, the `encoding/json` import it needs)
/// even when the crate has no sync functions or non-static methods — the only prior
/// triggers for `encoding/json`.
#[test]
fn generate_bindings_emits_duration_millis_helper_when_a_duration_field_exists() {
    use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "RateLimitConfig".to_string(),
            rust_path: "test_lib::RateLimitConfig".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "window".to_string(),
                ty: TypeRef::Duration,
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        binding.content.contains("type DurationMillis uint64"),
        "expected the DurationMillis wire helper. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("\"encoding/json\""),
        "DurationMillis's Marshal/UnmarshalJSON need encoding/json imported. Got:\n{}",
        binding.content
    );
    assert!(
        binding.content.contains("Window DurationMillis `json:\"window\"`"),
        "expected the field itself to use the wire-safe type. Got:\n{}",
        binding.content
    );
}

/// Counterpart of the above: a crate with no `Duration` field anywhere must not carry the
/// unused `DurationMillis` helper.
#[test]
fn generate_bindings_omits_duration_millis_helper_without_a_duration_field() {
    use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "PlainConfig".to_string(),
            rust_path: "test_lib::PlainConfig".to_string(),
            has_serde: true,
            fields: vec![FieldDef {
                name: "name".to_string(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let binding = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("binding.go"))
        .expect("binding.go present");

    assert!(
        !binding.content.contains("DurationMillis"),
        "no Duration field exists, so the helper must not be emitted. Got:\n{}",
        binding.content
    );
}

/// Replay of the write pipeline's stamping contract for a single emitted file.
///
/// `core_commands` only inserts a generated path into the set it hands
/// `finalize_hashes` when [`GeneratedFile::carries_alef_marker`] holds, and
/// `write_files_report` refuses to overwrite an existing markable file whose content
/// carries no marker. A `.go` file that fails this therefore gets neither provenance
/// nor future regeneration, silently. ~keep
fn assert_pipeline_stamps(file: &crate::core::backend::GeneratedFile) {
    use crate::core::hash;

    let path = file.path.display().to_string();
    assert!(
        file.carries_alef_marker(),
        "{path}: emitted without an alef marker and without `generated_header`, so the \
         path never reaches `finalize_hashes` and the write guard will refuse to rewrite it"
    );

    let on_disk = if hash::content_has_alef_marker(&file.content) {
        file.content.clone()
    } else {
        format!("{}\n{}", hash::header(hash::CommentStyle::DoubleSlash), file.content)
    };
    assert!(
        hash::content_has_alef_marker(&on_disk),
        "{path}: the bytes the writer puts on disk must carry the marker `finalize_hashes` \
         searches for, got:\n{on_disk}"
    );

    let inputs_hash = hash::compute_inputs_hash("sources", b"[workspace]\n");
    let body = hash::strip_hash_line(&on_disk);
    let stamped = hash::inject_hash_line(&body, &hash::compute_file_hash(&inputs_hash, &body));
    assert_eq!(
        hash::extract_hash(&stamped),
        Some(hash::compute_file_hash(&inputs_hash, &hash::strip_hash_line(&stamped))),
        "{path}: the injected alef:hash: line must re-verify the way `alef verify` derives it"
    );
}

#[test]
fn every_emitted_go_file_carries_a_hash_line_after_finalize() {
    use crate::core::ir::ApiSurface;

    let config = make_config();
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "1.2.3".to_string(),
        ..ApiSurface::default()
    };

    let files = GoBackend.generate_bindings(&api, &config).unwrap();

    let named = |name: &str| {
        files
            .iter()
            .find(|file| file.path.to_string_lossy().ends_with(name))
            .unwrap_or_else(|| {
                panic!(
                    "{name} must be emitted; got {:?}",
                    files.iter().map(|file| &file.path).collect::<Vec<_>>()
                )
            })
    };

    // Positive control: assert each file actually holds its generated payload, so the
    // stamping assertions below cannot pass over empty or missing output. ~keep
    assert!(
        named("binding.go").content.contains("package testlib"),
        "binding.go must hold real bindings, got:\n{}",
        named("binding.go").content
    );
    assert!(
        named("native_setup.go")
            .content
            .contains("RequireNativeSetup_1_2_3 = \"1.2.3\""),
        "native_setup.go must hold the version sentinel that changes on every release, got:\n{}",
        named("native_setup.go").content
    );
    assert!(
        named("embed_ffi.go").content.contains("//go:embed"),
        "embed_ffi.go must hold its embed directive, got:\n{}",
        named("embed_ffi.go").content
    );
    assert!(
        named("generate.go").content.contains("//go:generate"),
        "generate.go must hold its generate directive, got:\n{}",
        named("generate.go").content
    );
    assert!(
        named("cmd/setup/main.go").content.contains("func main()"),
        "cmd/setup/main.go must hold the setup tool, got:\n{}",
        named("cmd/setup/main.go").content
    );

    for file in &files {
        assert_pipeline_stamps(file);
    }
}

/// The whole-package invariant behind the cgo feature-macro defect: every FFI symbol the
/// generated Go sources call must still be *declared* after cgo runs the C preprocessor over the
/// header. cbindgen wraps each `#[cfg(feature = "x")]` export in `#if defined(PREFIX_FEATURE_X)`,
/// so a call site whose guard macro is not in the package's `#cgo CFLAGS` compiles to
/// `could not determine what C.<symbol> refers to`.
///
/// Both sides are derived, not pinned: the called set is read out of the emitted Go, the defined
/// set out of the emitted `#cgo` directives, and the required macro per symbol out of the IR gate
/// plus `c_consumer`'s symbol spelling — the same helper the FFI backend names its exports with.
/// A new gated export, a renamed macro, or a dropped `-D` all fail here.
///
/// Scope it cannot check: it models cgo's package-wide merge of `#cgo` directives (only
/// `binding.go` carries the `-D` line, as `service_file_preamble.jinja` already assumes for
/// `-I`), it only walks free functions, and it cannot see a feature the *library* was built
/// without — that is `warn_on_ffi_feature_drift`'s and the link step's job. ~keep
#[test]
fn every_gated_symbol_the_go_package_calls_has_its_guard_macro_defined() {
    use crate::codegen::c_consumer;
    use crate::core::ir::{ApiSurface, FunctionDef};
    use std::collections::BTreeSet;

    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi", "go"]
[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]
features = ["download", "document-render"]
[crates.ffi]
prefix = "test"
extra_features = ["wasm-http"]
[crates.go]
module = "github.com/test/test-lib"
"#,
    );
    let gates: Vec<(&str, Option<&str>)> = vec![
        ("ping", None),
        ("download", Some(r#"feature = "download""#)),
        (
            "render_document",
            Some(r#"all(feature = "document-render", feature = "download")"#),
        ),
        ("fetch_wasm", Some(r#"feature = "wasm-http""#)),
    ];
    let api = ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        functions: gates
            .iter()
            .map(|(name, cfg)| FunctionDef {
                name: (*name).to_string(),
                rust_path: format!("test_lib::{name}"),
                cfg: cfg.map(str::to_string),
                ..FunctionDef::default()
            })
            .collect(),
        ..ApiSurface::default()
    };

    let files = GoBackend.generate_bindings(&api, &config).unwrap();
    let go_sources: Vec<&str> = files
        .iter()
        .filter(|file| {
            let path = file.path.to_string_lossy().into_owned();
            // `cmd/setup` is a separate `package main`; cgo does not merge its directives into
            // the binding package, so it must not count towards either set. ~keep
            path.ends_with(".go") && !path.contains("/cmd/")
        })
        .map(|file| file.content.as_str())
        .collect();
    assert!(!go_sources.is_empty(), "control: the Go backend must emit .go sources");

    let called: HashSet<String> = go_sources
        .iter()
        .flat_map(|source| {
            source.split("C.test_").skip(1).map(|tail| {
                let end = tail
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .unwrap_or(tail.len());
                format!("test_{}", &tail[..end])
            })
        })
        .collect();

    let defined: HashSet<String> = go_sources
        .iter()
        .flat_map(|source| source.lines())
        .filter(|line| line.contains("#cgo") && line.contains("CFLAGS:"))
        .flat_map(str::split_whitespace)
        .filter_map(|token| token.strip_prefix("-D"))
        .map(|token| token.split('=').next().unwrap_or(token).to_string())
        .collect();

    let gated_symbol = c_consumer::free_function_symbol("test", "download");
    assert!(
        called.contains(&gated_symbol),
        "control: the Go package must call the gated export, otherwise this test is vacuous; called: {called:?}"
    );
    assert!(
        called.contains(&c_consumer::free_function_symbol("test", "ping")),
        "control: the Go package must also call the ungated export; called: {called:?}"
    );
    let declare_only = c_consumer::free_function_symbol("test", "fetch_wasm");
    assert!(
        !called.contains(&declare_only),
        "`extra_features` stay off, so the glue for {declare_only} must not be emitted at all"
    );
    assert!(
        !defined.contains("TEST_FEATURE_WASM_HTTP"),
        "a genuinely-disabled feature must stay genuinely invisible; defined: {defined:?}"
    );

    for func in &api.functions {
        let Some(cfg) = func.cfg.as_deref() else { continue };
        let symbol = c_consumer::free_function_symbol("test", &func.name);
        if !called.contains(&symbol) {
            continue;
        }
        let mut features = BTreeSet::new();
        crate::codegen::cfg::collect_cfg_feature_names(cfg, &mut features);
        for feature in features {
            let macro_name = crate::backends::go::cgo_features::guard_macro_name("test", &feature);
            assert!(
                defined.contains(&macro_name),
                "the Go package calls {symbol}, whose header declaration cbindgen guards with \
                 {macro_name}, but no #cgo CFLAGS defines it — cgo deletes the declaration. \
                 defined: {defined:?}"
            );
        }
    }
}
