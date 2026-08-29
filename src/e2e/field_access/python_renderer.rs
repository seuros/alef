//! Python accessor rendering, narrowing an `Optional` member before anything further reads it,
//! and switching between attribute access (`result.field`) and subscript access
//! (`result["field"]`) at exactly the links the pyo3 backend emits as a `TypedDict`.
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
//!
//! Separately, `render_dot_access` answered nothing about `TypedDict` return types either: a
//! `[workspace.dto] python_output = "typed-dict"` crate's return type is a plain `dict` at
//! runtime (`TypedDict` is a type-checking fiction only), so `.field` on it is
//! `AttributeError: 'dict' object has no attribute 'field'`. [`render_python_accessor`] walks
//! `segments` with a "current owner type" cursor (mirroring `render_swift_with_first_class_map`)
//! and consults [`PythonTypedDictMap::is_typeddict`] at each link to pick `["field"]` vs.
//! `.field` — asking the pyo3 backend's own predicate for the answer rather than re-deriving it.

use super::optional_renderers::{push_key_field_name, push_key_index_suffix};
use super::renderers::quoted_key_literal;
use super::types::{PathSegment, PythonTypedDictMap};
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
    typeddict_map: &PythonTypedDictMap,
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

    let mut expression = render_python_accessor(segments, result_var, typeddict_map);
    for &index in crossings.iter().rev() {
        let condition = render_python_accessor(&field_only_prefix(segments, index), result_var, typeddict_map);
        expression = format!("({expression} if {condition} else None)");
    }
    expression
}

/// Render a Python accessor expression for `segments`, tracking the IR type that "owns" each
/// segment so a field whose owner the pyo3 backend emits as a `TypedDict` gets subscript access
/// (`result["field"]`) while every other owner (dataclass / pydantic / msgspec / native
/// `#[pyclass]` — all attribute-access shapes in Python) keeps `.field`.
///
/// `TypedDict`-ness is checked at the OWNER of each segment, not at the segment's target type —
/// mirroring `render_swift_with_first_class_map`'s per-segment dispatch — so a path that starts
/// on a `TypedDict` result and descends into a field whose own type is not itself emitted as a
/// `TypedDict` correctly switches back to attribute access at that link, and does not need a
/// special case: the cursor just stops finding `is_typeddict(current_type) == true` for it.
pub(super) fn render_python_accessor(segments: &[PathSegment], result_var: &str, map: &PythonTypedDictMap) -> String {
    let mut out = result_var.to_string();
    let mut current_type = map.root_type.clone();
    for seg in segments {
        match seg {
            PathSegment::Field(f) => {
                push_field_access(&mut out, f, current_type.as_deref(), map);
                current_type = map.advance(current_type.as_deref(), f);
            }
            PathSegment::ArrayField { name, index } => {
                push_field_access(&mut out, name, current_type.as_deref(), map);
                out.push_str(&format!("[{index}]"));
                current_type = map.advance(current_type.as_deref(), name);
            }
            PathSegment::MapAccess { field, key } => {
                push_field_access(&mut out, field, current_type.as_deref(), map);
                // The map VALUE is a plain `dict` regardless of the owning struct's DTO style,
                // so the trailing key/index suffix is unaffected by `TypedDict`-ness — this
                // mirrors `render_dot_access`'s MapAccess handling exactly. The owner cursor
                // does not advance through a MapAccess segment either, matching `ir_enum`/
                // `ir_collection`, which never populate a `field_types` edge for a Map-typed
                // field.
                if key.chars().all(|c| c.is_ascii_digit()) {
                    let idx: usize = key.parse().unwrap_or(0);
                    out.push_str(&format!("[{idx}]"));
                } else {
                    out.push_str(&format!(".get({})", quoted_key_literal(key)));
                }
            }
            PathSegment::Length => {
                let current = std::mem::take(&mut out);
                out = format!("len({current})");
            }
        }
    }
    out
}

/// Append one field access (`.field` or `["field"]`) to `out`, per whether `owner_type` is
/// classified as a `TypedDict` in `map`.
fn push_field_access(out: &mut String, field: &str, owner_type: Option<&str>, map: &PythonTypedDictMap) {
    if map.is_typeddict(owner_type) {
        out.push_str(&format!("[{}]", quoted_key_literal(field)));
    } else {
        out.push('.');
        out.push_str(field);
    }
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

#[cfg(test)]
mod tests {
    use super::super::parse::parse_path;
    use super::*;

    fn typeddict_map(
        typeddict_types: &[&str],
        field_types: &[(&str, &str, &str)],
        root_type: &str,
    ) -> PythonTypedDictMap {
        let mut map = PythonTypedDictMap {
            typeddict_types: typeddict_types.iter().map(|s| s.to_string()).collect(),
            root_type: Some(root_type.to_string()),
            ..Default::default()
        };
        for (owner, field, target) in field_types {
            map.field_types
                .entry(owner.to_string())
                .or_default()
                .insert(field.to_string(), target.to_string());
        }
        map
    }

    /// A scalar field on a `TypedDict` result renders as a subscript, not an attribute — the
    /// exact defect reported against a consumer: `result.status_code` raised `AttributeError:
    /// 'dict' object has no attribute 'status_code'` because the result is a plain `dict` at
    /// runtime, not the (type-checking-only) `TypedDict` its annotation claims.
    #[test]
    fn a_scalar_field_on_a_typeddict_result_is_subscripted() {
        let map = typeddict_map(&["ApiResult"], &[], "ApiResult");
        let segments = parse_path("status_code");
        assert_eq!(
            render_python_accessor(&segments, "result", &map),
            r#"result["status_code"]"#
        );
    }

    /// CONTROL: the identical field on a result type NOT classified as `TypedDict` keeps plain
    /// attribute access — proving the new behaviour is conditional on the map, not blanket.
    #[test]
    fn a_scalar_field_on_a_non_typeddict_result_stays_attribute_access() {
        let map = PythonTypedDictMap::default();
        let segments = parse_path("status_code");
        assert_eq!(render_python_accessor(&segments, "result", &map), "result.status_code");
    }

    /// A `TypedDict` result with an `Optional` field: the narrowing ternary's condition AND
    /// consequent both use subscript access on the `TypedDict` owner, matching the shape
    /// `render_python_with_optionals` already produces for attribute access.
    #[test]
    fn a_typeddict_result_with_an_optional_field_narrows_via_subscript() {
        let map = typeddict_map(&["ApiResult"], &[], "ApiResult");
        let optional: HashSet<String> = ["markdown".to_string()].into_iter().collect();
        let segments = parse_path("markdown");
        assert_eq!(
            render_python_with_optionals(&segments, "result", &optional, &map),
            r#"result["markdown"]"#,
            "a crossing at the LAST segment needs no ternary guard"
        );
    }

    /// A `TypedDict` result descending through an `Optional` `TypedDict` field into a further
    /// scalar: the crossing's ternary condition subscripts the intermediate field, and the full
    /// expression continues subscripting past it.
    #[test]
    fn a_typeddict_result_with_an_optional_nested_typeddict_field_narrows_before_descending() {
        let map = typeddict_map(
            &["ApiResult", "Markdown"],
            &[("ApiResult", "markdown", "Markdown")],
            "ApiResult",
        );
        let optional: HashSet<String> = ["markdown".to_string()].into_iter().collect();
        let segments = parse_path("markdown.content");
        assert_eq!(
            render_python_with_optionals(&segments, "result", &optional, &map),
            r#"(result["markdown"]["content"] if result["markdown"] else None)"#
        );
    }

    /// A `TypedDict` result descending into a field whose OWN type is not itself classified as
    /// `TypedDict` (e.g. it stays a native `#[pyclass]`) must switch back to attribute access at
    /// that link — the classification is checked per-segment-owner, not inherited from the root.
    #[test]
    fn descending_from_a_typeddict_into_a_non_typeddict_nested_type_switches_to_attribute_access() {
        let map = typeddict_map(&["ApiResult"], &[("ApiResult", "metadata", "Metadata")], "ApiResult");
        let segments = parse_path("metadata.title");
        assert_eq!(
            render_python_accessor(&segments, "result", &map),
            r#"result["metadata"].title"#
        );
    }

    /// An indexed array field on a `TypedDict` owner subscripts the field name, then indexes the
    /// resulting list with plain `[N]` — list indexing is unaffected by the owning struct's DTO
    /// style.
    #[test]
    fn an_array_field_on_a_typeddict_result_subscripts_the_field_then_indexes_the_list() {
        let map = typeddict_map(&["ApiResult"], &[], "ApiResult");
        let segments = parse_path("pages[0]");
        assert_eq!(
            render_python_accessor(&segments, "result", &map),
            r#"result["pages"][0]"#
        );
    }

    /// CONTROL (pre-existing behaviour, unaffected by the empty default map): indexing past an
    /// `Optional` collection field still narrows before subscripting.
    #[test]
    fn indexing_past_an_optional_field_narrows_it_first() {
        let map = PythonTypedDictMap::default();
        let optional: HashSet<String> = ["choices[0].message.tool_calls".to_string()].into_iter().collect();
        let segments = parse_path("choices[0].message.tool_calls[0].function.name");
        assert_eq!(
            render_python_with_optionals(&segments, "result", &optional, &map),
            "(result.choices[0].message.tool_calls[0].function.name if result.choices[0].message.tool_calls else None)"
        );
    }
}
