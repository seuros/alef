//! Regression coverage for TypeScript wildcard-field assertion traversal.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use super::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn array_resolver(field: &str) -> FieldResolver {
    let result_fields: HashSet<String> = [field.to_string()].into_iter().collect();
    let array_fields: HashSet<String> = [field.to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &array_fields,
        &HashSet::new(),
    )
}

fn render(assertion: &Assertion, resolver: &FieldResolver) -> String {
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        resolver,
        false,
        &HashMap::new(),
        "typescript",
        false,
        false,
        false,
    );
    out
}

fn contains_on(field: &str, value: &str) -> Assertion {
    Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::Value::String(value.to_string())),
        ..Default::default()
    }
}

#[test]
fn typescript_wildcard_contains_quantifies_over_every_element() {
    let out = render(&contains_on("items[].name", "beta"), &array_resolver("items"));
    assert_eq!(
        out, "    expect((result.items ?? []).some((e) => String(e.name).includes(\"beta\"))).toBe(true);\n",
        "got: {out}"
    );
}

/// Regression lock: an explicit numeric index is a different, correct feature and must
/// keep lowering to a positional lookup, not a quantifier. ~keep
#[test]
fn typescript_explicit_index_still_lowers_to_a_positional_lookup() {
    let out = render(&contains_on("items[0].name", "beta"), &array_resolver("items"));
    assert!(out.contains("result.items[0].name"), "got: {out}");
    assert!(!out.contains(".some((e)"), "got: {out}");
}

/// CANARY. A code-generator unit test cannot execute JavaScript, so it cannot literally
/// run a fixture whose only match lives in element 1. The observable proxy is exact: the
/// pre-fix renderer emitted `result.items[0].name`, a lookup pinned to element 0, so a
/// value present only at element 1 could never be seen. This asserts no positional index
/// survives and the predicate is quantified over the whole array; it fails against the
/// pre-fix code, where `result.items[0]` is present. ~keep
#[test]
fn typescript_wildcard_match_in_a_non_first_element_is_not_pinned_to_element_zero() {
    let out = render(
        &contains_on("items[].name", "only-in-element-1"),
        &array_resolver("items"),
    );
    assert!(!out.contains("[0]"), "index-pinned lookup survived: {out}");
    assert!(out.contains("(result.items ?? []).some((e) =>"), "got: {out}");
    assert!(out.contains("String(e.name)"), "got: {out}");
}

/// `tools[].function` is also used as a DTO *type path* for input construction in
/// `test_file/render.rs` and `wasm.rs`. Those call sites never reach `render_assertion`,
/// so the wildcard branch here must not be the thing that keeps them working — this
/// only pins that the assertion path itself treats such a path as a traversal. ~keep
#[test]
fn typescript_wildcard_branch_is_scoped_to_assertion_rendering() {
    let out = render(&contains_on("items[].name", "beta"), &array_resolver("items"));
    assert!(out.starts_with("    expect("), "got: {out}");
}

/// `wildcard_split` consumes the first `[].` only, so before the guard the `.some()`
/// ranged over `pages` while its body read `e.links[0].url` — a whole-array claim that
/// only ever inspected element zero of the inner list. Pre-guard this test fails: the
/// skip line is absent and `links[0]` is present. ~keep
#[test]
fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
    let out = render(
        &contains_on("pages[].links[].url", "example.test"),
        &array_resolver("pages"),
    );
    assert_eq!(
        out, "    // skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
        "got: {out}"
    );
}
