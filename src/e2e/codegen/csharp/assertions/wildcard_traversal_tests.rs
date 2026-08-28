//! Regression coverage for C# wildcard-field assertion traversal.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use super::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

fn contains_assertion(field: &str, value: &str) -> Assertion {
    Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::Value::String(value.to_string())),
        ..Default::default()
    }
}

fn render_bare(assertion: &Assertion) -> String {
    let resolver = FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    );
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        "SampleClass",
        "SampleException",
        &resolver,
        false,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
        false,
    );
    out
}

#[test]
fn wildcard_contains_scans_every_element_not_just_index_zero() {
    let out = render_bare(&contains_assertion("links[].link_type", "external"));
    assert!(out.contains("?.Any("), "got: {out}");
    assert!(out.contains("Assert.True("), "got: {out}");
    assert!(!out.contains("[0]"), "wildcard must not lower to index 0, got: {out}");
}

#[test]
fn explicit_numeric_index_still_targets_that_element() {
    let out = render_bare(&contains_assertion("links[0].link_type", "external"));
    assert!(out.contains("[0]"), "explicit index must be preserved, got: {out}");
    assert!(
        !out.contains(".Any("),
        "explicit index must not become a scan, got: {out}"
    );
}

/// Codegen-level canary for the wildcard defect. A fixture array whose only match lives
/// in element 1 is caught by `Any` over the whole list and missed by the pre-fix
/// single-index accessor, so this fails against the pre-fix renderer. It cannot execute
/// the generated C#, so it pins the property structurally. ~keep
#[test]
fn wildcard_match_in_element_one_is_reachable() {
    let out = render_bare(&contains_assertion("links[].link_type", "internal"));
    assert!(
        out.contains("?.Any("),
        "an index-0 accessor would miss a match in element 1, got: {out}"
    );
    assert!(
        out.contains("\\\"internal\\\"") || out.contains("\"internal\""),
        "got: {out}"
    );
    assert!(!out.contains("[0]"), "got: {out}");
}

#[test]
fn wildcard_lambda_parameter_is_unique_per_assertion() {
    let first = render_bare(&contains_assertion("links[].link_type", "external"));
    let second = render_bare(&contains_assertion("links[].link_type", "internal"));
    let param_of = |s: &str| {
        let start = s.find("?.Any(").expect("expected an Any call") + "?.Any(".len();
        s[start..start + s[start..].find(' ').expect("param is space-delimited")].to_string()
    };
    assert_ne!(param_of(&first), param_of(&second), "lambda params must not collide");
}

/// `wildcard_split` consumes the first `[].` only, so before the guard the `Any` ranged
/// over `pages` while its body read `e.Links[0].Url` — a whole-array claim that only ever
/// inspected element zero of the inner list. Pre-guard this test fails on both
/// assertions: the skip line is absent and `[0]` is present. ~keep
#[test]
fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
    let out = render_bare(&contains_assertion("pages[].links[].url", "example.test"));
    assert_eq!(
        out, "        // skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
        "got: {out}"
    );
}

/// ~keep Registered here rather than in `csharp.rs` or `csharp/assertions.rs`, both of which are
/// at their recorded file-size ceilings and may not grow by even a `mod` declaration.
#[path = "wildcard_element_tests.rs"]
mod wildcard_element_tests;
