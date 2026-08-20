use crate::core::ir::{PrimitiveType, TypeRef};
use heck::AsSnakeCase;
use std::collections::HashSet;

/// Produce the Zig return expression for an opaque method result.
pub(super) fn method_unwrap_return_expr(
    raw: &str,
    ty: &TypeRef,
    prefix: &str,
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => format!("{raw} != 0"),
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            format!(
                "blk: {{\n            const value = {raw} orelse return error.OutOfMemory;\n            defer c.{prefix}_free_string(value);\n            const slice = std.mem.span(value);\n            const owned = try std.heap.c_allocator.dupe(u8, slice);\n            break :blk owned;\n        }}"
            )
        }
        // A raw enum discriminant is a plain integer, not a nullable pointer/handle: variant
        // 0 is frequently a legitimate value, so the `raw == 0` sentinel the struct/handle arms
        // below use for "allocation failed" must not run here. Before this arm existed, an enum
        // return fell to the bare-`Named` handle arm and produced `MyEnum{ ._handle = raw }` --
        // not valid Zig for an `enum { ... }` declaration, which has no `._handle` field and no
        // struct-literal syntax at all. ~keep
        TypeRef::Named(name) if enum_names.contains(name) => format!("@as({name}, @enumFromInt({raw}))"),
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
///
/// The bare-`Named` arm below has the SAME shape defect the non-optional arm above was fixed
/// for (`name{ ._handle = raw }` against a real enum type), for an `Optional<Enum>` return.
/// Deliberately left unfixed here: unlike the non-optional case, there is no visible, verified
/// convention in this codebase for how a `None` enum value is signalled across the C ABI for a
/// method return (`raw == 0` collides with a legitimate zero-valued enum variant, same as the
/// non-optional case, but the sentinel this branch WOULD need instead is not established
/// anywhere this function can consult). Guessing at one risks a wrong-but-plausible fix that
/// looks correct in generated text and is wrong at runtime. ~keep
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
        let string = method_unwrap_return_expr("_result", &TypeRef::String, "sample", &HashSet::new(), &HashSet::new());
        let handle = method_unwrap_return_expr(
            "_result",
            &TypeRef::Named("DocumentHandle".to_string()),
            "sample",
            &HashSet::new(),
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
            &HashSet::new(),
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
            &HashSet::new(),
        );

        assert_eq!(expr, "_result");
    }

    /// Regression test: a method returning `Char` must copy and free like `String` (both cross
    /// the C ABI as `*mut c_char`, freed via the same `_free_string`), not fall through to a
    /// bare passthrough of a pointer the declared `[]u8` return type cannot represent.
    #[test]
    fn char_return_copies_and_frees_like_string_instead_of_bare_passthrough() {
        let expr = method_unwrap_return_expr("_result", &TypeRef::Char, "sample", &HashSet::new(), &HashSet::new());

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
            &HashSet::new(),
        );

        assert!(expr.contains("orelse break :blk null;"), "{expr}");
        assert!(!expr.contains("orelse return error.OutOfMemory"), "{expr}");
    }

    /// The defect itself: before `enum_names` was consulted, a `Named` return not in
    /// `struct_names` always fell to the opaque-handle arm and produced `MyEnum{ ._handle =
    /// raw }` — not valid Zig for a real `enum { ... }` declaration. A genuine enum return must
    /// instead cast the raw discriminant back with `@enumFromInt`, and must NOT null-check
    /// `raw == 0` first (a real enum variant can legitimately serialize to 0).
    #[test]
    fn enum_return_casts_the_raw_discriminant_instead_of_wrapping_a_handle_struct() {
        let expr = method_unwrap_return_expr(
            "_result",
            &TypeRef::Named("Language".to_string()),
            "sample",
            &HashSet::new(),
            &HashSet::from(["Language".to_string()]),
        );

        assert_eq!(expr, "@as(Language, @enumFromInt(_result))");
        assert!(!expr.contains("._handle"), "{expr}");
        assert!(!expr.contains("== 0"), "{expr}");
    }

    /// Misclassification guard: a `Named` type that is in NEITHER `struct_names` nor
    /// `enum_names` (a genuine opaque handle, e.g. `Tree`) must keep the pre-existing
    /// `{ ._handle = raw }` shape — the enum arm must not swallow every bare-`Named` return.
    #[test]
    fn a_genuine_opaque_handle_return_is_not_misclassified_as_an_enum() {
        let expr = method_unwrap_return_expr(
            "_result",
            &TypeRef::Named("Tree".to_string()),
            "sample",
            &HashSet::new(),
            &HashSet::from(["Language".to_string()]),
        );

        assert!(expr.contains("Tree{ ._handle = _result }"), "{expr}");
    }
}
