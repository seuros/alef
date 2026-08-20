//! Regression coverage for the Java e2e generator's enum-field classification.
//!
//! `java_builder_expression` builds a Java `Options.builder()...build()` expression from a
//! fixture's `json_object` arg (e.g. a `chunk` config passed to `extract_file`). It decided
//! whether a field is enum-typed purely from the hand-maintained `enum_fields` config (a flat,
//! type-unaware `HashSet<String>` of camelCase field names), so a consumer whose `alef.toml`
//! never listed a field emitted `.withStyle("fenced")` — a `String` literal passed to a builder
//! method whose parameter type is a Java enum (`CodeBlockStyle`). That does not compile: the
//! declared parameter type cannot accept a `String`.
//!
//! `java_builder_expression` now also consults the IR-derived classification
//! (`FieldResolver::ir_enum_fields`, keyed by owner type), anchored at `type_name` — the exact
//! struct the current JSON object maps to, since a builder expression has no "declared Rust
//! return type" to anchor against the way a result-field assertion does. These tests drive the
//! real entry point, `java_builder_expression`, with no `enum_fields` config at all — the
//! classification must come from the IR alone.

use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;

use super::values::java_builder_expression;

fn data_node_kind_enum() -> EnumDef {
    EnumDef {
        name: "DataNodeKind".to_string(),
        variants: vec![
            EnumVariant {
                name: "Fenced".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Indented".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

fn style_field(ty: TypeRef) -> FieldDef {
    FieldDef {
        name: "style".to_string(),
        ty,
        ..FieldDef::default()
    }
}

/// `ChunkOptions.style` is `DataNodeKind` (a real enum). `OtherOptions.style` is `String` (same
/// leaf field name, unrelated non-enum type) — proves the classification is anchored per-type
/// rather than matching on the field name alone.
fn table_ir() -> (Vec<TypeDef>, Vec<EnumDef>) {
    let type_defs = vec![
        TypeDef {
            name: "ChunkOptions".to_string(),
            fields: vec![style_field(TypeRef::Named("DataNodeKind".to_string()))],
            ..TypeDef::default()
        },
        TypeDef {
            name: "OtherOptions".to_string(),
            fields: vec![style_field(TypeRef::String)],
            ..TypeDef::default()
        },
    ];
    (type_defs, vec![data_node_kind_enum()])
}

fn style_obj() -> serde_json::Map<String, serde_json::Value> {
    let mut obj = serde_json::Map::new();
    obj.insert("style".to_string(), serde_json::Value::String("fenced".to_string()));
    obj
}

/// The `EnumType.Variant` shape `java_builder_expression` emits for an enum-classified field —
/// the generated-code fingerprint of the enum branch. The enum type name is inferred from the
/// field name (pre-existing, unrelated heuristic; unaffected by this fix), not from the actual
/// IR enum name, so it reads `Style.Fenced` regardless of which real enum backs the field.
const ENUM_MARKER: &str = ".withStyle(Style.Fenced)";
/// The raw `String` literal emitted for a non-enum field — the shape that fails to compile
/// against a builder method whose declared parameter type is a Java enum.
const RAW_MARKER: &str = ".withStyle(\"fenced\")";

struct Case {
    name: &'static str,
    type_name: &'static str,
    expect_enum_branch: bool,
}

const CASES: &[Case] = &[
    Case {
        name: "an enum-typed field with no enum_fields config is classified as enum via the IR",
        type_name: "ChunkOptions",
        expect_enum_branch: true,
    },
    Case {
        name: "a same-named non-enum field on an unrelated type is not misclassified as enum",
        type_name: "OtherOptions",
        expect_enum_branch: false,
    },
];

#[test]
fn enum_field_classification_table() {
    let (type_defs, enums) = table_ir();
    let ir_enum_map = FieldResolver::ir_enum_fields(&type_defs, &enums);
    for case in CASES {
        let out = java_builder_expression(
            &style_obj(),
            case.type_name,
            &std::collections::HashSet::new(),
            &std::collections::HashMap::new(),
            true,
            &[],
            &ir_enum_map,
        );
        assert_eq!(
            out.contains(ENUM_MARKER),
            case.expect_enum_branch,
            "{}: expected enum branch = {}, got:\n{out}",
            case.name,
            case.expect_enum_branch
        );
        assert_eq!(
            out.contains(RAW_MARKER),
            !case.expect_enum_branch,
            "{}: a raw String literal must be emitted only for the non-enum field, got:\n{out}",
            case.name
        );
    }
}

/// An explicit `enum_fields` config entry keeps working unchanged (config wins) — the IR only
/// rescues fields the config never mentioned. `OtherOptions.style` is `String` in the IR, so
/// only the config entry can make this classify as enum.
#[test]
fn an_explicit_enum_fields_config_entry_still_classifies_as_enum() {
    let (type_defs, enums) = table_ir();
    let ir_enum_map = FieldResolver::ir_enum_fields(&type_defs, &enums);
    let enum_fields: std::collections::HashSet<String> = ["style".to_string()].into_iter().collect();
    let out = java_builder_expression(
        &style_obj(),
        "OtherOptions",
        &enum_fields,
        &std::collections::HashMap::new(),
        true,
        &[],
        &ir_enum_map,
    );
    assert!(
        out.contains(ENUM_MARKER),
        "explicit enum_fields config must still classify the field as enum, got:\n{out}"
    );
}
