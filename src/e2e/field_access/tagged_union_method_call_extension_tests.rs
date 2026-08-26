//! Regression coverage for [`FieldResolver::result_field_oracle_knows`] resolving a path that
//! EXTENDS past a `fields_method_calls`-covered tagged-union crossing, instead of refusing it at
//! the union boundary.
//!
//! A downstream consumer's regen emitted warnings claiming `results[0].metadata.format.excel.
//! sheet_count` and `...sheet_names` had no such member, even though both fields are real and
//! ARE exposed in every generated binding, and the consumer's own `alef.toml` declared
//! `fields_method_calls = ["results[0].metadata.format.excel", "metadata.format.excel"]` --
//! i.e. told alef exactly how to cross the union. The bare, undotted path
//! `metadata.format.excel` produced no warning; only paths one segment deeper did, because the
//! oracle understood the method-call entry at its own depth and forgot it one segment later.

use super::FieldResolver;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};
use std::collections::{HashMap, HashSet};

/// `Envelope { metadata: Metadata }`, `Metadata { format: FormatMetadata }`, `FormatMetadata`
/// (an enum, not a `TypeDef`) with one variant `Excel(ExcelMetadata)`, `ExcelMetadata {
/// sheet_count: String, sheet_names: String }`. Mirrors the consumer's own shape:
/// `metadata.format` is a real, declared field whose type is a tagged union the IR cannot walk
/// through as a struct.
fn type_defs_with_tagged_union() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Envelope".to_string(),
            fields: vec![FieldDef {
                name: "metadata".to_string(),
                ty: TypeRef::Named("Metadata".to_string()),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Metadata".to_string(),
            fields: vec![FieldDef {
                name: "format".to_string(),
                ty: TypeRef::Named("FormatMetadata".to_string()),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ExcelMetadata".to_string(),
            fields: vec![
                FieldDef {
                    name: "sheet_count".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                },
                FieldDef {
                    name: "sheet_names".to_string(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                },
            ],
            ..TypeDef::default()
        },
    ]
}

fn format_enum_def() -> EnumDef {
    EnumDef {
        name: "FormatMetadata".to_string(),
        variants: vec![EnumVariant {
            name: "Excel".to_string(),
            fields: vec![FieldDef {
                name: "excel".to_string(),
                ty: TypeRef::Named("ExcelMetadata".to_string()),
                ..FieldDef::default()
            }],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }
}

fn resolver_with_method_calls(method_calls: &[&str]) -> FieldResolver {
    let type_defs = type_defs_with_tagged_union();
    let enums = vec![format_enum_def()];
    let result_map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
    let enum_map = FieldResolver::ir_enum_fields(&type_defs, &enums);
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    let method_calls_set: HashSet<String> = method_calls.iter().map(|s| s.to_string()).collect();

    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &method_calls_set,
    )
    .with_ir_result_fields(result_map, Some("Envelope".to_string()))
    .with_ir_enum_map(enum_map, Some("Envelope".to_string()))
    .with_ir_fields(reachable, excluded, optional)
}

/// The regression: `fields_method_calls` names exactly how to cross the union, and the suffix
/// past it (`sheet_count`) is a real field of the variant's own payload type. The oracle must
/// resolve it rather than refuse at the union boundary.
#[test]
fn a_path_through_a_method_call_covered_union_resolves_against_the_variant_payload() {
    let resolver = resolver_with_method_calls(&["metadata.format.excel"]);

    assert_eq!(
        resolver.result_field_oracle_knows("metadata.format.excel.sheet_count"),
        Some(true),
    );
    assert_eq!(
        resolver.result_field_oracle_knows("metadata.format.excel.sheet_names"),
        Some(true),
    );
}

/// The negative control: the identical path, with no `fields_method_calls` entry vouching for
/// the crossing, must still be refused. Proves the check can still fail -- without this, a
/// version of the fix that always resolved `Some(true)` for any tagged-union path would pass the
/// test above for the wrong reason.
#[test]
fn the_same_path_without_a_covering_method_calls_entry_still_refuses() {
    let resolver = resolver_with_method_calls(&[]);

    assert_eq!(
        resolver.result_field_oracle_knows("metadata.format.excel.sheet_count"),
        Some(false),
    );
}

/// The bare path stopping exactly AT the covered variant -- the exact shape the task's own
/// evidence cites as producing no warning today. Confirms the fix answers `Some(true)` for it
/// too, not just for paths one segment deeper.
#[test]
fn a_path_stopping_exactly_at_the_covered_variant_resolves() {
    let resolver = resolver_with_method_calls(&["metadata.format.excel"]);

    assert_eq!(resolver.result_field_oracle_knows("metadata.format.excel"), Some(true));
}

/// A field whose type is unjudgeable (a `serde_json::Value`, not a tagged union) must keep
/// abstaining past its own leaf rather than being rejected -- the fix must not turn every
/// declared-but-unresolvable field into a hard refusal, only ones a `fields_method_calls` entry
/// fails to cover AND the IR positively identifies as another named (non-struct) type.
#[test]
fn a_path_through_an_opaque_json_field_still_abstains_rather_than_refuses() {
    let type_defs = vec![TypeDef {
        name: "Envelope".to_string(),
        fields: vec![FieldDef {
            name: "payload".to_string(),
            ty: TypeRef::Json,
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }];
    let result_map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
    let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_result_fields(result_map, Some("Envelope".to_string()))
    .with_ir_fields(reachable, excluded, optional);

    assert_ne!(
        resolver.result_field_oracle_knows("payload.anything"),
        Some(false),
        "a map/JSON-valued field must stay unjudgeable past its own leaf, not rejected"
    );
}
