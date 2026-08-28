use super::gen_struct;
use crate::backends::napi::type_map::NapiMapper;
use crate::core::ir::{FieldDef, SerdeContainerConversion, TypeDef, TypeRef};

/// gen_struct (pub(super)) is accessible from mod.rs — smoke test via trait.
/// The actual output is tested via the integration test (gen_bindings_test.rs).
#[test]
fn struct_gen_function_exists() {}

/// A field's `#[napi(js_name = ...)]` must come from casing policy alone, never from
/// `#[serde(rename = ...)]` on the core struct -- the two are separate name surfaces (the
/// public JS identifier vs. the JSON wire key), and `gen_dts` (this backend's `.d.ts`
/// generator) already computes the JS-visible name from casing policy only. Before this
/// fix, a field with an explicit `serde_rename` made the *compiled* binding expose that
/// wire name in JS while the generated `.d.ts` kept the camelCase name for the same field
/// from the same IR -- the artifact and its own tracked declaration disagreed.
#[test]
fn js_name_ignores_serde_rename_but_wire_rename_is_preserved() {
    let typ = TypeDef {
        name: "ChunkerConfig".to_string(),
        fields: vec![FieldDef {
            name: "max_characters".to_string(),
            serde_rename: Some("max_chars".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".to_string());
    let opaque_types = ahash::AHashSet::default();
    let never_skip_cfg_field_names: Vec<String> = Vec::new();

    let out = gen_struct(
        &typ,
        &mapper,
        "Js",
        true,
        &opaque_types,
        &never_skip_cfg_field_names,
        &[],
        "sample_core",
        &ahash::AHashSet::default(),
        None,
    );

    assert!(
        out.contains("js_name = \"maxCharacters\""),
        "js_name must use casing policy (maxCharacters), matching gen_dts's .d.ts output:\n{out}"
    );
    assert!(
        !out.contains("js_name = \"max_chars\""),
        "js_name must not bleed the wire (serde) rename into the public JS identifier:\n{out}"
    );
    assert!(
        out.contains("serde(rename = \"max_chars\")"),
        "the field's own wire rename must still reach #[serde(rename = ...)] independently:\n{out}"
    );
}

fn f64_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::F64),
        ..Default::default()
    }
}

fn container_conversion() -> SerdeContainerConversion {
    SerdeContainerConversion {
        from: Some("WireShape".to_string()),
        into: Some("WireShape".to_string()),
        try_from: None,
        transparent: false,
    }
}

/// Extracts the `#[derive(...)]` line so assertions on delegation don't get fooled by
/// "serde::Deserialize" also appearing inside the delegating impl body text.
fn derive_line(rendered: &str) -> &str {
    rendered
        .lines()
        .find(|line| line.trim_start().starts_with("#[derive("))
        .expect("rendered struct has a derive line")
}

#[test]
fn delegates_deserialize_for_sound_two_field_pair_in_convertible_set() {
    let typ = TypeDef {
        name: "Point".to_string(),
        rust_path: "sample_core::Point".to_string(),
        fields: vec![f64_field("x"), f64_field("y")],
        is_opaque: false,
        has_serde: true,
        serde_container_conversion: container_conversion(),
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".to_string());
    let opaque_types = ahash::AHashSet::default();
    let never_skip_cfg_field_names: Vec<String> = Vec::new();
    let convertible: ahash::AHashSet<String> = ["Point".to_string()].into_iter().collect();

    let out = gen_struct(
        &typ,
        &mapper,
        "Js",
        true,
        &opaque_types,
        &never_skip_cfg_field_names,
        &[],
        "sample_core",
        &convertible,
        None,
    );

    assert!(
        !derive_line(&out).contains("serde::Deserialize"),
        "derive line must drop Deserialize when delegating: {out}"
    );
    assert!(
        out.contains("impl<'de> serde::Deserialize<'de> for JsPoint {"),
        "expected a delegating Deserialize impl in: {out}"
    );
    assert!(
        out.contains("<sample_core::Point as serde::Deserialize>::deserialize(deserializer).map(Into::into)"),
        "delegating impl must read the core type: {out}"
    );
}

#[test]
fn keeps_derive_when_type_not_confirmed_in_convertible_set() {
    // Sound fields and a real container conversion, but the caller never proved a matching
    // `From<core::Type>` impl will exist for this run (empty convertible set) -- must NOT
    // delegate, since `.into()` would call a `From` impl that might not be emitted.
    let typ = TypeDef {
        name: "Point".to_string(),
        rust_path: "sample_core::Point".to_string(),
        fields: vec![f64_field("x"), f64_field("y")],
        is_opaque: false,
        has_serde: true,
        serde_container_conversion: container_conversion(),
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".to_string());
    let opaque_types = ahash::AHashSet::default();
    let never_skip_cfg_field_names: Vec<String> = Vec::new();

    let out = gen_struct(
        &typ,
        &mapper,
        "Js",
        true,
        &opaque_types,
        &never_skip_cfg_field_names,
        &[],
        "sample_core",
        &ahash::AHashSet::default(),
        None,
    );

    assert!(derive_line(&out).contains("serde::Deserialize"));
    assert!(!out.contains("impl<'de> serde::Deserialize<'de> for JsPoint"));
}

#[test]
fn keeps_derive_when_unsound_opaque_field() {
    let typ = TypeDef {
        name: "Wrapper".to_string(),
        rust_path: "sample_core::Wrapper".to_string(),
        fields: vec![FieldDef {
            name: "handle".to_string(),
            ty: TypeRef::Named("OpaqueHandle".to_string()),
            ..Default::default()
        }],
        is_opaque: false,
        has_serde: true,
        serde_container_conversion: container_conversion(),
        ..Default::default()
    };
    let mapper = NapiMapper::new("Js".to_string());
    let opaque_types: ahash::AHashSet<String> = ["OpaqueHandle".to_string()].into_iter().collect();
    let never_skip_cfg_field_names: Vec<String> = Vec::new();
    let convertible: ahash::AHashSet<String> = ["Wrapper".to_string()].into_iter().collect();

    let out = gen_struct(
        &typ,
        &mapper,
        "Js",
        true,
        &opaque_types,
        &never_skip_cfg_field_names,
        &[],
        "sample_core",
        &convertible,
        None,
    );

    // Falls back to the derived, field-by-field Deserialize -- the existing
    // SerdeContainerConversionUnsupported diagnostic keeps naming the real gap here.
    assert!(derive_line(&out).contains("serde::Deserialize"));
    assert!(!out.contains("impl<'de> serde::Deserialize<'de> for JsWrapper"));
}
