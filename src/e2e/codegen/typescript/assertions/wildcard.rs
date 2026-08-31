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
    match assertion.assertion_type.as_str() {
        "contains" => {
            if let Some(expected) = &assertion.value {
                push_quantifier(out, &guarded, elem_accessor, expected, true, field);
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    push_quantifier(out, &guarded, elem_accessor, val, true, field);
                }
            }
        }
        "not_contains" => {
            for expected in assertion.expected_values() {
                push_quantifier(out, &guarded, elem_accessor, expected, false, field);
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

/// Push one quantified element check, or — when the expected value admits no sound element
/// comparison — a visible skip instead of an assertion that would pass for the wrong reason.
fn push_quantifier(
    out: &mut String,
    guarded: &str,
    elem_accessor: &str,
    expected: &serde_json::Value,
    truth: bool,
    field: &str,
) {
    match element_predicate(elem_accessor, expected) {
        Some(predicate) => {
            out.push_str(&format!("    expect({guarded}.some((e) => {predicate})).toBe({truth});\n"));
        }
        None => out.push_str(&unlowerable_value_skip_line(field, expected)),
    }
}

/// The JS predicate comparing ONE wildcard element against the fixture's expected value.
///
/// ~keep `String(elem).includes(v)` is sound only for a STRING expectation, where substring
/// containment is what the fixture is asking for. Every non-string expectation used to take that
/// same route, and `String.prototype.includes` coerces its argument to a string: a numeric
/// `contains: 42` rendered `String(e.bar).includes(42)`, which is `"421".includes("42")` — TRUE —
/// for an element of 421, and true again for 3.142. The assertion passed against values that do
/// not contain 42 at all, and a false pass is counted as coverage. Executed under node against
/// `[{bar: 421}]` the old text reports the assertion as passing and this one as failing.
///
/// A numeric expectation is therefore compared numerically rather than textually. `Number(...)`
/// rather than a bare `===` because wasm-bindgen hands JavaScript a `BigInt` for a Rust
/// `u64`/`i64` leaf and `42n === 42` is false — the same coercion `assertions.rs` already applies
/// on the non-wildcard wasm path. The `!= null` guard is load-bearing rather than defensive:
/// `Number(null)` is `0`, so without it `contains: 0` would be satisfied by an absent leaf.
///
/// `None` means no sound comparison exists for this value shape — a null, array or object
/// expectation against a single element — and the caller must skip visibly rather than emit one.
fn element_predicate(elem_accessor: &str, expected: &serde_json::Value) -> Option<String> {
    match expected {
        serde_json::Value::String(_) => Some(format!(
            "String({elem_accessor}).includes({})",
            json_to_js(expected)
        )),
        serde_json::Value::Number(_) => Some(format!(
            "{elem_accessor} != null && Number({elem_accessor}) === {}",
            json_to_js(expected)
        )),
        serde_json::Value::Bool(_) => Some(format!("{elem_accessor} === {}", json_to_js(expected))),
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => None,
    }
}

/// ~keep Deliberately NOT a registered `field_skip::FieldSkip`: that funnel's own doc puts skips
/// whose cause is the assertion's shape rather than the field ("'<name>' assertion missing value")
/// explicitly out of scope, and the sibling arm of the same `match` already spells its
/// unsupported-assertion skip as plain prose. This stays in that one wording family rather than
/// opening a second, parallel one.
fn unlowerable_value_skip_line(field: &str, expected: &serde_json::Value) -> String {
    let value_kind = match expected {
        serde_json::Value::Null => "null",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
        serde_json::Value::String(_) | serde_json::Value::Number(_) | serde_json::Value::Bool(_) => "value",
    };
    format!("    // skipped: unsupported traversal assertion {value_kind} value on '{field}'\n")
}
