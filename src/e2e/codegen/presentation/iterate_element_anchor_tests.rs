//! An `Iterate` operation's per-item `fields` must be resolved against the collection's OWN
//! element type, never against the call's declared result type.
//!
//! ~keep The collision `authored_operation_validation_tests.rs` does not cover: there, the
//! per-item field name (`value`) is not reachable as any nested path from the call's result type
//! at all, so the wrongly-anchored resolver's fallback (an unresolvable dotted path renders flat)
//! produced the right accessor by accident. Here the per-item field (`content`) is ALSO reachable
//! from the result type through the very collection being iterated (`results.content`, because
//! `ExtractionResult.results: Vec<ExtractedDocument>` and `ExtractedDocument.content` both exist),
//! so a resolver still anchored at `ExtractionResult` finds that nested path and renders
//! `item.results?.[0]?.content` — a real bug that shipped in `extract_batch`-shaped fixtures
//! across every backend sharing this one presentation layer, not only TypeScript's.
//!
//! Reproducing that collision needs more than the IR shape: `FieldResolver::anchor_leaf` (the
//! mechanism that projects a bare leaf like `content` onto `results[0].content`) only tries a
//! collection field as a candidate prefix when the CONSUMER's own `result_fields` config names
//! it (`anchor_leaf_via_result_fields` iterates `self.result_fields`, not the IR blind). Measured
//! by sabotage: with `result_fields` empty, the wrongly-anchored resolver falls through to a
//! flat, unprefixed accessor regardless of anchoring, and the fix and the bug render
//! identically — a test built that way cannot fire. `result_fields = ["results"]` is what makes
//! the sabotaged resolver actually reach for the nested path; `fields_array` was not needed here
//! because `is_collection_root` already falls back to the anchored IR collection map
//! (`ir_collection_map`/`is_collection_path`) once real `type_defs` are wired in, and this test
//! wires them in via `resolve()`. ~keep

use super::*;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use std::collections::HashSet;

fn docs_fixture(presentation_operations: serde_json::Value) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {},
        "docs": {
            "topic": "smoke",
            "stem": "sample_fixture",
            "presentation": {"operations": presentation_operations},
        }
    }))
    .expect("fixture must parse")
}

/// `result_fields = ["results"]` is the load-bearing config key: it is what the consumer's own
/// `alef.toml` names as an envelope-projection candidate, and without it `anchor_leaf` never
/// tries `results` as a prefix for a bare leaf field, so the collision this file exists to catch
/// never fires regardless of resolver anchoring. ~keep
fn config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "extract_batch".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        result_fields: HashSet::from(["results".to_string()]),
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

/// `extract_batch` -> `ExtractionResult { results: Option<Vec<ExtractedDocument>>, errors:
/// Option<Vec<ExtractionError>> }`, `ExtractedDocument { content: Option<String> }`,
/// `ExtractionError { message: String }` — the exact shape the real bug shipped in, plus a
/// second collection (`errors`) the negative control below iterates. `errors` is deliberately
/// NOT named in `result_fields`, so `message` cannot collide with any result-anchored nested
/// path no matter how the resolver is anchored -- the negative control's whole point.
fn type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "ExtractionResult".to_string(),
            fields: vec![
                field(
                    "results",
                    TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named(
                        "ExtractedDocument".to_string(),
                    ))))),
                ),
                field(
                    "errors",
                    TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named(
                        "ExtractionError".to_string(),
                    ))))),
                ),
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ExtractedDocument".to_string(),
            fields: vec![field("content", TypeRef::Optional(Box::new(TypeRef::String)))],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ExtractionError".to_string(),
            fields: vec![field("message", TypeRef::String)],
            ..TypeDef::default()
        },
    ]
}

fn extract_batch_returning(type_name: &str) -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "extract_batch".to_string(),
        return_type: TypeRef::Named(type_name.to_string()),
        ..FunctionDef::default()
    }]
}

fn resolved(fixture: &Fixture, language: &str) -> Vec<PresentationOperation> {
    resolve(
        fixture,
        &config(),
        language,
        &type_defs(),
        &[],
        &extract_batch_returning("ExtractionResult"),
    )
}

/// The bug: a per-item field name that also happens to be reachable, nested, from the call's own
/// result type through the collection being iterated must still resolve against the element type,
/// not re-derive the whole `results[].content` path underneath the already-peeled loop variable.
#[test]
fn an_iterate_field_that_collides_with_a_result_nested_path_resolves_against_the_element_type() {
    let fixture = docs_fixture(serde_json::json!([{
        "op": "iterate",
        "path": "results",
        "item": "item",
        "fields": ["content"],
    }]));
    for language in ["python", "rust"] {
        let operations = resolved(&fixture, language);
        assert_eq!(operations.len(), 1);
        assert_eq!(operations[0].fields, vec!["item.content".to_string()]);
    }
}

/// The TypeScript rendering of the same collision: without the fix, this renders
/// `item.results?.[0]?.content` (Node/TypeScript's array-indexed optional-chaining rendering)
/// instead of `item.content` -- a bare optional leaf needs no `?.` of its own, since nothing
/// follows it in the chain.
#[test]
fn the_typescript_rendering_does_not_reintroduce_the_results_path_under_the_loop_item() {
    let fixture = docs_fixture(serde_json::json!([{
        "op": "iterate",
        "path": "results",
        "item": "item",
        "fields": ["content"],
    }]));
    let operations = resolved(&fixture, "node");
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].fields, vec!["item.content".to_string()]);
}

/// The genuine negative control: `errors` is a real IR collection field but, unlike `results`,
/// is never named in `result_fields`, so `anchor_leaf_via_result_fields` never tries it as a
/// prefix and `message` cannot collide with any result-anchored nested path -- with or without
/// the fix, both resolvers fall through to the same flat accessor. This must keep passing
/// whether `presentation.rs` resolves per-item fields against the element type or the result
/// type, so it would catch a fix that over-corrected into ALWAYS rewriting a per-item accessor
/// (e.g. one that broke `collection_element_type` resolution or forced an unconditional cast)
/// rather than only fixing the genuine collision above.
#[test]
fn an_iterate_field_with_no_result_fields_entry_for_its_collection_stays_flat() {
    let fixture = docs_fixture(serde_json::json!([{
        "op": "iterate",
        "path": "errors",
        "item": "item",
        "fields": ["message"],
    }]));
    for language in ["python", "rust", "node"] {
        let operations = resolved(&fixture, language);
        assert_eq!(operations.len(), 1);
        assert_eq!(
            operations[0].fields,
            vec!["item.message".to_string()],
            "a per-item field whose collection has no `result_fields` entry must never be \
             rewritten through the result type, in {language}"
        );
    }
}
