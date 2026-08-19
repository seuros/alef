//! Regression tests for a whitespace-control regression in `java/assertion.jinja`.
//!
//! Every branch in the template used both a leading (`{%-`) and a trailing (`-%}`)
//! whitespace-trimming dash. With `trim_blocks`/`lstrip_blocks` already enabled, a plain
//! (non-dashed) tag already strips exactly the one newline that separates a tag from its
//! branch content; the extra `-` greedily eats all whitespace up to the next
//! non-whitespace character, which deletes the branch's own leading indentation and, in a
//! `{% for %}` loop, glues every iteration onto a single line (no newline is left between
//! them). These tests pin the corrected indentation and inter-statement newlines for a
//! single-line assertion and a looped (`contains_all`) assertion.

use super::assertions::render_assertion;
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

#[allow(clippy::too_many_arguments)]
fn render(assertion: &Assertion) -> String {
    let resolver = empty_resolver();
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
        &std::collections::HashSet::new(),
        &std::collections::HashMap::new(),
        false,
        &std::collections::HashSet::new(),
    );
    out
}

/// A single-line assertion (`equals`) must keep the same 8-space indent as every other
/// statement `render_assertion` emits — not the 0-space indent the `-%}`/`{%-` regression
/// produced.
#[test]
fn an_equals_assertion_keeps_its_statement_indentation() {
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some("content".to_string()),
        value: Some(serde_json::Value::String("hello".to_string())),
        ..Assertion::default()
    };
    assert_eq!(
        render(&assertion),
        "        assertEquals(\"hello\", result.content());\n"
    );
}

/// A looped assertion (`contains_all` over multiple values) must emit one statement per
/// value, each on its own line at the same 8-space indent — not all of them glued onto a
/// single line with no separating newline, which the `{% for %}`/`{% endfor %}` pair's
/// dashes produced.
#[test]
fn a_contains_all_assertion_keeps_one_statement_per_line() {
    let assertion = Assertion {
        assertion_type: "contains_all".to_string(),
        field: Some("content".to_string()),
        values: Some(vec![
            serde_json::Value::String("alef".to_string()),
            serde_json::Value::String("binding".to_string()),
        ]),
        ..Assertion::default()
    };
    assert_eq!(
        render(&assertion),
        "        assertTrue(result.content().contains(\"alef\"), \"expected to contain: \" + \"alef\");\n        assertTrue(result.content().contains(\"binding\"), \"expected to contain: \" + \"binding\");\n"
    );
}
