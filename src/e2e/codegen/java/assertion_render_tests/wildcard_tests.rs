//! Regression coverage for Java wildcard-field assertion traversal.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use super::{make_contains_assertion, render_bare};

#[test]
fn wildcard_contains_scans_every_element_not_just_index_zero() {
    let out = render_bare(&make_contains_assertion("links[].link_type", "external"));
    assert!(out.contains(".stream().anyMatch("), "got: {out}");
    assert!(out.contains("assertTrue("), "got: {out}");
    assert!(
        !out.contains(".get(0)"),
        "wildcard must not lower to index 0, got: {out}"
    );
    assert!(!out.contains("[0]"), "wildcard must not lower to index 0, got: {out}");
}

#[test]
fn explicit_numeric_index_still_targets_that_element() {
    let out = render_bare(&make_contains_assertion("links[0].link_type", "external"));
    assert!(
        out.contains(".get(0)") || out.contains("[0]"),
        "explicit index must be preserved, got: {out}"
    );
    assert!(
        !out.contains("anyMatch"),
        "explicit index must not become a scan, got: {out}"
    );
}

/// Codegen-level canary for the wildcard defect. A fixture array whose only match lives
/// in element 1 is caught by `anyMatch` over the whole stream and missed by the pre-fix
/// single-index accessor, so this fails against the pre-fix renderer. It cannot execute
/// the generated Java, so it pins the property structurally. ~keep
#[test]
fn wildcard_match_in_element_one_is_reachable() {
    let out = render_bare(&make_contains_assertion("links[].link_type", "internal"));
    assert!(
        out.contains(".stream().anyMatch("),
        "an index-0 accessor would miss a match in element 1, got: {out}"
    );
    assert!(out.contains("\"internal\""), "got: {out}");
    assert!(!out.contains(".get(0)"), "got: {out}");
}

/// `wildcard_split` consumes the first `[].` only, so before the guard the `anyMatch`
/// ranged over `pages` while its body read `e.links().get(0).url()` — a whole-array claim
/// that only ever inspected element zero of the inner list. Java hides the collapse
/// behind `.get(0)` rather than a bracket index. Pre-guard this test fails on both
/// assertions: the skip line is absent and `.get(0)` is present. ~keep
#[test]
fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
    let out = render_bare(&make_contains_assertion("pages[].links[].url", "example.test"));
    assert_eq!(
        out, "        // skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
        "got: {out}"
    );
}

#[test]
fn wildcard_lambda_parameter_is_unique_per_assertion() {
    let first = render_bare(&make_contains_assertion("links[].link_type", "external"));
    let second = render_bare(&make_contains_assertion("links[].link_type", "internal"));
    let param_of = |s: &str| {
        let start = s.find("anyMatch(").expect("expected an anyMatch call") + "anyMatch(".len();
        s[start..start + s[start..].find(' ').expect("param is space-delimited")].to_string()
    };
    assert_ne!(param_of(&first), param_of(&second), "lambda params must not collide");
}
