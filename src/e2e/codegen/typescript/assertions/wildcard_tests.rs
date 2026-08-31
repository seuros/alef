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

fn contains_value_on(field: &str, value: serde_json::Value) -> Assertion {
    Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Default::default()
    }
}

/// FALSE-POSITIVE CONTROL, and the reason this lowering changed. The pre-fix renderer emitted
/// `String(e.bar).includes(42)`; `String.prototype.includes` stringifies its argument, so that
/// expression is `"421".includes("42")` — TRUE — for an element of 421, and true again for
/// 3.142. Executed under node against `[{bar: 421}]` the old text reports the assertion as
/// PASSING and the new text reports it as failing. This test fails against the pre-fix code,
/// where `String(e.bar).includes` is present and `Number(` is not. ~keep
#[test]
fn a_numeric_wildcard_containment_compares_numerically_not_as_a_substring() {
    let out = render(
        &contains_value_on("items[].bar", serde_json::json!(42)),
        &array_resolver("items"),
    );
    assert_eq!(
        out,
        "    expect((result.items ?? []).some((e) => e.bar != null && Number(e.bar) === 42)).toBe(true);\n",
        "got: {out}"
    );
    assert!(!out.contains("String(e.bar).includes"), "substring lowering survived: {out}");
}

/// `Number(null)` is `0`, so an unguarded numeric comparison would report `contains: 0` as
/// satisfied by an absent leaf. Pins the guard that stops it. ~keep
#[test]
fn a_zero_expectation_is_guarded_against_an_absent_leaf() {
    let out = render(
        &contains_value_on("items[].bar", serde_json::json!(0)),
        &array_resolver("items"),
    );
    assert!(out.contains("e.bar != null &&"), "null guard missing: {out}");
}

/// OVER-APPLICATION CONTROL: a string expectation is still substring containment, byte for
/// byte. The numeric fix must not move the case that was already correct. ~keep
#[test]
fn a_string_wildcard_containment_keeps_its_substring_lowering() {
    let out = render(&contains_on("items[].name", "beta"), &array_resolver("items"));
    assert_eq!(
        out, "    expect((result.items ?? []).some((e) => String(e.name).includes(\"beta\"))).toBe(true);\n",
        "got: {out}"
    );
}

#[test]
fn a_boolean_wildcard_containment_compares_by_identity() {
    let out = render(
        &contains_value_on("items[].flag", serde_json::json!(true)),
        &array_resolver("items"),
    );
    assert_eq!(
        out,
        "    expect((result.items ?? []).some((e) => e.flag === true)).toBe(true);\n",
        "got: {out}"
    );
}

/// A `not_contains` inherits the same predicate, so the false positive it inherited is gone
/// with it — and inverted, the old lowering made `not_contains 42` FAIL against 421. ~keep
#[test]
fn a_numeric_wildcard_not_contains_uses_the_same_numeric_predicate() {
    let assertion = Assertion {
        assertion_type: "not_contains".to_string(),
        field: Some("items[].bar".to_string()),
        value: Some(serde_json::json!(42)),
        ..Default::default()
    };
    let out = render(&assertion, &array_resolver("items"));
    assert_eq!(
        out,
        "    expect((result.items ?? []).some((e) => e.bar != null && Number(e.bar) === 42)).toBe(false);\n",
        "got: {out}"
    );
}

/// LOUD SKIP, not a quiet pass: an object expectation has no sound single-element comparison,
/// so nothing executable is emitted for it. The pre-fix renderer emitted
/// `String(e.bar).includes({ a: 1 })`, i.e. `.includes("[object Object]")`, which is false for
/// every real value — a permanently-failing assertion dressed as coverage. ~keep
#[test]
fn an_object_wildcard_expectation_is_skipped_visibly_rather_than_lowered() {
    let out = render(
        &contains_value_on("items[].bar", serde_json::json!({ "a": 1 })),
        &array_resolver("items"),
    );
    assert_eq!(
        out,
        "    // skipped: unsupported traversal assertion object value on 'items[].bar'\n",
        "got: {out}"
    );
    assert!(!out.contains("expect("), "an assertion was emitted anyway: {out}");
}

#[test]
fn a_null_wildcard_expectation_is_skipped_visibly() {
    let out = render(
        &contains_value_on("items[].bar", serde_json::Value::Null),
        &array_resolver("items"),
    );
    assert_eq!(
        out,
        "    // skipped: unsupported traversal assertion null value on 'items[].bar'\n",
        "got: {out}"
    );
}
