//! A hand-authored `docs.presentation.operations` entry must be checked against the IR exactly
//! like a derived one already is.
//!
//! ~keep Before this, [`default_operations_from_assertions`]'s [`shows_on_result`] gate only ever
//! saw paths IT derived from `assertions`. `resolve_with` reached that gate only when `docs.shows`
//! / `docs.presentation.operations` came back empty — an explicit author-written operation took
//! the OTHER branch and skipped the IR check entirely. A fixture author who spelled a field wrong,
//! or wrote a path through a field the IR cannot walk any further into (a tagged union, in the
//! consumer shape this module was written for), got exactly the accessor spelled — silently,
//! until the per-language snippet validator compiled it and failed identically in every backend
//! sharing this one resolved path. [`validate_authored_operations`] closes that gap.

use super::*;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};

fn docs_fixture(presentation_operations: serde_json::Value, assertions: serde_json::Value) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"html": "<p>Hello World</p>"},
        "assertions": assertions,
        "docs": {
            "topic": "smoke",
            "stem": "sample_fixture",
            "presentation": {"operations": presentation_operations},
        }
    }))
    .expect("fixture must parse")
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

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

fn convert_returning(type_name: &str) -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "convert".to_string(),
        return_type: TypeRef::Named(type_name.to_string()),
        ..FunctionDef::default()
    }]
}

/// `convert` -> `Envelope { document: Document }`, `Document { label: String, format: FormatInfo,
/// items: Vec<Row> }`, `Row { value: String }`. `FormatInfo` is deliberately absent from
/// `type_defs` — the shape a tagged union has: `format` is a genuine field, but nothing further
/// can be walked into from it.
fn type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Envelope".to_string(),
            fields: vec![field("document", TypeRef::Named("Document".to_string()))],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Document".to_string(),
            fields: vec![
                field("label", TypeRef::String),
                field("format", TypeRef::Named("FormatInfo".to_string())),
                field("items", TypeRef::Vec(Box::new(TypeRef::Named("Row".to_string())))),
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Row".to_string(),
            fields: vec![field("value", TypeRef::String)],
            ..TypeDef::default()
        },
    ]
}

fn resolved(fixture: &Fixture, language: &str) -> Vec<PresentationOperation> {
    resolve(
        fixture,
        &config(),
        language,
        &type_defs(),
        &convert_returning("Envelope"),
    )
}

/// The renamed-field shape: `wrong_label` does not exist, `label` does. The bad `show` is
/// dropped; nothing takes its place because a hand-authored `presentation.operations` list is
/// never combined with derivation from `assertions`.
#[test]
fn a_misspelled_authored_show_is_dropped() {
    let fixture = docs_fixture(
        serde_json::json!([{"op": "show", "path": "document.wrong_label"}]),
        serde_json::json!([]),
    );
    assert_eq!(resolved(&fixture, "python"), Vec::new());
}

/// The tagged-union shape: `format` is real, but `accessor()` cannot walk a plain field access
/// past it into `variant.detail`. Dropped for the same reason as the misspelling above — the IR
/// cannot vouch for either — even though `format` itself is a genuine field.
#[test]
fn an_authored_show_through_an_unwalkable_field_is_dropped() {
    let fixture = docs_fixture(
        serde_json::json!([{"op": "show", "path": "document.format.variant.detail"}]),
        serde_json::json!([]),
    );
    assert_eq!(resolved(&fixture, "python"), Vec::new());
}

/// The negative control: a correctly-spelled authored `show` must survive untouched, both
/// languages agreeing on the exact same IR-derived field name. Two backends built from one
/// fixture and one resolver call is the check that catches a fixture-level fix which only ever
/// gets exercised in a single language.
#[test]
fn a_correct_authored_show_survives_in_every_language() {
    let fixture = docs_fixture(
        serde_json::json!([{"op": "show", "path": "document.label"}]),
        serde_json::json!([]),
    );
    assert_eq!(
        resolved(&fixture, "python")
            .into_iter()
            .map(|op| op.expression)
            .collect::<Vec<_>>(),
        vec!["result.document.label"]
    );
    assert_eq!(
        resolved(&fixture, "rust")
            .into_iter()
            .map(|op| op.expression)
            .collect::<Vec<_>>(),
        vec!["result.document.label"]
    );
}

/// The per-item field shape: `items` (the collection path) is real, but `wrong_value` is not a
/// member of `Row`, the collection's OWN element type — not `Document`, the call's result type.
/// The field is dropped and the `iterate` operation survives with an empty `fields` list, rather
/// than the whole operation being discarded: `items` itself is perfectly renderable.
#[test]
fn an_authored_iterate_drops_only_the_unknown_per_item_field() {
    let fixture = docs_fixture(
        serde_json::json!([{
            "op": "iterate",
            "path": "document.items",
            "item": "row",
            "fields": ["wrong_value"],
        }]),
        serde_json::json!([]),
    );
    let operations = resolved(&fixture, "python");
    assert_eq!(operations.len(), 1, "the iterate operation itself must survive");
    assert_eq!(operations[0].kind, "iterate");
    assert_eq!(operations[0].fields, Vec::<String>::new());
}

/// The negative control for `iterate`: a per-item field the element type genuinely declares must
/// survive, in both languages, agreeing on the same IR-derived accessor.
#[test]
fn an_authored_iterate_keeps_a_real_per_item_field_in_every_language() {
    let fixture = docs_fixture(
        serde_json::json!([{
            "op": "iterate",
            "path": "document.items",
            "item": "row",
            "fields": ["value"],
        }]),
        serde_json::json!([]),
    );
    for language in ["python", "rust"] {
        let operations = resolved(&fixture, language);
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].fields, vec!["row.value".to_string()]);
    }
}

/// When every authored operation is dropped, `resolve_with` must fall back to deriving from
/// `assertions` — the same fallback an entirely un-annotated fixture already gets — rather than
/// leaving the snippet with nothing to show at all.
#[test]
fn dropping_every_authored_operation_falls_back_to_deriving_from_assertions() {
    let fixture = docs_fixture(
        serde_json::json!([{"op": "show", "path": "document.wrong_label"}]),
        serde_json::json!([{"type": "equals", "field": "document.label", "value": "Hello"}]),
    );
    assert_eq!(
        resolved(&fixture, "python")
            .into_iter()
            .map(|op| op.expression)
            .collect::<Vec<_>>(),
        vec!["result.document.label"]
    );
}
