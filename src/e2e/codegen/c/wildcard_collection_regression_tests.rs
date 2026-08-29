//! Regression coverage for the `field[].key` wildcard-leaf collection-lowering defect
//! (alef task #59).
//!
//! ~keep New module rather than growing `assertions.rs` or `test_function.rs` (both already
//! over the repo's 1,000-line cap; see `file-modularization` in CLAUDE.md).
//!
//! Before the fix, a `field[].key` fixture path (e.g. `items[].kind`) lowered to a SCALAR
//! `alef_json_get_string(items_json, "kind")` call against the ARRAY's own JSON text —
//! `items_json` never has a `"kind"` property, so every "contains"-shaped assertion built
//! from that local was unsatisfiable by construction, no matter what the array actually
//! contained. These tests assert on the generated C TEXT itself (not a hand-written mirror of
//! the intended semantics), because the mirror would pass unchanged against the buggy
//! generator — the giveaway here is specifically what the generator emits.

use std::collections::{HashMap, HashSet};

use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::assertions::{EffectiveConfigSource, FieldConfigSources, emit_nested_accessor, render_assertion};
use super::collection_wildcard::NestedLeafOutcome;

fn item_list_types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "ProcessResult".into(),
            fields: vec![FieldDef {
                name: "items".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Item".into()))),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Item".into(),
            fields: vec![FieldDef {
                name: "kind".into(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

fn global_sources() -> FieldConfigSources {
    FieldConfigSources {
        result_fields: EffectiveConfigSource::Global,
        fields: EffectiveConfigSource::Global,
    }
}

fn permissive_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

/// The extraction phase must hand back a `Wildcard` outcome for `items[].kind`, not a scalar
/// `char*` local — and must NOT emit the broken `alef_json_get_string(items_json, "kind")`
/// call against the array's own JSON text.
#[test]
fn wildcard_leaf_returns_a_quantifier_outcome_not_a_broken_scalar_accessor() {
    let types = item_list_types();
    let mut output = String::new();
    let mut handles = Vec::new();

    let outcome = emit_nested_accessor(
        &mut output,
        "sample",
        "items[].kind",
        "items__kind",
        "result",
        &HashMap::new(),
        &HashSet::new(),
        &mut handles,
        "ProcessResult",
        "items[].kind",
        &types,
        &global_sources(),
    )
    .expect("items[].kind resolves: items is on ProcessResult, kind is on Item")
    .expect("a wildcard leaf returns Some(..), not None");

    assert_eq!(
        outcome,
        NestedLeafOutcome::Wildcard {
            array_var: "items_json".to_string(),
            key_snake: "kind".to_string(),
        },
        "got: {outcome:?}"
    );
    assert!(
        output.contains("char* items_json = sample_process_result_items(result);"),
        "the array itself must still be extracted once: {output}"
    );
    assert!(
        !output.contains("alef_json_get_string(items_json, \"kind\")"),
        "must not emit the scalar accessor that reads a non-existent \"kind\" property off the \
         array's own JSON text: {output}"
    );
}

/// The decisive assertion-generation check: a `contains` assertion against `items[].kind` must
/// render a loop that can pass when SOME element's `kind` contains the expected substring, not
/// a single scalar `strstr` call that can never succeed. Matches the shape reported in
/// a consumer's generated `test_process.c` (alef task #59).
#[test]
fn contains_on_a_wildcard_field_renders_a_per_element_quantifier() {
    let assertion = Assertion {
        assertion_type: "contains".to_string(),
        field: Some("items[].kind".to_string()),
        value: Some(serde_json::json!("Widget")),
        ..Default::default()
    };
    let accessed_fields = [("items[].kind".to_string(), "items__kind".to_string(), true)];
    let mut wildcard_locals = HashMap::new();
    wildcard_locals.insert(
        "items__kind".to_string(),
        ("items_json".to_string(), "kind".to_string()),
    );

    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "sample",
        &permissive_resolver(),
        &accessed_fields,
        &HashMap::new(),
        &HashMap::new(),
        &wildcard_locals,
    );

    assert!(
        out.contains("alef_json_array_count(items_json)"),
        "must iterate the array, not read one scalar off it: {out}"
    );
    assert!(
        out.contains("alef_json_array_get_index(items_json"),
        "must fetch each element in turn: {out}"
    );
    assert!(
        out.contains("alef_json_get_string(_wc_elem, \"kind\")"),
        "must extract \"kind\" from the ELEMENT, not the array: {out}"
    );
    assert!(
        out.contains("strstr(_wc_val, \"Widget\") != NULL"),
        "must test the per-element value against the expected substring: {out}"
    );
    assert!(
        out.contains("assert(found &&"),
        "must gate on having found a match: {out}"
    );

    // The exact unsatisfiable shape from the bug report must be gone.
    assert!(
        !out.contains("items__kind != NULL && strstr(items__kind, \"Widget\") != NULL"),
        "must not emit the old scalar-vs-array assertion that could never pass: {out}"
    );
}

/// Two separate `contains` assertions against the same wildcard field (as tslp's own fixture
/// does, once per expected substring) must each independently quantify over the array — not
/// collapse onto one already-extracted scalar that could only ever match one substring.
#[test]
fn two_contains_assertions_on_the_same_wildcard_field_each_get_their_own_quantifier() {
    let accessed_fields = [("items[].kind".to_string(), "items__kind".to_string(), true)];
    let mut wildcard_locals = HashMap::new();
    wildcard_locals.insert(
        "items__kind".to_string(),
        ("items_json".to_string(), "kind".to_string()),
    );

    let mut out = String::new();
    for expected in ["Widget", "Gadget"] {
        let assertion = Assertion {
            assertion_type: "contains".to_string(),
            field: Some("items[].kind".to_string()),
            value: Some(serde_json::json!(expected)),
            ..Default::default()
        };
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "sample",
            &permissive_resolver(),
            &accessed_fields,
            &HashMap::new(),
            &HashMap::new(),
            &wildcard_locals,
        );
    }

    assert!(out.contains("strstr(_wc_val, \"Widget\") != NULL"), "got: {out}");
    assert!(out.contains("strstr(_wc_val, \"Gadget\") != NULL"), "got: {out}");
    assert_eq!(
        out.matches("assert(found &&").count(),
        2,
        "each assertion must render its own independent quantifier block: {out}"
    );
}

/// `equals` against a wildcard field means "some element equals exactly", rendered with
/// `strcmp`, not `strstr`.
#[test]
fn equals_on_a_wildcard_field_uses_strcmp_not_strstr() {
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("items[].kind".to_string()),
        value: Some(serde_json::json!("Widget")),
        ..Default::default()
    };
    let accessed_fields = [("items[].kind".to_string(), "items__kind".to_string(), true)];
    let mut wildcard_locals = HashMap::new();
    wildcard_locals.insert(
        "items__kind".to_string(),
        ("items_json".to_string(), "kind".to_string()),
    );

    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "sample",
        &permissive_resolver(),
        &accessed_fields,
        &HashMap::new(),
        &HashMap::new(),
        &wildcard_locals,
    );

    assert!(out.contains("strcmp(_wc_val, \"Widget\") == 0"), "got: {out}");
    assert!(!out.contains("strstr"), "equals must not substring-match: {out}");
}

/// An assertion type this module does not implement for wildcard fields renders an honest skip
/// comment rather than a silently-wrong quantifier.
#[test]
fn unsupported_assertion_type_on_a_wildcard_field_renders_a_skip_comment_not_broken_code() {
    let assertion = Assertion {
        assertion_type: "greater_than".to_string(),
        field: Some("items[].kind".to_string()),
        value: Some(serde_json::json!(1)),
        ..Default::default()
    };
    let accessed_fields = [("items[].kind".to_string(), "items__kind".to_string(), true)];
    let mut wildcard_locals = HashMap::new();
    wildcard_locals.insert(
        "items__kind".to_string(),
        ("items_json".to_string(), "kind".to_string()),
    );

    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "sample",
        &permissive_resolver(),
        &accessed_fields,
        &HashMap::new(),
        &HashMap::new(),
        &wildcard_locals,
    );

    assert!(
        out.contains("// skipped: unsupported traversal assertion 'greater_than' on 'items[].kind'"),
        "got: {out}"
    );
    assert!(
        !out.contains("assert("),
        "an unimplemented predicate must not emit a fake assert: {out}"
    );
}

/// `not_empty` on a wildcard field must quantify over the array's elements — some element's
/// key holds a non-empty value — matching the sibling wildcard renderers in
/// `python`/`dart`/`elixir`'s `assertions.rs`. Before this, `not_empty` fell through to the
/// generic skip alongside genuinely unimplemented types like `greater_than`, even though C
/// already has every JSON primitive (`alef_json_get_string`, `strlen`) this predicate needs.
#[test]
fn not_empty_on_a_wildcard_field_renders_a_per_element_quantifier_not_a_skip() {
    let assertion = Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some("items[].kind".to_string()),
        ..Default::default()
    };
    let accessed_fields = [("items[].kind".to_string(), "items__kind".to_string(), true)];
    let mut wildcard_locals = HashMap::new();
    wildcard_locals.insert(
        "items__kind".to_string(),
        ("items_json".to_string(), "kind".to_string()),
    );

    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "sample",
        &permissive_resolver(),
        &accessed_fields,
        &HashMap::new(),
        &HashMap::new(),
        &wildcard_locals,
    );

    assert!(
        !out.contains("skipped"),
        "not_empty is now implemented for wildcard fields, must not skip: {out}"
    );
    assert!(
        out.contains("alef_json_array_count(items_json)"),
        "must iterate the array, not read one scalar off it: {out}"
    );
    assert!(
        out.contains("alef_json_get_string(_wc_elem, \"kind\")"),
        "must extract \"kind\" from the ELEMENT: {out}"
    );
    assert!(
        out.contains("if (_wc_val != NULL && strlen(_wc_val) > 0) { found = 1; }"),
        "must test each element's value for non-emptiness: {out}"
    );
    assert!(
        out.contains("assert(found && \"expected some element to have a non-empty value\");"),
        "got: {out}"
    );
}
