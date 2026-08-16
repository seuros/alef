use crate::backends::zig::gen_bindings::errors::resolve_zig_error_type;
use crate::backends::zig::gen_bindings::functions::{return_uses_bytes_out_params, zig_return_type};
use crate::backends::zig::gen_bindings::helpers::emit_cleaned_zig_doc;
use crate::core::ir::{MethodDef, ParamDef, ReceiverKind, TypeDef, TypeRef};
use heck::AsSnakeCase;
use std::collections::{HashMap, HashSet};

use super::params::{
    emit_method_param_conversion, emit_method_param_free, method_c_arg_names, method_param_needs_alloc,
    method_param_needs_from_json, param_zig_type_with_enums,
};
use super::render;
use super::returns::method_unwrap_return_expr;
use super::streaming::{StreamingContext, emit_opaque_streaming_method};

/// Emit a single method on an opaque handle wrapper struct.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_opaque_method(
    method: &MethodDef,
    ty: &TypeDef,
    prefix: &str,
    type_snake: &str,
    declared_errors: &[String],
    struct_names: &HashSet<String>,
    streaming_item_types: &HashMap<String, String>,
    enum_names: &HashSet<String>,
    out: &mut String,
) {
    if let Some(item_type) = streaming_item_types.get(&method.name) {
        emit_opaque_streaming_method(
            method,
            &StreamingContext {
                ty,
                prefix,
                type_snake,
                item_type,
                declared_errors,
                streaming_item_types,
            },
            out,
        );
        return;
    }

    emit_cleaned_zig_doc(out, &method.doc, "    ");

    let renamed_params = renamed_method_params(method);
    let effective_params: &[ParamDef] = &renamed_params;
    let params_str = method_params_signature(ty, effective_params, struct_names, enum_names);

    let zig_error_type = method
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));
    let return_ty = method_return_type(method, effective_params, struct_names, zig_error_type.as_ref());

    out.push_str(&render(
        "opaque_method_signature.jinja",
        minijinja::context! {
            method_name => &method.name,
            params => &params_str,
            return_ty => &return_ty,
        },
    ));
    out.push_str("        const handle = self._handle;\n");
    out.push_str("        if (handle == 0) return error.HandleClosed;\n");

    let json_error_return = if zig_error_type.is_some() {
        "return error.UnknownFfiError;".to_string()
    } else {
        "return error.InvalidJson;".to_string()
    };
    for p in effective_params {
        emit_method_param_conversion(p, prefix, struct_names, enum_names, &json_error_return, out);
    }

    let returns_bytes = return_uses_bytes_out_params(&method.return_type);
    if returns_bytes {
        out.push_str(&render("opaque_bytes_out_vars.jinja", minijinja::context! {}));
    }

    let c_call = method_c_call(
        method,
        ty,
        prefix,
        type_snake,
        effective_params,
        struct_names,
        enum_names,
    );
    emit_method_body(
        method,
        prefix,
        struct_names,
        effective_params,
        returns_bytes,
        &c_call,
        zig_error_type.as_ref(),
        out,
    );

    out.push_str("    }\n");
}

/// Emit a `free()` method that releases the underlying FFI handle by calling
/// `c.{prefix}_{snake_type}_free(self._handle)`. The C destructor is generated
/// by the FFI crate for every opaque handle type.
pub(super) fn emit_opaque_free(ty: &TypeDef, prefix: &str, type_snake: &str, out: &mut String) {
    let upper_prefix = prefix.to_uppercase();
    out.push_str(&render(
        "opaque_free_method.jinja",
        minijinja::context! {
            type_name => &ty.name,
            prefix => prefix,
            type_snake => type_snake,
            upper_prefix => &upper_prefix,
        },
    ));
}

fn renamed_method_params(method: &MethodDef) -> Vec<ParamDef> {
    method
        .params
        .iter()
        .map(|p| {
            if p.name == method.name {
                let mut p2 = p.clone();
                p2.name = "value".to_string();
                p2
            } else {
                p.clone()
            }
        })
        .collect()
}

fn method_params_signature(
    ty: &TypeDef,
    params: &[ParamDef],
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> String {
    let mut param_parts = Vec::new();
    param_parts.push(format!("self: *{}", ty.name));
    for p in params {
        let ty_str = param_zig_type_with_enums(&p.ty, p.optional, struct_names, enum_names);
        param_parts.push(format!("{}: {}", p.name, ty_str));
    }
    param_parts.join(", ")
}

fn method_return_type(
    method: &MethodDef,
    params: &[ParamDef],
    struct_names: &HashSet<String>,
    zig_error_type: Option<&String>,
) -> String {
    let body_needs_try = params.iter().any(method_param_needs_alloc)
        || matches!(
            &method.return_type,
            TypeRef::String
                | TypeRef::Char
                | TypeRef::Path
                | TypeRef::Json
                | TypeRef::Bytes
                | TypeRef::Vec(_)
                | TypeRef::Map(_, _)
        )
        || return_uses_bytes_out_params(&method.return_type)
        || matches!(&method.return_type, TypeRef::Named(name) if struct_names.contains(name));
    let body_needs_invalid_json = params.iter().any(|p| method_param_needs_from_json(p, struct_names));

    let ret_ty_inner = zig_return_type(&method.return_type, struct_names);
    if let Some(err_ty) = zig_error_type {
        format!("({err_ty}||error{{OutOfMemory,HandleClosed}})!{ret_ty_inner}")
    } else if body_needs_try || body_needs_invalid_json {
        let err_set = if body_needs_invalid_json {
            "error{OutOfMemory,InvalidJson,HandleClosed}"
        } else {
            "error{OutOfMemory,HandleClosed}"
        };
        format!("{err_set}!{ret_ty_inner}")
    } else {
        format!("error{{HandleClosed}}!{ret_ty_inner}")
    }
}

fn method_c_call(
    method: &MethodDef,
    _ty: &TypeDef,
    prefix: &str,
    type_snake: &str,
    params: &[ParamDef],
    struct_names: &HashSet<String>,
    enum_names: &HashSet<String>,
) -> String {
    let method_snake = AsSnakeCase(&method.name).to_string();
    let mut c_args = vec!["handle".to_string()];
    for p in params {
        c_args.extend(method_c_arg_names(p, struct_names, enum_names));
    }
    if return_uses_bytes_out_params(&method.return_type) {
        c_args.push("&_out_ptr".to_string());
        c_args.push("&_out_len".to_string());
        c_args.push("&_out_cap".to_string());
    }
    format!(
        "c.{prefix}_{type_snake}_{method_snake}({args})",
        args = c_args.join(", ")
    )
}

#[allow(clippy::too_many_arguments)]
fn emit_method_body(
    method: &MethodDef,
    prefix: &str,
    struct_names: &HashSet<String>,
    params: &[ParamDef],
    returns_bytes: bool,
    c_call: &str,
    zig_error_type: Option<&String>,
    out: &mut String,
) {
    if let Some(err_ty) = zig_error_type {
        emit_fallible_method_body(method, prefix, struct_names, params, returns_bytes, c_call, err_ty, out);
    } else {
        emit_infallible_method_body(method, prefix, struct_names, params, returns_bytes, c_call, out);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_fallible_method_body(
    method: &MethodDef,
    prefix: &str,
    struct_names: &HashSet<String>,
    params: &[ParamDef],
    returns_bytes: bool,
    c_call: &str,
    err_ty: &str,
    out: &mut String,
) {
    let has_return_value = !(matches!(method.return_type, TypeRef::Unit) || returns_bytes);
    if !has_return_value {
        out.push_str(&render(
            "opaque_method_call_discard.jinja",
            minijinja::context! {
                c_call => c_call,
            },
        ));
    } else {
        out.push_str(&render(
            "opaque_method_call_result.jinja",
            minijinja::context! {
                c_call => c_call,
            },
        ));
    }

    emit_consumed_receiver_invalidation(method, out);

    out.push_str(&render(
        "opaque_method_error_check.jinja",
        minijinja::context! {
            prefix => prefix,
            error_type => err_ty,
        },
    ));

    for p in params {
        emit_method_param_free(p, struct_names);
    }

    if returns_bytes {
        out.push_str(&render(
            "opaque_bytes_return.jinja",
            minijinja::context! {
                prefix => prefix,
                is_optional => matches!(method.return_type, TypeRef::Optional(_)),
            },
        ));
    } else if !matches!(method.return_type, TypeRef::Unit) {
        let ret_expr = method_unwrap_return_expr("_result", &method.return_type, prefix, struct_names);
        out.push_str(&render(
            "opaque_method_return.jinja",
            minijinja::context! {
                ret_expr => &ret_expr,
            },
        ));
    }
}

fn emit_infallible_method_body(
    method: &MethodDef,
    prefix: &str,
    struct_names: &HashSet<String>,
    params: &[ParamDef],
    returns_bytes: bool,
    c_call: &str,
    out: &mut String,
) {
    for p in params {
        emit_method_param_free(p, struct_names);
    }
    if returns_bytes {
        out.push_str(&render(
            "opaque_method_call_discard.jinja",
            minijinja::context! {
                c_call => c_call,
            },
        ));
        emit_consumed_receiver_invalidation(method, out);
        out.push_str(&render(
            "opaque_bytes_return.jinja",
            minijinja::context! {
                prefix => prefix,
                is_optional => matches!(method.return_type, TypeRef::Optional(_)),
            },
        ));
    } else if matches!(method.return_type, TypeRef::Unit) {
        out.push_str(&render(
            "opaque_method_unit_call.jinja",
            minijinja::context! {
                c_call => c_call,
            },
        ));
        emit_consumed_receiver_invalidation(method, out);
    } else {
        out.push_str(&render(
            "opaque_method_call_result.jinja",
            minijinja::context! {
                c_call => c_call,
            },
        ));
        emit_consumed_receiver_invalidation(method, out);
        let ret_expr = method_unwrap_return_expr("_result", &method.return_type, prefix, struct_names);
        out.push_str(&render(
            "opaque_method_return.jinja",
            minijinja::context! {
                ret_expr => &ret_expr,
            },
        ));
    }
}

fn emit_consumed_receiver_invalidation(method: &MethodDef, out: &mut String) {
    if method.receiver == Some(ReceiverKind::Owned) {
        out.push_str(&render(
            "opaque_consumed_handle_invalidate.jinja",
            minijinja::context! {},
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test: an infallible method (no declared Rust error type) returning `Char`
    /// generates a body that copies via `try std.heap.c_allocator.dupe` (see
    /// `method_unwrap_return_expr`'s owned-copy arm), so the declared error set must include
    /// `OutOfMemory` — matching how `String` is already covered — or the generated method
    /// signature and body disagree on whether the call can fail. ~keep
    #[test]
    fn method_return_type_includes_out_of_memory_for_infallible_char_return() {
        let method = MethodDef {
            name: "first_char".to_string(),
            return_type: TypeRef::Char,
            ..MethodDef::default()
        };

        let return_ty = method_return_type(&method, &[], &HashSet::new(), None);

        assert!(
            return_ty.contains("OutOfMemory"),
            "Char return must need OutOfMemory in the error set. Got: {return_ty}"
        );
        assert_eq!(return_ty, "error{OutOfMemory,HandleClosed}![]u8");
    }
}
