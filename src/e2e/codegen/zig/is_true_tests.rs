//! Regression tests for `is_true`/`is_false` on an `Option<T>` field, in both zig
//! assertion modes (`result_is_json_struct = true` and the typed-struct path).
//!
//! Split into its own file rather than added to `zig/assertions.rs`: that file is already
//! over the repo's 1,000-line cap (see `file-modularization` in CLAUDE.md), so new test
//! coverage goes into a fresh module instead of growing it. ~keep

use super::assertions::{render_assertion, render_json_assertion};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

fn optional_resolver(field: &str) -> FieldResolver {
    let optional: HashSet<String> = [field.to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn is_true_assertion(field: &str) -> Assertion {
    Assertion {
        assertion_type: "is_true".to_string(),
        field: Some(field.to_string()),
        ..Assertion::default()
    }
}

fn render_json(resolver: &FieldResolver, assertion: &Assertion) -> String {
    let mut out = String::new();
    render_json_assertion(&mut out, assertion, "result", resolver, false);
    out
}

/// `result_is_json_struct = true` mode: `field_expr.bool` unconditionally accesses the
/// `std.json.Value` union's bool variant, which is an "access of inactive union field"
/// panic at runtime when the value at this path is actually a JSON object (`Option
/// <DataNode>`'s wire shape). `!= .null` holds for any JSON value shape.
#[test]
fn json_struct_is_true_on_optional_struct_field_checks_presence() {
    let out = render_json(&optional_resolver("data"), &is_true_assertion("data"));
    assert_eq!(out, "    try testing.expect(result.object.get(\"data\").? != .null);\n");
}

#[test]
fn json_struct_is_false_on_optional_struct_field_checks_absence() {
    let out = render_json(
        &optional_resolver("data"),
        &Assertion {
            assertion_type: "is_false".to_string(),
            field: Some("data".to_string()),
            ..Assertion::default()
        },
    );
    assert_eq!(out, "    try testing.expect(result.object.get(\"data\").? == .null);\n");
}

#[test]
fn json_struct_is_true_on_non_optional_field_is_unchanged() {
    let out = render_json(&empty_resolver(), &is_true_assertion("active"));
    assert_eq!(out, "    try testing.expect(result.object.get(\"active\").?.bool);\n");
}

fn render_typed(resolver: &FieldResolver, assertion: &Assertion) -> String {
    let mut out = String::new();
    render_assertion(&mut out, assertion, "result", resolver, &HashSet::new(), false, false);
    out
}

/// Typed-struct mode: the accessor already force-unwraps optional segments with `.?`
/// (`render_zig_with_optionals`), so `is_true` on `data` alone previously emitted
/// `try testing.expect(result.data.?);` -- a runtime panic on `null` before the value is
/// even compared, and even past that a struct does not satisfy `testing.expect`'s `bool`
/// parameter. Stripping the forced `.?` and comparing `!= null` holds for any T.
#[test]
fn typed_is_true_on_optional_struct_field_checks_presence() {
    let out = render_typed(&optional_resolver("data"), &is_true_assertion("data"));
    assert_eq!(out, "    try testing.expect(result.data != null);\n");
}

#[test]
fn typed_is_true_on_non_optional_field_is_unchanged() {
    let out = render_typed(&empty_resolver(), &is_true_assertion("active"));
    assert_eq!(out, "    try testing.expect(result.active);\n");
}
