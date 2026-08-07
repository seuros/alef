use super::*;

/// Regression test for xberg#392: `StructuredDataResult.metadata` (a
/// `HashMap<String, String>`) was excluded from `#[php(prop)]` and from the generated
/// constructor even though ext-php-rs 0.15's `HashMap<K, V>` conversions
/// (`types/array/conversions/hash_map.rs`) give `IntoZval`/`FromZval` to any String-keyed
/// map whose value type is itself IntoZval/FromZval-compatible. PHP callers could read the
/// field via `get_metadata()` but had no way to ever set it.
///
/// A `Map<String, String>` field must now be treated exactly like a scalar field: it gets
/// `#[php(prop, name = "...")]` on the struct field and a matching constructor parameter.
#[test]
fn string_keyed_string_valued_map_field_becomes_a_real_php_prop() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![TypeDef {
            name: "StructuredDataResult".to_string(),
            rust_path: "test_lib::StructuredDataResult".to_string(),
            original_rust_path: String::new(),
            fields: vec![
                make_field("content", TypeRef::String, false),
                make_field(
                    "metadata",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    false,
                ),
            ],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: true,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            super_traits: vec![],
            doc: String::new(),
            cfg: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            is_variant_wrapper: false,
            has_lifetime_params: false,
            has_private_fields: false,
            version: Default::default(),
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

    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs generated");
    let content = &lib.content;

    assert!(
        content.contains("php(prop, name = \"metadata\")"),
        "HashMap<String, String> field must be a real #[php(prop)], not just a getter:\n{content}"
    );

    assert!(
        content.contains("metadata: HashMap<String, String>")
            || content.contains("metadata: std::collections::HashMap<String, String>"),
        "constructor must accept `metadata` as a named param so PHP can set it:\n{content}"
    );

    assert!(
        !content.contains("metadata: Default::default()"),
        "metadata must no longer be silently Default::default()-ed out of the constructor:\n{content}"
    );
}

/// Negative control: a `Map<String, Named>` whose value type is a plain (non-enum) struct
/// cannot cross the ext-php-rs FFI boundary as a settable property — `HashMap<K, V>` only
/// implements `IntoZval`/`FromZval` when `V` itself does, and a `#[php_class]`-derived struct
/// does not. This field must remain getter-only, and the omission must be visible in the
/// generated source (not silently dropped), per the standing rule against silent alef drops
/// (xberg#374, #380).
#[test]
fn map_of_structs_field_stays_getter_only_and_documents_the_omission() {
    let backend = PhpBackend;

    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        types: vec![
            TypeDef {
                name: "SchemeInfo".to_string(),
                rust_path: "test_lib::SchemeInfo".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field("scheme", TypeRef::String, false)],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
            TypeDef {
                name: "OpenApiConfig".to_string(),
                rust_path: "test_lib::OpenApiConfig".to_string(),
                original_rust_path: String::new(),
                fields: vec![make_field(
                    "security_schemes",
                    TypeRef::Map(
                        Box::new(TypeRef::String),
                        Box::new(TypeRef::Named("SchemeInfo".to_string())),
                    ),
                    false,
                )],
                methods: vec![],
                is_opaque: false,
                is_clone: true,
                is_copy: false,
                is_trait: false,
                has_default: true,
                has_stripped_cfg_fields: false,
                is_return_type: false,
                serde_rename_all: None,
                has_serde: true,
                super_traits: vec![],
                doc: String::new(),
                cfg: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_variant_wrapper: false,
                has_lifetime_params: false,
                has_private_fields: false,
                version: Default::default(),
            },
        ],
        functions: vec![],
        enums: vec![],
        errors: vec![],
        excluded_type_paths: ::std::collections::HashMap::new(),
        excluded_trait_names: ::std::collections::HashSet::new(),
        services: vec![],
        handler_contracts: vec![],
        unsupported_public_items: Vec::new(),
    };

    let files = backend.generate_bindings(&api, &make_config()).unwrap();
    let lib = files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("lib.rs generated");
    let content = &lib.content;

    assert!(
        !content.contains("php(prop, name = \"securitySchemes\")"),
        "Map<String, Struct> cannot be a settable #[php(prop)] — value type isn't IntoZval/FromZval:\n{content}"
    );

    assert!(
        content.contains("pub fn get_security_schemes"),
        "the field must still be reachable via a getter:\n{content}"
    );

    assert!(
        content.contains("this field's value type cannot cross"),
        "the omission from #[php(prop)]/constructor must be documented in the generated source, not silent:\n{content}"
    );
}
