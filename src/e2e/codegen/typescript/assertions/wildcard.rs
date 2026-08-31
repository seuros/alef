//! The `foo[].bar` wildcard assertion lowering: an `Array.prototype.some` quantifier over every
//! element rather than an index-0 lookup.
//!
//! Split out of `assertions.rs`, which is over the 1,000-line cap and may not grow.

use crate::e2e::fixture::Assertion;

use super::json_to_js;

/// Render the `foo[].bar` wildcard forms as an `Array.prototype.some` quantifier over
/// every element, rather than an index-0 lookup. The array expression is `??`-guarded
/// because an absent optional list is `undefined` and `.some` would throw on it.
pub(super) fn render_wildcard_assertion(
    out: &mut String,
    assertion: &Assertion,
    array_accessor: &str,
    elem_accessor: &str,
    field: &str,
) {
    let guarded = format!("({array_accessor} ?? [])");
    let some_expr = |js_val: &str| format!("{guarded}.some((e) => String({elem_accessor}).includes({js_val}))");
    match assertion.assertion_type.as_str() {
        "contains" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                out.push_str(&format!("    expect({}).toBe(true);\n", some_expr(&js_val)));
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let js_val = json_to_js(val);
                    out.push_str(&format!("    expect({}).toBe(true);\n", some_expr(&js_val)));
                }
            }
        }
        "not_contains" => {
            for expected in assertion.expected_values() {
                let js_val = json_to_js(expected);
                out.push_str(&format!("    expect({}).toBe(false);\n", some_expr(&js_val)));
            }
        }
        "not_empty" => {
            out.push_str(&format!(
                "    expect({guarded}.some((e) => String({elem_accessor}).length > 0)).toBe(true);\n"
            ));
        }
        other => {
            out.push_str(&format!(
                "    // skipped: unsupported traversal assertion '{other}' on '{field}'\n"
            ));
        }
    }
}
