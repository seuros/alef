use crate::e2e::escape::escape_java;
use crate::e2e::field_access::IrEnumMap;
use heck::{ToLowerCamelCase, ToUpperCamelCase};

/// True when the IR knows `field_name` (the raw, snake_case fixture key) is an enum-typed
/// field on `type_name` — the exact struct the current JSON object maps to (an `Options`
/// type, or a nested config record within one).
///
/// `java_builder_expression` anchors by `type_name` directly rather than by field name alone,
/// so a same-named field on an unrelated type is never misclassified — the same invariant the
/// rust/csharp/gleam/swift/dart/kotlin/python/elixir/ruby e2e generators keep for result
/// fields, applied here to argument-builder fields instead (there is no "declared Rust return
/// type" to anchor against for an argument; `type_name` already names the exact type being
/// built). Purely additive: callers check the hand-maintained `enum_fields` config FIRST, so
/// an explicit config entry still wins. ~keep
fn is_ir_enum_field(ir_enum_map: &IrEnumMap, type_name: &str, field_name: &str) -> bool {
    ir_enum_map
        .enum_fields
        .get(type_name)
        .is_some_and(|fields| fields.contains(field_name))
}

/// Check if a type name is a numeric type hint (f32, float, etc.) vs. a complex type name.
pub(super) fn is_numeric_type_hint(ty: &str) -> bool {
    matches!(ty, "f32" | "f64" | "float" | "double" | "Float" | "Double")
}

/// Check if a type name is a Java built-in type that doesn't need an import.
pub(super) fn is_java_builtin_type(ty: &str) -> bool {
    matches!(
        ty,
        "String" | "Boolean" | "Integer" | "Long" | "Double" | "Float" | "Byte" | "Short" | "Character" | "Void"
    )
}

/// The JVM's `CONSTANT_Utf8` constant-pool entry -- and `javac`'s own enforcement of the same
/// cap on any single string literal -- tops out at 65535 bytes of modified UTF-8. No amount of
/// escaping raises that ceiling; a value long enough to threaten it has to stop being one
/// literal.
///
/// The budget is counted in raw characters, not bytes, because a single non-BMP character (an
/// emoji, for instance) costs up to 6 bytes in modified UTF-8 (a CESU-8 surrogate pair) while
/// counting as one `char` here -- so the budget has to assume every character could be that
/// expensive. `8_000 * 6 = 48_000`, comfortably under the 65535 cap even before the escaping
/// `java_string_literal` already performs on top of it.
const JAVA_STRING_LITERAL_CHUNK_CHARS: usize = 8_000;

/// Render `s` as a Java string-literal expression that compiles regardless of length.
///
/// A single `"..."` literal cannot exceed the JVM's per-constant byte cap -- see
/// [`JAVA_STRING_LITERAL_CHUNK_CHARS`]. Values under the safe budget (the overwhelming
/// majority: identifiers, URLs, short fixture fields) render exactly as before, one `"..."`
/// literal. A value long enough to threaten the cap -- e.g. a large fixture body inlined into a
/// generated doc snippet or e2e test -- is split into `+`-concatenated literal chunks, each
/// small enough that no single chunk can approach the limit, and the chunks are handed to
/// `String.join("", ...)` so a caller can chain a method off it (`.replace(...)`) or pass it as a
/// call argument without knowing whether it rendered one literal or several.
///
/// `String.join` rather than `+`, and this is the whole point: `"a" + "b"` between two literals is
/// a *compile-time constant expression* under JLS 15.29, so `javac` folds it back into a single
/// `CONSTANT_Utf8` entry and rejects it with `constant string too long` — the very error the
/// chunking exists to avoid. Splitting into `+`-joined literals therefore changed nothing an
/// assertion over the rendered text could see, which is exactly why it survived: every test here
/// measured segment lengths in the emitted string and none of them ever ran `javac`. A method
/// call is not a constant expression, so each chunk stays its own pool entry. ~keep
pub(super) fn java_string_literal(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= JAVA_STRING_LITERAL_CHUNK_CHARS {
        return format!("\"{}\"", escape_java(s));
    }
    let joined = chars
        .chunks(JAVA_STRING_LITERAL_CHUNK_CHARS)
        .map(|chunk| format!("\"{}\"", escape_java(&chunk.iter().collect::<String>())))
        .collect::<Vec<_>>()
        .join(", ");
    format!("String.join(\"\", {joined})")
}

/// Emit a Java list of deserialized objects via JsonUtil.
/// E.g., `[{"type": "click", ...}, ...]` becomes `java.util.Arrays.asList(JsonUtil.fromJson(...))`.
pub(super) fn emit_java_object_array(arr: &serde_json::Value, elem_type: &str) -> String {
    if let Some(items) = arr.as_array() {
        if items.is_empty() {
            return "java.util.List.of()".to_string();
        }
        let item_strs: Vec<String> = items
            .iter()
            .map(|item| {
                let json_str = serde_json::to_string(item).unwrap_or_default();
                let literal = java_string_literal(&json_str);
                format!("JsonUtil.fromJson({literal}, {elem_type}.class)")
            })
            .collect();
        format!("java.util.Arrays.asList({})", item_strs.join(", "))
    } else {
        "java.util.List.of()".to_string()
    }
}

/// Convert a `serde_json::Value` to a Java literal string.
pub(super) fn json_to_java(value: &serde_json::Value) -> String {
    json_to_java_typed(value, None)
}

/// Convert a JSON value to a Java literal, optionally overriding number type for array elements.
/// `element_type` controls how numeric array elements are emitted: "f32" -> `1.0f`, otherwise `1.0d`.
pub(super) fn json_to_java_typed(value: &serde_json::Value, element_type: Option<&str>) -> String {
    match value {
        serde_json::Value::String(s) => java_string_literal(s),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => {
            if n.is_f64() {
                match element_type {
                    Some("f32" | "float" | "Float") => format!("{}f", n),
                    _ => format!("{}d", n),
                }
            } else {
                n.to_string()
            }
        }
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(|v| json_to_java_typed(v, element_type)).collect();
            format!("java.util.List.of({})", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            java_string_literal(&json_str)
        }
    }
}

/// Generate a Java builder expression for a JSON object.
/// E.g., `obj = {"language": "abl", "chunk_max_size": 50}`
/// becomes: `TypeName.builder().withLanguage("abl").withChunkMaxSize(50L).build()`
///
/// For enums: emit `EnumType.VariantName` (detected via camelCase lookup in enum_fields)
/// For strings and bools: use the value directly
/// For plain numbers: emit the literal with type suffix (long uses L, double uses d)
/// For nested objects: recurse with Options suffix
/// When `nested_types_optional` is false, nested builders are passed directly without
/// Optional.of() wrapping, allowing non-optional nested config types.
pub(super) fn java_builder_expression(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    enum_fields: &std::collections::HashSet<String>,
    nested_types: &std::collections::HashMap<String, String>,
    nested_types_optional: bool,
    path_fields: &[String],
    ir_enum_map: &IrEnumMap,
) -> String {
    let mut expr = format!("{}.builder()", type_name);
    for (key, val) in obj {
        // Convert snake_case key to camelCase for method name
        let camel_key = key.to_lower_camel_case();
        let method_name = format!("with{}", camel_key.to_upper_camel_case());

        let java_val = match val {
            serde_json::Value::String(s) => {
                // Check if this field is an enum type by checking enum_fields. When the
                // hand-maintained config is silent, fall back to the IR-derived classification
                // (anchored at `type_name`, the exact struct this object maps to) so a
                // consumer that never configured `enum_fields` still gets `EnumType.Variant`
                // instead of a quoted String literal passed to a builder method whose
                // parameter type is a Java enum — a CS1503-shaped compile error the field's
                // declared type can't satisfy. ~keep
                if enum_fields.contains(&camel_key) || is_ir_enum_field(ir_enum_map, type_name, key) {
                    // Enum field: infer type name from field name (e.g., "codeBlockStyle" -> "CodeBlockStyle")
                    let enum_type_name = camel_key.to_upper_camel_case();
                    let variant_name = s.to_upper_camel_case();
                    format!("{}.{}", enum_type_name, variant_name)
                } else if path_fields.contains(key) {
                    // Path field: wrap in Optional.of(java.nio.file.Path.of(...))
                    format!("Optional.of(java.nio.file.Path.of(\"{}\"))", escape_java(s))
                } else {
                    // String field: emit as a quoted literal (safe at any length).
                    java_string_literal(s)
                }
            }
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => "null".to_string(),
            serde_json::Value::Number(n) => {
                // Number field: emit literal with type suffix.
                // Java records/classes use either `long` (primitive, not nullable) or
                // `Optional<Long>` (nullable). The codegen wraps in `Optional.of(...)`
                // by default since most options builder fields are Optional. Calls that
                // use primitive builder fields can opt into bare values by setting
                // `nested_types_optional = false`.
                let camel_key = key.to_lower_camel_case();
                let is_plain_field = matches!(camel_key.as_str(), "listIndentWidth" | "wrapWidth");
                let is_primitive_builder = !nested_types_optional;

                if is_plain_field || is_primitive_builder {
                    // Plain numeric field: no Optional wrapper
                    if n.is_f64() {
                        format!("{}d", n)
                    } else {
                        format!("{}L", n)
                    }
                } else {
                    // Optional numeric field: wrap in Optional.of()
                    if n.is_f64() {
                        format!("Optional.of({}d)", n)
                    } else {
                        format!("Optional.of({}L)", n)
                    }
                }
            }
            serde_json::Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(|v| json_to_java_typed(v, None)).collect();
                format!("java.util.List.of({})", items.join(", "))
            }
            serde_json::Value::Object(nested) => {
                // Recurse with the type from nested_types mapping, or default to snake_case -> PascalCase + "Options".
                let nested_type = nested_types
                    .get(key.as_str())
                    .cloned()
                    .unwrap_or_else(|| format!("{}Options", key.to_upper_camel_case()));
                let inner = java_builder_expression(
                    nested,
                    &nested_type,
                    enum_fields,
                    nested_types,
                    nested_types_optional,
                    &[],
                    ir_enum_map,
                );
                // Top-level config builders usually declare nested record fields as
                // `Optional<T>`. Calls with non-optional nested config builders can opt
                // into passing the bare builder result.
                let is_primitive_builder = !nested_types_optional;
                if is_primitive_builder || !nested_types_optional {
                    inner
                } else {
                    format!("Optional.of({inner})")
                }
            }
        };
        expr.push_str(&format!(".{}({})", method_name, java_val));
    }
    expr.push_str(".build()");
    expr
}

/// Recursively collect enum types and nested option types used in a builder expression.
/// Enums are keyed in the enum_fields map by camelCase names (e.g., "codeBlockStyle" -> "CodeBlockStyle").
#[allow(dead_code)]
pub(super) fn collect_enum_and_nested_types(
    obj: &serde_json::Map<String, serde_json::Value>,
    enum_fields: &std::collections::HashMap<String, String>,
    types_out: &mut std::collections::BTreeSet<String>,
) {
    for (key, val) in obj {
        // enum_fields is keyed by camelCase, not snake_case.
        let camel_key = key.to_lower_camel_case();
        if let Some(enum_type) = enum_fields.get(&camel_key) {
            // Add the enum type from the mapping (e.g., "CodeBlockStyle").
            types_out.insert(enum_type.clone());
        }
        // Recurse into nested objects to find their nested enum types.
        if let Some(nested) = val.as_object() {
            collect_enum_and_nested_types(nested, enum_fields, types_out);
        }
    }
}

pub(super) fn collect_nested_type_names(
    obj: &serde_json::Map<String, serde_json::Value>,
    nested_types: &std::collections::HashMap<String, String>,
    types_out: &mut std::collections::BTreeSet<String>,
) {
    for (key, val) in obj {
        if let Some(type_name) = nested_types.get(key.as_str()) {
            types_out.insert(type_name.clone());
        }
        if let Some(nested) = val.as_object() {
            collect_nested_type_names(nested, nested_types, types_out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The JVM's `CONSTANT_Utf8` constant-pool cap is exactly 65535 bytes. Values comfortably
    /// under it must render exactly as before: one plain `"..."` literal, no parentheses, no
    /// concatenation.
    #[test]
    fn a_short_value_stays_a_single_quoted_literal() {
        assert_eq!(java_string_literal("hello world"), "\"hello world\"");
    }

    /// Neutral synthetic payload, well over the JVM's 65535-byte `CONSTANT_Utf8` cap. Not any
    /// real consumer's data -- chosen only to be unambiguously larger than the limit, per
    /// `project-agnostic-codegen`.
    fn oversized_payload() -> String {
        "abcdefghij".repeat(10_000) // 100,000 bytes
    }

    /// A value long enough to threaten the JVM's per-constant byte cap must never render as a
    /// single literal segment, and must never render as a `+`-joined one either.
    ///
    /// ~keep The `+` form was the original fix and it did not work: `"a" + "b"` between two
    /// literals is a compile-time constant expression (JLS 15.29), so `javac` folds it back into
    /// one `CONSTANT_Utf8` entry and still reports `constant string too long`. The test that
    /// guarded it asserted `literal.contains(" + ")` and measured each segment's length — both
    /// true of output `javac` rejects. Assert the join call here, and let
    /// `java::snippet`'s javac-backed test be the one that can actually see the failure.
    #[test]
    fn a_value_over_the_jvm_constant_cap_is_never_a_single_literal_segment() {
        let literal = java_string_literal(&oversized_payload());
        assert!(
            literal.starts_with("String.join(\"\", "),
            "an oversized value must be joined at runtime, not concatenated at compile time: {literal}"
        );
        let inner = literal
            .strip_prefix("String.join(\"\", ")
            .and_then(|rest| rest.strip_suffix(')'))
            .unwrap_or_else(|| panic!("expected a String.join expression: {literal}"));
        let segments: Vec<&str> = inner.split(", ").collect();
        assert!(
            segments.len() > 1,
            "an oversized value must be split into several literals: {literal}"
        );
        for segment in segments {
            assert!(
                segment.len() <= 65_535,
                "a single Java string literal segment must never exceed the JVM's 65535-byte \
                 CONSTANT_Utf8 cap: got {} bytes in {segment:?}",
                segment.len()
            );
        }
    }

    /// ~keep A constant-folding form is what made the previous fix inert, so pin its absence
    /// directly: re-introducing `+` between the chunks would restore the exact defect while
    /// keeping every length assertion above satisfied.
    #[test]
    fn an_oversized_literal_never_uses_compile_time_concatenation() {
        let literal = java_string_literal(&oversized_payload());
        assert!(
            !literal.contains(" + "),
            "`+` between literals folds to one constant pool entry and `javac` rejects it: {literal}"
        );
    }

    /// The joined expression must still be usable everywhere a bare literal was -- as a call
    /// argument, or with a method chained directly onto it (e.g. `.replace(...)`, as
    /// `test_method.rs`'s mock-URL substitution does). A call expression already is.
    #[test]
    fn an_oversized_literal_is_a_single_expression_a_caller_can_chain_a_method_onto() {
        let literal = java_string_literal(&oversized_payload());
        assert!(
            literal.starts_with("String.join(") && literal.ends_with(')'),
            "expected a single self-contained expression: {literal}"
        );
    }
}
