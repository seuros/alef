//! Regression coverage for `is_empty`/`not_empty` on a `char*` leaf that holds SERIALIZED JSON
//! ARRAY (or object) text rather than a plain string (alef task #59, second symptom).
//!
//! ~keep New module rather than growing `assertions.rs` (already over the repo's 1,000-line
//! cap; see `file-modularization` in CLAUDE.md). Distinct from
//! `wildcard_collection_regression_tests.rs`: that module covers the `field[].key`
//! quantifier defect; this one covers a different code path entirely (a plain `field.child`
//! leaf with no `[]` in its path at all) that happens to share the same root shape — a
//! collection value read as if it were a scalar string.
//!
//! An empty Rust `Vec`/`HashMap` field serializes across the C ABI as the two-byte JSON text
//! `"[]"`/`"{}"`, not `""`. `strlen(field_expr) == 0` reads that as non-empty, so
//! `is_empty`/`not_empty` against a collection-typed field asserted the wrong thing regardless
//! of what the collection actually held — reproduced by
//! `tree-sitter-language-pack/e2e/c/test_data_extraction.c`'s
//! `data_extraction_json_empty_object` and `data_extraction_properties_empty` fixtures, both
//! `is_empty` on `data.children` (a `Vec<DataNode>`).

use std::collections::{HashMap, HashSet};

use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::assertions::render_assertion;

fn permissive_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

/// The exact shape from `test_data_extraction.c`: an empty `Vec` field serializes as `"[]"`,
/// and `is_empty` must accept that, not just the empty string.
#[test]
fn is_empty_on_a_serialized_json_array_field_accepts_the_empty_array_literal() {
    let assertion = Assertion {
        assertion_type: "is_empty".to_string(),
        field: Some("data.children".to_string()),
        value: None,
        ..Default::default()
    };
    let accessed_fields = [("data.children".to_string(), "data_children".to_string(), false)];

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
        &HashMap::new(),
    );

    assert!(
        out.contains("strcmp(data_children, \"[]\") == 0"),
        "must accept the serialized-empty-array form: {out}"
    );
    assert!(
        out.contains("strlen(data_children) == 0"),
        "must still accept a genuinely empty string too: {out}"
    );
}

/// `not_empty`'s mirror: a field holding `"[]"` must NOT satisfy `not_empty` just because
/// `strlen("[]") > 0`.
#[test]
fn not_empty_on_a_serialized_json_array_field_rejects_the_empty_array_literal() {
    let assertion = Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some("data.children".to_string()),
        value: None,
        ..Default::default()
    };
    let accessed_fields = [("data.children".to_string(), "data_children".to_string(), false)];

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
        &HashMap::new(),
    );

    assert!(
        out.contains("strcmp(data_children, \"[]\") != 0"),
        "\"[]\" must not count as non-empty: {out}"
    );
}
