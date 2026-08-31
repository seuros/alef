//! Numeric-comparison assertion families (`greater_than`, `less_than`,
//! `greater_than_or_equal`, `less_than_or_equal`) for the Go e2e generator.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

use super::super::assertion_render_helpers::render_guarded_scalar_comparison;
use super::super::json_values::json_to_go;
use super::target::ResolvedAssertionTarget;

pub(super) fn render_greater_than(out_ref: &mut String, assertion: &Assertion, target: &ResolvedAssertionTarget) {
    let field_expr = target.field_expr.clone();
    let deref_field_expr = target.deref_field_expr.clone();
    let field_is_nullable = target.field_is_nullable;
    let nil_guard_expr = target.nil_guard_expr.as_deref();

    if let Some(val) = &assertion.value {
        let go_val = json_to_go(val);
        let (operator, comparison) = val
            .as_u64()
            .map(|value| ("<", (value + 1).to_string()))
            .unwrap_or_else(|| ("<=", go_val.clone()));
        if render_guarded_scalar_comparison(
            out_ref,
            nil_guard_expr,
            &field_expr,
            operator,
            &comparison,
            &format!("> {go_val}"),
        ) {
        } else if field_is_nullable {
            let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
            if let Some(n) = val.as_u64() {
                let next = n + 1;
                let _ = writeln!(out_ref, "\t\tif {deref_field_expr} < {next} {{");
            } else {
                let _ = writeln!(out_ref, "\t\tif {deref_field_expr} <= {go_val} {{");
            }
            let _ = writeln!(
                out_ref,
                "\t\t\tt.Errorf(\"expected > {go_val}, got %v\", {deref_field_expr})"
            );
            let _ = writeln!(out_ref, "\t\t}}");
            let _ = writeln!(out_ref, "\t}}");
        } else if let Some(n) = val.as_u64() {
            let next = n + 1;
            let _ = writeln!(out_ref, "\tif {field_expr} < {next} {{");
            let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected > {go_val}, got %v\", {field_expr})");
            let _ = writeln!(out_ref, "\t}}");
        } else {
            let _ = writeln!(out_ref, "\tif {field_expr} <= {go_val} {{");
            let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected > {go_val}, got %v\", {field_expr})");
            let _ = writeln!(out_ref, "\t}}");
        }
    }
}

pub(super) fn render_less_than(out_ref: &mut String, assertion: &Assertion, target: &ResolvedAssertionTarget) {
    let field_expr = target.field_expr.clone();
    let deref_field_expr = target.deref_field_expr.clone();
    let field_is_nullable = target.field_is_nullable;
    let nil_guard_expr = target.nil_guard_expr.as_deref();

    if let Some(val) = &assertion.value {
        let go_val = json_to_go(val);
        if render_guarded_scalar_comparison(
            out_ref,
            nil_guard_expr,
            &field_expr,
            ">=",
            &go_val,
            &format!("< {go_val}"),
        ) {
        } else if field_is_nullable && !field_expr.starts_with("len(") {
            let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
            let _ = writeln!(out_ref, "\t\tif {deref_field_expr} >= {go_val} {{");
            let _ = writeln!(
                out_ref,
                "\t\t\tt.Errorf(\"expected < {go_val}, got %v\", {deref_field_expr})"
            );
            let _ = writeln!(out_ref, "\t\t}}");
            let _ = writeln!(out_ref, "\t}}");
        } else {
            let _ = writeln!(out_ref, "\tif {field_expr} >= {go_val} {{");
            let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected < {go_val}, got %v\", {field_expr})");
            let _ = writeln!(out_ref, "\t}}");
        }
    }
}

pub(super) fn render_greater_than_or_equal(
    out_ref: &mut String,
    assertion: &Assertion,
    target: &ResolvedAssertionTarget,
) {
    let field_expr = target.field_expr.clone();
    let deref_field_expr = target.deref_field_expr.clone();
    let field_is_nullable = target.field_is_nullable;
    let nil_guard_expr = target.nil_guard_expr.as_deref();

    if let Some(val) = &assertion.value {
        let go_val = json_to_go(val);
        if render_guarded_scalar_comparison(
            out_ref,
            nil_guard_expr,
            &field_expr,
            "<",
            &go_val,
            &format!(">= {go_val}"),
        ) {
        } else if field_is_nullable && !field_expr.starts_with("len(") {
            let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
            let _ = writeln!(out_ref, "\t\tif {deref_field_expr} < {go_val} {{");
            let _ = writeln!(
                out_ref,
                "\t\t\tt.Errorf(\"expected >= {go_val}, got %v\", {deref_field_expr})"
            );
            let _ = writeln!(out_ref, "\t\t}}");
            let _ = writeln!(out_ref, "\t}}");
        } else {
            let _ = writeln!(out_ref, "\tif {field_expr} < {go_val} {{");
            let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected >= {go_val}, got %v\", {field_expr})");
            let _ = writeln!(out_ref, "\t}}");
        }
    }
}

pub(super) fn render_less_than_or_equal(out_ref: &mut String, assertion: &Assertion, target: &ResolvedAssertionTarget) {
    let field_expr = target.field_expr.clone();
    let deref_field_expr = target.deref_field_expr.clone();
    let field_is_nullable = target.field_is_nullable;
    let nil_guard_expr = target.nil_guard_expr.as_deref();

    if let Some(val) = &assertion.value {
        let go_val = json_to_go(val);
        if render_guarded_scalar_comparison(
            out_ref,
            nil_guard_expr,
            &field_expr,
            ">",
            &go_val,
            &format!("<= {go_val}"),
        ) {
        } else if field_is_nullable && !field_expr.starts_with("len(") {
            let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
            let _ = writeln!(out_ref, "\t\tif {deref_field_expr} > {go_val} {{");
            let _ = writeln!(
                out_ref,
                "\t\t\tt.Errorf(\"expected <= {go_val}, got %v\", {deref_field_expr})"
            );
            let _ = writeln!(out_ref, "\t\t}}");
            let _ = writeln!(out_ref, "\t}}");
        } else {
            let _ = writeln!(out_ref, "\tif {field_expr} > {go_val} {{");
            let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected <= {go_val}, got %v\", {field_expr})");
            let _ = writeln!(out_ref, "\t}}");
        }
    }
}
