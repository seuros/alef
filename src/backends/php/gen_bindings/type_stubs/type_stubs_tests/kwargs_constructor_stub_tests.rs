//! Stub emission for the kwargs constructor shape.
//!
//! Split out of `type_stubs_tests` to keep that file at or below its recorded ceiling in
//! `tests/file_size_baseline.txt`; it is a remediation target that may shrink but never grow.

use super::*;

/// A `binding_excluded` field is absent from `constructor_fields`, so it must be absent from the
/// stub too -- the one filter the two algorithms genuinely share.
#[test]
fn kwargs_constructor_stub_omits_binding_excluded_fields() {
    let typ = TypeDef {
        name: "RetryConfig".to_string(),
        has_default: true,
        fields: vec![
            FieldDef {
                binding_excluded: true,
                ..field("internal", TypeRef::String, false)
            },
            field("jitter", TypeRef::Primitive(PrimitiveType::Bool), true),
        ],
        ..Default::default()
    };

    assert_eq!(
        gen_kwargs_constructor_stub_params(&typ, &AHashSet::new()),
        vec!["        ?bool $jitter = null".to_string()]
    );
}

/// The `#[php(prop)]` properties still exist on a `Kwargs` type, but constructor-property
/// promotion cannot express them: the parameter is `?T` under the snake_case field ident while the
/// property is non-nullable `T` under the `to_php_name` camelCase name. They are declared
/// separately, writable (the ext-php-rs derive registers a setter for every `#[php(prop)]` field).
#[test]
fn kwargs_property_declarations_use_php_names_and_field_nullability() {
    let declarations = gen_kwargs_property_declarations(&kwargs_config_type(), &AHashSet::new(), false);
    let joined = declarations.join("");

    assert!(joined.contains("public ?bool $jitter;"), "{joined}");
    assert!(joined.contains("public int $maxRetries;"), "{joined}");
    assert!(joined.contains("public string $metricsLabel;"), "{joined}");
    assert!(!joined.contains("readonly"), "{joined}");
}

/// Positive control for the common path: the `Positional` shape's derivation is untouched by the
/// `Kwargs` work -- it still filters `cfg`-gated fields out, sorts required before optional, and
/// renames parameters with `to_php_name`, exactly as the runtime `#[php(constructor)] pub fn
/// new(...)` does. A regression in this shared path cannot hide behind the `Kwargs` tests.
#[test]
fn positional_constructor_stub_is_unaffected_by_the_kwargs_derivation() {
    let typ = TypeDef {
        name: "RequestOptions".to_string(),
        has_serde: true,
        fields: vec![
            field("jitter", TypeRef::Primitive(PrimitiveType::Bool), true),
            field("max_retries", TypeRef::Primitive(PrimitiveType::U32), false),
            FieldDef {
                cfg: Some("feature = \"metrics\"".to_string()),
                ..field("metrics_label", TypeRef::String, false)
            },
        ],
        ..Default::default()
    };

    assert_eq!(
        stub_constructor_shape(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), true),
        StubConstructorShape::Positional
    );

    let joined =
        gen_struct_constructor_stub_params(&typ, &AHashSet::new(), &AHashSet::new(), &AHashSet::new(), true).join("\n");

    assert!(
        joined.contains("public readonly int $maxRetries"),
        "required field keeps its non-nullable, camelCase, promoted form: {joined}"
    );
    assert!(joined.contains("public readonly ?bool $jitter = null"), "{joined}");
    assert!(
        joined.find("$maxRetries") < joined.find("$jitter"),
        "required must still sort before optional: {joined}"
    );
    assert!(
        !joined.contains("metricsLabel"),
        "cfg-gated fields are still filtered out of the positional shape: {joined}"
    );
}
