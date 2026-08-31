//! Which array-element shapes may be answered from the `_alefE2eItemTexts` TEXT SURFACE, and
//! which must not.
//!
//! ~keep The text surface exists to reach INTO a structured element — a chunk's `text`, a node's
//! `name` — and it does that by stringifying candidate members and substring-matching them. For
//! an element that is a bare number that operation is not "reaching in", it is a coercion: the
//! generated check for `contains: "42"` against a `Vec<u32>` was
//! `_alefE2eItemTexts(item).some((text) => text.includes("42"))`, and `_alefE2eItemTexts(421)`
//! is `["421"]`, so `"421".includes("42")` reported an array holding 421 as containing 42.
//! `[3.142]` matched it too. Executed under node, the old lowering passes both and the new one
//! fails both.
//!
//! Split out of `assertions.rs`, which is over the 1,000-line cap and may not grow.

use super::render_assertion;
use crate::core::ir::{FieldDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

/// `Report { codes: Vec<u32>, warnings: Vec<String>, chunks: Vec<Chunk> }` — one numeric
/// collection, one textual one, and one structured one, so the routing decision is exercised
/// against all three from a single resolver.
fn type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Report".to_string(),
            fields: vec![
                field(
                    "codes",
                    TypeRef::Vec(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
                ),
                field("warnings", TypeRef::Vec(Box::new(TypeRef::String))),
                field(
                    "chunks",
                    TypeRef::Vec(Box::new(TypeRef::Named("Chunk".to_string()))),
                ),
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Chunk".to_string(),
            fields: vec![field("text", TypeRef::String)],
            ..TypeDef::default()
        },
    ]
}

fn resolver() -> FieldResolver {
    let defs = type_defs();
    let result_fields: HashSet<String> = ["codes", "warnings", "chunks"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let array_fields = result_fields.clone();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &array_fields,
        &HashSet::new(),
    )
    .with_ir_collection_map(
        FieldResolver::ir_collection_fields(&defs),
        Some("Report".to_string()),
    )
}

fn render(assertion: &Assertion) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        &resolver(),
        false,
        &HashMap::new(),
        "typescript",
        false,
        false,
        false,
    );
    out
}

fn contains_on(field_path: &str, value: &str) -> Assertion {
    Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field_path.to_string()),
        value: Some(serde_json::Value::String(value.to_string())),
        ..Default::default()
    }
}

/// FALSE-POSITIVE CONTROL. Against the pre-fix generator this test fails: the emitted text was
/// the `_alefE2eItemTexts` substring form, which node reports as PASSING for `[421]`. ~keep
#[test]
fn a_string_expectation_against_a_numeric_collection_leaves_the_text_surface() {
    let out = render(&contains_on("codes", "42"));
    assert_eq!(
        out,
        "    expect(result.codes.some((item) => String(item) === \"42\")).toBe(true);\n",
        "got: {out}"
    );
    assert!(!out.contains("_alefE2eItemTexts"), "text surface survived: {out}");
    assert!(!out.contains(".includes("), "substring comparison survived: {out}");
}

/// OVER-APPLICATION CONTROL. A `Vec<String>` resolves to no struct-to-struct edge either, so a
/// fix keyed on "the IR could not name an element type" would have moved this too. Substring
/// containment over textual elements is the behaviour that was already correct. ~keep
#[test]
fn a_string_collection_keeps_the_text_surface() {
    let out = render(&contains_on("warnings", "deprecated"));
    assert_eq!(
        out,
        "    expect(result.warnings.some((item) => _alefE2eItemTexts(item).some((text) => text.includes(\"deprecated\")))).toBe(true);\n",
        "got: {out}"
    );
}

/// OVER-APPLICATION CONTROL: reaching into a struct element's prose members is exactly what the
/// text surface is for, and it does not move. ~keep
#[test]
fn a_struct_collection_keeps_the_text_surface() {
    let out = render(&contains_on("chunks", "hello"));
    assert!(out.contains("_alefE2eItemTexts(item)"), "got: {out}");
    assert!(!out.contains("String(item) ==="), "got: {out}");
}

#[test]
fn not_contains_on_a_numeric_collection_uses_the_same_equality_predicate() {
    let assertion = Assertion {
        assertion_type: "not_contains".to_string(),
        field: Some("codes".to_string()),
        value: Some(serde_json::Value::String("42".to_string())),
        ..Default::default()
    };
    let out = render(&assertion);
    assert_eq!(
        out,
        "    expect(result.codes.some((item) => String(item) === \"42\")).toBe(false);\n",
        "got: {out}"
    );
}

#[test]
fn contains_all_on_a_numeric_collection_uses_the_equality_predicate_for_every_value() {
    let assertion = Assertion {
        assertion_type: "contains_all".to_string(),
        field: Some("codes".to_string()),
        values: Some(vec![
            serde_json::Value::String("42".to_string()),
            serde_json::Value::String("7".to_string()),
        ]),
        ..Default::default()
    };
    let out = render(&assertion);
    assert_eq!(
        out,
        concat!(
            "    expect(result.codes.some((item) => String(item) === \"42\")).toBe(true);\n",
            "    expect(result.codes.some((item) => String(item) === \"7\")).toBe(true);\n",
        ),
        "got: {out}"
    );
}

#[test]
fn contains_any_on_a_numeric_collection_uses_the_equality_predicate() {
    let assertion = Assertion {
        assertion_type: "contains_any".to_string(),
        field: Some("codes".to_string()),
        values: Some(vec![
            serde_json::Value::String("42".to_string()),
            serde_json::Value::String("7".to_string()),
        ]),
        ..Default::default()
    };
    let out = render(&assertion);
    assert_eq!(
        out,
        "    expect([\"42\", \"7\"].some((v) => result.codes.some((item) => String(item) === v))).toBe(true);\n",
        "got: {out}"
    );
}

/// A resolver with no anchored IR root has no positive evidence about any element type, so every
/// field keeps the behaviour it had before this distinction existed. ~keep
#[test]
fn a_resolver_without_an_ir_root_keeps_the_text_surface_for_everything() {
    let result_fields: HashSet<String> = ["codes".to_string()].into_iter().collect();
    let unanchored = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &result_fields,
        &HashSet::new(),
    );
    let mut out = String::new();
    render_assertion(
        &mut out,
        &contains_on("codes", "42"),
        "result",
        &unanchored,
        false,
        &HashMap::new(),
        "typescript",
        false,
        false,
        false,
    );
    assert!(out.contains("_alefE2eItemTexts(item)"), "got: {out}");
}
