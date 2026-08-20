//! Regression tests for a whitespace-control regression in `csharp/assertion.jinja`.
//!
//! `-%}`/`-#}` (trailing dash) whitespace-trimming tags in the template stripped the
//! leading indentation off the *first* line of the branch that followed them — not just
//! the redundant single newline `trim_blocks` already removes. With `trim_blocks` and
//! `lstrip_blocks` both enabled, a plain (non-dashed) tag already strips exactly the one
//! newline that separates the tag from its branch content; the extra `-` greedily eats
//! all whitespace up to the next non-whitespace character, which — when that next
//! character starts an indented statement — deletes the statement's own indent. In real
//! generated output this collapsed `Assert.Equal(...)` and `Assert.True(...) switch { ...
//! }` onto a 4-space indent while their sibling `Assert.NotNull(...)` (emitted directly
//! from Rust, not through this template) stayed at the correct 8-space indent. These
//! tests pin the corrected indentation for both a single-line assertion and a multi-line
//! (`switch` expression) assertion.

use super::render_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &std::collections::HashMap::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    )
}

/// A single-line assertion (`equals`) must keep the same 8-space indent as every other
/// statement `render_assertion` emits — not the 0-space indent the `-%}` regression
/// produced.
#[test]
fn an_equals_assertion_keeps_its_statement_indentation() {
    let resolver = empty_resolver();
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("content".to_string()),
        value: Some(serde_json::Value::String("hello".to_string())),
        ..Assertion::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        "SampleException",
        &resolver,
        false,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
    );
    assert_eq!(out, "        Assert.Equal(\"hello\", result.Content);\n");
}

/// A multi-line assertion (`not_empty` on a non-JSON-serialized field renders a `switch`
/// expression spanning several lines) must keep its *opening* line at the same 8-space
/// indent as a single-line assertion, with the `switch` body indented relative to it —
/// not the 0-space indent the regression produced on the opening line while its own
/// continuation lines (copied verbatim from the template) stayed at their original
/// depth, leaving the statement internally inconsistent with itself.
#[test]
fn a_multiline_not_empty_switch_assertion_keeps_its_opening_line_indentation() {
    let resolver = empty_resolver();
    let assertion = Assertion {
        assertion_type: "not_empty".to_string(),
        field: Some("content".to_string()),
        ..Assertion::default()
    };
    let mut out = String::new();
    render_assertion(
        &mut out,
        &assertion,
        "result",
        "SampleClass",
        "SampleException",
        &resolver,
        false,
        false,
        false,
        false,
        &std::collections::HashMap::new(),
    );
    assert_eq!(
        out,
        "        Assert.True(((object?)result.Content) switch\n        {\n            null => false,\n            string text => text.Length > 0,\n            System.Collections.ICollection items => items.Count > 0,\n            _ => true,\n        }, \"expected non-empty value\");\n"
    );
}
