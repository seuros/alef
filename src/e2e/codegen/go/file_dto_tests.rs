//! Regression coverage for `native_go_dto_literal`'s file/JSON-typed field handling and for
//! `qualified_go_type`'s Go-acronym casing agreement with the real Go backend.

use super::setup::{GoValueContext, native_go_dto_literal};
use crate::core::ir::{FieldDef, TypeDef, TypeRef};

fn json_field_owner(optional: bool) -> [TypeDef; 1] {
    [TypeDef {
        name: "ResponseFormat".into(),
        fields: vec![FieldDef {
            name: "schema".into(),
            ty: TypeRef::Json,
            optional,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }]
}

fn render_response_format(value: serde_json::Value, types: &[TypeDef]) -> Option<String> {
    native_go_dto_literal(
        &value,
        "ResponseFormat",
        GoValueContext {
            import_alias: "sample",
            type_defs: types,
            enums: &[],
            files: &[],
        },
    )
    .expect("no refusal")
}

/// alef #234: a `TypeRef::Json` field used to fall through
/// [`go_struct_field_expression`]'s catch-all and be dropped from the emitted literal, so
/// the published snippet compiled while omitting the schema it exists to document. The Go
/// binding declares the field `json.RawMessage`, whose underlying `[]byte` accepts the raw
/// JSON text as a conversion operand. ~keep
#[test]
fn renders_json_typed_field_as_a_raw_message_conversion() {
    let types = json_field_owner(false);
    let rendered = render_response_format(
        serde_json::json!({"schema": {"type": "object", "required": ["name"]}}),
        &types,
    )
    .expect("native DTO");

    assert!(
        // serde_json serialises a Map as a BTreeMap (the `preserve_order` feature is off), so
        // the compact literal is key-sorted: `required` precedes `type` regardless of the order
        // the fixture author wrote. Pinning source order here pins a spelling alef never emits. ~keep
        rendered.contains("Schema: json.RawMessage(`{\"required\":[\"name\"],\"type\":\"object\"}`)"),
        "the schema itself must appear in the literal, not be dropped: {rendered}"
    );
    assert!(
        !rendered.contains("sample.ResponseFormat{}"),
        "the only field must not be dropped, leaving an empty literal: {rendered}"
    );
}

/// An optional `TypeRef::Json` field is emitted as `*json.RawMessage`, so its value has to
/// be address-taken through the generic `ptr` helper like every other pointer field. ~keep
#[test]
fn renders_optional_json_typed_field_through_the_pointer_helper() {
    let types = json_field_owner(true);
    let rendered =
        render_response_format(serde_json::json!({"schema": {"type": "object"}}), &types).expect("native DTO");

    assert!(
        rendered.contains("Schema: ptr(json.RawMessage(`{\"type\":\"object\"}`))"),
        "{rendered}"
    );
}

/// A JSON `null` has no `json.RawMessage` worth spelling — the field is omitted so Go's
/// `nil` zero value stands in, which marshals back to `null`. ~keep
#[test]
fn omits_a_null_json_typed_field() {
    let types = json_field_owner(false);
    let rendered = render_response_format(serde_json::json!({"schema": null}), &types).expect("native DTO");

    assert_eq!(rendered, "sample.ResponseFormat{}", "{rendered}");
}

#[test]
fn renders_nested_file_pointer_as_byte_read() {
    let types = [TypeDef {
        name: "Upload".into(),
        fields: vec![FieldDef {
            name: "content".into(),
            ty: TypeRef::Bytes,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let files = [crate::e2e::fixture::FixtureDocsFileInput {
        field: "/content".into(),
        path: "guide.pdf".into(),
    }];
    let rendered = native_go_dto_literal(
        &serde_json::json!({"content": "guide.pdf"}),
        "Upload",
        GoValueContext {
            import_alias: "sample",
            type_defs: &types,
            enums: &[],
            files: &files,
        },
    )
    .expect("no refusal")
    .expect("native DTO");
    assert!(rendered.contains("Content: mustReadFile(`guide.pdf`)"), "{rendered}");
}

/// The generic `ptr[T any](value T) *T` helper infers `T` from its argument. A bare
/// numeric literal like `300` defaults to Go's untyped-constant rule (`int`), so
/// `ptr(300)` produces `*int` — a type error against a `*uint64` field. The literal
/// must be explicitly widened (`ptr(uint64(300))`) so `ptr[T any]` infers the field's
/// actual pointer type instead of Go's default. ~keep
#[test]
fn renders_pointer_integer_field_with_explicit_width_cast() {
    let types = [TypeDef {
        name: "ChunkingConfig".into(),
        fields: vec![FieldDef {
            name: "max_characters".into(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U64),
            optional: true,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let rendered = native_go_dto_literal(
        &serde_json::json!({"max_characters": 300}),
        "ChunkingConfig",
        GoValueContext {
            import_alias: "xberg",
            type_defs: &types,
            enums: &[],
            files: &[],
        },
    )
    .expect("no refusal")
    .expect("native DTO");

    assert!(rendered.contains("MaxCharacters: ptr(uint64(300))"), "{rendered}");
    assert!(
        !rendered.contains("ptr(300)"),
        "must not emit a bare untyped literal for a non-bool pointer field: {rendered}"
    );
}

/// `bool` has exactly one Go type, so `ptr(true)` never needs a width cast — this pins
/// that the cast is scoped to non-bool primitives only. ~keep
#[test]
fn renders_pointer_bool_field_without_a_cast() {
    let types = [TypeDef {
        name: "SampleConfig".into(),
        fields: vec![FieldDef {
            name: "retry".into(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
            optional: true,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let rendered = native_go_dto_literal(
        &serde_json::json!({"retry": true}),
        "SampleConfig",
        GoValueContext {
            import_alias: "xberg",
            type_defs: &types,
            enums: &[],
            files: &[],
        },
    )
    .expect("no refusal")
    .expect("native DTO");

    assert!(rendered.contains("Retry: ptr(true)"), "{rendered}");
}

/// Pins the defect behind task #540: the Go binding backend
/// (`backends::go::gen_bindings::types::structs::gen_struct_type`) exports a struct
/// field named after the Rust field identifier and tags it with the field's resolved
/// `#[serde(rename)]` wire name, e.g. Rust `max_characters` (`#[serde(rename =
/// "max_chars")]`) becomes Go `MaxCharacters *uint \`json:"max_chars,omitempty"\``.
///
/// Fixture JSON is authored in that same wire format — the format `json.Unmarshal`
/// would actually accept — so the Go snippet's native DTO-literal builder must look
/// values up by the wire name and then still emit the BINDING's Go identifier
/// (`MaxCharacters`, not `MaxChars`). Before the fix, `native_go_dto_literal_at`
/// looked values up under the raw Rust field name (`max_characters`), which is absent
/// from wire-format fixture JSON, so the field was silently dropped from the emitted
/// struct literal instead of being rendered under the binding's identifier.
#[test]
fn go_snippet_uses_binding_field_identifier_not_wire_name() {
    let types = [TypeDef {
        name: "ChunkingConfig".into(),
        fields: vec![FieldDef {
            name: "max_characters".into(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::Usize),
            serde_rename: Some("max_chars".into()),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let rendered = native_go_dto_literal(
        &serde_json::json!({"max_chars": 300}),
        "ChunkingConfig",
        GoValueContext {
            import_alias: "xberg",
            type_defs: &types,
            enums: &[],
            files: &[],
        },
    )
    .expect("no refusal")
    .expect("native DTO");

    assert!(
        rendered.contains("MaxCharacters: 300"),
        "expected the binding's Go identifier `MaxCharacters` (matching \
         `to_go_name(\"max_characters\")`) keyed by the wire name `max_chars`, got: {rendered}"
    );
    assert!(
        !rendered.contains("MaxChars:"),
        "must not PascalCase the serde wire name into an unknown field identifier: {rendered}"
    );
}

/// Task #363: `qualified_go_type` used to splice the raw IR type name straight after the
/// import alias (`format!("{import_alias}.{type_name}")`), instead of routing it through
/// `crate::codegen::naming::go_type_name` the way every real emitter call site does
/// (`backends::go::gen_bindings::types::structs::gen_struct_type`,
/// `backends::go::gen_bindings::types::enums`). For a type whose Rust name contains an
/// initialism, that produced a snippet referencing a type the real binding never declares
/// (`pkg.JsonSchemaFormat` instead of `pkg.JSONSchemaFormat`), which does not compile.
///
/// This asserts the snippet generator's qualified type name is byte-identical to what the
/// Go backend's own naming helper produces for the same IR name — not merely that some name
/// is emitted, which would reproduce the original blind spot. ~keep
#[test]
fn qualified_dto_type_name_matches_the_go_backends_acronym_casing() {
    let types = [TypeDef {
        name: "SampleJsonSchema".into(),
        fields: vec![FieldDef {
            name: "strict".into(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let rendered = native_go_dto_literal(
        &serde_json::json!({"strict": true}),
        "SampleJsonSchema",
        GoValueContext {
            import_alias: "pkg",
            type_defs: &types,
            enums: &[],
            files: &[],
        },
    )
    .expect("no refusal")
    .expect("native DTO");

    let backend_name = crate::codegen::naming::go_type_name("SampleJsonSchema");
    let expected = format!("pkg.{backend_name}");
    assert!(
        rendered.starts_with(&expected),
        "snippet literal's qualified type name must be byte-identical to the Go backend's \
         own naming helper (`{expected}`), got: {rendered}"
    );
    assert!(
        !rendered.contains("pkg.SampleJsonSchema"),
        "must not re-derive casing locally and emit the raw un-acronymed IR name: {rendered}"
    );
}
