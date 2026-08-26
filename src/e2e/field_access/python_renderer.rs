//! Python accessor rendering, narrowing an `Optional` member before anything further reads it.
//!
//! Python has no null-safe navigation operator (`?.`), so unlike the TypeScript/Kotlin/C#
//! `_with_optionals` renderers in `optional_renderers.rs`, a crossing cannot be spelled as a
//! single extra token on the link itself. Instead each crossing wraps the expression built so
//! far in a conditional expression (`tail if prefix else None`) that repeats the same raw prefix
//! text as both the condition and (as part of `tail`) the consequent — type checkers narrow a
//! member-access expression when it recurs unchanged across a ternary's condition and its
//! consequent, so this deliberately re-renders the prefix rather than binding it to a local name.
//!
//! `render_accessor`'s plain `render_dot_access` fallback answered nothing about optionality, so
//! a `list[T] | None` field reached a subscript with no guard at all
//! (`reportOptionalSubscript`/`reportOptionalMemberAccess`) — this is the Python counterpart to
//! the fix `csharp_optional_index_tests.rs` already covers for C#'s null-forgiving operator.

use super::optional_renderers::{push_key_field_name, push_key_index_suffix};
use super::renderers::render_dot_access;
use super::types::PathSegment;
use std::collections::HashSet;

/// Render a Python accessor expression, wrapping every point where the chain crosses an
/// `Optional` field (per `optional_fields`) in a narrowing conditional expression.
///
/// A crossing at the last segment needs no guard: nothing further reads the value, so leaving it
/// `Optional` in the rendered expression is correct as-is. A crossing anywhere earlier gets its
/// own nested ternary, innermost (deepest into the path) first, so each guard's own condition is
/// always evaluated either at the very start of the path (safe by construction) or already inside
/// an enclosing guard's narrowed branch (safe because that guard ran first).
pub(super) fn render_python_with_optionals(
    segments: &[PathSegment],
    result_var: &str,
    optional_fields: &HashSet<String>,
) -> String {
    let last_index = segments.len().saturating_sub(1);
    let mut crossings: Vec<usize> = Vec::new();
    let mut path_so_far = String::new();
    for (index, segment) in segments.iter().enumerate() {
        push_key_field_name(&mut path_so_far, segment);
        if index != last_index && optional_fields.contains(&path_so_far) {
            crossings.push(index);
        }
        push_key_index_suffix(&mut path_so_far, segment);
    }

    let mut expression = render_dot_access(segments, result_var, "python");
    for &index in crossings.iter().rev() {
        let condition = render_dot_access(&field_only_prefix(segments, index), result_var, "python");
        expression = format!("({expression} if {condition} else None)");
    }
    expression
}

/// `segments[..=index]` with segment `index`'s own `[..]`/key suffix stripped — the "is this
/// collection present at all" question, asked before the index that would subscript a `None`.
fn field_only_prefix(segments: &[PathSegment], index: usize) -> Vec<PathSegment> {
    let mut prefix: Vec<PathSegment> = segments[..index].to_vec();
    prefix.push(match &segments[index] {
        PathSegment::ArrayField { name, .. } => PathSegment::Field(name.clone()),
        PathSegment::MapAccess { field, .. } => PathSegment::Field(field.clone()),
        other => other.clone(),
    });
    prefix
}
