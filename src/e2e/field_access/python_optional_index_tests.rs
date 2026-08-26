//! An `Optional` member subscripted with no narrowing at all is a `reportOptionalSubscript`
//! against the very stub alef generated: `tool_calls: list[ToolCall] | None` declared, and the
//! plain `render_dot_access` fallback Python fell through to (every other `_with_optionals`
//! renderer in this module tree consulted `optional_fields`; Python's catch-all did not) emitted
//! `result.choices[0].message.tool_calls[0].function.name` unguarded.

use super::FieldResolver;
use std::collections::{HashMap, HashSet};

fn resolver_with_optional(field: &str) -> FieldResolver {
    let optional: HashSet<String> = [field.to_string()].into_iter().collect();
    FieldResolver::new(
        &HashMap::new(),
        &optional,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

/// REPRODUCTION: an `Optional` collection field indexed further down the path must be narrowed
/// before the subscript, not indexed bare.
#[test]
fn indexing_past_an_optional_field_narrows_it_first() {
    let resolver = resolver_with_optional("choices[0].message.tool_calls");

    let rendered = resolver.accessor("choices[0].message.tool_calls[0].function.name", "python", "result");

    assert_eq!(
        rendered,
        "(result.choices[0].message.tool_calls[0].function.name if result.choices[0].message.tool_calls else None)",
        "the optional collection must be narrowed before it is subscripted"
    );
}

/// CONTROL: a member nothing declares optional must still be indexed directly — no ternary, no
/// narrowing, matching the pre-existing behavior for the common case.
#[test]
fn indexing_a_non_optional_field_is_unguarded() {
    let resolver = FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    assert_eq!(
        resolver.accessor("choices[0].message.tool_calls[0].function.name", "python", "result"),
        "result.choices[0].message.tool_calls[0].function.name"
    );
}

/// An `Optional` field with nothing reading further past it needs no guard at all — the rendered
/// value stays `Optional`, which is correct for a leaf that is only ever printed or assigned, not
/// dereferenced further.
#[test]
fn a_terminal_optional_field_is_left_unguarded() {
    let resolver = resolver_with_optional("summary");

    assert_eq!(resolver.accessor("summary", "python", "result"), "result.summary");
}
