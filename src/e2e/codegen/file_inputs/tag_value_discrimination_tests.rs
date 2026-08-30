//! Proves `variant_payload` discriminates by the tag's actual VALUE, not merely by tag
//! presence.
//!
//! Before this fix, the internally-tagged branch returned the whole object for EVERY variant
//! unconditionally, and the adjacently-tagged branch extracted the content key the same way --
//! neither compared the tag key's value against the candidate variant's wire name.
//! `enum_value_uses_test_documents` then `.any()`s over every variant against that same
//! candidate, so a multi-variant tagged enum where two variants share a field NAME but differ
//! in type could report a file input via a variant the tag never actually selected. The
//! `..._ignores_the_non_selected_variant` tests below previously asserted `true` for exactly
//! this over-inclusion, deliberately pinned as accepted, fail-safe behaviour. They now assert
//! `false`: this commit closes the gap by comparing the tag key's value against
//! `wire_variant_value(variant, ...)` -- the SAME central helper `variant_payload` already used
//! for externally tagged enums -- before a variant's fields are ever walked.
//!
//! Tightening this comparison risks the opposite, WORSE failure: a wrong wire-name comparison
//! could newly miss a real file input (false negative) instead of harmlessly over-including one
//! (false positive). Every other test here exists specifically to rule that out. The
//! `..._still_detects_..._real_file_input` tests are the load-bearing ones: they prove a
//! genuine file input, under the variant the tag actually names, is still found -- including
//! when that variant is reached only through its own `serde_rename`, exactly where a
//! wire-name miscomputation would bite. The remaining cases cover `serde_rename`,
//! `serde_rename_all`, their precedence, and a genuine non-match, all through the one
//! comparison. ~keep

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

fn bytes_field(name: &str) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty: TypeRef::Bytes,
        ..Default::default()
    }
}

fn found_file_input(input: serde_json::Value, event_enum: EnumDef) -> bool {
    let fixture = Fixture {
        input,
        ..Default::default()
    };
    let call = CallConfig {
        args: vec![object_arg()],
        ..Default::default()
    };

    super::fixture_uses_test_documents(&fixture, &call, &[request_with_event_type()], &[event_enum])
}

/// Two variants that both declare a field named `value`, typed differently: `TextNote.value`
/// is a `String`, `FileNote.value` is `Bytes`. Only one variant is ever really present -- the
/// tag says which -- but before this fix neither tagging branch consulted it. `file_variant_rename`
/// optionally gives `FileNote` its own `serde_rename`, so the same shape also covers a renamed
/// variant being correctly selected among siblings. ~keep
fn colliding_field_name_enum(tag: Option<&str>, content: Option<&str>, file_variant_rename: Option<&str>) -> EnumDef {
    EnumDef {
        name: "SampleEvent".into(),
        serde_tag: tag.map(str::to_string),
        serde_content: content.map(str::to_string),
        variants: vec![
            EnumVariant {
                name: "TextNote".into(),
                fields: vec![FieldDef {
                    name: "value".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "FileNote".into(),
                serde_rename: file_variant_rename.map(str::to_string),
                fields: vec![FieldDef {
                    name: "value".into(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn internally_tagged_discrimination_ignores_the_non_selected_variant() {
    // The tag names `TextNote` (a `String` field). Before this fix this test asserted `true`,
    // documenting that `FileNote.value: Bytes` was checked against this same object regardless
    // of the tag and produced a false match. It is now correctly `false`. ~keep
    let input = serde_json::json!({"event": {"type": "TextNote", "value": "documents/sample.bin"}});
    assert!(!found_file_input(
        input,
        colliding_field_name_enum(Some("type"), None, None)
    ));
}

#[test]
fn adjacently_tagged_discrimination_ignores_the_non_selected_variant() {
    // Same gap, adjacently tagged: `value.get(content_key)` ignored the tag value too, so
    // `FileNote.value: Bytes` was checked against the `TextNote` instance's own content object.
    // Previously asserted `true`; now correctly `false`. ~keep
    let input = serde_json::json!({
        "event": {"type": "TextNote", "payload": {"value": "documents/sample.bin"}}
    });
    assert!(!found_file_input(
        input,
        colliding_field_name_enum(Some("type"), Some("payload"), None)
    ));
}

#[test]
fn internally_tagged_discrimination_still_detects_the_selected_variants_real_file_input() {
    // The false-negative guard: the tag now genuinely selects `FileNote`, whose `value: Bytes`
    // field IS a real file path, alongside a sibling `TextNote` that must be correctly skipped.
    // A broken tag comparison could over-tighten and silently miss this. ~keep
    let input = serde_json::json!({"event": {"type": "FileNote", "value": "documents/sample.bin"}});
    assert!(found_file_input(
        input,
        colliding_field_name_enum(Some("type"), None, None)
    ));
}

#[test]
fn adjacently_tagged_discrimination_still_detects_the_selected_variants_real_file_input() {
    // Same false-negative guard, adjacently tagged. ~keep
    let input = serde_json::json!({
        "event": {"type": "FileNote", "payload": {"value": "documents/sample.bin"}}
    });
    assert!(found_file_input(
        input,
        colliding_field_name_enum(Some("type"), Some("payload"), None)
    ));
}

#[test]
fn internally_tagged_discrimination_still_detects_a_renamed_variants_real_file_input() {
    // The strongest false-negative guard: `FileNote` is reached only through its own
    // `serde_rename` ("file_note"), alongside a sibling `TextNote` that must still be skipped.
    // This is exactly where a wire-name miscomputation in the tag comparison would bite --
    // and is exactly where this test would catch it. ~keep
    let input = serde_json::json!({"event": {"type": "file_note", "value": "documents/sample.bin"}});
    let event_enum = colliding_field_name_enum(Some("type"), None, Some("file_note"));
    assert!(found_file_input(input, event_enum));
}

fn single_variant_enum(rename: Option<&str>, rename_all: Option<&str>) -> EnumDef {
    EnumDef {
        name: "SampleEvent".into(),
        serde_tag: Some("type".into()),
        serde_rename_all: rename_all.map(str::to_string),
        variants: vec![EnumVariant {
            name: "Uploaded".into(),
            serde_rename: rename.map(str::to_string),
            fields: vec![bytes_field("file")],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn tag_matches_variant_via_explicit_serde_rename() {
    // The variant's real wire tag is `file_uploaded`, not its Rust name `Uploaded`. The
    // comparison must use `wire_variant_value`, the same helper the (unchanged) externally
    // tagged branch already uses -- not the raw variant name. ~keep
    let input = serde_json::json!({"event": {"type": "file_uploaded", "file": "documents/sample.bin"}});
    assert!(found_file_input(
        input,
        single_variant_enum(Some("file_uploaded"), None)
    ));
}

#[test]
fn tag_matches_variant_via_enum_rename_all() {
    // No explicit rename; the enum's `SCREAMING_SNAKE_CASE` cases the tag to `UPLOADED`. ~keep
    let input = serde_json::json!({"event": {"type": "UPLOADED", "file": "documents/sample.bin"}});
    assert!(found_file_input(
        input,
        single_variant_enum(None, Some("SCREAMING_SNAKE_CASE"))
    ));
}

#[test]
fn serde_rename_takes_precedence_over_rename_all_for_tag_matching() {
    // Both an explicit rename AND an enum-level rename_all are set. `wire_variant_value` gives
    // the explicit rename priority (see naming.rs::serde_wire_name), so the real wire tag stays
    // `file_uploaded`, not `UPLOADED`. ~keep
    let input = serde_json::json!({"event": {"type": "file_uploaded", "file": "documents/sample.bin"}});
    let event_enum = single_variant_enum(Some("file_uploaded"), Some("SCREAMING_SNAKE_CASE"));
    assert!(found_file_input(input, event_enum));
}

#[test]
fn rename_all_cased_tag_does_not_match_when_variant_has_its_own_rename() {
    // Same enum as the precedence test above, but the tag uses the rename_all-only form
    // (`UPLOADED`) that would apply if `serde_rename` were absent. It must NOT match -- proving
    // precedence holds in both directions, not just that either form happens to work. ~keep
    let input = serde_json::json!({"event": {"type": "UPLOADED", "file": "documents/sample.bin"}});
    let event_enum = single_variant_enum(Some("file_uploaded"), Some("SCREAMING_SNAKE_CASE"));
    assert!(!found_file_input(input, event_enum));
}

#[test]
fn unmatched_tag_value_finds_no_file_input() {
    // The tag names a variant that does not exist at all. A real file-looking value sits right
    // next to it, but with no variant selected there is nothing to walk. Proves discrimination
    // actually discriminates rather than degenerating back to "any object matches". ~keep
    let input = serde_json::json!({"event": {"type": "Deleted", "file": "documents/sample.bin"}});
    assert!(!found_file_input(input, single_variant_enum(None, None)));
}
