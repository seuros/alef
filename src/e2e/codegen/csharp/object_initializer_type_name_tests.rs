//! Task #372: `csharp_object_initializer` used to splice its `type_name` parameter straight
//! into `new {type_name}()` / `new {type_name} { ... }`, and `resolve_csharp_field_type_from_struct`
//! returned the raw IR `TypeRef::Named` name unchanged into a `JsonSerializer.Deserialize<{field_type}>(...)`
//! splice -- both bypassing `crate::codegen::naming::csharp_type_name`, the same call every real
//! `backends::csharp::gen_bindings` emitter makes before writing a type name. For a DTO whose Rust
//! name contains a C# initialism (`GraphQL`, `UUID`) or an all-uppercase acronym segment, that
//! produced a snippet referencing a type the real binding never declares -- mirroring the Go
//! defect fixed in `qualified_go_type` (task #363).
//!
//! These fixture names are deliberately chosen so `csharp_type_name` changes their spelling --
//! a fixture whose name already normalizes to itself cannot distinguish fixed code from broken
//! code. ~keep

use super::setup::csharp_object_initializer;
use crate::codegen::naming::csharp_type_name;
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use std::collections::HashMap;

#[test]
fn top_level_initializer_type_name_matches_the_csharp_backends_naming_helper() {
    let type_defs = [TypeDef {
        name: "GraphQlConfig".into(),
        fields: vec![FieldDef {
            name: "enabled".into(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::Bool),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let rendered = csharp_object_initializer(
        serde_json::json!({"enabled": true}).as_object().expect("object"),
        "GraphQlConfig",
        &HashMap::new(),
        &HashMap::new(),
        &type_defs,
        &[],
        "",
    );

    let expected = csharp_type_name("GraphQlConfig");
    assert!(
        rendered.starts_with(&format!("new {expected} {{ ")),
        "snippet initializer's type name must be byte-identical to the C# backend's own naming \
         helper (`{expected}`), got: {rendered}"
    );
    assert!(
        !rendered.contains("GraphQlConfig"),
        "must not re-derive casing locally and emit the raw un-normalized IR name: {rendered}"
    );
}

#[test]
fn nested_deserialize_field_type_matches_the_csharp_backends_naming_helper() {
    let type_defs = [TypeDef {
        name: "UuidRequest".into(),
        fields: vec![FieldDef {
            name: "token".into(),
            ty: TypeRef::Named("UuidToken".into()),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    // `token` is a plain string fixture value (not an object), so it takes the
    // `JsonSerializer.Deserialize<{field_type}>` branch rather than recursing into a
    // nested object initializer.
    let rendered = csharp_object_initializer(
        serde_json::json!({"token": "abc-123"}).as_object().expect("object"),
        "UuidRequest",
        &HashMap::new(),
        &HashMap::new(),
        &type_defs,
        &[],
        "",
    );

    let expected = csharp_type_name("UuidToken");
    assert!(
        rendered.contains(&format!("JsonSerializer.Deserialize<{expected}>(")),
        "snippet's deserialize target type must be byte-identical to the C# backend's own \
         naming helper (`{expected}`), got: {rendered}"
    );
    assert!(
        !rendered.contains("Deserialize<UuidToken>"),
        "must not re-derive casing locally and emit the raw un-normalized IR name: {rendered}"
    );
}
