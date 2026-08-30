//! JSON-to-JavaScript literal conversion utilities.

use crate::codegen::naming::underscore_camel_case;
use crate::e2e::escape::{escape_js, expand_fixture_templates};

/// Convert a `serde_json::Value` to a JavaScript literal string.
pub(super) fn json_to_js(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => {
            let expanded = expand_fixture_templates(s);
            format!("\"{}\"", escape_js(&expanded))
        }
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => {
            // For integers outside JS safe range, emit as string to avoid precision loss.
            if let Some(i) = n.as_i64()
                && !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&i)
            {
                return format!("Number(\"{i}\")");
            }
            if let Some(u) = n.as_u64()
                && u > 9_007_199_254_740_991
            {
                return format!("Number(\"{u}\")");
            }
            n.to_string()
        }
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_js).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    // Quote keys that aren't valid JS identifiers (contain hyphens, spaces, etc.)
                    let key = if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                        && !k.starts_with(|c: char| c.is_ascii_digit())
                    {
                        k.clone()
                    } else {
                        format!("\"{}\"", escape_js(k))
                    };
                    format!("{key}: {}", json_to_js(v))
                })
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
    }
}

/// Convert a `serde_json::Value` to an indented multi-line JavaScript literal.
///
/// Top-level objects are always expanded to multi-line form with trailing commas
/// so that formatters (e.g. oxfmt) leave the output unchanged. Scalar values and
/// arrays are emitted inline. Nested objects are also expanded to multi-line.
///
/// The `indent` parameter controls the base indentation in spaces for all but
/// the outermost `{`/`}`. Pass 4 for a top-level `expect(data).toEqual({...})`
/// inside a two-space-indented test body.
pub(super) fn json_to_js_multiline(value: &serde_json::Value, indent: usize) -> String {
    match value {
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            let pad = " ".repeat(indent);
            let inner_pad = " ".repeat(indent + 2);
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let key = if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                        && !k.starts_with(|c: char| c.is_ascii_digit())
                    {
                        k.clone()
                    } else {
                        format!("\"{}\"", escape_js(k))
                    };
                    format!("{inner_pad}{key}: {},", json_to_js_multiline(v, indent + 2))
                })
                .collect();
            format!("{{\n{}\n{pad}}}", entries.join("\n"))
        }
        // Non-object values are emitted inline.
        other => json_to_js(other),
    }
}

/// Render `key` as an object-literal key, quoting it when it is not a bare JS identifier
/// (hyphens, spaces, a leading digit).
pub(super) fn js_object_key(key: &str) -> String {
    if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !key.starts_with(|c: char| c.is_ascii_digit())
    {
        key.to_string()
    } else {
        format!("\"{}\"", escape_js(key))
    }
}

/// Convert a `serde_json::Value` to a JavaScript literal string with camelCase object keys.
///
/// NAPI-RS bindings use camelCase for JavaScript field names. This variant converts
/// snake_case object keys (as written in fixture JSON) to camelCase so that the
/// generated config objects match the NAPI binding's expected field names.
pub(super) fn json_to_js_camel(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let key = js_object_key(&underscore_camel_case(k));
                    format!("{key}: {}", json_to_js_camel(v))
                })
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_js_camel).collect();
            format!("[{}]", items.join(", "))
        }
        // Scalars and null delegate to the standard converter.
        other => json_to_js(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_to_js_string_escapes_double_quotes() {
        let val = serde_json::Value::String("say \"hello\"".to_string());
        let out = json_to_js(&val);
        assert!(out.contains("\\\""), "got: {out}");
    }

    #[test]
    fn json_to_js_null_returns_null_literal() {
        assert_eq!(json_to_js(&serde_json::Value::Null), "null");
    }

    #[test]
    fn json_to_js_camel_converts_object_keys() {
        let val = serde_json::json!({ "my_field": 1 });
        let out = json_to_js_camel(&val);
        assert!(out.contains("myField"), "got: {out}");
        assert!(!out.contains("my_field"), "got: {out}");
    }
}
