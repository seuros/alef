//! Regression coverage for Python wildcard-field assertion traversal.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use super::{render_field_contains, resolver_with_array_field};

#[test]
fn python_wildcard_contains_iterates_every_element() {
    let out = render_field_contains(&resolver_with_array_field("links"), "links[].link_type", "external");
    assert!(out.contains("any("), "got: {out}");
    assert!(
        out.contains("in str(_e.link_type) for _e in (result.links or [])"),
        "got: {out}"
    );
    assert!(!out.contains("[0]"), "wildcard must not pin element 0, got: {out}");
}

#[test]
fn python_explicit_index_still_pins_element_zero() {
    let out = render_field_contains(&resolver_with_array_field("links"), "links[0].link_type", "external");
    assert!(out.contains("result.links[0].link_type"), "got: {out}");
    assert!(
        !out.contains("for _e in"),
        "explicit index must not become a traversal, got: {out}"
    );
}

/// Canary for the wildcard defect. `links[].link_type` lowered to `links[0]`, so a
/// fixture whose match lives in element 1 asserted against element 0 and passed by
/// accident. This pins the only property observable at codegen level that separates
/// the two: the emitted comprehension must quantify over the whole list rather than
/// name an index. Pre-fix the emitted text is `result.links[0].link_type` and every
/// assertion below fails. ~keep
#[test]
fn python_wildcard_match_in_second_element_is_not_missed() {
    let out = render_field_contains(&resolver_with_array_field("links"), "links[].link_type", "canonical");
    assert!(out.contains("for _e in"), "got: {out}");
    assert!(!out.contains("links[0]"), "got: {out}");
    assert!(!out.contains("links[1]"), "predicate must be index-free, got: {out}");
}

/// `wildcard_split` consumes the first `[].` only, so before the guard the comprehension
/// ranged over `pages` while its body read `_e.links[0].url` — a whole-array claim that
/// only ever inspected element zero of the inner list. Pre-guard this test fails on both
/// assertions: the skip line is absent and `links[0]` is present. ~keep
#[test]
fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
    let out = render_field_contains(
        &resolver_with_array_field("pages"),
        "pages[].links[].url",
        "example.test",
    );
    assert!(
        out.contains("# skipped: nested array-wildcard field 'pages[].links[].url' not supported"),
        "expected a visible skip, got: {out}"
    );
    assert!(!out.contains("links[0]"), "inner wildcard collapsed to index 0: {out}");
    assert!(
        !out.contains("any("),
        "no quantifier may be emitted for a refused path: {out}"
    );
}

/// ~keep Registered here rather than in `python/mod.rs`, which is at its recorded file-size
/// ceiling and may not grow by even a `mod` declaration.
#[path = "assertion_wildcard_element_tests.rs"]
mod assertion_wildcard_element_tests;

/// ~keep Same reason as `assertion_wildcard_element_tests` above.
#[path = "wildcard_typeddict_tests.rs"]
mod wildcard_typeddict_tests;
