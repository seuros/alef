//! Core regression coverage for `render_assertion` -- the Java e2e assertion generator.
//!
//! Split out of `assertions.rs` (file-modularization cap): see that file's module doc.

use std::collections::{HashMap, HashSet};

use super::assertions::{fractional_scalar_fields, render_assertion};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

fn make_resolver(optional: HashSet<String>, dat: HashSet<String>) -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_display_as_text_fields(dat)
}

fn make_equals_assertion(field: &str, value: &str) -> Assertion {
    Assertion {
        assertion_type: "equals".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::Value::String(value.to_string())),
        ..Default::default()
    }
}

fn make_contains_assertion(field: &str, value: &str) -> Assertion {
    Assertion {
        assertion_type: "contains".to_string(),
        field: Some(field.to_string()),
        value: Some(serde_json::Value::String(value.to_string())),
        ..Default::default()
    }
}

fn render_bare(assertion: &Assertion) -> String {
    let resolver = make_resolver(HashSet::new(), HashSet::new());
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        "Result",
        &resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        true,
    );
    out
}

fn render_with_optional(assertion: &Assertion, optional_field: &str) -> String {
    let optional: HashSet<String> = [optional_field.to_string()].into_iter().collect();
    let resolver = make_resolver(optional, HashSet::new());
    let mut out = String::new();
    render_assertion(
        &mut out,
        assertion,
        "result",
        "Result",
        &resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        true,
    );
    out
}

fn is_true_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "is_true".to_string(),
        field: Some(field.to_string()),
        ..Default::default()
    }
}

/// `Option<DataNode>` presence: before the fix this fell through to the generic
/// `.map(Objects::toString).orElse("")` string-coercion arm, so `assertTrue` received
/// a `String` argument -- a compile error, since `assertTrue` requires `boolean`.
#[test]
fn is_true_on_optional_struct_field_checks_presence() {
    let out = render_with_optional(&is_true_assertion("data"), "data");
    assert_eq!(
        out,
        "        assertTrue(java.util.Optional.ofNullable(result.data()).isPresent(), \"expected true (present)\");\n"
    );
}

#[test]
fn is_false_on_optional_struct_field_checks_absence() {
    let out = render_with_optional(
        &Assertion {
            assertion_type: "is_false".to_string(),
            field: Some("data".to_string()),
            ..Default::default()
        },
        "data",
    );
    assert_eq!(
        out,
        "        assertTrue(java.util.Optional.ofNullable(result.data()).isEmpty(), \"expected false (absent)\");\n"
    );
}

/// A follow-on member access through the same optional field must still compile: the
/// leaf (`equals` on `data.kind`) is unaffected by the `is_true` fix, so it continues to
/// route through the existing `Optional.ofNullable(...).map(Objects::toString).orElse("")`
/// coercion rather than needing an unwrap of its own -- Java's binding returns `@Nullable`
/// types, not `Optional<T>`, so `result.data().kind()` already compiles regardless of
/// nullability. ~keep
#[test]
fn equals_on_nested_field_through_optional_parent_is_unchanged() {
    let out = render_with_optional(&make_equals_assertion("data.kind", "KeyValue"), "data");
    assert!(out.contains("result.data().kind()"), "got: {out}");
}

#[test]
fn is_true_on_non_optional_field_is_unchanged() {
    let out = render_bare(&is_true_assertion("active"));
    assert_eq!(out, "        assertTrue(result.active(), \"expected true\");\n");
}
#[cfg(test)]
#[path = "wildcard_tests.rs"]
mod wildcard_tests;

/// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
/// (present, non-`binding_excluded`, on some IR type) but missing from the
/// hand-maintained `result_fields` config must still render a real assertion,
/// not a "skipped: field not available" stub — `java/test_method.rs` now
/// threads `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
#[test]
fn java_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
    let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(reachable, HashSet::new(), HashSet::new());
    let assertion = make_equals_assertion("data", "hello");
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        &resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        true,
    );
    assert!(!out.contains("skipped"), "got: {out}");
}

/// The negative-control half of the same regression: `internal_diagnostics`
/// represents a field carrying `#[doc(hidden)]` or `#[cfg_attr(alef,
/// alef(skip))]` in the real struct (a genuine `binding_excluded` field) —
/// NOT `#[serde(skip)]`, which alone does not exclude a field from the
/// binding surface. Even though it is listed in `result_fields` (a stale/
/// wrong config entry), the IR must still win and reject it. ~keep
#[test]
fn java_ir_excluded_field_present_in_result_fields_is_still_skipped() {
    let result_fields: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
    let excluded: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_fields(HashSet::new(), excluded, HashSet::new());
    let assertion = make_equals_assertion("internal_diagnostics", "hello");
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        &resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        true,
    );
    assert!(out.contains("skipped"), "got: {out}");
}

/// A plain `Option<String>` field should use `Objects::toString` in the
/// Java equals expression — NOT `.text()`. Guards against DAT path bleeding
/// into regular optional string fields.
#[test]
fn java_plain_optional_string_uses_objects_to_string() {
    let mut optional = HashSet::new();
    optional.insert("content".to_string());
    let resolver = make_resolver(optional, HashSet::new());
    let assertion = make_equals_assertion("content", "hello");
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        &resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        true,
    );
    assert!(
        out.contains("Objects::toString"),
        "plain optional string field must use Objects::toString; got: {out}"
    );
    assert!(
        !out.contains(".text()"),
        "plain optional string must NOT use .text(); got: {out}"
    );
}

/// A `display_as_text` field (e.g. `Option<AssistantContent>`) should use
/// `.map(v -> v.text()).orElse("")` so the Java assertion sees the textual
/// representation, not the class-name string from `Objects::toString`.
#[test]
fn java_display_as_text_optional_uses_text_accessor() {
    let mut optional = HashSet::new();
    optional.insert("content".to_string());
    let mut dat = HashSet::new();
    dat.insert("content".to_string());
    let resolver = make_resolver(optional, dat);
    let assertion = make_equals_assertion("content", "Hello, world!");
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        &resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        true,
    );
    assert!(
        out.contains(".map(v -> v.text()).orElse(\"\")"),
        "display_as_text field must use .map(v -> v.text()).orElse(\"\"); got: {out}"
    );
    assert!(
        !out.contains("Objects::toString"),
        "display_as_text field must NOT use Objects::toString; got: {out}"
    );
}

fn make_not_error_assertion() -> Assertion {
    Assertion {
        assertion_type: "not_error".to_string(),
        ..Default::default()
    }
}

/// Regression test for the not_error vacuous-test defect: `java/assertion.jinja`'s
/// if/elif chain has no `not_error` branch and no final `else`, so before this fix
/// a fixture whose only assertion was `not_error` rendered nothing at all — not
/// even a comment. Must emit a real `assertNotNull` instead.
#[test]
fn not_error_emits_a_real_assert_not_null_on_the_result() {
    let resolver = make_resolver(HashSet::new(), HashSet::new());
    let assertion = make_not_error_assertion();
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        &resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        true,
    );
    assert_eq!(out, "        assertNotNull(result, \"expected non-null response\");\n");
}

#[test]
fn not_error_on_a_streaming_fixture_asserts_on_drained_chunks_not_result() {
    let resolver = make_resolver(HashSet::new(), HashSet::new());
    let assertion = make_not_error_assertion();
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        &resolver,
        false,
        false,
        false,
        true,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        true,
    );
    assert_eq!(
        out,
        "        assertNotNull(chunks, \"expected drained chunks list\");\n"
    );
}

/// A `returns_void` call binds no `result_var` at all (see
/// `java/test_method.jinja`'s `{% if returns_void %}` branch) — asserting on it
/// would not compile. The real assertion for this case lives one level up: see
/// `test_method.rs`'s `void_not_error_call_wraps_call_expr_in_assert_does_not_throw`,
/// which wraps `call_expr` in `assertDoesNotThrow` at the call-emission site instead.
#[test]
fn not_error_on_a_returns_void_call_emits_nothing() {
    let resolver = make_resolver(HashSet::new(), HashSet::new());
    let assertion = make_not_error_assertion();
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        &resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        true,
        &HashSet::new(),
        true,
    );
    assert!(
        out.is_empty(),
        "a returns_void call must not reference an unbound result_var, got: {out}"
    );
}

fn make_range_assertion(assertion_type: &str, field: &str, value: f64) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: Some(field.to_string()),
        value: serde_json::Number::from_f64(value).map(serde_json::Value::Number),
        ..Default::default()
    }
}

/// Regression test for the `qualityScore` range-assertion defect: an
/// `Optional<Double>` field's range comparators must NOT coerce through
/// `Number::longValue()` — that truncates every legal fractional value to `0L`
/// before the comparison runs, so a `[0.0, 1.0]` range check on a `Double` becomes
/// a tautology that can never fail. With the field registered in
/// `fractional_fields`, the emitted comparison must use `Number::doubleValue()`
/// instead, so it can actually observe (and fail on) an out-of-range value. ~keep
#[test]
fn fractional_optional_field_range_assertion_uses_double_value_not_long_value() {
    let mut optional = HashSet::new();
    optional.insert("quality_score".to_string());
    let resolver = make_resolver(optional, HashSet::new());
    let fractional: HashSet<String> = ["quality_score".to_string()].into_iter().collect();
    let assertion = make_range_assertion("greater_than_or_equal", "quality_score", 0.0);
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        &resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &fractional,
        true,
    );
    assert!(
        out.contains("Number::doubleValue"),
        "fractional Optional field must coerce via Number::doubleValue, got: {out}"
    );
    assert!(
        !out.contains("Number::longValue"),
        "fractional Optional field must NOT truncate via Number::longValue, got: {out}"
    );
}

/// Negative control: an integer `Optional` field (e.g. `sheetCount`, correctly
/// handled at `SmokeTest.java:149`) is absent from `fractional_fields` and must
/// keep using `Number::longValue()` — the fractional-type fix must not regress
/// the already-correct integer path.
#[test]
fn integer_optional_field_range_assertion_still_uses_long_value() {
    let mut optional = HashSet::new();
    optional.insert("sheet_count".to_string());
    let resolver = make_resolver(optional, HashSet::new());
    let assertion = make_range_assertion("greater_than_or_equal", "sheet_count", 1.0);
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        &resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        true,
    );
    assert!(
        out.contains("Number::longValue"),
        "integer Optional field must keep Number::longValue, got: {out}"
    );
    assert!(
        !out.contains("Number::doubleValue"),
        "integer Optional field must not use Number::doubleValue, got: {out}"
    );
}

/// `fractional_scalar_fields` must recognize `f64`/`f32` fields, including
/// through `Option<T>`, and must NOT flag integer fields.
#[test]
fn fractional_scalar_fields_detects_float_types_through_optional() {
    use crate::core::ir::{FieldDef, PrimitiveType, TypeDef, TypeRef};

    let type_defs = vec![TypeDef {
        name: "SampleResult".to_string(),
        fields: vec![
            FieldDef {
                name: "quality_score".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::F64))),
                ..Default::default()
            },
            FieldDef {
                name: "ratio".to_string(),
                ty: TypeRef::Primitive(PrimitiveType::F32),
                ..Default::default()
            },
            FieldDef {
                name: "sheet_count".to_string(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::U32))),
                ..Default::default()
            },
        ],
        ..Default::default()
    }];

    let fractional = fractional_scalar_fields(&type_defs);
    assert!(fractional.contains("quality_score"), "got: {fractional:?}");
    assert!(fractional.contains("ratio"), "got: {fractional:?}");
    assert!(
        !fractional.contains("sheet_count"),
        "integer field must not be classified as fractional, got: {fractional:?}"
    );
}
