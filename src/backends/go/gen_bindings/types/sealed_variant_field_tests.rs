//! Struct-shaped enum variant field-naming tests: the spellings `gen_data_enum_type` and its
//! sibling struct-enum generators declare for a variant's own payload fields, and the
//! `rename_all_fields` container rule those spellings must honor.
//!
//! Split out of `types/tests.rs` when `rename_all_fields` landed, both to keep that file under
//! its file-size ceiling and because this concern -- struct-variant field wire naming -- is
//! exactly what `go_data_enum_variant_field` governs. ~keep

use super::gen_enum_type;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};

fn simple_field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

fn variant(name: &str, fields: Vec<FieldDef>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
        ..EnumVariant::default()
    }
}

fn enum_def(name: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: name.to_string(),
        rust_path: format!("samplelib::{name}"),
        variants,
        ..EnumDef::default()
    }
}

/// The spellings `e2e::codegen::go::enum_literals` builds its expressions out of must be the
/// ones the binding actually declares.
///
/// That module can no longer fall back on a conversion for a struct- or interface-shaped enum;
/// it writes the variant's field name, the field's JSON key, the discriminator field, the
/// adjacent-tagged constructor and the concrete variant struct by name. Any one of those it got
/// wrong would name nothing in the emitted package, and the published snippet would fail to
/// compile for a new reason instead of the old one. So assert every accessor against the real
/// emitted text rather than against a second copy of the naming rule. ~keep
#[test]
fn go_enum_variant_spellings_match_the_emitted_declarations() {
    let internally_tagged = EnumDef {
        serde_tag: Some("kind".to_string()),
        ..enum_def(
            "SampleChoice",
            vec![variant(
                "Explicit",
                vec![simple_field("_0", TypeRef::Named("SampleTarget".to_string()))],
            )],
        )
    };
    let emitted = gen_enum_type(&internally_tagged, &[]);
    let (tag_field, tag_json) = super::enums::go_struct_enum_tag_field(&internally_tagged).expect("a tag field");
    assert!(
        emitted.contains(&format!("{tag_field} string `json:\"{tag_json}\"`")),
        "the discriminator spelling must be the emitted one:\n{emitted}"
    );
    let variant_fields = super::enums::go_struct_enum_variant_fields(&internally_tagged);
    let explicit = variant_fields.first().expect("one variant field");
    assert!(
        emitted.contains(&format!(
            "{} *SampleTarget `json:\"{},omitempty\"`",
            explicit.field_name, explicit.json_key
        )),
        "the variant field spelling must be the emitted one:\n{emitted}"
    );

    let externally_tagged = enum_def(
        "SampleExternal",
        vec![variant(
            "Explicit",
            vec![simple_field("_0", TypeRef::Named("SampleTarget".to_string()))],
        )],
    );
    let emitted = gen_enum_type(&externally_tagged, &[]);
    assert!(
        super::enums::go_struct_enum_tag_field(&externally_tagged).is_none(),
        "an externally tagged union declares no discriminator field"
    );
    let variant_fields = super::enums::go_struct_enum_variant_fields(&externally_tagged);
    let explicit = variant_fields.first().expect("one variant field");
    assert!(
        emitted.contains(&format!(
            "{} *SampleTarget `json:\"{},omitempty\"`",
            explicit.field_name, explicit.json_key
        )),
        "an externally tagged union keys its field by the variant's wire name:\n{emitted}"
    );

    let sealed = EnumDef {
        serde_tag: Some("type".to_string()),
        ..enum_def(
            "SampleDocument",
            vec![variant("Url", vec![simple_field("url", TypeRef::String)])],
        )
    };
    let emitted = gen_enum_type(&sealed, &[]);
    let url_variant = &sealed.variants[0];
    let struct_name = super::enums::go_data_enum_variant_struct(&sealed, url_variant);
    assert!(
        emitted.contains(&format!("type {struct_name} struct {{")),
        "the concrete variant struct spelling must be the emitted one:\n{emitted}"
    );
    let (field_name, json_key) =
        super::field_shape::go_data_enum_variant_field(&sealed, &url_variant.fields[0]).expect("a named field");
    assert!(
        emitted.contains(&format!("{field_name} string `json:\"{json_key}\"`")),
        "the variant struct's field spelling must be the emitted one:\n{emitted}"
    );
    assert!(
        super::field_shape::go_data_enum_variant_field(&sealed, &simple_field("_0", TypeRef::String)).is_none(),
        "a positional field has no declared Go field on the variant struct"
    );

    let adjacent = EnumDef {
        serde_tag: Some("kind".to_string()),
        serde_content: Some("payload".to_string()),
        ..enum_def(
            "SampleAdjacent",
            vec![variant(
                "Text",
                vec![simple_field("_0", TypeRef::Named("SampleTarget".to_string()))],
            )],
        )
    };
    let emitted = gen_enum_type(&adjacent, &[]);
    let constructor = super::enums::go_adjacent_tagged_constructor(&adjacent, &adjacent.variants[0]);
    assert!(
        emitted.contains(&format!("func {constructor}(")),
        "the adjacent-tagged constructor spelling must be the emitted one:\n{emitted}"
    );
}

/// `go_data_enum_variant_field` resolves a struct-shaped sealed-interface variant field's JSON
/// key through `wire_field_name`: the field's own `#[serde(rename = "...")]` beats the enum's
/// `rename_all_fields`, which beats the raw field name. `rename_all` (the enum's variant-name
/// casing) must play no part -- it is a different serde namespace. ~keep
#[test]
fn go_data_enum_variant_field_field_rename_beats_rename_all_fields() {
    let sealed = EnumDef {
        serde_tag: Some("type".to_string()),
        rename_all_fields: Some("SCREAMING_SNAKE_CASE".to_string()),
        ..enum_def(
            "SampleDocument",
            vec![variant(
                "Url",
                vec![FieldDef {
                    name: "inner_path".to_string(),
                    ty: TypeRef::String,
                    serde_rename: Some("path".to_string()),
                    ..FieldDef::default()
                }],
            )],
        )
    };
    let url_variant = &sealed.variants[0];
    let (_, json_key) =
        super::field_shape::go_data_enum_variant_field(&sealed, &url_variant.fields[0]).expect("a named field");
    assert_eq!(
        json_key, "path",
        "the field's own serde_rename must win over the enum's rename_all_fields"
    );
}

/// With no field-level `serde_rename`, the enum's `rename_all_fields` cases the field's own
/// name -- not `rename_all`, which is left unset here and must have nothing to say about it.
#[test]
fn go_data_enum_variant_field_applies_rename_all_fields_without_field_rename() {
    let sealed = EnumDef {
        serde_tag: Some("type".to_string()),
        rename_all_fields: Some("camelCase".to_string()),
        ..enum_def(
            "SampleDocument",
            vec![variant("Url", vec![simple_field("inner_path", TypeRef::String)])],
        )
    };
    let url_variant = &sealed.variants[0];
    let (_, json_key) =
        super::field_shape::go_data_enum_variant_field(&sealed, &url_variant.fields[0]).expect("a named field");
    assert_eq!(json_key, "innerPath");
}

/// `rename_all` (variant-name casing) and `rename_all_fields` (struct-variant field-name
/// casing) are independent: setting the enum's `rename_all` alone must not leak into the
/// field's wire key that `go_data_enum_variant_field` resolves. ~keep
#[test]
fn go_data_enum_variant_field_ignores_rename_all_variant_name_rule() {
    let sealed = EnumDef {
        serde_tag: Some("type".to_string()),
        serde_rename_all: Some("SCREAMING_SNAKE_CASE".to_string()),
        ..enum_def(
            "SampleDocument",
            vec![variant("Url", vec![simple_field("inner_path", TypeRef::String)])],
        )
    };
    let url_variant = &sealed.variants[0];
    let (_, json_key) =
        super::field_shape::go_data_enum_variant_field(&sealed, &url_variant.fields[0]).expect("a named field");
    assert_eq!(
        json_key, "inner_path",
        "the enum's rename_all (variant-name rule) must not case the field name"
    );
}
