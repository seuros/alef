//! Rendering of handle-config values into Python expressions.
//!
//! `handle_nested_types` maps a JSON key to the Python class that key's value must be
//! constructed with. This module is the single authority on how that map is applied: both the
//! emitted test body and the import list run the same traversal, so they cannot disagree about
//! which pyclasses a fixture actually references.

use std::collections::{BTreeSet, HashMap, HashSet};

use heck::ToSnakeCase;

use crate::e2e::escape::escape_python;

use super::super::json::json_to_python_literal;

/// Milliseconds-to-seconds divisor for the `request_timeout` handle field, whose fixture value is
/// authored in milliseconds while the Python binding takes whole seconds. ~keep
const MILLIS_PER_SECOND: u64 = 1000;

/// Render one handle-config field value as a Python expression.
pub(crate) fn build_handle_kwarg_value(
    key: &str,
    value: &serde_json::Value,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &HashSet<String>,
) -> String {
    if key == "request_timeout"
        && let Some(millis) = value.as_u64()
    {
        return (millis / MILLIS_PER_SECOND).to_string();
    }
    let mut used_types = BTreeSet::new();
    render_value(key, value, handle_nested_types, handle_dict_types, &mut used_types)
}

/// Record every pyclass name the rendered form of `value` will reference.
///
/// Implemented by rendering and discarding the expression rather than by walking the map a second
/// time: an import list derived independently from the emitted body is exactly how a nested type
/// ends up constructed but never imported (`NameError` at collection time). ~keep
pub(crate) fn collect_used_nested_types(
    key: &str,
    value: &serde_json::Value,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &HashSet<String>,
    used_types: &mut BTreeSet<String>,
) {
    let _rendered = render_value(key, value, handle_nested_types, handle_dict_types, used_types);
}

/// Render `value`, which hangs off JSON key `key`, recording constructors used along the way.
///
/// The key is the only lookup into `handle_nested_types`, at every depth. A list has no key of its
/// own, so its elements inherit the list's key — that is what makes an object inside a typed list
/// get the same constructor a directly nested object gets. A key absent from the map renders as a
/// plain literal, so genuinely untyped dicts and lists are untouched. ~keep
fn render_value(
    key: &str,
    value: &serde_json::Value,
    handle_nested_types: &HashMap<String, String>,
    handle_dict_types: &HashSet<String>,
    used_types: &mut BTreeSet<String>,
) -> String {
    match value {
        serde_json::Value::Array(elements) => {
            let rendered: Vec<String> = elements
                .iter()
                .map(|element| render_value(key, element, handle_nested_types, handle_dict_types, used_types))
                .collect();
            format!("[{}]", rendered.join(", "))
        }
        serde_json::Value::Object(fields) => {
            // A `handle_dict_types` key takes its value as a dict, so it keeps the literal form —
            // except when the object is empty, where the zero-argument constructor is still the
            // right rendering. ~keep
            match handle_nested_types.get(key) {
                Some(type_name) if fields.is_empty() || !handle_dict_types.contains(key) => {
                    used_types.insert(type_name.clone());
                    let kwargs: Vec<String> = fields
                        .iter()
                        .map(|(field, field_value)| {
                            let rendered =
                                render_value(field, field_value, handle_nested_types, handle_dict_types, used_types);
                            format!("{}={rendered}", field.to_snake_case())
                        })
                        .collect();
                    format!("{type_name}({})", kwargs.join(", "))
                }
                _ => {
                    let entries: Vec<String> = fields
                        .iter()
                        .map(|(field, field_value)| {
                            let rendered =
                                render_value(field, field_value, handle_nested_types, handle_dict_types, used_types);
                            format!("\"{}\": {rendered}", escape_python(field))
                        })
                        .collect();
                    format!("{{{}}}", entries.join(", "))
                }
            }
        }
        scalar => json_to_python_literal(scalar),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested_types(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, type_name)| ((*key).to_string(), (*type_name).to_string()))
            .collect()
    }

    fn dict_types(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|key| (*key).to_string()).collect()
    }

    #[test]
    fn object_inside_typed_list_is_constructed_with_its_pyclass() {
        let value = serde_json::json!({"items": [{"deny": true}, {"deny": false}]});
        let rendered = build_handle_kwarg_value(
            "policy",
            &value,
            &nested_types(&[("policy", "Policy"), ("items", "ItemMatcher")]),
            &dict_types(&[]),
        );
        assert_eq!(
            rendered,
            "Policy(items=[ItemMatcher(deny=True), ItemMatcher(deny=False)])"
        );
    }

    #[test]
    fn untyped_list_elements_stay_bare_dicts() {
        let value = serde_json::json!({"items": [{"deny": true}]});
        let rendered = build_handle_kwarg_value(
            "policy",
            &value,
            &nested_types(&[("policy", "Policy")]),
            &dict_types(&[]),
        );
        assert_eq!(rendered, "Policy(items=[{\"deny\": True}])");
    }

    #[test]
    fn untyped_top_level_list_stays_a_bare_list_of_dicts() {
        let value = serde_json::json!([{"deny": true}, {"deny": false}]);
        let rendered = build_handle_kwarg_value(
            "rules",
            &value,
            &nested_types(&[("policy", "Policy")]),
            &dict_types(&[]),
        );
        assert_eq!(rendered, "[{\"deny\": True}, {\"deny\": False}]");
    }

    #[test]
    fn directly_nested_object_still_uses_keyword_arguments() {
        let value = serde_json::json!({"deny": true, "max_depth": 2});
        let rendered = build_handle_kwarg_value(
            "policy",
            &value,
            &nested_types(&[("policy", "Policy")]),
            &dict_types(&[]),
        );
        assert_eq!(rendered, "Policy(deny=True, max_depth=2)");
    }

    #[test]
    fn empty_nested_object_renders_zero_argument_constructor() {
        let value = serde_json::json!({});
        let rendered = build_handle_kwarg_value(
            "policy",
            &value,
            &nested_types(&[("policy", "Policy")]),
            &dict_types(&[]),
        );
        assert_eq!(rendered, "Policy()");
    }

    #[test]
    fn dict_typed_key_keeps_its_literal_dict_form() {
        let value = serde_json::json!({"kind": "basic", "user": "u"});
        let rendered = build_handle_kwarg_value(
            "auth",
            &value,
            &nested_types(&[("auth", "AuthConfig")]),
            &dict_types(&["auth"]),
        );
        assert_eq!(rendered, "{\"kind\": \"basic\", \"user\": \"u\"}");
    }

    #[test]
    fn typed_object_nested_inside_a_typed_object_is_constructed() {
        let value = serde_json::json!({"matcher": {"deny": true}});
        let rendered = build_handle_kwarg_value(
            "policy",
            &value,
            &nested_types(&[("policy", "Policy"), ("matcher", "ItemMatcher")]),
            &dict_types(&[]),
        );
        assert_eq!(rendered, "Policy(matcher=ItemMatcher(deny=True))");
    }

    #[test]
    fn typed_object_inside_an_untyped_dict_is_constructed() {
        let value = serde_json::json!({"extra": {"matcher": {"deny": true}}});
        let rendered = build_handle_kwarg_value(
            "policy",
            &value,
            &nested_types(&[("policy", "Policy"), ("matcher", "ItemMatcher")]),
            &dict_types(&[]),
        );
        assert_eq!(rendered, "Policy(extra={\"matcher\": ItemMatcher(deny=True)})");
    }

    #[test]
    fn nested_lists_construct_elements_at_every_level() {
        let value = serde_json::json!({"items": [[{"deny": true}]]});
        let rendered = build_handle_kwarg_value(
            "policy",
            &value,
            &nested_types(&[("policy", "Policy"), ("items", "ItemMatcher")]),
            &dict_types(&[]),
        );
        assert_eq!(rendered, "Policy(items=[[ItemMatcher(deny=True)]])");
    }

    #[test]
    fn camel_case_fields_are_snake_cased_on_the_constructor() {
        let value = serde_json::json!({"maxDepth": 3});
        let rendered = build_handle_kwarg_value(
            "policy",
            &value,
            &nested_types(&[("policy", "Policy")]),
            &dict_types(&[]),
        );
        assert_eq!(rendered, "Policy(max_depth=3)");
    }

    #[test]
    fn request_timeout_millis_are_converted_to_seconds() {
        let value = serde_json::json!(5000u64);
        let rendered = build_handle_kwarg_value("request_timeout", &value, &HashMap::new(), &HashSet::new());
        assert_eq!(rendered, "5");
    }

    #[test]
    fn collect_used_nested_types_reports_types_reached_through_a_list() {
        let value = serde_json::json!({"items": [{"deny": true}]});
        let mut used_types = BTreeSet::new();
        collect_used_nested_types(
            "policy",
            &value,
            &nested_types(&[("policy", "Policy"), ("items", "ItemMatcher")]),
            &dict_types(&[]),
            &mut used_types,
        );
        assert_eq!(
            used_types,
            ["ItemMatcher".to_string(), "Policy".to_string()]
                .into_iter()
                .collect::<BTreeSet<String>>()
        );
    }

    #[test]
    fn collect_used_nested_types_reports_nothing_for_an_untyped_value() {
        let value = serde_json::json!({"items": [{"deny": true}]});
        let mut used_types = BTreeSet::new();
        collect_used_nested_types(
            "rules",
            &value,
            &nested_types(&[("policy", "Policy")]),
            &dict_types(&[]),
            &mut used_types,
        );
        assert!(used_types.is_empty(), "got: {used_types:?}");
    }
}
