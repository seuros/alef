//! Table-driven coverage for enum tagging styles, tuple/unit variant shapes, and the
//! `#[serde(flatten)]` shapes the enum-payload and flatten fixes in `file_inputs.rs` support.
//! Each case pins already-implemented behaviour (`variant_payload`'s four tagging branches,
//! `tuple_variant_uses_test_documents`, and same-level flatten recursion) -- none of these
//! should require a change to `file_inputs.rs` to pass. ~keep

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

/// `SampleRequest.event` is always `Named("SampleEvent")`; only the enum's tagging/variant
/// shape and the fixture JSON vary between cases. ~keep
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

fn single_struct_variant_enum(tag: Option<&str>, content: Option<&str>, untagged: bool) -> EnumDef {
    EnumDef {
        name: "SampleEvent".into(),
        serde_tag: tag.map(str::to_string),
        serde_content: content.map(str::to_string),
        serde_untagged: untagged,
        variants: vec![EnumVariant {
            name: "Uploaded".into(),
            fields: vec![bytes_field("file")],
            ..Default::default()
        }],
        ..Default::default()
    }
}

struct TagCase {
    name: &'static str,
    event_enum: EnumDef,
    input: serde_json::Value,
    expected: bool,
}

fn internally_tagged_case() -> TagCase {
    TagCase {
        name: "internally tagged struct variant detects nested bytes",
        event_enum: single_struct_variant_enum(Some("type"), None, false),
        input: serde_json::json!({"event": {"type": "Uploaded", "file": "documents/sample.bin"}}),
        expected: true,
    }
}

fn adjacently_tagged_case() -> TagCase {
    TagCase {
        name: "adjacently tagged struct variant detects nested bytes",
        event_enum: single_struct_variant_enum(Some("type"), Some("payload"), false),
        input: serde_json::json!({
            "event": {"type": "Uploaded", "payload": {"file": "documents/sample.bin"}}
        }),
        expected: true,
    }
}

fn untagged_case() -> TagCase {
    TagCase {
        name: "untagged struct variant detects nested bytes",
        event_enum: single_struct_variant_enum(None, None, true),
        input: serde_json::json!({"event": {"file": "documents/sample.bin"}}),
        expected: true,
    }
}

fn multi_field_tuple_case() -> TagCase {
    let event_enum = EnumDef {
        name: "SampleEvent".into(),
        variants: vec![EnumVariant {
            name: "Pair".into(),
            is_tuple: true,
            fields: vec![
                FieldDef {
                    name: "0".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
                bytes_field("1"),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    TagCase {
        name: "multi-field tuple variant detects a bytes element",
        event_enum,
        input: serde_json::json!({"event": {"Pair": ["label", "documents/sample.bin"]}}),
        expected: true,
    }
}

fn unit_variant_case() -> TagCase {
    let event_enum = EnumDef {
        name: "SampleEvent".into(),
        variants: vec![EnumVariant {
            name: "Idle".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    TagCase {
        name: "unit variant's bare string representation is not a file input",
        event_enum,
        input: serde_json::json!({"event": "Idle"}),
        expected: false,
    }
}

fn tag_cases() -> Vec<TagCase> {
    vec![
        internally_tagged_case(),
        adjacently_tagged_case(),
        untagged_case(),
        multi_field_tuple_case(),
        unit_variant_case(),
    ]
}

#[test]
fn enum_tag_mode_and_variant_shape_table() {
    for case in tag_cases() {
        let fixture = Fixture {
            input: case.input.clone(),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        let got = super::fixture_uses_test_documents(&fixture, &call, &[request_with_event_type()], &[case.event_enum]);
        assert_eq!(got, case.expected, "{}: input = {}", case.name, case.input);
    }
}

struct FlattenCase {
    name: &'static str,
    type_defs: Vec<TypeDef>,
    enums: Vec<EnumDef>,
    input: serde_json::Value,
}

fn flattened_enum_case() -> FlattenCase {
    let request = TypeDef {
        name: "SampleRequest".into(),
        fields: vec![FieldDef {
            name: "event".into(),
            ty: TypeRef::Named("SampleEvent".into()),
            serde_flatten: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let event_enum = EnumDef {
        name: "SampleEvent".into(),
        variants: vec![EnumVariant {
            name: "Uploaded".into(),
            fields: vec![bytes_field("file")],
            ..Default::default()
        }],
        ..Default::default()
    };
    FlattenCase {
        name: "a flattened enum field detects nested bytes",
        type_defs: vec![request],
        enums: vec![event_enum],
        input: serde_json::json!({"Uploaded": {"file": "documents/sample.bin"}}),
    }
}

fn flattened_optional_named_case() -> FlattenCase {
    let request = TypeDef {
        name: "SampleRequest".into(),
        fields: vec![FieldDef {
            name: "extra".into(),
            ty: TypeRef::Optional(Box::new(TypeRef::Named("SampleLeaf".into()))),
            serde_flatten: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let leaf = TypeDef {
        name: "SampleLeaf".into(),
        fields: vec![bytes_field("content")],
        ..Default::default()
    };
    FlattenCase {
        name: "a flattened Option<Named> field detects nested bytes",
        type_defs: vec![request, leaf],
        enums: vec![],
        input: serde_json::json!({"content": "documents/sample.bin"}),
    }
}

fn two_flattened_fields_case() -> FlattenCase {
    let request = TypeDef {
        name: "SampleRequest".into(),
        fields: vec![
            FieldDef {
                name: "meta".into(),
                ty: TypeRef::Named("SampleMeta".into()),
                serde_flatten: true,
                ..Default::default()
            },
            FieldDef {
                name: "payload".into(),
                ty: TypeRef::Named("SampleLeaf".into()),
                serde_flatten: true,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let meta = TypeDef {
        name: "SampleMeta".into(),
        fields: vec![FieldDef {
            name: "label".into(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let leaf = TypeDef {
        name: "SampleLeaf".into(),
        fields: vec![bytes_field("content")],
        ..Default::default()
    };
    FlattenCase {
        name: "two flattened fields at the same level each contribute their own fields",
        type_defs: vec![request, meta, leaf],
        enums: vec![],
        input: serde_json::json!({"label": "hello", "content": "documents/sample.bin"}),
    }
}

fn flatten_cases() -> Vec<FlattenCase> {
    vec![
        flattened_enum_case(),
        flattened_optional_named_case(),
        two_flattened_fields_case(),
    ]
}

#[test]
fn flatten_shape_table_detects_nested_bytes() {
    for case in flatten_cases() {
        let fixture = Fixture {
            input: case.input.clone(),
            ..Default::default()
        };
        let call = CallConfig {
            args: vec![object_arg()],
            ..Default::default()
        };

        assert!(
            super::fixture_uses_test_documents(&fixture, &call, &case.type_defs, &case.enums),
            "{}: input = {}",
            case.name,
            case.input
        );
    }
}
