use super::super::FfiBackend;
use super::common::*;
use crate::core::backend::Backend;
use crate::core::ir::*;

/// Regression test: Option<Option<Primitive>> (update-struct pattern) must generate
/// a getter that returns the primitive type — not *mut c_char — and collapses both
/// None cases to the primitive's zero sentinel.
#[test]
fn test_option_option_primitive_getter_returns_primitive_type() {
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "ConfigUpdate".to_string(),
            rust_path: "my_lib::ConfigUpdate".to_string(),
            original_rust_path: String::new(),
            fields: vec![FieldDef {
                version: Default::default(),
                name: "max_depth".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::Usize))),
                optional: true,
                default: None,
                doc: String::new(),
                sanitized: false,
                is_boxed: false,
                type_rust_path: None,
                cfg: None,
                typed_default: None,
                core_wrapper: crate::core::ir::CoreWrapper::None,
                vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
                newtype_wrapper: None,
                serde_rename: None,
                serde_flatten: false,
                serde_with: None,
                serde_skip_serializing_if: false,
                original_type: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
            }],
            methods: vec![],
            is_opaque: false,
            is_clone: true,
            is_copy: false,
            is_trait: false,
            has_default: false,
            has_stripped_cfg_fields: false,
            is_return_type: false,
            serde_rename_all: None,
            has_serde: true,
            serde_container_default: false,
            serde_container_from: None,
            serde_container_into: None,
            serde_container_try_from: None,
            serde_transparent: false,
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
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("-> usize"),
        "expected `-> usize` in getter but got:\n{}",
        lib.content
    );
    assert!(
        !lib.content.contains("-> *mut std::ffi::c_char"),
        "getter must not return *mut c_char for Option<Option<usize>>"
    );

    assert!(
        lib.content.contains("None => 0"),
        "expected `None => 0` sentinel in generated getter"
    );

    assert!(
        lib.content.contains("*inner_val"),
        "expected `*inner_val` deref for inner primitive in generated getter"
    );
}

/// Build a minimal `ApiSurface` with one struct that has a Named field,
/// controlling `is_clone` on the field's referenced type.
fn api_with_named_field(field_type: &str, is_clone: bool) -> ApiSurface {
    let holder = TypeDef {
        name: "Holder".to_string(),
        rust_path: "my_lib::Holder".to_string(),
        original_rust_path: String::new(),
        fields: vec![FieldDef {
            version: Default::default(),
            name: "inner".to_string(),
            ty: TypeRef::Named(field_type.to_string()),
            optional: false,
            default: None,
            doc: String::new(),
            sanitized: false,
            is_boxed: false,
            type_rust_path: None,
            cfg: None,
            typed_default: None,
            core_wrapper: crate::core::ir::CoreWrapper::None,
            vec_inner_core_wrapper: crate::core::ir::CoreWrapper::None,
            newtype_wrapper: None,
            serde_rename: None,
            serde_flatten: false,
            serde_with: None,
            serde_skip_serializing_if: false,
            original_type: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
        }],
        methods: vec![],
        is_opaque: false,
        is_clone: false,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_from: None,
        serde_container_into: None,
        serde_container_try_from: None,
        serde_transparent: false,
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
    let named_type = TypeDef {
        name: field_type.to_string(),
        rust_path: format!("my_lib::{field_type}"),
        original_rust_path: String::new(),
        fields: vec![],
        methods: vec![],
        is_opaque: true,
        is_clone,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        serde_container_default: false,
        serde_container_from: None,
        serde_container_into: None,
        serde_container_try_from: None,
        serde_transparent: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    };
    ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![holder, named_type],
        functions: vec![],
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
fn test_named_field_non_clone_does_not_return_borrow_as_owned() {
    let api = api_with_named_field("LanguageRegistry", false);
    let config = sample_config();
    let backend = FfiBackend;

    let error = backend
        .generate_bindings(&api, &config)
        .expect_err("non-Clone named fields cannot produce owned handles");
    assert!(
        error
            .to_string()
            .contains("non-Copy, non-Clone type `LanguageRegistry`")
    );
}

/// Clone-capable Named-type fields must still emit `.clone()` in the accessor.
#[test]
fn test_named_field_clone_capable_emits_clone() {
    let api = api_with_named_field("ConversionOptions", true);
    let config = sample_config();
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains(".clone()"),
        "Clone-capable Named field must emit .clone() in accessor:\n{}",
        lib.content
    );
}

#[test]
fn test_optional_trait_bridge_handle_getter_clones_owned_handle() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "sample"

[[crates.trait_bridges]]
trait_name = "DocumentVisitor"
type_alias = "VisitorHandle"
bind_via = "options_field"
options_type = "RenderOptions"
options_field = "visitor"

"#,
    );
    let api = ApiSurface {
        crate_name: "my-lib".to_string(),
        version: "1.0.0".to_string(),
        types: vec![TypeDef {
            name: "RenderOptions".to_string(),
            rust_path: "my_lib::RenderOptions".to_string(),
            fields: vec![FieldDef {
                name: "visitor".to_string(),
                ty: TypeRef::Named("VisitorHandle".to_string()),
                optional: true,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    };

    let files = FfiBackend
        .generate_bindings(&api, &config)
        .expect("trait bridge handle getter");
    let lib = files.iter().find(|file| file.path.ends_with("lib.rs")).unwrap();
    let accessor = lib
        .content
        .split("fn sample_render_options_visitor")
        .nth(1)
        .expect("visitor accessor");

    assert!(accessor.contains("Some(val) =>"), "{accessor}");
    assert!(accessor.contains("insert_handle(val.clone())"), "{accessor}");
    assert!(!accessor.contains("Some(val) => {\n            0"), "{accessor}");
}

#[test]
fn test_options_field_visitor_callbacks_use_configured_renderer_setter() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "syn"
visitor_callbacks = true

[[crates.trait_bridges]]
trait_name = "SyntaxWalker"
type_alias = "SyntaxWalkerHandle"
param_name = "renderer"
bind_via = "options_field"
options_type = "ParseOptions"
options_field = "renderer"
context_type = "SyntaxContext"
result_type = "WalkOutcome"
"#,
    );
    let mut api = sample_api();
    api.types.push(TypeDef {
        name: "SyntaxWalker".to_string(),
        rust_path: "my_lib::syntax::SyntaxWalker".to_string(),
        methods: vec![MethodDef {
            name: "visit_token".to_string(),
            params: vec![ParamDef {
                name: "context".to_string(),
                ty: TypeRef::Named("SyntaxContext".to_string()),
                is_ref: true,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Named("WalkOutcome".to_string()),
            receiver: Some(ReceiverKind::RefMut),
            cfg: None,
            is_async: false,
            is_static: false,
            error_type: None,
            doc: String::new(),
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }],
        is_trait: true,
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "SyntaxContext".to_string(),
        rust_path: "my_lib::syntax::SyntaxContext".to_string(),
        fields: vec![FieldDef {
            name: "rule_name".to_string(),
            ty: TypeRef::String,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "ParseOptions".to_string(),
        rust_path: "my_lib::ParseOptions".to_string(),
        is_clone: true,
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "ParseResult".to_string(),
        rust_path: "my_lib::ParseResult".to_string(),
        is_clone: true,
        is_return_type: true,
        ..TypeDef::default()
    });
    api.enums.push(EnumDef {
        name: "WalkOutcome".to_string(),
        rust_path: "my_lib::syntax::WalkOutcome".to_string(),
        variants: vec![
            EnumVariant {
                name: "Continue".to_string(),
                is_default: true,
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Stop".to_string(),
                ..EnumVariant::default()
            },
        ],
        has_serde: true,
        has_default: false,
        ..EnumDef::default()
    });
    api.functions.push(FunctionDef {
        name: "parse".to_string(),
        rust_path: "my_lib::parse".to_string(),
        params: vec![
            ParamDef {
                name: "source".to_string(),
                ty: TypeRef::String,
                is_ref: true,
                ..ParamDef::default()
            },
            ParamDef {
                name: "options".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("ParseOptions".to_string()))),
                optional: true,
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::Named("ParseResult".to_string()),
        error_type: Some("ParseError".to_string()),
        ..FunctionDef::default()
    });
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("syn_options_set_renderer"),
        "options-field setter must derive from configured renderer field"
    );
    assert!(
        !lib.content.contains("syn_options_set_visitor_handle"),
        "options-field mode must not emit the legacy visitor_handle setter"
    );
    assert!(
        lib.content.contains("pub struct SynVisitorCallbacks"),
        "Java callback lifecycle support should remain available"
    );
    assert!(
        lib.content.contains("syn_visitor_create") && lib.content.contains("syn_visitor_free"),
        "visitor create/free symbols should remain available"
    );
    let convert_count = lib.content.matches("fn syn_parse(").count();
    assert_eq!(convert_count, 1, "syn_parse must appear exactly once");
    assert!(
        !lib.content.contains("syn_parse_with_visitor"),
        "options-field mode must not emit the legacy with_visitor wrapper"
    );
    assert!(
        lib.content
            .contains("fn syn_options_set_renderer(options: AlefHandle, visitor: AlefHandle)"),
        "options-field setter must use the public scalar managed-handle ABI"
    );
    assert!(
        !lib.content.contains("visitor: *mut SynSyntaxWalkerBridge"),
        "options-field setter must not require the trait-bridge handle when visitor_callbacks is enabled"
    );
    assert!(
        lib.content.contains("options: AlefHandle") && lib.content.contains(") -> AlefHandle"),
        "options-field wrapper parameters and results must use scalar managed handles"
    );
    assert!(
        lib.content.contains("with_handle::<my_lib::ParseOptions")
            && lib.content.contains("with_handle_mut::<SynVisitor")
            && lib.content.contains("insert_handle(result)"),
        "options-field wrapper must resolve every managed value through the handle registry"
    );
    syn::parse_file(&lib.content).expect("scalar options-field bridge output must parse as Rust");
    assert!(
        !lib.content.contains("SynSyntaxWalkerBridge"),
        "legacy visitor callbacks must not ship an unattached generic bridge with an independent destructor"
    );
    assert!(!lib.content.contains("syn_syntax_walker_bridge_new"));
    assert!(!lib.content.contains("syn_syntax_walker_bridge_free"));
}

#[test]
fn test_options_field_bridge_generates_non_convert_function_from_ir() {
    let config = resolved_one(
        r#"
[workspace]
languages = ["ffi"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "doc"

[[crates.trait_bridges]]
trait_name = "HtmlVisitor"
type_alias = "RenderHandle"
param_name = "renderer"
bind_via = "options_field"
options_type = "RenderSettings"
options_field = "renderer"
"#,
    );
    let mut api = visitor_api();
    api.types.push(TypeDef {
        name: "RenderSettings".to_string(),
        rust_path: "my_lib::RenderSettings".to_string(),
        fields: vec![],
        is_clone: true,
        ..TypeDef::default()
    });
    api.types.push(TypeDef {
        name: "RenderedDocument".to_string(),
        rust_path: "my_lib::RenderedDocument".to_string(),
        fields: vec![],
        is_clone: true,
        ..TypeDef::default()
    });
    api.functions.push(FunctionDef {
        name: "render_document".to_string(),
        rust_path: "my_lib::render_document".to_string(),
        original_rust_path: String::new(),
        params: vec![
            ParamDef {
                name: "source".to_string(),
                ty: TypeRef::String,
                is_ref: true,
                ..ParamDef::default()
            },
            ParamDef {
                name: "settings".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Named("RenderSettings".to_string()))),
                optional: true,
                ..ParamDef::default()
            },
        ],
        return_type: TypeRef::Named("RenderedDocument".to_string()),
        is_async: false,
        error_type: Some("RenderError".to_string()),
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
    });
    let backend = FfiBackend;

    let files = backend.generate_bindings(&api, &config).unwrap();
    let lib = files.iter().find(|f| f.path.ends_with("lib.rs")).unwrap();

    assert!(
        lib.content.contains("fn doc_render_document("),
        "must generate IR-derived symbol"
    );
    assert!(
        lib.content.contains("settings: AlefHandle"),
        "must carry the configured options type through the managed handle ABI"
    );
    assert!(
        lib.content.contains(") -> AlefHandle"),
        "must carry the actual return type through the managed handle ABI"
    );
    assert!(
        lib.content.contains("with_handle::<my_lib::RenderSettings") && lib.content.contains("insert_handle(result)"),
        "must resolve options and register results through the handle registry"
    );
    assert!(
        lib.content
            .contains("match my_lib::render_document(source_rs, settings_rs)"),
        "must call actual core function with actual parameters"
    );
    assert!(
        !lib.content.contains("my_lib::convert("),
        "must not hardcode conversion call"
    );
    assert!(
        !lib.content.contains("ConversionOptions") && !lib.content.contains("ConversionResult"),
        "must not leak conversion-shaped type names in generic wrapper"
    );
    // ~keep Every failure path of a bridge returning `AlefHandle` must yield the scalar
    // sentinel 0. `catch_ffi_panic(0, ..)` and the terminal arms were migrated to the scalar
    // ABI, but the null-parameter guard and the UTF-8 guard still emitted
    // `std::ptr::null_mut()` -- a `*mut` where a `u64` is expected, so the generated crate did
    // not compile at all (E0308). It reached h2m's committed ffi crate that way.
    //
    // `rfind`, not `find`: the backend emits a "Not implemented" stub for this symbol BEFORE
    // the real bridge, and the stub body contains no sentinel at all. Anchoring on the first
    // match slices the stub and the check passes no matter what the bridge emits -- verified,
    // that is exactly how the first version of this assertion passed against the bug it was
    // written to catch. The positive assertions below keep an incorrectly anchored slice loud.
    let definition = "pub unsafe extern \"C\" fn doc_render_document(";
    let start = lib
        .content
        .rfind(definition)
        .expect("options-field bridge definition must exist");
    let after = &lib.content[start..];
    let bridge_body = after.split_once("\npub ").map_or(after, |(body, _)| body);
    assert!(
        bridge_body.contains("is_null()") && bridge_body.contains("catch_ffi_panic(0"),
        "slice must cover the real bridge body, or the sentinel check below is vacuous: {bridge_body}"
    );
    assert!(
        !bridge_body.contains("null_mut"),
        "AlefHandle bridge must return the scalar sentinel on every failure path: {bridge_body}"
    );

    syn::parse_file(&lib.content).expect("generic scalar options-field bridge output must parse as Rust");
}

/// Regression: a field marked `binding_excluded` (e.g. a global `[crates.exclude].fields`
/// entry hiding a pipeline-invariant field of a foreign `source_crate` type) must NOT get a
/// generated FFI accessor. Previously the FFI backend filtered only on `sanitized`, so an
/// excluded field still emitted a getter — and a name-colliding foreign type (h2m
/// `OutputFormat` vs host `OutputFormat`) made that getter fail to compile.
#[test]
fn test_binding_excluded_field_emits_no_accessor() {
    let backend = FfiBackend;
    let config = sample_config();

    let baseline = backend
        .generate_bindings(&sample_api(), &config)
        .unwrap()
        .into_iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .unwrap()
        .content;
    assert!(
        baseline.contains("_verbose("),
        "baseline should emit a `verbose` accessor"
    );

    let mut api = sample_api();
    let verbose = api.types[0].fields.iter_mut().find(|f| f.name == "verbose").unwrap();
    verbose.binding_excluded = true;
    verbose.binding_exclusion_reason = Some("exclude.fields".to_string());

    let excluded = backend
        .generate_bindings(&api, &config)
        .unwrap()
        .into_iter()
        .find(|f| f.path.ends_with("lib.rs"))
        .unwrap()
        .content;
    assert!(
        !excluded.contains("_verbose("),
        "excluded field must not emit an accessor, got:\n{excluded}"
    );
    assert!(
        excluded.contains("_name("),
        "sibling non-excluded fields must still emit accessors"
    );
}
