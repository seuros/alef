//! Regression coverage for `enum_arms.rs`'s sanitized-field JSON parse: a data-carrying binding
//! enum (rustler/magnus, `binding_enums_have_data: true`) must warn and fall back to
//! `Default::default()` on unparseable JSON instead of silently swallowing the error. Split out
//! of `tests.rs` (already over the `file-modularization` line cap) rather than grown in place.

use super::*;
use crate::core::ir::*;

/// A sanitized struct-variant field (data-carrying binding enum, e.g. rustler/magnus) must warn
/// and fall back to `Default::default()` instead of silently swallowing an unparseable JSON
/// payload with no diagnostic. Round-trip must still work unchanged for a value alef itself
/// produced.
#[test]
fn sanitized_struct_variant_field_binding_to_core_warns_and_defaults_on_bad_json() {
    let mut enum_def = tests::simple_enum();
    enum_def.variants = vec![EnumVariant {
        name: "Remote".into(),
        fields: vec![FieldDef {
            name: "settings".into(),
            ty: TypeRef::Named("RemoteSettings".into()),
            sanitized: true,
            ..FieldDef::default()
        }],
        ..EnumVariant::default()
    }];

    let result = gen_enum_from_binding_to_core_cfg(
        &enum_def,
        "my_crate",
        &ConversionConfig {
            binding_enums_have_data: true,
            ..ConversionConfig::default()
        },
    );

    assert!(
        result.contains("settings: match serde_json::from_str(&settings) {"),
        "{result}"
    );
    assert!(result.contains("Ok(v) => v,"), "{result}");
    assert!(
        result.contains(
            "tracing::warn!(variant = \"Remote\", field = \"settings\", value = %settings, error = %e, \
             \"binding provided unparseable JSON for enum variant field; substituting default\");"
        ),
        "{result}"
    );
    assert!(result.contains("Default::default()"), "{result}");
    assert!(!result.contains("unwrap_or_default()"), "{result}");
}

/// Tuple-variant counterpart of the struct-variant case above: a sanitized field on a
/// tuple-shaped data variant must warn and default rather than silently swallowing bad JSON.
/// The field name must match `is_tuple_variant`'s `_<digit>` convention (`eligibility.rs`) —
/// a plain name like `settings` falls through to the struct branch and the tuple code path
/// this test targets never runs. ~keep
#[test]
fn sanitized_tuple_variant_field_binding_to_core_warns_and_defaults_on_bad_json() {
    let mut enum_def = tests::simple_enum();
    enum_def.variants = vec![EnumVariant {
        name: "Remote".into(),
        is_tuple: true,
        fields: vec![FieldDef {
            name: "_0".into(),
            ty: TypeRef::Named("RemoteSettings".into()),
            sanitized: true,
            ..FieldDef::default()
        }],
        ..EnumVariant::default()
    }];

    let result = gen_enum_from_binding_to_core_cfg(
        &enum_def,
        "my_crate",
        &ConversionConfig {
            binding_enums_have_data: true,
            binding_tuple_form_for_variants: true,
            ..ConversionConfig::default()
        },
    );

    assert!(result.contains("match serde_json::from_str(&_0) {"), "{result}");
    assert!(result.contains("Ok(v) => v,"), "{result}");
    assert!(
        result.contains(
            "tracing::warn!(variant = \"Remote\", field = \"_0\", value = %_0, error = %e, \
             \"binding provided unparseable JSON for enum variant field; substituting default\");"
        ),
        "{result}"
    );
    assert!(result.contains("Default::default()"), "{result}");
    assert!(!result.contains("unwrap_or_default()"), "{result}");
}

/// A non-sanitized tuple-variant field whose type is explicitly configured as `exclude_types`
/// (a separate JSON-round-trip path from `f.sanitized`, gated on `config.exclude_types`) must
/// get the same warn-then-default treatment on unparseable JSON.
#[test]
fn excluded_type_tuple_variant_field_binding_to_core_warns_and_defaults_on_bad_json() {
    let mut enum_def = tests::simple_enum();
    enum_def.variants = vec![EnumVariant {
        name: "Remote".into(),
        is_tuple: true,
        fields: vec![FieldDef {
            name: "_0".into(),
            ty: TypeRef::Named("RemoteSettings".into()),
            ..FieldDef::default()
        }],
        ..EnumVariant::default()
    }];

    let result = gen_enum_from_binding_to_core_cfg(
        &enum_def,
        "my_crate",
        &ConversionConfig {
            binding_enums_have_data: true,
            binding_tuple_form_for_variants: true,
            exclude_types: &["RemoteSettings".to_string()],
            ..ConversionConfig::default()
        },
    );

    assert!(result.contains("match serde_json::from_str(&_0) {"), "{result}");
    assert!(
        result.contains(
            "tracing::warn!(variant = \"Remote\", field = \"_0\", value = %_0, error = %e, \
             \"binding provided unparseable JSON for enum variant field; substituting default\");"
        ),
        "{result}"
    );
    assert!(result.contains("Default::default()"), "{result}");
    assert!(!result.contains("unwrap_or_default()"), "{result}");
}
