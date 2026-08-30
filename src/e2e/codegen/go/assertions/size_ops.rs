//! Size assertion families (`count_min`, `count_equals`, `min_length`, `max_length`) for
//! the Go e2e generator.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::fixture::Assertion;

use super::super::assertion_render_helpers::{render_count_assertion, render_length_assertion};
use super::target::ResolvedAssertionTarget;

pub(super) fn render_count_min(out_ref: &mut String, assertion: &Assertion, target: &ResolvedAssertionTarget) {
    let field_expr = target.field_expr.clone();
    let field_is_slice = target.field_is_slice;
    let nullable_guard_expr = target.nullable_guard_expr.as_deref();

    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        render_count_assertion(
            out_ref,
            &field_expr,
            n,
            nullable_guard_expr.as_deref(),
            field_is_slice,
            false,
        );
    }
}

pub(super) fn render_count_equals(out_ref: &mut String, assertion: &Assertion, target: &ResolvedAssertionTarget) {
    let field_expr = target.field_expr.clone();
    let field_is_slice = target.field_is_slice;
    let nullable_guard_expr = target.nullable_guard_expr.as_deref();

    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        render_count_assertion(
            out_ref,
            &field_expr,
            n,
            nullable_guard_expr.as_deref(),
            field_is_slice,
            true,
        );
    }
}

pub(super) fn render_min_length(out_ref: &mut String, assertion: &Assertion, target: &ResolvedAssertionTarget) {
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let nullable_guard_expr = target.nullable_guard_expr.as_deref();

    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        render_length_assertion(
            out_ref,
            &field_expr,
            n,
            nullable_guard_expr.as_deref(),
            field_is_pointer,
            true,
        );
    }
}

pub(super) fn render_max_length(out_ref: &mut String, assertion: &Assertion, target: &ResolvedAssertionTarget) {
    let field_expr = target.field_expr.clone();
    let field_is_pointer = target.field_is_pointer;
    let nullable_guard_expr = target.nullable_guard_expr.as_deref();

    if let Some(val) = &assertion.value
        && let Some(n) = val.as_u64()
    {
        render_length_assertion(
            out_ref,
            &field_expr,
            n,
            nullable_guard_expr.as_deref(),
            field_is_pointer,
            false,
        );
    }
}
