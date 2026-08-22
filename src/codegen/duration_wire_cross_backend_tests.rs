//! Cross-backend guard: a `Duration` field carrying `#[serde(with = "...")]` must not be treated
//! as serde's default derive-shape object (`{"secs":u64,"nanos":u32}`) by any backend that
//! special-cases `TypeRef::Duration`'s wire form.
//!
//! Go, C#, and Java all special-case `Duration` struct fields because serde's derive shape is a
//! map, not a scalar — but a field routed through a hand-written codec (the common `duration_ms`
//! convention writes a bare millisecond integer) never goes through that derive at all. Every
//! backend below must make the same call, sourced from the single predicate
//! `crate::codegen::naming::field_uses_duration_map_wire`, so a future backend — or a future edit
//! to one of these three — cannot silently reintroduce the divergence that made a consumer's C#
//! and Java bindings fail Rust deserialization with `invalid type: map, expected u64` while Go
//! (which already consulted `serde_with`) kept working.

use crate::backends::csharp::CsharpBackend;
use crate::backends::go::GoBackend;
use crate::backends::java::JavaBackend;
use crate::core::backend::{Backend, GeneratedFile};
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, FieldDef, TypeDef, TypeRef};

/// A single required, non-optional, no-default `Duration` field — the shape that hits each
/// backend's plain "wire-shape decision" branch without a nullability/default detour.
fn duration_field(name: &str, serde_with: Option<&str>) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty: TypeRef::Duration,
        serde_with: serde_with.map(str::to_string),
        ..FieldDef::default()
    }
}

fn api_with_one_field(type_name: &str, field: FieldDef) -> ApiSurface {
    ApiSurface {
        crate_name: "sample".to_string(),
        types: vec![TypeDef {
            name: type_name.to_string(),
            rust_path: format!("sample_core::{type_name}"),
            has_serde: true,
            fields: vec![field],
            ..TypeDef::default()
        }],
        ..ApiSurface::default()
    }
}

fn joined_content(files: &[GeneratedFile]) -> String {
    files.iter().map(|f| f.content.as_str()).collect::<Vec<_>>().join("\n")
}

/// Marker strings each backend uses only when it believes a `Duration` field's wire shape is
/// serde's derive object. Their absence means the backend emitted (or relied on) the plain
/// millisecond scalar instead.
const GO_MAP_WIRE_MARKER: &str = "DurationMillis";
const CSHARP_MAP_WIRE_MARKER: &str = "DurationMillisJsonConverter";
const JAVA_MAP_WIRE_MARKER: &str = "DurationMillisSerializer";

#[test]
fn go_field_without_serde_with_uses_the_derived_map_wire() {
    let api = api_with_one_field("MapWireConfig", duration_field("window", None));
    let files = GoBackend
        .generate_bindings(&api, &ResolvedCrateConfig::default())
        .expect("Go bindings");
    assert!(
        joined_content(&files).contains(GO_MAP_WIRE_MARKER),
        "a Duration field with no serde codec must round-trip through the derive map shape"
    );
}

#[test]
fn go_field_with_serde_with_uses_the_scalar_wire() {
    let api = api_with_one_field("ScalarWireConfig", duration_field("period", Some("crate::duration_ms")));
    let files = GoBackend
        .generate_bindings(&api, &ResolvedCrateConfig::default())
        .expect("Go bindings");
    assert!(
        !joined_content(&files).contains(GO_MAP_WIRE_MARKER),
        "a Duration field with `#[serde(with = \"...\")]` must not be wrapped in the derive map \
         shape — the wire form is a bare millisecond integer, and wrapping it produces \
         `invalid type: map, expected u64` on the Rust side"
    );
}

#[test]
fn csharp_field_without_serde_with_uses_the_derived_map_wire() {
    let api = api_with_one_field("MapWireConfig", duration_field("window", None));
    let files = CsharpBackend
        .generate_bindings(&api, &ResolvedCrateConfig::default())
        .expect("C# bindings");
    assert!(
        joined_content(&files).contains(CSHARP_MAP_WIRE_MARKER),
        "a Duration field with no serde codec must get the DurationMillisJsonConverter"
    );
}

#[test]
fn csharp_field_with_serde_with_uses_the_scalar_wire() {
    let api = api_with_one_field("ScalarWireConfig", duration_field("period", Some("crate::duration_ms")));
    let files = CsharpBackend
        .generate_bindings(&api, &ResolvedCrateConfig::default())
        .expect("C# bindings");
    assert!(
        !joined_content(&files).contains(CSHARP_MAP_WIRE_MARKER),
        "a Duration field with `#[serde(with = \"...\")]` must not get the \
         DurationMillisJsonConverter — that converter writes/reads the derive map shape, which \
         is not what a hand-written duration_ms codec speaks on the Rust side"
    );
}

#[test]
fn java_field_without_serde_with_uses_the_derived_map_wire() {
    let api = api_with_one_field("MapWireConfig", duration_field("window", None));
    let files = JavaBackend
        .generate_bindings(&api, &ResolvedCrateConfig::default())
        .expect("Java bindings");
    assert!(
        joined_content(&files).contains(JAVA_MAP_WIRE_MARKER),
        "a Duration field with no serde codec must get the DurationMillisSerializer"
    );
}

#[test]
fn java_field_with_serde_with_uses_the_scalar_wire() {
    let api = api_with_one_field("ScalarWireConfig", duration_field("period", Some("crate::duration_ms")));
    let files = JavaBackend
        .generate_bindings(&api, &ResolvedCrateConfig::default())
        .expect("Java bindings");
    assert!(
        !joined_content(&files).contains(JAVA_MAP_WIRE_MARKER),
        "a Duration field with `#[serde(with = \"...\")]` must not get the \
         DurationMillisSerializer — that serializer writes the derive map shape, which is not \
         what a hand-written duration_ms codec speaks on the Rust side"
    );
}
