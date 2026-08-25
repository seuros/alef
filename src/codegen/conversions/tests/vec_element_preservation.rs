use crate::codegen::conversions::*;
use crate::core::ir::*;
use ahash::AHashSet;

// Regression coverage for task #405: a `Vec` field element that fails to (de)serialize must
// not silently vanish from the collected `Vec` (via `.filter_map(...).ok())`) — that shrinks
// the element count and shifts every later element's index, which is worse than a wrong value
// because it is undetectable from the output shape alone. Every one of these conversions runs
// inside an infallible `fn from(val) -> Self`, so `?`-propagation cannot be used here; the fix
// keeps the count aligned with the source by substituting a default (or `Value::Null`) for the
// element that failed, instead of dropping it.

#[test]
fn vec_json_binding_to_core_preserves_count_on_parse_failure_non_optional() {
    let result = field_conversion_to_core("payloads", &TypeRef::Vec(Box::new(TypeRef::Json)), false);
    assert_eq!(
        result,
        "payloads: val.payloads.into_iter().map(|s| serde_json::from_str(&s).unwrap_or_default()).collect()"
    );
}

#[test]
fn vec_json_binding_to_core_preserves_count_on_parse_failure_optional() {
    let result = field_conversion_to_core("payloads", &TypeRef::Vec(Box::new(TypeRef::Json)), true);
    assert_eq!(
        result,
        "payloads: val.payloads.map(|v| v.into_iter().map(|s| serde_json::from_str(&s).unwrap_or_default()).collect())"
    );
}

#[test]
fn map_of_vec_json_binding_to_core_preserves_count_on_parse_failure() {
    let result = field_conversion_to_core(
        "grouped",
        &TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Vec(Box::new(TypeRef::Json))),
        ),
        false,
    );
    assert_eq!(
        result,
        "grouped: val.grouped.into_iter().map(|(k, v)| (k.into(), v.into_iter().map(|s| serde_json::from_str(&s).unwrap_or_default()).collect())).collect()"
    );
}

#[test]
fn vec_untagged_enum_binding_to_core_preserves_count_on_deserialize_failure() {
    let mut untagged = AHashSet::new();
    untagged.insert("Shape".to_string());
    let config = ConversionConfig {
        untagged_data_enum_names: Some(&untagged),
        ..ConversionConfig::default()
    };

    let result = field_conversion_to_core_cfg(
        "shapes",
        &TypeRef::Vec(Box::new(TypeRef::Named("Shape".into()))),
        false,
        &config,
    );

    assert_eq!(
        result,
        "shapes: val.shapes.into_iter().map(|x| serde_json::from_value(x).unwrap_or_default()).collect()"
    );
}

#[test]
fn optional_vec_untagged_enum_binding_to_core_preserves_count_on_deserialize_failure() {
    let mut untagged = AHashSet::new();
    untagged.insert("Shape".to_string());
    let config = ConversionConfig {
        untagged_data_enum_names: Some(&untagged),
        ..ConversionConfig::default()
    };

    let result = field_conversion_to_core_cfg(
        "shapes",
        &TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Shape".into()))))),
        false,
        &config,
    );

    assert_eq!(
        result,
        "shapes: val.shapes.map(|v| v.into_iter().map(|x| serde_json::from_value(x).unwrap_or_default()).collect())"
    );
}

#[test]
fn vec_untagged_enum_core_to_binding_preserves_count_on_serialize_failure() {
    let mut untagged = AHashSet::new();
    untagged.insert("Shape".to_string());
    let config = ConversionConfig {
        untagged_data_enum_names: Some(&untagged),
        ..ConversionConfig::default()
    };
    let opaque_types = AHashSet::new();

    let result = field_conversion_from_core_cfg(
        "shapes",
        &TypeRef::Vec(Box::new(TypeRef::Named("Shape".into()))),
        false,
        false,
        &opaque_types,
        &config,
    );

    assert_eq!(
        result,
        "shapes: val.shapes.iter().map(|x| serde_json::to_value(x).unwrap_or(serde_json::Value::Null)).collect()"
    );
}

#[test]
fn optional_vec_untagged_enum_core_to_binding_preserves_count_on_serialize_failure() {
    let mut untagged = AHashSet::new();
    untagged.insert("Shape".to_string());
    let config = ConversionConfig {
        untagged_data_enum_names: Some(&untagged),
        ..ConversionConfig::default()
    };
    let opaque_types = AHashSet::new();

    let result = field_conversion_from_core_cfg(
        "shapes",
        &TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Shape".into()))))),
        false,
        false,
        &opaque_types,
        &config,
    );

    assert_eq!(
        result,
        "shapes: val.shapes.as_ref().map(|v| v.iter().map(|x| serde_json::to_value(x).unwrap_or(serde_json::Value::Null)).collect())"
    );
}
