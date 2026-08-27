//! `PresentationOperation::field_displays` — the per-item-field companion to
//! `downgrade_display_unsafe_operations`'s whole-operation `display` downgrade.
//!
//! ~keep `FieldResolver::is_display_unsafe` (the whole-operation oracle `Show` and a
//! `fields`-less `Iterate` use) answers from `field_types`, which only ever records a field whose
//! declared type unwraps to a `Named` struct/enum — so a `Vec<Vec<String>>` per-item field never
//! appears there and reads as "safe". `field_displays` is a SEPARATE, allowlist-based answer
//! (`String`/`char`/numeric/`bool` primitive only) computed at the loop item's own element type,
//! not the call's anchored result type. The two controls here pin that the whole-operation oracle
//! is untouched by this addition.

use super::*;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

/// `SampleResult { widget: Widget, tables: Vec<Table> }`, `Widget { label: String }`,
/// `Table { name: String, cells: Vec<Vec<String>> }`.
fn type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "SampleResult".to_string(),
            fields: vec![
                field("widget", TypeRef::Named("Widget".to_string())),
                field("tables", TypeRef::Vec(Box::new(TypeRef::Named("Table".to_string())))),
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Widget".to_string(),
            fields: vec![field("label", TypeRef::String)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Table".to_string(),
            fields: vec![
                field("name", TypeRef::String),
                field("cells", TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String))))),
            ],
            ..TypeDef::default()
        },
    ]
}

fn functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "convert".to_string(),
        return_type: TypeRef::Named("SampleResult".to_string()),
        ..FunctionDef::default()
    }]
}

fn config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "convert".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    }
}

fn docs_fixture(operations: serde_json::Value) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {},
        "assertions": [],
        "docs": {
            "topic": "smoke",
            "stem": "sample_fixture",
            "presentation": {"operations": operations},
        }
    }))
    .expect("fixture must parse")
}

fn resolved_rust(operations: serde_json::Value) -> Vec<PresentationOperation> {
    resolve(
        &docs_fixture(operations),
        &config(),
        "rust",
        &type_defs(),
        &[],
        &functions(),
    )
}

/// The fix: a `Vec<Vec<String>>` per-item field is refused by the allowlist, and a `String`
/// per-item field on the same element type keeps its `display: true`, side by side in one
/// operation so a mistaken "downgrade the whole thing" implementation would fail this too.
#[test]
fn field_displays_matches_the_allowlist_per_field() {
    let operations = resolved_rust(serde_json::json!([{
        "op": "iterate", "path": "tables", "item": "table", "fields": ["name", "cells"], "display": true,
    }]));
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].fields.len(), 2);
    assert_eq!(
        operations[0].field_displays,
        vec![true, false],
        "`name` (String) stays display-safe, `cells` (Vec<Vec<String>>) is refused"
    );
}

/// When the operation itself never requested `display: true`, every entry is `false` without
/// consulting the allowlist — matching the template's pre-existing behaviour for that case.
#[test]
fn field_displays_is_all_false_when_the_operation_did_not_request_display() {
    let operations = resolved_rust(serde_json::json!([{
        "op": "iterate", "path": "tables", "item": "table", "fields": ["name", "cells"],
    }]));
    assert_eq!(operations[0].field_displays, vec![false, false]);
}

/// Only Rust computes the allowlist answer; another language keeps `*display` for every entry —
/// the pre-existing per-operation behaviour every non-Rust template still reads.
#[test]
fn field_displays_is_untouched_for_non_rust_languages() {
    let operations = resolve(
        &docs_fixture(serde_json::json!([{
            "op": "iterate", "path": "tables", "item": "table", "fields": ["name", "cells"], "display": true,
        }])),
        &config(),
        "python",
        &type_defs(),
        &[],
        &functions(),
    );
    assert_eq!(operations[0].field_displays, vec![true, true]);
}

/// Control: a `fields`-less `Iterate` over `Vec<Table>` (a `Named` element type) with
/// `display: true` keeps going through `downgrade_display_unsafe_operations` exactly as before —
/// unaffected by `field_displays`, which only ever computes entries for a NON-empty `fields` list.
#[test]
fn a_fields_less_iterate_keeps_its_existing_whole_operation_downgrade() {
    let operations = resolved_rust(serde_json::json!([{
        "op": "iterate", "path": "tables", "item": "table", "fields": [], "display": true,
    }]));
    assert_eq!(operations.len(), 1);
    assert!(
        !operations[0].display,
        "a fields-less iterate over a Named element type must still downgrade `display`"
    );
    assert_eq!(operations[0].field_displays, Vec::<bool>::new());
}

/// Control: a `show` of a `Named`-typed value with `display: true` keeps going through
/// `FieldResolver::is_display_unsafe`, unaffected by the new per-item-field allowlist.
#[test]
fn a_show_of_a_named_type_keeps_its_existing_whole_operation_downgrade() {
    let operations = resolved_rust(serde_json::json!([{
        "op": "show", "path": "widget", "display": true,
    }]));
    assert_eq!(operations.len(), 1);
    assert!(
        !operations[0].display,
        "a `show` of a `Named` type must still downgrade `display`"
    );
}
