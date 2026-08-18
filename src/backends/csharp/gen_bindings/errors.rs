//! C# exception class generation.

use crate::core::ir::TypeRef;
use std::collections::HashSet;

/// Generate a generic `{ClassName} : Exception` class used as the fallback error type.
pub(super) fn gen_exception_class(namespace: &str, class_name: &str) -> String {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    render(
        "exception_class.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "class_name": class_name,
        })),
    )
}

/// Compute the set of types that are returned as opaque handles (matching `*mut T` pattern).
/// A type is considered opaque-handle-returned if any public function or method returns the
/// type directly or wrapped in Optional/Vec — those all surface across FFI as `*mut T`.
/// Includes types with NO serde support (truly opaque handles only); serde-capable types
/// are routed through JSON marshalling, even when the FFI layer omits the `*_to_json`
/// helper for the type itself (the consumer constructs the handle by calling the
/// corresponding `*_from_json` on the engine result).
pub(super) fn compute_handle_returned_types(api: &crate::core::ir::ApiSurface) -> HashSet<String> {
    fn inner_named(ty: &crate::core::ir::TypeRef) -> Option<&str> {
        match ty {
            crate::core::ir::TypeRef::Named(n) => Some(n.as_str()),
            crate::core::ir::TypeRef::Optional(inner) | crate::core::ir::TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }

    let mut type_def_map = std::collections::HashMap::new();
    for typ in &api.types {
        type_def_map.insert(typ.name.clone(), typ);
    }

    let mut handle_types = HashSet::new();

    for func in &api.functions {
        if let Some(name) = inner_named(&func.return_type)
            && let Some(type_def) = type_def_map.get(name)
            && !type_def.has_serde
        {
            handle_types.insert(name.to_string());
        }
    }

    for typ in &api.types {
        for method in &typ.methods {
            if let Some(name) = inner_named(&method.return_type)
                && let Some(type_def) = type_def_map.get(name)
                && !type_def.has_serde
            {
                handle_types.insert(name.to_string());
            }
        }
    }

    handle_types
}

/// Emit the final `return returnValue;` statement after cleanup.
pub(super) fn emit_return_statement(out: &mut String, return_type: &TypeRef) {
    emit_return_statement_indented(out, return_type, "        ");
}

/// Emit the return-value marshalling code with configurable indentation.
///
/// Like `emit_return_marshalling` this stores the value in `returnValue` without emitting
/// the final `return` statement.  Callers must call `emit_return_statement_indented` after.
pub(super) fn emit_return_marshalling_indented(
    out: &mut String,
    return_type: &TypeRef,
    indent: &str,
    enum_names: &HashSet<String>,
    true_opaque_types: &HashSet<String>,
    handle_returned_types: &HashSet<String>,
    enum_data_variant_names: &HashSet<String>,
) {
    use super::{returns_bool_via_int, returns_json_object, returns_string};
    use crate::backends::csharp::template_env::render;
    use crate::backends::csharp::type_map::csharp_type;
    use crate::codegen::naming::csharp_type_name;

    if *return_type == TypeRef::Unit {
        return;
    }

    if returns_string(return_type) {
        out.push_str(&render("return_string_utf8.jinja", minijinja::context! { indent }));
        out.push_str(&render("free_native_string.jinja", minijinja::context! { indent }));
    } else if returns_bool_via_int(return_type) {
        out.push_str(&render("return_bool_from_int.jinja", minijinja::context! { indent }));
    } else if let TypeRef::Named(type_name) = return_type {
        let pascal = csharp_type_name(type_name);
        if true_opaque_types.contains(type_name)
            || true_opaque_types.contains(&pascal)
            || handle_returned_types.contains(type_name)
            || handle_returned_types.contains(&pascal)
        {
            out.push_str(&render(
                "return_opaque_ctor.jinja",
                minijinja::context! { indent, pascal },
            ));
        } else if !enum_names.contains(&pascal) || enum_data_variant_names.contains(&pascal) {
            // A data struct, or a data-carrying enum, is boxed by `insert_handle` exactly like
            // any other `Named` return (`gen_owned_value_to_c` in the FFI crate has no
            // enum-ness branch for owned conversion) — so `nativeResult` here is the
            // `AlefHandle` scalar, not a pointer, and must be exchanged for the JSON string via
            // the `{Pascal}ToJson` companion before it can be freed and deserialised. Passing
            // `nativeResult` straight to `Marshal.PtrToStringUTF8` (the `else` branch below) is
            // the CS1503 `ulong`-to-`nint` defect this condition exists to avoid. ~keep
            let to_json_method = format!("{pascal}ToJson");
            let free_method = format!("{pascal}Free");
            let cs_ty = csharp_type(return_type);
            out.push_str(&render(
                "native_to_json_ptr.jinja",
                minijinja::context! { indent, to_json_method },
            ));
            out.push_str(&render(
                "json_from_ptr.jinja",
                minijinja::context! { indent, ptr_var => "jsonPtr" },
            ));
            out.push_str(&render(
                "free_string_ptr.jinja",
                minijinja::context! { indent, ptr_var => "jsonPtr" },
            ));
            out.push_str(&render(
                "free_native_handle.jinja",
                minijinja::context! { indent, free_method },
            ));
            out.push_str(&render(
                "deserialize_json.jinja",
                minijinja::context! { indent, cs_type => cs_ty },
            ));
        } else {
            let cs_ty = csharp_type(return_type);
            out.push_str(&render(
                "json_from_ptr.jinja",
                minijinja::context! { indent, ptr_var => "nativeResult" },
            ));
            out.push_str(&render(
                "free_string_ptr.jinja",
                minijinja::context! { indent, ptr_var => "nativeResult" },
            ));
            out.push_str(&render(
                "deserialize_json.jinja",
                minijinja::context! { indent, cs_type => cs_ty },
            ));
        }
    } else if returns_json_object(return_type) {
        if let TypeRef::Optional(inner) = return_type {
            if returns_string(inner) {
                out.push_str(&render("return_ptr_as_string.jinja", minijinja::context! { indent }));
                out.push_str(&render("free_native_string.jinja", minijinja::context! { indent }));
                return;
            }
            if let TypeRef::Named(type_name) = inner.as_ref() {
                let pascal = csharp_type_name(type_name);
                if true_opaque_types.contains(type_name)
                    || true_opaque_types.contains(&pascal)
                    || handle_returned_types.contains(type_name)
                    || handle_returned_types.contains(&pascal)
                {
                    out.push_str(&render(
                        "return_opaque_ctor.jinja",
                        minijinja::context! { indent, pascal },
                    ));
                    return;
                }
                let to_json_method = format!("{pascal}ToJson");
                let free_method = format!("{pascal}Free");
                let cs_ty = csharp_type(return_type);
                out.push_str(&render(
                    "native_to_json_ptr.jinja",
                    minijinja::context! { indent, to_json_method },
                ));
                out.push_str(&render(
                    "json_from_ptr.jinja",
                    minijinja::context! { indent, ptr_var => "jsonPtr" },
                ));
                out.push_str(&render(
                    "free_string_ptr.jinja",
                    minijinja::context! { indent, ptr_var => "jsonPtr" },
                ));
                out.push_str(&render(
                    "free_native_handle.jinja",
                    minijinja::context! { indent, free_method },
                ));
                out.push_str(&render(
                    "deserialize_json.jinja",
                    minijinja::context! { indent, cs_type => cs_ty },
                ));
                return;
            }
        }
        let cs_ty = csharp_type(return_type);
        out.push_str(&render(
            "json_from_ptr.jinja",
            minijinja::context! { indent, ptr_var => "nativeResult" },
        ));
        out.push_str(&render(
            "free_string_ptr.jinja",
            minijinja::context! { indent, ptr_var => "nativeResult" },
        ));
        out.push_str(&render(
            "deserialize_json.jinja",
            minijinja::context! { indent, cs_type => cs_ty },
        ));
    } else {
        out.push_str(&render("return_native_result.jinja", minijinja::context! { indent }));
    }
}

/// Emit the final `return returnValue;` with configurable indentation.
pub(super) fn emit_return_statement_indented(out: &mut String, return_type: &TypeRef, indent: &str) {
    if *return_type != TypeRef::Unit {
        out.push_str(&crate::backends::csharp::template_env::render(
            "return_value.jinja",
            minijinja::context! { indent },
        ));
    }
}
