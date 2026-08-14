use super::*;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

fn make_field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        version: Default::default(),
        name: name.to_string(),
        ty,
        optional,
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
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn make_typedef(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        fields,
        methods: vec![],
        is_opaque: false,
        is_clone: true,
        is_copy: false,
        is_trait: false,
        has_default: false,
        has_stripped_cfg_fields: false,
        is_return_type: false,
        serde_rename_all: None,
        has_serde: false,
        super_traits: vec![],
        doc: String::new(),
        cfg: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_variant_wrapper: false,
        has_lifetime_params: false,
        has_private_fields: false,
        version: Default::default(),
    }
}

#[test]
fn explicit_default_impl_preserves_serde_default_fn_instead_of_type_zero_value() {
    // Regression: this generator exists *because* a struct has field-level defaults that differ
    // from the derived Default, yet it called the context-free `default_value_for_field` and so
    // emitted `Default::default()` for `#[serde(default = "path")]` fields — the exact value it
    // was written to avoid. Against html-to-markdown's real `GridCell` that shipped
    // `GridCell.default().row_span == 0` while `default_span()` returns 1, and the kwargs
    // constructor in the same generated file returned the correct 1: two different defaults for
    // one field. ~keep
    let mut span = make_field(
        "row_span",
        TypeRef::Primitive(crate::core::ir::PrimitiveType::U32),
        false,
    );
    span.typed_default = Some(crate::core::ir::DefaultValue::FunctionCall("default_span".to_string()));

    let mut typ = make_typedef(
        "GridCell",
        vec![
            make_field("content", TypeRef::String, false),
            make_field("row", TypeRef::Primitive(crate::core::ir::PrimitiveType::U32), false),
            span,
        ],
    );
    typ.has_serde = true;

    let map_fn = |ty: &TypeRef| match ty {
        TypeRef::String => "String".to_string(),
        _ => "u32".to_string(),
    };

    let output = gen_struct_default_impl_explicit(&typ, &map_fn, &[])
        .expect("a struct with a field-level default must get an explicit Default impl");

    assert!(
        !output.contains("row_span: Default::default()"),
        "row_span must not fall back to the type's zero value, which is not `default_span()`:\n{output}"
    );
    assert!(
        output.contains("serde_json::from_str::<test_lib::GridCell>"),
        "row_span must recover the real serde default by deserializing a stub:\n{output}"
    );
}

#[test]
fn gen_enum_unit_variants_emit_ruby_symbols() {
    let enum_def = EnumDef {
        name: "Status".to_string(),
        rust_path: "test_lib::Status".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            EnumVariant {
                name: "Pending".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
            EnumVariant {
                name: "Done".to_string(),
                fields: vec![],
                doc: String::new(),
                is_default: false,
                serde_rename: None,
                binding_excluded: false,
                binding_exclusion_reason: None,
                is_tuple: false,
                originally_had_data_fields: false,
                cfg: None,
                version: Default::default(),
            },
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: false,
        has_default: false,
        serde_content: None,
        serde_tag: None,
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };
    let code = gen_enum(&enum_def);
    assert!(code.contains("enum Status"), "must emit enum definition");
    assert!(code.contains("to_symbol"), "unit enums use Ruby symbols");
    assert!(code.contains("\"pending\""), "variant snake_case symbol key");
}

fn make_variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        doc: String::new(),
        is_default: false,
        serde_rename: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        is_tuple: false,
        originally_had_data_fields: false,
        cfg: None,
        version: Default::default(),
    }
}

fn make_data_enum(name: &str, serde_tag: Option<&str>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants: vec![
            make_variant("Png", vec![]),
            make_variant("Jpeg", vec![make_field("quality", TypeRef::String, false)]),
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: serde_tag.map(str::to_string),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

#[test]
fn gen_enum_wraps_string_for_internally_tagged_enum() {
    // For an internally-tagged enum (`#[serde(tag = "...")]`), serde cannot deserialize a bare
    let code = gen_enum(&make_data_enum("ImageOutputFormat", Some("type")));
    assert!(
        code.contains(r#".or_else(|_| serde_json::from_value(serde_json::json!({ "type": json_str })))"#),
        "expected tagged string wrap for internally-tagged enum: {code}"
    );
}

#[test]
fn gen_enum_keeps_bare_string_for_externally_tagged_enum() {
    // An externally-tagged data enum (no `#[serde(tag)]`) must not gain the tag-wrap branch.
    let code = gen_enum(&make_data_enum("ExternallyTagged", None));
    assert!(
        !code.contains("serde_json::from_value(serde_json::json!({"),
        "externally-tagged enum must not wrap the string in a tag object: {code}"
    );
    assert!(
        code.contains("serde_json::from_str(&json_str)"),
        "data enum must keep the from_str path: {code}"
    );
}

#[test]
fn gen_enum_emits_adjacent_serde_representation() {
    let mut enum_def = make_data_enum("OperationResult", Some("type"));
    enum_def.serde_content = Some("output".to_string());
    enum_def.variants[1].is_tuple = true;
    enum_def.variants[1].fields[0].name = "_0".to_string();

    let code = gen_enum(&enum_def);

    assert!(code.contains(r#"#[serde(tag = "type", content = "output")]"#));
    assert!(code.contains("Jpeg(String)"));
    assert!(code.contains("Self::Jpeg(_0) => Some(_0)"), "{code}");
    assert!(!code.contains("Self::Jpeg { _0 }"), "{code}");
    syn::parse_file(&code).unwrap_or_else(|error| panic!("generated Rust must parse: {error}\n{code}"));
}

#[test]
fn adjacent_tuple_default_uses_tuple_constructor_syntax() {
    let mut enum_def = make_data_enum("OperationResult", Some("type"));
    enum_def.serde_content = Some("output".to_string());
    enum_def.variants[1].is_tuple = true;
    enum_def.variants[1].is_default = true;
    enum_def.variants[1].fields[0].name = "_0".to_string();

    let code = gen_enum(&enum_def);

    assert!(code.contains("Self::Jpeg(Default::default())"), "{code}");
    assert!(!code.contains("Self::Jpeg { _0:"), "{code}");
    syn::parse_file(&code).unwrap_or_else(|error| panic!("generated Rust must parse: {error}\n{code}"));
}

#[test]
fn gen_struct_emits_magnus_wrap_attribute() {
    let typ = make_typedef("Config", vec![make_field("value", TypeRef::String, false)]);
    let mapper = crate::backends::magnus::type_map::MagnusMapper;
    let api = crate::core::ir::ApiSurface {
        crate_name: "test_lib".to_string(),
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
    let code = gen_struct(&typ, &mapper, "TestLib", &api, false, &[]);
    assert!(code.contains("magnus::wrap"), "struct must have magnus::wrap");
    assert!(code.contains("struct Config"), "must emit struct Config");
}

#[test]
fn gen_opaque_struct_emits_arc_inner() {
    let typ = make_typedef("Handle", vec![]);
    let code = gen_opaque_struct(&typ, "test_lib", "TestLib");
    assert!(code.contains("inner: Arc<"), "opaque struct must have Arc inner");
    assert!(code.contains("struct Handle"), "must emit struct Handle");
}

use crate::core::ir::MethodDef;

fn shape_enum() -> EnumDef {
    EnumDef {
        name: "Shape".to_string(),
        rust_path: "test_lib::Shape".to_string(),
        original_rust_path: String::new(),
        variants: vec![
            make_variant("Circle", vec![make_field("radius", TypeRef::String, false)]),
            make_variant(
                "Rect",
                vec![
                    make_field("width", TypeRef::String, false),
                    make_field("height", TypeRef::String, false),
                ],
            ),
        ],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    }
}

#[test]
fn variant_constructors_emit_singleton_per_struct_variant() {
    let code = gen_data_enum_variant_constructors(&shape_enum());

    assert!(code.contains("impl Shape {"), "must emit an impl block: {code}");
    assert!(
        code.contains("pub fn _factory_circle(radius: String) -> Self"),
        "{code}"
    );
    assert!(code.contains("Self::Circle { radius }"), "{code}");
    assert!(
        code.contains("pub fn _factory_rect(width: String, height: String) -> Self"),
        "{code}"
    );
    assert!(code.contains("Self::Rect { width, height }"), "{code}");
}

#[test]
fn variant_constructors_use_serde_shaped_named_field_type() {
    let def = EnumDef {
        name: "Wrapper".to_string(),
        rust_path: "test_lib::Wrapper".to_string(),
        original_rust_path: String::new(),
        variants: vec![make_variant(
            "Llm",
            vec![
                make_field("llm", TypeRef::Named("LlmConfig".to_string()), false),
                make_field(
                    "opts",
                    TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                    false,
                ),
            ],
        )],
        methods: vec![],
        doc: String::new(),
        cfg: None,
        is_copy: false,
        has_serde: true,
        has_default: false,
        serde_content: None,
        serde_tag: Some("type".to_string()),
        serde_untagged: false,
        serde_rename_all: None,
        binding_excluded: false,
        binding_exclusion_reason: None,
        excluded_variants: vec![],
        version: Default::default(),
    };

    let code = gen_data_enum_variant_constructors(&def);

    assert!(
        code.contains("pub fn _factory_llm(llm: LlmConfig, opts: String) -> Self"),
        "{code}"
    );
    assert!(code.contains("Self::Llm { llm, opts }"), "{code}");
    assert!(
        !code.contains("_core"),
        "magnus enum is binding-shaped, no core conversion: {code}"
    );
}

#[test]
fn variant_constructors_skip_unit_tuple_and_excluded() {
    let mut tuple_variant = make_variant("Pair", vec![make_field("_0", TypeRef::String, false)]);
    tuple_variant.is_tuple = true;
    let mut excluded = make_variant("Hidden", vec![make_field("value", TypeRef::String, false)]);
    excluded.binding_excluded = true;

    let def = EnumDef {
        variants: vec![
            make_variant("Empty", vec![]),
            tuple_variant,
            excluded,
            make_variant("Real", vec![make_field("value", TypeRef::String, false)]),
        ],
        ..shape_enum()
    };

    let code = gen_data_enum_variant_constructors(&def);

    assert!(!code.contains("_factory_empty"), "{code}");
    assert!(!code.contains("_factory_pair"), "{code}");
    assert!(!code.contains("_factory_hidden"), "{code}");
    assert!(code.contains("pub fn _factory_real(value: String) -> Self"), "{code}");
}

#[test]
fn variant_constructors_yield_to_hand_written_method() {
    let def = EnumDef {
        methods: vec![MethodDef {
            name: "circle".to_string(),
            is_static: true,
            ..Default::default()
        }],
        ..shape_enum()
    };

    let code = gen_data_enum_variant_constructors(&def);

    assert!(
        !code.contains("Self::Circle"),
        "consumer method must win for Circle: {code}"
    );
    assert!(
        code.contains("pub fn _factory_rect(width: String, height: String) -> Self"),
        "{code}"
    );
}

#[test]
fn variant_constructors_empty_for_unit_only_enum() {
    let def = EnumDef {
        variants: vec![make_variant("A", vec![]), make_variant("B", vec![])],
        ..shape_enum()
    };
    let code = gen_data_enum_variant_constructors(&def);
    assert!(code.is_empty(), "expected no output for unit-only enum: {code}");
}

/// Issue #232: an adjacently-tagged enum (`tag` + `content`) emits tuple-form variants
/// exactly like an untagged one, but the conversion match arms keyed only on
/// `serde_untagged` and so destructured struct-form. Definition and `From` impls
/// disagreed in shape and rustc rejected them (E0559 / E0769). Both sides must now
/// consult the same predicate.
#[test]
fn adjacently_tagged_tuple_variant_uses_tuple_form_in_both_definition_and_conversions() {
    use crate::codegen::conversions::helpers::variant_emits_tuple_form;

    let mut adjacent = make_data_enum("OperationResult", Some("type"));
    adjacent.serde_content = Some("output".to_string());
    adjacent.variants[1].is_tuple = true;
    adjacent.variants[1].fields[0].name = "_0".to_string();

    // The definition emits tuple form ...
    let code = gen_enum(&adjacent);
    assert!(code.contains("Jpeg(String)"), "{code}");
    assert!(!code.contains("Self::Jpeg { _0 }"), "{code}");

    // ... and the shared predicate agrees, so conversions destructure the same way.
    assert!(
        variant_emits_tuple_form(&adjacent, &adjacent.variants[1]),
        "adjacently-tagged tuple variant must report tuple form to the conversion layer"
    );

    // Untagged keeps working.
    let mut untagged = make_data_enum("OperationResult", None);
    untagged.serde_untagged = true;
    untagged.variants[1].is_tuple = true;
    untagged.variants[1].fields[0].name = "_0".to_string();
    assert!(variant_emits_tuple_form(&untagged, &untagged.variants[1]));

    // A non-tuple variant of an adjacently-tagged enum keeps struct form.
    assert!(
        !variant_emits_tuple_form(&adjacent, &adjacent.variants[0]),
        "struct-form variants must not be reported as tuple form"
    );
}
