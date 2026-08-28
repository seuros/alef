//! C# struct-field type resolution for object-initializer values.
//!
//! Split out of `setup.rs`, which sits at the file-size ratchet's frozen ceiling
//! (`tests/file_size_baseline.txt`) and may not grow further — see the `file-modularization`
//! rule. Behavior is unchanged from the block this replaces, re-exported back to `setup.rs`
//! through `values::` so every existing call site (`csharp_object_initializer`) keeps calling
//! these by their bare names.

use crate::e2e::escape::escape_csharp;

use super::{json_to_csharp, render_collection_literal};

/// Convert a JSON array to a typed C# `List<T>` (or, for `"u8"`, `byte[]`) expression.
///
/// Mapping from `ArgMapping::element_type`:
/// - `None` or any string type → `List<string>`
/// - `"f32"` → `List<float>` with `(float)` casts
/// - `"u8"` → `byte[]`, from a fixture value of raw byte numbers (`resolve_csharp_field_element_type_from_struct`'s
///   `Bytes` arm feeds this for a `Vec<u8>` struct field; `new byte[] { }` is valid C# even when
///   empty, unlike the implicitly-typed `new[] { }` `json_to_csharp` would otherwise emit)
/// - `"(String, String)"` → `List<List<string>>` for key-value pair arrays
pub(crate) fn json_array_to_csharp_list(arr: &[serde_json::Value], element_type: Option<&str>) -> String {
    match element_type {
        Some("f32") => {
            let items: Vec<String> = arr.iter().map(|v| format!("(float){}", json_to_csharp(v))).collect();
            render_collection_literal("new List<float>()", items)
        }
        Some("u8") => {
            let items: Vec<String> = arr
                .iter()
                .map(|v| v.as_u64().map(|n| n.to_string()).unwrap_or_else(|| json_to_csharp(v)))
                .collect();
            render_collection_literal("new byte[]", items)
        }
        Some("(String, String)") => {
            let items: Vec<String> = arr
                .iter()
                .map(|v| {
                    let strs: Vec<String> = v
                        .as_array()
                        .map_or_else(Vec::new, |a| a.iter().map(json_to_csharp).collect());
                    render_collection_literal("new List<string>()", strs)
                })
                .collect();
            render_collection_literal("new List<List<string>>()", items)
        }
        Some(et) if et != "f32" && et != "u8" && et != "(String, String)" && et != "string" => {
            // Class/record types: deserialize each element from JSON
            let items: Vec<String> = arr
                .iter()
                .map(|v| {
                    let json_str = serde_json::to_string(v).unwrap_or_default();
                    let escaped = escape_csharp(&json_str);
                    format!("JsonSerializer.Deserialize<{et}>(\"{escaped}\", ConfigOptions)!")
                })
                .collect();
            render_collection_literal(&format!("new List<{et}>()"), items)
        }
        _ => {
            let items: Vec<String> = arr.iter().map(json_to_csharp).collect();
            render_collection_literal("new List<string>()", items)
        }
    }
}

/// Resolve the actual C# field type from a struct definition in type_defs.
///
/// Given a struct name and a field key (in snake_case), looks up the struct in type_defs
/// and returns the C# type name of that field. For sealed unions (discriminated unions),
/// returns the correct variant type (e.g., RerankerModelType for RerankerConfig.model).
pub(crate) fn resolve_csharp_field_type_from_struct(
    struct_name: &str,
    field_key: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    // Find the struct definition
    let struct_def = type_defs.iter().find(|td| td.name == struct_name)?;

    // field_key is snake_case from fixture JSON and matches Rust field names
    let field_name = field_key;

    // Find the field in the struct
    let field = struct_def.fields.iter().find(|f| f.name == field_name)?;

    // Extract type name from TypeRef
    match &field.ty {
        crate::core::ir::TypeRef::Named(name) => Some(name.clone()),
        crate::core::ir::TypeRef::Json => Some("JsonElement".to_string()),
        crate::core::ir::TypeRef::Optional(inner) => match inner.as_ref() {
            crate::core::ir::TypeRef::Named(name) => Some(name.clone()),
            crate::core::ir::TypeRef::Json => Some("JsonElement".to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// Resolve the C# element type of a collection-typed struct field (`Vec<T>` / `Bytes` /
/// `Option<...>` of either), for array-valued fields inside an object initializer.
///
/// `resolve_csharp_field_type_from_struct` only unwraps `Named`/`Json` at the top
/// level, so it returns `None` for any `Vec<_>` field — which is exactly the field
/// shape an array-valued JSON property has. Without this, `csharp_object_initializer`
/// had no way to learn a collection field's real element type and hardcoded
/// `List<string>` for every array, silently corrupting genuinely-typed collections
/// (`List<Message>`, `List<RerankDocument>`, ...) into unusable string lists. The `Bytes`
/// arm (`Vec<u8>` in Rust, not IR-wrapped in `Vec`) closes the same gap for a `byte[]`
/// field: an empty fixture value used to fall through to that same `List<string>`
/// default and get spliced into a `byte[]`-typed property — CS0029. ~keep
pub(crate) fn resolve_csharp_field_element_type_from_struct(
    struct_name: &str,
    field_key: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    let struct_def = type_defs.iter().find(|td| td.name == struct_name)?;
    let field = struct_def.fields.iter().find(|f| f.name == field_key)?;
    let ty = match &field.ty {
        crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    };
    match ty {
        crate::core::ir::TypeRef::Bytes => Some("u8".to_string()),
        crate::core::ir::TypeRef::Vec(inner) => match inner.as_ref() {
            crate::core::ir::TypeRef::Named(name) => Some(name.clone()),
            crate::core::ir::TypeRef::Json => Some("JsonElement".to_string()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    fn struct_with_bytes_field(optional: bool) -> Vec<TypeDef> {
        let ty = if optional {
            TypeRef::Optional(Box::new(TypeRef::Bytes))
        } else {
            TypeRef::Bytes
        };
        vec![TypeDef {
            name: "SampleConfig".to_string(),
            fields: vec![FieldDef {
                name: "payload".to_string(),
                ty,
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        }]
    }

    /// Regression for the CS0029 this closes: an empty `Vec<u8>` struct field must resolve to
    /// the `"u8"` element type, not fall through to `None` (which `json_array_to_csharp_list`
    /// renders as an untyped `List<string>` -- incompatible with a `byte[]`-typed property).
    #[test]
    fn a_bytes_struct_field_resolves_the_u8_element_type() {
        let type_defs = struct_with_bytes_field(false);

        assert_eq!(
            resolve_csharp_field_element_type_from_struct("SampleConfig", "payload", &type_defs),
            Some("u8".to_string())
        );
    }

    /// The same field wrapped in `Option<Vec<u8>>` must resolve identically -- an optional byte
    /// field is still a byte field once a fixture supplies a value for it at all.
    #[test]
    fn an_optional_bytes_struct_field_also_resolves_the_u8_element_type() {
        let type_defs = struct_with_bytes_field(true);

        assert_eq!(
            resolve_csharp_field_element_type_from_struct("SampleConfig", "payload", &type_defs),
            Some("u8".to_string())
        );
    }

    /// An empty `"u8"`-typed array must render `new byte[] { }`, a valid explicitly-typed empty
    /// array literal, and never `List<string>` -- the exact CS0029 shape this fix closes.
    #[test]
    fn an_empty_u8_array_renders_an_explicitly_typed_empty_byte_array() {
        let rendered = json_array_to_csharp_list(&[], Some("u8"));

        assert_eq!(rendered, "new byte[] {  }");
        assert!(!rendered.contains("List<string>"), "{rendered}");
    }

    /// A non-empty `"u8"` array renders each JSON number as a bare integer literal inside the
    /// `byte[]` initializer, not a `JsonSerializer.Deserialize` call (the generic named-type arm)
    /// or a quoted string (the untyped fallback arm).
    #[test]
    fn a_non_empty_u8_array_renders_byte_literals() {
        let rendered = json_array_to_csharp_list(&[serde_json::json!(111), serde_json::json!(107)], Some("u8"));

        assert_eq!(rendered, "new byte[] { 111, 107 }");
    }
}
