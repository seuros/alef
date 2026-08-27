//! Regression for the `wasm` docs-snippet TS18048 family: a `result`-level field declared
//! `Option<T>` in the IR (no `alef.toml` `fields_optional` entry at all) must render `?.` on
//! every wasm accessor that reaches through it, exactly like the field's own struct-field
//! reads: `data`, `data.kind`, `data.children`.
//!
//! ~keep Reproduces the shape one consumer measured against a released alef: a result type
//! `ProcessResult { data: Option<DataNode> }` with no `[crates.e2e] fields_optional` entry for
//! `data` anywhere in its `alef.toml` -- optionality has to come entirely from the IR's
//! `FieldDef.optional`, the same fact `resolve_show_unwraps_ir_only_optional_field_in_non_leaf_position`
//! (in the parent module's own `tests`) proves for the `rust` target. `fa9733e73` routed `wasm`
//! through the same optionality-aware renderer `node` already used; this file is the case that
//! commit fixed, kept as its own fixture so a future change to the `wasm` accessor path breaks a
//! named test instead of silently reintroducing the gap.

use super::*;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};

fn docs_fixture(shows: &[&str]) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"source": "irrelevant"},
        "docs": {
            "topic": "guides",
            "stem": "sample_fixture",
            "shows": shows,
        }
    }))
    .expect("fixture must parse")
}

/// No `fields_optional` entry anywhere -- matches the consumer's `alef.toml`, which declares
/// `data` only in `result_fields`, never in `fields_optional`.
fn config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "process".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        result_fields: ["data".to_string()].into_iter().collect(),
        ..E2eConfig::default()
    }
}

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

fn process_returning(type_name: &str) -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "process".to_string(),
        return_type: TypeRef::Named(type_name.to_string()),
        ..FunctionDef::default()
    }]
}

/// `process` -> `ProcessResult { data: Option<DataNode> }`, `DataNode { kind: String, children:
/// Vec<DataNode> }` -- the exact field names and shape of the reproduced fixture.
fn type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "ProcessResult".to_string(),
            fields: vec![field("data", TypeRef::Named("DataNode".to_string()), true)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "DataNode".to_string(),
            fields: vec![
                field("kind", TypeRef::String, false),
                field(
                    "children",
                    TypeRef::Vec(Box::new(TypeRef::Named("DataNode".to_string()))),
                    false,
                ),
            ],
            ..TypeDef::default()
        },
    ]
}

fn shown_expressions(language: &str, shows: &[&str]) -> Vec<String> {
    resolve(
        &docs_fixture(shows),
        &config(),
        language,
        &type_defs(),
        &[],
        &process_returning("ProcessResult"),
    )
    .into_iter()
    .map(|operation| operation.expression)
    .collect()
}

/// The bare optional field itself needs no guard -- there is nothing after it to chain through.
#[test]
fn wasm_shows_the_bare_optional_field_unguarded() {
    assert_eq!(shown_expressions("wasm", &["data"]), vec!["result.data".to_string()]);
}

/// The defect: a nested access through the optional field must gain `?.`, or `tsc` under
/// `strict` rejects it as TS18048 ("'result.data' is possibly 'undefined'").
#[test]
fn wasm_chains_through_the_ir_only_optional_field() {
    assert_eq!(
        shown_expressions("wasm", &["data.kind"]),
        vec!["result.data?.kind".to_string()],
    );
    assert_eq!(
        shown_expressions("wasm", &["data.children"]),
        vec!["result.data?.children".to_string()],
    );
}

/// `node` and `wasm` are one TypeScript surface (`fa9733e73`); the fix must not be wasm-only.
#[test]
fn node_and_wasm_agree_on_the_ir_only_optional_field() {
    assert_eq!(
        shown_expressions("node", &["data.kind"]),
        shown_expressions("wasm", &["data.kind"]),
    );
}

/// Negative control: a required nested field must NOT gain a spurious `?.`, or the fix silences
/// real errors on genuinely non-optional access instead of only guarding optional ones.
#[test]
fn wasm_does_not_guard_a_required_nested_field() {
    let type_defs = vec![
        TypeDef {
            name: "ProcessResult".to_string(),
            fields: vec![field("data", TypeRef::Named("DataNode".to_string()), false)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "DataNode".to_string(),
            fields: vec![field("kind", TypeRef::String, false)],
            ..TypeDef::default()
        },
    ];
    let expressions: Vec<String> = resolve(
        &docs_fixture(&["data.kind"]),
        &config(),
        "wasm",
        &type_defs,
        &[],
        &process_returning("ProcessResult"),
    )
    .into_iter()
    .map(|operation| operation.expression)
    .collect();
    assert_eq!(expressions, vec!["result.data.kind".to_string()]);
}
