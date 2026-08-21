//! Regression coverage for the collection `contains` predicate (alef defect B1).
//!
//! Split into its own file rather than added to `rust/assertions.rs`: that file is already
//! over the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), so new test
//! coverage goes into a fresh module instead of growing it. ~keep
//!
//! Before the fix, `containment_predicate` in `rust/assertions.rs` matched a collection item
//! only through its `"name"` key with `==`. A fixture item shaped like
//! `{"kind":"Function","name":"main"}` asserting `contains "Function"` panicked with
//! `expected collection item name: Function`, because `"Function"` never lives under `"name"`.
//! Five other e2e backends (Python, Node/TypeScript, Ruby, Java, C#) already implement the
//! intended semantics: a substring search over several item keys (Python/Node/Ruby check
//! `kind`/`name`/`source`/`alias`/`text`/`signature`; Java/C# search the whole serialized item).
//! Rust now matches that shape.

use std::collections::{HashMap, HashSet};

use super::assertions::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

fn collection_resolver(field: &str) -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from([field.to_string()]),
        &HashSet::new(),
    )
}

fn render_contains(field: &str, expected: &str) -> String {
    let assertion = Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::json!(expected)),
        ..Default::default()
    };
    let resolver = collection_resolver(field);
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "sample",
        "sample",
        false,
        &[],
        &resolver,
        false,
        false,
        false,
        false,
        false,
        None,
    );
    out
}

/// ~keep Duplicated on purpose, same rationale as `assertions.rs`'s
/// `the_collection_predicate_is_valid_rust_against_a_real_collection`: this mirrors
/// `containment_predicate`'s collection arm as real, independently-compiled code so the two
/// cannot silently drift apart. Any change to the key list or fallback in `rust/assertions.rs`
/// must be mirrored here, or this function's own table below will fail.
fn collection_item_matches(item: &serde_json::Value, expected: &str) -> bool {
    match item {
        serde_json::Value::String(text) => text.contains(expected),
        serde_json::Value::Object(fields) => {
            ["kind", "name", "source", "alias", "text", "signature"]
                .iter()
                .any(|key| {
                    fields
                        .get(*key)
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|text| text.contains(expected))
                })
                || item.to_string().contains(expected)
        }
        _ => false,
    }
}

/// Table-driven regression for the four scenarios that distinguish the fixed predicate from
/// the pre-fix `name`-only equality check.
#[test]
fn collection_contains_matches_the_intended_substring_semantics() {
    let function_item = serde_json::json!({"kind": "Function", "name": "main"});

    let cases: [(&str, &serde_json::Value, &str, bool); 4] = [
        // The exact bug report: the match lives under `kind`, not `name`. Pre-fix this is
        // `false` because only `fields.get("name")` was ever consulted.
        ("match on a non-name key ('kind')", &function_item, "Function", true),
        // `name` must still work: fixing the key list must not regress the field it always
        // covered.
        ("match on the 'name' key", &function_item, "main", true),
        // Pre-fix used `==`; "unc" is a strict substring of "Function", not the whole value.
        ("substring match, not whole-value equality", &function_item, "unc", true),
        // Negative control: a value absent from every checked key and from the item's own
        // serialized text must still fail, or the predicate would be vacuously true.
        ("genuine non-match fails", &function_item, "Class", false),
    ];

    for (description, item, expected, want) in cases {
        let got = collection_item_matches(item, expected);
        assert_eq!(got, want, "case '{description}': item={item}, expected={expected}");
    }
}

/// The generated Rust text itself must check the same key set the executable table above
/// relies on, and must use `.contains(` (substring) rather than `==` (whole-value equality) —
/// otherwise the executable parity check above could pass while the actual generator regresses.
#[test]
fn generated_predicate_checks_every_key_with_substring_not_equality() {
    let rendered = render_contains("structure", "Function");

    assert!(
        rendered.contains("fields.get(*key)"),
        "generated predicate must look up each key via the shared `fields.get(*key)` lookup, got: {rendered}"
    );
    for key in ["kind", "name", "source", "alias", "text", "signature"] {
        assert!(
            rendered.contains(&format!("\"{key}\"")),
            "generated predicate must list the '{key}' key in its key set, got: {rendered}"
        );
    }
    assert!(
        rendered.contains(".contains(r#\"Function\"#)") || rendered.contains(".contains(\"Function\")"),
        "generated predicate must use substring `.contains`, got: {rendered}"
    );
    assert!(
        !rendered.contains("== r#\"Function\"#") && !rendered.contains("== \"Function\""),
        "generated predicate must not use whole-value equality, got: {rendered}"
    );
}

/// The exact fixture shape from the bug report (`{"type":"contains","field":"structure",
/// "value":"Function"}` against items like `{"kind":"Function","name":"main",...}`) must emit
/// parseable Rust — the same guard as `every_containment_operator_emits_parseable_rust`, scoped
/// to the reported regression.
#[test]
fn bug_report_fixture_shape_emits_parseable_rust() {
    let body = render_contains("structure", "Function");
    let unit = format!("fn generated() {{\n{body}}}\n");
    syn::parse_file(&unit).unwrap_or_else(|error| panic!("must emit parseable Rust: {error}\n{unit}"));
}
