use crate::core::ir::{PrimitiveType, TypeRef};
use heck::AsSnakeCase;
use std::collections::HashSet;

/// Produce the Zig return expression for an opaque method result.
pub(super) fn method_unwrap_return_expr(
    raw: &str,
    ty: &TypeRef,
    prefix: &str,
    struct_names: &HashSet<String>,
) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => format!("{raw} != 0"),
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            format!(
                "blk: {{\n            const value = {raw} orelse return error.OutOfMemory;\n            defer c.{prefix}_free_string(value);\n            const slice = std.mem.span(value);\n            const owned = try std.heap.c_allocator.dupe(u8, slice);\n            break :blk owned;\n        }}"
            )
        }
        TypeRef::Named(name) if struct_names.contains(name) => {
            let snake = AsSnakeCase(name).to_string();
            format!(
                "blk: {{\n            if ({raw} == 0) return error.OutOfMemory;\n            const value = {raw};\n            defer c.{prefix}_{snake}_free(value);\n            const _json_ptr = c.{prefix}_{snake}_to_json(value) orelse return error.OutOfMemory;\n            defer c.{prefix}_free_string(_json_ptr);\n            const _json_slice = std.mem.span(_json_ptr);\n            const owned = try std.heap.c_allocator.dupe(u8, _json_slice);\n            break :blk owned;\n        }}"
            )
        }
        TypeRef::Named(name) => {
            format!("blk: {{ if ({raw} == 0) return error.OutOfMemory; break :blk {name}{{ ._handle = {raw} }}; }}")
        }
        TypeRef::Optional(inner) => optional_unwrap_return_expr(raw, inner, prefix, struct_names),
        _ => raw.to_string(),
    }
}

/// The `Optional<T>` half of [`method_unwrap_return_expr`], split out because every
/// variant here must degrade to Zig `null` on a zero/absent raw value instead of the
/// error paths the non-optional variants take. ~keep
fn optional_unwrap_return_expr(raw: &str, inner: &TypeRef, prefix: &str, struct_names: &HashSet<String>) -> String {
    match inner {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            format!(
                "blk: {{\n            const value = {raw} orelse break :blk null;\n            defer c.{prefix}_free_string(value);\n            const slice = std.mem.span(value);\n            const owned = try std.heap.c_allocator.dupe(u8, slice);\n            break :blk owned;\n        }}"
            )
        }
        TypeRef::Named(name) if struct_names.contains(name) => {
            let snake = AsSnakeCase(name).to_string();
            format!(
                "blk: {{\n            if ({raw} == 0) break :blk null;\n            const value = {raw};\n            defer c.{prefix}_{snake}_free(value);\n            const _json_ptr = c.{prefix}_{snake}_to_json(value) orelse return error.OutOfMemory;\n            defer c.{prefix}_free_string(_json_ptr);\n            const _json_slice = std.mem.span(_json_ptr);\n            const owned = try std.heap.c_allocator.dupe(u8, _json_slice);\n            break :blk owned;\n        }}"
            )
        }
        TypeRef::Named(name) => {
            format!("if ({raw} == 0) null else {name}{{ ._handle = {raw} }}")
        }
        _ => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_returns_validate_null_and_defer_native_release() {
        let string = method_unwrap_return_expr("_result", &TypeRef::String, "sample", &HashSet::new());
        let handle = method_unwrap_return_expr(
            "_result",
            &TypeRef::Named("DocumentHandle".to_string()),
            "sample",
            &HashSet::new(),
        );

        assert!(string.contains("orelse return error.OutOfMemory"));
        assert!(string.contains("defer c.sample_free_string(value)"));
        assert!(!string.contains("std.mem.span(_result)"));
        assert!(handle.contains("if (_result == 0) return error.OutOfMemory"));
        assert!(!handle.contains(".?"));
    }

    #[test]
    fn optional_handle_returns_null_instead_of_bare_raw_value() {
        let expr = method_unwrap_return_expr(
            "_result",
            &TypeRef::Optional(Box::new(TypeRef::Named("Tree".to_string()))),
            "sample",
            &HashSet::new(),
        );

        assert_eq!(expr, "if (_result == 0) null else Tree{ ._handle = _result }");
    }

    #[test]
    fn optional_json_struct_returns_null_instead_of_erroring_on_absence() {
        let expr = method_unwrap_return_expr(
            "_result",
            &TypeRef::Optional(Box::new(TypeRef::Named("DocumentHandle".to_string()))),
            "sample",
            &HashSet::from(["DocumentHandle".to_string()]),
        );

        assert!(expr.contains("if (_result == 0) break :blk null;"), "{expr}");
        assert!(
            !expr.contains("return error.OutOfMemory;\n            const value"),
            "{expr}"
        );
    }

    #[test]
    fn optional_string_returns_null_instead_of_erroring_on_absence() {
        let expr = method_unwrap_return_expr(
            "_result",
            &TypeRef::Optional(Box::new(TypeRef::String)),
            "sample",
            &HashSet::new(),
        );

        assert!(expr.contains("orelse break :blk null;"), "{expr}");
        assert!(!expr.contains("orelse return error.OutOfMemory"), "{expr}");
    }

    /// Positive control: an `Optional<primitive>` has no owned resource to release and no
    /// null/zero sentinel to translate, so it must stay a bare passthrough. If this test ever
    /// fails because the raw value got wrapped, the other assertions in this module are no
    /// longer proving anything — they would pass merely because everything got wrapped.
    #[test]
    fn optional_primitive_stays_a_bare_passthrough() {
        let expr = method_unwrap_return_expr(
            "_result",
            &TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I64))),
            "sample",
            &HashSet::new(),
        );

        assert_eq!(expr, "_result");
    }

    /// Regression test: a method returning `Char` must copy and free like `String` (both cross
    /// the C ABI as `*mut c_char`, freed via the same `_free_string`), not fall through to a
    /// bare passthrough of a pointer the declared `[]u8` return type cannot represent.
    #[test]
    fn char_return_copies_and_frees_like_string_instead_of_bare_passthrough() {
        let expr = method_unwrap_return_expr("_result", &TypeRef::Char, "sample", &HashSet::new());

        assert!(expr.contains("orelse return error.OutOfMemory"), "{expr}");
        assert!(expr.contains("defer c.sample_free_string(value)"), "{expr}");
        assert_ne!(expr, "_result");
    }

    #[test]
    fn optional_char_returns_null_instead_of_erroring_on_absence() {
        let expr = method_unwrap_return_expr(
            "_result",
            &TypeRef::Optional(Box::new(TypeRef::Char)),
            "sample",
            &HashSet::new(),
        );

        assert!(expr.contains("orelse break :blk null;"), "{expr}");
        assert!(!expr.contains("orelse return error.OutOfMemory"), "{expr}");
    }
}
