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
        TypeRef::String | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
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
}
