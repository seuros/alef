use super::{gen_data_enum_variant_constructor_stubs, gen_struct_constructor_stub_params, struct_needs_from_json_stub};
use crate::backends::php::gen_bindings::functions::has_unsupported_static_params;
use crate::core::ir::{
    CoreWrapper, EnumDef, EnumVariant, FieldDef, MethodDef, ParamDef, PrimitiveType, TypeDef, TypeRef,
};
use ahash::AHashSet;

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
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
        core_wrapper: CoreWrapper::None,
        vec_inner_core_wrapper: CoreWrapper::None,
        newtype_wrapper: None,
        serde_rename: None,
        serde_flatten: false,
        binding_excluded: false,
        binding_exclusion_reason: None,
        original_type: None,
    }
}

fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
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

fn enum_def(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        original_rust_path: String::new(),
        variants,
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

fn shape_enum() -> EnumDef {
    enum_def(
        "Shape",
        vec![
            variant(
                "Circle",
                vec![field("radius", TypeRef::Primitive(PrimitiveType::F64), false)],
            ),
            variant(
                "Rect",
                vec![
                    field("width", TypeRef::Primitive(PrimitiveType::U32), false),
                    field("height", TypeRef::Primitive(PrimitiveType::U32), false),
                ],
            ),
        ],
    )
}

#[test]
fn emits_static_factory_per_struct_variant() {
    let stubs = gen_data_enum_variant_constructor_stubs(&shape_enum()).join("");

    assert!(
        stubs.contains("public static function circle(float $radius): Shape"),
        "{stubs}"
    );
    assert!(
        stubs.contains("public static function rect(int $width, int $height): Shape"),
        "{stubs}"
    );
}

#[test]
fn maps_named_dto_field_to_its_type() {
    let def = enum_def(
        "Source",
        vec![variant(
            "Llm",
            vec![field("config", TypeRef::Named("LlmConfig".to_string()), false)],
        )],
    );

    let stubs = gen_data_enum_variant_constructor_stubs(&def).join("");

    assert!(
        stubs.contains("public static function llm(LlmConfig $config): Source"),
        "{stubs}"
    );
}

#[test]
fn emits_param_phpdoc_for_map_and_vec_variant_fields() {
    // `@param array<...>` PHPDoc, otherwise PHPStan (level max) flags the bare `array`
    let def = enum_def(
        "CacheBackend",
        vec![
            variant(
                "OpenDal",
                vec![
                    field("scheme", TypeRef::String, false),
                    field(
                        "config",
                        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                        false,
                    ),
                ],
            ),
            variant(
                "Tags",
                vec![field("labels", TypeRef::Vec(Box::new(TypeRef::String)), false)],
            ),
        ],
    );

    let stubs = gen_data_enum_variant_constructor_stubs(&def).join("");

    assert!(
        stubs.contains("/** @param array<string, string> $config */"),
        "map parameter should get a typed @param PHPDoc:\n{stubs}"
    );
    assert!(
        stubs.contains("/** @param array<string> $labels */"),
        "vec parameter should get a typed @param PHPDoc:\n{stubs}"
    );
    assert!(
        stubs.contains("public static function openDal(string $scheme, array $config): CacheBackend"),
        "{stubs}"
    );
}

#[test]
fn optional_field_is_nullable_with_default() {
    let def = enum_def(
        "Source",
        vec![variant("Tag", vec![field("label", TypeRef::String, true)])],
    );

    let stubs = gen_data_enum_variant_constructor_stubs(&def).join("");

    assert!(
        stubs.contains("public static function tag(?string $label = null): Source"),
        "{stubs}"
    );
}

#[test]
fn skips_unit_tuple_excluded_and_sanitized_variants() {
    let mut tuple_variant = variant("Pair", vec![field("_0", TypeRef::String, false)]);
    tuple_variant.is_tuple = true;
    let mut excluded = variant("Hidden", vec![field("value", TypeRef::String, false)]);
    excluded.binding_excluded = true;
    let mut sanitized_field = field("raw", TypeRef::String, false);
    sanitized_field.sanitized = true;
    let sanitized_variant = variant("Raw", vec![sanitized_field]);

    let def = enum_def(
        "Shape",
        vec![
            variant("Empty", vec![]),
            tuple_variant,
            excluded,
            sanitized_variant,
            variant("Real", vec![field("value", TypeRef::String, false)]),
        ],
    );

    let stubs = gen_data_enum_variant_constructor_stubs(&def).join("");

    assert!(!stubs.contains("function empty("), "{stubs}");
    assert!(!stubs.contains("function pair("), "{stubs}");
    assert!(!stubs.contains("function hidden("), "{stubs}");
    assert!(!stubs.contains("function raw("), "{stubs}");
    assert!(
        stubs.contains("public static function real(string $value): Shape"),
        "{stubs}"
    );
}

/// Regression for the `ContentPart` bug: a hand-written inherent static method
/// (`enum_def.methods`, extracted from a separate `impl EnumType { .. }` block) is never forwarded
/// into the generated `#[php_impl]` block, so suppressing the derived factory stub on a name
/// collision left the stub disagreeing with (and hiding) a reachable runtime method. The stub must
/// declare a factory for every data-carrying variant, matching `gen_flat_data_enum_variant_constructors`.
#[test]
fn emits_factory_stub_even_with_colliding_hand_written_method() {
    let def = EnumDef {
        methods: vec![MethodDef {
            name: "circle".to_string(),
            is_static: true,
            ..Default::default()
        }],
        ..shape_enum()
    };

    let stubs = gen_data_enum_variant_constructor_stubs(&def).join("");

    assert!(
        stubs.contains("public static function circle(float $radius): Shape"),
        "{stubs}"
    );
    assert!(
        stubs.contains("public static function rect(int $width, int $height): Shape"),
        "{stubs}"
    );
}

/// Regression: the real extension (`gen_bindings/types/structs.rs`'s `use_from_json` gate)
/// emits `#[php(name = "from_json")]` for a serde struct with a non-scalar (named/complex)
/// field, since `#[php(constructor)]` can't represent that field. The PHPStan stub must declare
/// the same static constructor or the method is invisible to editors and static analysis even
/// though it's the only way to build the type's nested config from PHP.
#[test]
fn needs_from_json_stub_for_struct_with_named_field() {
    let typ = TypeDef {
        name: "Wrapper".to_string(),
        has_serde: true,
        fields: vec![field("inner", TypeRef::Named("Nested".to_string()), false)],
        ..Default::default()
    };

    assert!(struct_needs_from_json_stub(&typ, &ahash::AHashSet::new()));
}

/// A struct with only scalar fields, no `Default` impl, and no field defaults is fully
/// constructible via `#[php(constructor)]` alone — the extension does not emit `from_json`
/// for it, so the stub must not claim one exists either.
#[test]
fn does_not_need_from_json_stub_for_plain_scalar_struct() {
    let typ = TypeDef {
        name: "Point".to_string(),
        has_serde: true,
        fields: vec![
            field("x", TypeRef::Primitive(PrimitiveType::F64), false),
            field("y", TypeRef::Primitive(PrimitiveType::F64), false),
        ],
        ..Default::default()
    };

    assert!(!struct_needs_from_json_stub(&typ, &ahash::AHashSet::new()));
}

/// A struct with an explicit hand-written static `new` constructor keeps its own constructor
/// and must not additionally get a generated `from_json` stub.
#[test]
fn does_not_need_from_json_stub_when_explicit_static_new_exists() {
    let typ = TypeDef {
        name: "Custom".to_string(),
        has_serde: true,
        fields: vec![field("inner", TypeRef::Named("Nested".to_string()), false)],
        methods: vec![MethodDef {
            name: "new".to_string(),
            is_static: true,
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(!struct_needs_from_json_stub(&typ, &ahash::AHashSet::new()));
}

/// A `#[derive(Default)]` struct needs `from_json` even if every field is scalar, because the
/// extension's gate treats `has_default` as sufficient on its own (matching `structs.rs`).
#[test]
fn needs_from_json_stub_when_struct_has_default_impl() {
    let typ = TypeDef {
        name: "Config".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![field("timeout", TypeRef::Primitive(PrimitiveType::U32), true)],
        ..Default::default()
    };

    assert!(struct_needs_from_json_stub(&typ, &ahash::AHashSet::new()));
}

/// Regression for the `BudgetConfig` bug: the stub's constructor param list must sort
/// required fields before optional ones (stable, preserving relative order within each group)
/// regardless of raw field-declaration order — mirroring `BudgetConfig`'s core struct, where the
/// optional `global_limit: Option<f64>` is declared first, ahead of two required fields.
#[test]
fn required_fields_sort_before_optional_regardless_of_declaration_order() {
    let typ = TypeDef {
        name: "BudgetConfig".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![
            field("global_limit", TypeRef::Primitive(PrimitiveType::F64), true),
            field("model_limits", TypeRef::String, false),
            field("enforcement", TypeRef::String, false),
        ],
        ..Default::default()
    };

    let params = gen_struct_constructor_stub_params(&typ, &AHashSet::new(), &AHashSet::new());
    let joined = params.join("\n");

    let model_limits_idx = joined.find("$modelLimits").expect("modelLimits param present");
    let enforcement_idx = joined.find("$enforcement").expect("enforcement param present");
    let global_limit_idx = joined.find("$globalLimit").expect("globalLimit param present");

    assert!(
        model_limits_idx < global_limit_idx && enforcement_idx < global_limit_idx,
        "required fields must precede the optional field despite declaration order: {joined}"
    );
    assert!(joined.contains("float $globalLimit = null"), "{joined}");
}

/// Regression for the `RateLimitConfig` bug: a `Duration` field on a type with a `Default` impl
/// is widened to an optional, nullable `int` param (the FFI boundary carries it as milliseconds),
/// even though the field itself is required in the IR. Previously the stub used the raw
/// `f.optional` (false) for both the type/nullability AND the sort key, so `window` rendered as
/// `public readonly float $window` (required, wrong type) and sorted ahead of the genuinely
/// optional fields — disagreeing with the runtime constructor on type, nullability, AND position.
#[test]
fn duration_field_widened_by_default_impl_is_optional_int_and_sorts_last() {
    let typ = TypeDef {
        name: "RateLimitConfig".to_string(),
        has_serde: true,
        has_default: true,
        fields: vec![
            field("rpm", TypeRef::Primitive(PrimitiveType::U32), true),
            field("tpm", TypeRef::Primitive(PrimitiveType::U32), true),
            field("window", TypeRef::Duration, false),
        ],
        ..Default::default()
    };

    let params = gen_struct_constructor_stub_params(&typ, &AHashSet::new(), &AHashSet::new());
    let joined = params.join("\n");

    assert!(
        joined.contains("?int $window = null"),
        "Duration field on a Default-impl type must be a nullable int, not float: {joined}"
    );
    let rpm_idx = joined.find("$rpm").expect("rpm param present");
    let window_idx = joined.find("$window").expect("window param present");
    assert!(rpm_idx < window_idx, "{joined}");
}

fn param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

/// Regression: `gen_static_method` (`functions/methods.rs`) falls back to `String::new()` — no
/// `#[php_impl]` method at all — for a static method whose params `has_unsupported_static_params`
/// flags. The PHPStan stub calls this exact function to decide whether to declare the method
/// (`type_stubs.rs`'s `non_excluded_methods` filter), so it must agree with the binding on every
/// param shape the binding can't cross, not just restate a copy of the same logic that can drift.
#[test]
fn map_param_is_unsupported_for_static_delegation() {
    let params = vec![param(
        "index",
        TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
    )];
    assert!(
        has_unsupported_static_params(&params, &AHashSet::new(), &AHashSet::new()),
        "a Map param is never delegatable — gen_static_method unconditionally bails on it"
    );
}

#[test]
fn non_opaque_non_enum_named_param_is_unsupported_for_static_delegation() {
    let params = vec![param("options", TypeRef::Named("ConversionOptions".to_string()))];
    assert!(
        has_unsupported_static_params(&params, &AHashSet::new(), &AHashSet::new()),
        "a Named param that is neither an opaque type nor a string enum can't cross the FFI \
         boundary gen_static_method builds"
    );
}

#[test]
fn opaque_or_string_enum_named_params_are_supported_for_static_delegation() {
    let opaque_types: AHashSet<String> = ["Client".to_string()].into_iter().collect();
    let string_enum_names: AHashSet<String> = ["Mode".to_string()].into_iter().collect();
    let params = vec![
        param("client", TypeRef::Named("Client".to_string())),
        param("mode", TypeRef::Named("Mode".to_string())),
    ];
    assert!(
        !has_unsupported_static_params(&params, &opaque_types, &string_enum_names),
        "opaque and string-enum Named params are exactly what gen_static_method can delegate"
    );
}
