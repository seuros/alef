//! `method_result` assertion rendering for the Go e2e generator.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

use super::super::json_values::json_to_go;
use super::super::method_calls::build_go_method_call;
use super::AssertionRenderContext;

/// Render a `method_result` assertion, which calls a method on the result value
/// directly (unlike every other assertion type, which resolves a field accessor via
/// `ResolvedAssertionTarget`) -- so this takes the render context but no target.
pub(super) fn render_method_result(out_ref: &mut String, context: &AssertionRenderContext<'_>, assertion: &Assertion) {
    let Some(method_name) = &assertion.method else {
        panic!("Go e2e generator: method_result assertion missing 'method' field");
    };
    let info = build_go_method_call(context.result_var, method_name, assertion.args.as_ref(), context.import_alias);
    let check = assertion.check.as_deref().unwrap_or("is_true");
    let deref_expr = if info.is_pointer {
        format!("*{}", info.call_expr)
    } else {
        info.call_expr.clone()
    };
    match check {
        "equals" => render_method_result_equals(out_ref, assertion, &deref_expr, info.value_cast),
        "is_true" => {
            let _ = writeln!(out_ref, "\tassert.True(t, {deref_expr}, \"expected true\")");
        }
        "is_false" => {
            let _ = writeln!(out_ref, "\tassert.False(t, {deref_expr}, \"expected false\")");
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let n = val.as_u64().unwrap_or(0);
                let cast = info.value_cast.unwrap_or("uint");
                let _ = writeln!(
                    out_ref,
                    "\tassert.GreaterOrEqual(t, {deref_expr}, {cast}({n}), \"expected >= {n}\")"
                );
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value {
                let n = val.as_u64().unwrap_or(0);
                let _ = writeln!(
                    out_ref,
                    "\tassert.GreaterOrEqual(t, len({deref_expr}), {n}, \"expected at least {n} elements\")"
                );
            }
        }
        "contains" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                let _ = writeln!(
                    out_ref,
                    "\tassert.Contains(t, {deref_expr}, {go_val}, \"expected result to contain value\")"
                );
            }
        }
        "is_error" => {
            let _ = writeln!(out_ref, "\t{{");
            let _ = writeln!(out_ref, "\t\t_, methodErr := {}", info.call_expr);
            let _ = writeln!(out_ref, "\t\tassert.Error(t, methodErr)");
            let _ = writeln!(out_ref, "\t}}");
        }
        other_check => {
            panic!("Go e2e generator: unsupported method_result check type: {other_check}");
        }
    }
}

/// The `"equals"` check sub-case of [`render_method_result`], split out because it alone
/// carries the boolean-vs-numeric-vs-string value-narrowing logic.
fn render_method_result_equals(
    out_ref: &mut String,
    assertion: &Assertion,
    deref_expr: &str,
    value_cast: Option<&str>,
) {
    if let Some(val) = &assertion.value {
        if val.is_boolean() {
            if val.as_bool() == Some(true) {
                let _ = writeln!(out_ref, "\tassert.True(t, {deref_expr}, \"expected true\")");
            } else {
                let _ = writeln!(out_ref, "\tassert.False(t, {deref_expr}, \"expected false\")");
            }
        } else {
            let go_val = if let Some(cast) = value_cast {
                if val.is_number() {
                    format!("{cast}({})", json_to_go(val))
                } else {
                    json_to_go(val)
                }
            } else {
                json_to_go(val)
            };
            let _ = writeln!(
                out_ref,
                "\tassert.Equal(t, {go_val}, {deref_expr}, \"method_result equals assertion failed\")"
            );
        }
    }
}
