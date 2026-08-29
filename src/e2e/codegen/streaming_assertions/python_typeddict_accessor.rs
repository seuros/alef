//! Python `TypedDict`-vs-attribute-access awareness for the streaming accessors module.
//!
//! `streaming_assertions::accessors` builds its expressions as hand-rolled `format!` strings per
//! field, not through `field_access::FieldResolver`/`PathSegment` — see that module's doc for why.
//! This gives the Python arms of those hand-rolled expressions the same [`PythonTypedDictMap`]
//! classification `field_access::python_renderer` already applies to ordinary (non-streaming)
//! field paths, without rebuilding a second classifier: the map itself is built once, by
//! `field_access::python_typeddict::build_python_typeddict_map`, from the pyo3 backend's own
//! predicate, and handed in here by the caller. ~keep

use crate::e2e::field_access::PythonTypedDictMap;

use super::renderers::{TailSeg, parse_tail};

/// Append one Python field access to `owner_type`, choosing subscript (`["field"]`) when
/// `owner_type` is classified `TypedDict` in `map`, or attribute (`.field`) otherwise — the exact
/// per-segment rule `field_access::python_renderer::push_field_access` applies to ordinary field
/// paths. Returns the field's own IR-resolved next type (if `map` knows one), so the caller can
/// advance its own cursor for a further hop.
///
/// `map: None` renders `.field` unconditionally and returns `None` for the next type — the
/// pre-existing, unconditional dotted-access behaviour every streaming accessor had before this
/// map existed. ~keep
pub(super) fn python_field_access(
    field: &str,
    owner_type: Option<&str>,
    map: Option<&PythonTypedDictMap>,
) -> (String, Option<String>) {
    match map {
        Some(m) if m.is_typeddict(owner_type) => (format!("[{}]", quote(field)), m.advance(owner_type, field)),
        Some(m) => (format!(".{field}"), m.advance(owner_type, field)),
        None => (format!(".{field}"), None),
    }
}

/// Double-quote a Python dict-subscript literal. Streaming accessor field names are always plain
/// Rust field identifiers (snake_case, no quotes or control characters), so a bare wrap is safe —
/// unlike `field_access::renderers::quoted_key_literal`, which additionally escapes user-facing
/// fixture text this module never handles.
fn quote(field: &str) -> String {
    format!("\"{field}\"")
}

/// The IR type `tool_calls`'s element resolves to, walking `item_type --choices--> --delta-->
/// --tool_calls-->` — exactly the same chain the `"tool_calls"` accessor arm itself builds.
/// `None` whenever `map` is absent or any hop is unresolvable, in which case the deep-path
/// renderer falls back to plain dotted access, matching pre-existing behaviour.
pub(super) fn python_tool_call_element_type(
    item_type: Option<&str>,
    map: Option<&PythonTypedDictMap>,
) -> Option<String> {
    let map = map?;
    let choice_type = map.advance(item_type, "choices")?;
    let delta_type = map.advance(Some(choice_type.as_str()), "delta")?;
    map.advance(Some(delta_type.as_str()), "tool_calls")
}

/// Render a Python deep accessor for `tool_calls[N]...` paths, choosing subscript vs. attribute
/// access at each hop per [`python_field_access`]. Mirrors `render_swift_tool_calls_deep`'s shape,
/// one level of indirection removed (`TypedDict`/attribute instead of first-class/opaque).
pub(super) fn render_python_tool_calls_deep(
    root_expr: &str,
    tail: &str,
    tool_call_type: Option<&str>,
    map: Option<&PythonTypedDictMap>,
) -> String {
    let segs = parse_tail(tail);
    let mut expr = root_expr.to_string();
    let mut current_type = tool_call_type.map(str::to_string);
    for seg in &segs {
        match seg {
            TailSeg::Index(n) => {
                expr = format!("({expr})[{n}]");
            }
            TailSeg::Field(f) => {
                let (access, next_type) = python_field_access(f, current_type.as_deref(), map);
                expr.push_str(&access);
                current_type = next_type;
            }
        }
    }
    expr
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typeddict_map(typeddict_types: &[&str], field_types: &[(&str, &str, &str)]) -> PythonTypedDictMap {
        let mut map = PythonTypedDictMap {
            typeddict_types: typeddict_types.iter().map(|s| s.to_string()).collect(),
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

    #[test]
    fn a_field_on_a_typeddict_owner_subscripts_and_advances() {
        let map = typeddict_map(&["Chunk"], &[("Chunk", "choices", "Choice")]);
        let (access, next) = python_field_access("choices", Some("Chunk"), Some(&map));
        assert_eq!(access, r#"["choices"]"#);
        assert_eq!(next, Some("Choice".to_string()));
    }

    /// CONTROL: the identical field on a non-`TypedDict` owner keeps dotted access.
    #[test]
    fn a_field_on_a_non_typeddict_owner_stays_dotted() {
        let map = typeddict_map(&[], &[("Chunk", "choices", "Choice")]);
        let (access, next) = python_field_access("choices", Some("Chunk"), Some(&map));
        assert_eq!(access, ".choices");
        assert_eq!(next, Some("Choice".to_string()));
    }

    /// CONTROL: no map at all (every pre-existing caller) always renders dotted access and
    /// reports no next type.
    #[test]
    fn a_field_with_no_map_stays_dotted_and_untyped() {
        let (access, next) = python_field_access("choices", Some("Chunk"), None);
        assert_eq!(access, ".choices");
        assert_eq!(next, None);
    }

    #[test]
    fn tool_call_element_type_walks_the_full_choices_delta_tool_calls_chain() {
        let map = typeddict_map(
            &[],
            &[
                ("Chunk", "choices", "Choice"),
                ("Choice", "delta", "Delta"),
                ("Delta", "tool_calls", "ToolCall"),
            ],
        );
        assert_eq!(
            python_tool_call_element_type(Some("Chunk"), Some(&map)),
            Some("ToolCall".to_string())
        );
    }

    #[test]
    fn tool_call_element_type_is_none_without_a_map() {
        assert_eq!(python_tool_call_element_type(Some("Chunk"), None), None);
    }

    #[test]
    fn deep_tail_subscripts_a_typeddict_tool_call_element_then_keeps_walking() {
        let map = typeddict_map(&["ToolCall"], &[("ToolCall", "function", "FunctionCall")]);
        let expr = render_python_tool_calls_deep("root", "[0].function.name", Some("ToolCall"), Some(&map));
        assert_eq!(expr, r#"(root)[0]["function"].name"#);
    }

    /// CONTROL: pre-existing dotted behaviour when no `TypedDict` classification applies.
    #[test]
    fn deep_tail_stays_dotted_without_a_map() {
        let expr = render_python_tool_calls_deep("root", "[0].function.name", Some("ToolCall"), None);
        assert_eq!(expr, "(root)[0].function.name");
    }
}
