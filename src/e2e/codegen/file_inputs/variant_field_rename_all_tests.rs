//! Pins the fix for a serde-namespace conflation in `variant_uses_test_documents`: the enum's
//! own `serde_rename_all` (which cases VARIANT names) used to also be passed as the casing rule
//! for a struct-shaped variant's PAYLOAD FIELD names -- two different serde namespaces, and
//! `EnumVariant` carries no per-variant field-casing rule in the IR to use instead. Detection
//! must now rely only on a field's raw name or its own explicit `serde_rename`, never on the
//! enum's variant-name casing rule. If a future edit reaches back for
//! `definition.serde_rename_all` here, `enum_rename_all_is_not_applied_to_field_names` below
//! starts finding a field it must not, and fails. ~keep

use crate::core::config::e2e::{ArgMapping, CallConfig};
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::fixture::Fixture;

fn object_arg() -> ArgMapping {
    ArgMapping {
        name: "request".into(),
        field: "input".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: Some("SampleRequest".into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn request_with_event_type() -> TypeDef {
    TypeDef {
        name: "SampleRequest".into(),
        fields: vec![FieldDef {
            name: "event".into(),
            ty: TypeRef::Named("SampleEvent".into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// One struct variant with a single field, under an enum that sets a variant-name casing rule
/// unrelated to the field's own casing. ~keep
fn event_enum_with_field(field: FieldDef) -> EnumDef {
    EnumDef {
        name: "SampleEvent".into(),
        serde_rename_all: Some("SCREAMING_SNAKE_CASE".into()),
        variants: vec![EnumVariant {
            name: "Uploaded".into(),
            fields: vec![field],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn call_with_input(input: serde_json::Value) -> (Fixture, CallConfig) {
    let fixture = Fixture {
        input,
        ..Default::default()
    };
    let call = CallConfig {
        args: vec![object_arg()],
        ..Default::default()
    };
    (fixture, call)
}

#[test]
fn enum_rename_all_is_not_applied_to_field_names() {
    // The tag key is `UPLOADED` because the enum's `SCREAMING_SNAKE_CASE` legitimately governs
    // VARIANT-name casing (untouched by this fix, see `variant_payload`). The same rule would
    // ALSO spell `file_name` as `FILE_NAME` if (wrongly) reused as the field-casing rule; the
    // payload only carries that SCREAMING_SNAKE_CASE key, never the raw `file_name` -- so if the
    // enum's rule leaked into field lookup, this would find it. It must not: correct field-name
    // resolution has no rule to derive `FILE_NAME` from `file_name` without an explicit
    // `serde_rename`, so this must report no file input. ~keep
    let field = FieldDef {
        name: "file_name".into(),
        ty: TypeRef::Bytes,
        ..Default::default()
    };
    let (fixture, call) = call_with_input(serde_json::json!({
        "event": {"UPLOADED": {"FILE_NAME": "documents/sample.bin"}}
    }));

    assert!(!super::fixture_uses_test_documents(
        &fixture,
        &call,
        &[request_with_event_type()],
        &[event_enum_with_field(field)]
    ));
}

#[test]
fn explicit_field_serde_rename_still_resolves_inside_a_struct_variant() {
    // The field's OWN explicit rename must keep working even though the enum's unrelated
    // variant-casing rule no longer feeds field lookup at all. ~keep
    let field = FieldDef {
        name: "file_name".into(),
        ty: TypeRef::Bytes,
        serde_rename: Some("attachment".into()),
        ..Default::default()
    };
    let (fixture, call) = call_with_input(serde_json::json!({
        "event": {"UPLOADED": {"attachment": "documents/sample.bin"}}
    }));

    assert!(super::fixture_uses_test_documents(
        &fixture,
        &call,
        &[request_with_event_type()],
        &[event_enum_with_field(field)]
    ));
}
