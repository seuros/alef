//! Regression for the `typescript_first_item` tail-casing defect: an `Iterate` operation whose
//! `path` splits on `"[0]."` (e.g. `results[0].matched_terms`, iterating a nested collection
//! reached through the first element of an outer array) used to splice the tail segment
//! (`matched_terms`) into the generated expression verbatim, as `first?.matched_terms`. Both
//! node (napi-rs's default `#[napi(object)]` derive) and wasm (`to_node_name` in
//! `backends/wasm/gen_bindings/types.rs`'s `gen_getter`) expose struct fields camelCased, never
//! the fixture's snake_case IR/wire name, so the generated snippet referenced a member neither
//! binding declares (`first?.matched_terms` against a binding that only exports
//! `.matchedTerms`) -- a `tsc` `TS2551`/`Cannot find name`-shaped failure at typecheck.
//!
//! `source` (the part before `"[0]."`) already went through `resolver.accessor`, which does
//! apply this casing (`render_typescript_with_optionals` calls `.to_lower_camel_case()` on every
//! segment) -- only the tail bypassed it, via a hand-rolled `format!("first?.{tail}")` splice.

use super::*;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};

fn docs_fixture() -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"query": "irrelevant"},
        "docs": {
            "topic": "guides",
            "stem": "sample_fixture",
            "presentation": {
                "operations": [
                    {
                        "op": "iterate",
                        "path": "results[0].matched_terms",
                        "item": "term",
                        "fields": ["value"],
                        "display": true
                    }
                ]
            }
        }
    }))
    .expect("fixture must parse")
}

fn config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "search".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        result_fields: ["results".to_string()].into_iter().collect(),
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

fn search_returning(type_name: &str) -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "search".to_string(),
        return_type: TypeRef::Named(type_name.to_string()),
        ..FunctionDef::default()
    }]
}

/// `search` -> `Envelope { results: Vec<ResultItem> }`, `ResultItem { matched_terms:
/// Vec<TermMatch> }`, `TermMatch { value: String }` -- an outer array of per-item results, each
/// carrying its own nested array field, matching the shape that reproduces the defect.
fn type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Envelope".to_string(),
            fields: vec![field(
                "results",
                TypeRef::Vec(Box::new(TypeRef::Named("ResultItem".to_string()))),
            )],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ResultItem".to_string(),
            fields: vec![field(
                "matched_terms",
                TypeRef::Vec(Box::new(TypeRef::Named("TermMatch".to_string()))),
            )],
            ..TypeDef::default()
        },
        TypeDef {
            name: "TermMatch".to_string(),
            fields: vec![field("value", TypeRef::String)],
            ..TypeDef::default()
        },
    ]
}

fn iterate_expression(language: &str) -> String {
    resolve(
        &docs_fixture(),
        &config(),
        language,
        &type_defs(),
        &search_returning("Envelope"),
    )
    .into_iter()
    .find(|operation| operation.kind == "iterate")
    .expect("fixture declares one iterate operation")
    .expression
}

/// The defect: the tail segment after `"[0]."` must be camelCased for wasm, exactly like every
/// other field segment `resolver.accessor` renders.
#[test]
fn wasm_camel_cases_the_iterate_tail_segment() {
    assert_eq!(iterate_expression("wasm"), "first?.matchedTerms");
}

/// `node` and `wasm` share the same TypeScript surface for this split (see
/// `typescript_first_item`'s `matches!(language, "node" | "wasm")` gate) -- the fix must not be
/// wasm-only.
#[test]
fn node_camel_cases_the_iterate_tail_segment_identically_to_wasm() {
    // ~keep Assert the concrete expected text first. Comparing node's output to wasm's alone
    // passes whenever both are equally wrong, which is precisely the failure this test exists
    // to catch.
    assert_eq!(iterate_expression("node"), "first?.matchedTerms");
    assert_eq!(iterate_expression("node"), iterate_expression("wasm"));
}

/// Negative control: a tail segment that is already a single lower-camel-case word (no
/// underscores to convert) must render unchanged, not gain spurious casing artifacts.
#[test]
fn wasm_leaves_an_already_camel_tail_segment_unchanged() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"query": "irrelevant"},
        "docs": {
            "topic": "guides",
            "stem": "sample_fixture",
            "presentation": {
                "operations": [
                    {
                        "op": "iterate",
                        "path": "results[0].tags",
                        "item": "tag",
                        "fields": [],
                        "display": true
                    }
                ]
            }
        }
    }))
    .expect("fixture must parse");
    let type_defs = vec![
        TypeDef {
            name: "Envelope".to_string(),
            fields: vec![field(
                "results",
                TypeRef::Vec(Box::new(TypeRef::Named("ResultItem".to_string()))),
            )],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ResultItem".to_string(),
            fields: vec![field("tags", TypeRef::Vec(Box::new(TypeRef::String)))],
            ..TypeDef::default()
        },
    ];
    let expression = resolve(&fixture, &config(), "wasm", &type_defs, &search_returning("Envelope"))
        .into_iter()
        .find(|operation| operation.kind == "iterate")
        .expect("fixture declares one iterate operation")
        .expression;
    assert_eq!(expression, "first?.tags");
}
