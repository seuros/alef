use crate::core::ir::{FunctionDef, ParamDef, PrimitiveType, TypeRef};

use super::errors::resolve_zig_error_type;
use super::helpers::emit_cleaned_zig_doc;
use super::types::{c_symbol_component, zig_field_type};

/// Returns true if `ty` (or its `Optional<>` inner) is a struct named in
/// `struct_names`. Struct parameters are passed across the FFI as opaque
/// handles, so the wrapper accepts a JSON `[]const u8` and converts to the
/// handle via the FFI's `<prefix>_<snake>_from_json` helper.
fn is_struct_named(ty: &TypeRef, struct_names: &std::collections::HashSet<String>) -> bool {
    match ty {
        TypeRef::Named(name) => struct_names.contains(name),
        TypeRef::Optional(inner) => is_struct_named(inner, struct_names),
        _ => false,
    }
}

/// Return the inner `Named(name)` for a struct parameter type.
fn struct_named_inner(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => struct_named_inner(inner),
        _ => None,
    }
}

/// Like `struct_named_inner` but searches for any Named type (used for opaque handle detection).
/// Returns the type name if `ty` (or its Optional inner) is a Named type.
pub(crate) fn opaque_type_name_inner(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name.as_str()),
        TypeRef::Optional(inner) => opaque_type_name_inner(inner),
        _ => None,
    }
}

/// Returns the opaque type name if `ty` is (or wraps in Optional) a Named type
/// that is in `opaque_creator_map`.
fn get_opaque_named<'a>(
    ty: &'a TypeRef,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> Option<&'a str> {
    match ty {
        TypeRef::Named(name) if opaque_creator_map.contains_key(name.as_str()) => Some(name.as_str()),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) if opaque_creator_map.contains_key(name.as_str()) => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// Returns true if generating the param-conversion boilerplate for `p` will
/// emit a `try` expression (heap allocation or fallible operation).
fn needs_alloc_param(p: &ParamDef) -> bool {
    let inner = match &p.ty {
        TypeRef::Optional(t) => t.as_ref(),
        other => other,
    };
    matches!(
        inner,
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) | TypeRef::Named(_)
    )
}

fn needs_from_json_param(
    p: &ParamDef,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> bool {
    get_opaque_named(&p.ty, opaque_creator_map).is_some() || is_struct_named(&p.ty, struct_names)
}

fn return_type_can_be_null(ty: &TypeRef, _struct_names: &std::collections::HashSet<String>) -> bool {
    match ty {
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Path
        | TypeRef::Json
        | TypeRef::Bytes
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _) => true,
        // `Bytes` is deliberately absent here: unlike String/Path/Json/Vec/Map, an
        // `Optional<Bytes>` return gets no `_len()` companion (see `returns_c_char` in the
        // FFI backend) and its `None` case never sets the FFI last-error state — so treating
        // a null as an error here would misclassify a legitimate `None` as a failure.
        // `Optional<Bytes>` instead rides the byte-buffer out-param convention (see
        // `return_uses_bytes_out_params`), where the C function returns an `i32` status and
        // absence arrives as a null `_out_ptr`; that makes `result_is_pointer` false, so this
        // predicate is never consulted for it. It stays excluded as a guard against a future
        // change routing it back through the pointer-return path. ~keep
        TypeRef::Optional(inner) => matches!(
            inner.as_ref(),
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _)
        ),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_function(
    f: &FunctionDef,
    prefix: &str,
    declared_errors: &[String],
    top_level_names: &std::collections::HashSet<String>,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    capsule_types: &std::collections::HashMap<String, crate::core::config::HostCapsuleTypeConfig>,
    out: &mut String,
) {
    // `opaque_type_name_inner` matches both bare `Named(name)` and `Optional(Named(name))` —
    // capsule returns share one raw C ABI (`*const T`) in both cases, see
    // `backends::ffi::gen_bindings::capsule::capsule_c_return_type` and `capsule_return_name`.
    // ~keep
    if let Some(name) = opaque_type_name_inner(&f.return_type)
        && let Some(cap) = capsule_types.get(name)
    {
        emit_capsule_function(f, prefix, struct_names, opaque_creator_map, cap, declared_errors, out);
        return;
    }

    emit_cleaned_zig_doc(out, &f.doc, "");

    let renamed_params: Vec<ParamDef> = f
        .params
        .iter()
        .map(|p| {
            let mut p2 = p.clone();
            if top_level_names.contains(&p2.name) {
                p2.name = format!("{}_arg", p2.name);
            }
            p2
        })
        .collect();
    let f_local = FunctionDef {
        params: renamed_params,
        ..f.clone()
    };
    let f = &f_local;

    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format_param_wrapper(p, struct_names, opaque_creator_map))
        .collect();

    let zig_error_type = f
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));

    let body_needs_try = f.params.iter().any(needs_alloc_param)
        || matches!(
            &f.return_type,
            TypeRef::String
                | TypeRef::Char
                | TypeRef::Path
                | TypeRef::Json
                | TypeRef::Bytes
                | TypeRef::Vec(_)
                | TypeRef::Map(_, _)
        )
        || return_uses_bytes_out_params(&f.return_type)
        || matches!(&f.return_type, TypeRef::Named(name) if struct_names.contains(name));
    let body_needs_invalid_json = f
        .params
        .iter()
        .any(|p| needs_from_json_param(p, struct_names, opaque_creator_map));

    let return_ty = if let Some(error_type) = &zig_error_type {
        format!("{}!{}", error_type, zig_return_type(&f.return_type, struct_names))
    } else if body_needs_try || body_needs_invalid_json {
        let err_set = if body_needs_invalid_json {
            "error{OutOfMemory,InvalidJson}"
        } else {
            "error{OutOfMemory}"
        };
        format!("{err_set}!{}", zig_return_type(&f.return_type, struct_names))
    } else {
        zig_return_type(&f.return_type, struct_names)
    };

    out.push_str(&crate::backends::zig::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            func_name => &f.name,
            params => params.join(", "),
            return_ty => &return_ty,
        },
    ));

    let json_error_return = zig_error_type
        .as_ref()
        .map_or("return error.InvalidJson;".to_string(), |err| {
            format!("return _error_with_message({err});")
        });
    for p in &f.params {
        emit_param_conversion(p, prefix, struct_names, opaque_creator_map, &json_error_return, out);
    }

    let returns_bytes = return_uses_bytes_out_params(&f.return_type);
    let returns_optional_bytes = returns_bytes && matches!(f.return_type, TypeRef::Optional(_));
    if returns_bytes {
        out.push_str("    var _out_ptr: [*c]u8 = undefined;\n");
        out.push_str("    var _out_len: usize = 0;\n");
        out.push_str("    var _out_cap: usize = 0;\n");
    }

    let mut c_args: Vec<String> = f
        .params
        .iter()
        .flat_map(|p| c_arg_names(p, struct_names, opaque_creator_map))
        .collect();
    if returns_bytes {
        c_args.push("&_out_ptr".to_string());
        c_args.push("&_out_len".to_string());
        c_args.push("&_out_cap".to_string());
    }
    let c_call = format!("c.{prefix}_{}({})", f.name, c_args.join(", "));
    let returns_c_char_like = return_uses_len_companion(&f.return_type);
    let c_len_call = if returns_c_char_like {
        Some(format!("c.{prefix}_{}_len({})", f.name, c_args.join(", ")))
    } else {
        None
    };

    if let Some(error_type) = &zig_error_type {
        let result_is_pointer = !(matches!(f.return_type, TypeRef::Unit) || returns_bytes);
        let result_can_be_null = return_type_can_be_null(&f.return_type, struct_names);
        if !result_is_pointer {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_unit.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
        } else {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_result.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
        }
        if result_is_pointer {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_check.jinja",
                minijinja::context! {
                    prefix => prefix,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_return.jinja",
                minijinja::context! {
                    error_type => error_type,
                },
            ));
            out.push_str("    }\n");
            if result_can_be_null {
                out.push_str("    if (_result == null) {\n");
                out.push_str(&crate::backends::zig::template_env::render(
                    "function_error_return.jinja",
                    minijinja::context! {
                        error_type => error_type,
                    },
                ));
                out.push_str("    }\n");
            }
        } else {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_check.jinja",
                minijinja::context! {
                    prefix => prefix,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "function_error_return.jinja",
                minijinja::context! {
                    error_type => error_type,
                },
            ));
            out.push_str("    }\n");
        }
        if let Some(len_call) = &c_len_call {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_result_len.jinja",
                minijinja::context! {
                    len_call => len_call,
                },
            ));
        }

        for p in &f.params {
            emit_param_free(p, prefix, struct_names, opaque_creator_map, out);
        }

        if returns_bytes {
            if returns_optional_bytes {
                out.push_str("    if (_out_ptr == null) return null;\n");
            }
            out.push_str("    const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);\n");
            out.push_str(&crate::backends::zig::template_env::render(
                "function_free_bytes.jinja",
                minijinja::context! {
                    prefix => prefix,
                },
            ));
            out.push_str("    return _owned;\n");
        } else if matches!(f.return_type, TypeRef::Unit) {
            out.push_str("    return;\n");
        } else {
            let ret_expr = unwrap_return_expr(
                "_result",
                &f.return_type,
                prefix,
                struct_names,
                Some(error_type.as_str()),
            );
            out.push_str(&crate::backends::zig::template_env::render(
                "function_return.jinja",
                minijinja::context! {
                    ret_expr => ret_expr,
                },
            ));
        }
    } else {
        for p in &f.params {
            emit_param_free(p, prefix, struct_names, opaque_creator_map, out);
        }
        if returns_bytes {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_unit.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
            if returns_optional_bytes {
                out.push_str("    if (_out_ptr == null) return null;\n");
            }
            out.push_str("    const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);\n");
            out.push_str(&crate::backends::zig::template_env::render(
                "function_free_bytes.jinja",
                minijinja::context! {
                    prefix => prefix,
                },
            ));
            out.push_str("    return _owned;\n");
        } else if matches!(f.return_type, TypeRef::Unit) {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_unit.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
        } else {
            out.push_str(&crate::backends::zig::template_env::render(
                "function_call_result.jinja",
                minijinja::context! {
                    c_call => &c_call,
                },
            ));
            if let Some(len_call) = &c_len_call {
                out.push_str(&crate::backends::zig::template_env::render(
                    "function_result_len.jinja",
                    minijinja::context! {
                        len_call => len_call,
                    },
                ));
            }
            let ret_expr = unwrap_return_expr("_result", &f.return_type, prefix, struct_names, None);
            out.push_str(&crate::backends::zig::template_env::render(
                "function_return.jinja",
                minijinja::context! {
                    ret_expr => ret_expr,
                },
            ));
        }
    }

    out.push_str("}\n");
}

/// Emit a Zig wrapper for a function returning a host-native capsule (Language) type.
///
/// The C symbol returns the host runtime's raw grammar pointer; the wrapper constructs the
/// host `Language` using the expression from `cap.construct_expr`.
///
/// `cap.host_type` and `cap.construct_expr` are required; missing values produce a
/// `// ALEF ERROR:` comment in the generated output rather than silently falling
/// back to a hardcoded default.
fn emit_capsule_function(
    f: &FunctionDef,
    prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    cap: &crate::core::config::HostCapsuleTypeConfig,
    declared_errors: &[String],
    out: &mut String,
) {
    emit_cleaned_zig_doc(out, &f.doc, "");

    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format_param_wrapper(p, struct_names, opaque_creator_map))
        .collect();

    let body_needs_try = f.params.iter().any(needs_alloc_param);
    let host_type = match cap.required_host_type("Language", "zig") {
        Ok(t) => t.to_string(),
        Err(e) => {
            out.push_str(&format!("// ALEF ERROR: {e}\n"));
            return;
        }
    };
    let zig_error_type = f
        .error_type
        .as_ref()
        .map(|e| resolve_zig_error_type(e, declared_errors));
    let return_ty = if let Some(err) = &zig_error_type {
        format!("{err}!{host_type}")
    } else if body_needs_try {
        format!("error{{OutOfMemory}}!{host_type}")
    } else {
        host_type.clone()
    };

    out.push_str(&crate::backends::zig::template_env::render(
        "function_signature.jinja",
        minijinja::context! {
            func_name => &f.name,
            params => params.join(", "),
            return_ty => &return_ty,
        },
    ));

    for p in &f.params {
        emit_param_conversion(
            p,
            prefix,
            struct_names,
            opaque_creator_map,
            "return error.OutOfMemory;",
            out,
        );
    }

    let c_args: Vec<String> = f
        .params
        .iter()
        .flat_map(|p| c_arg_names(p, struct_names, opaque_creator_map))
        .collect();
    let c_call = format!("c.{prefix}_{}({})", f.name, c_args.join(", "));
    out.push_str(&crate::backends::zig::template_env::render(
        "function_call_result.jinja",
        minijinja::context! {
            c_call => &c_call,
        },
    ));

    for p in &f.params {
        emit_param_free(p, prefix, struct_names, opaque_creator_map, out);
    }

    if let Some(error_type) = &zig_error_type {
        out.push_str(&crate::backends::zig::template_env::render(
            "function_error_check.jinja",
            minijinja::context! { prefix => prefix },
        ));
        out.push_str(&crate::backends::zig::template_env::render(
            "function_error_return.jinja",
            minijinja::context! { error_type => error_type },
        ));
        out.push_str("    }\n");
    }

    out.push_str("    if (_result == null) return null;\n");
    let construct = match cap.construct_required("_result", "Language", "zig") {
        Ok(c) => c,
        Err(e) => {
            out.push_str(&format!("    // ALEF ERROR: {e}\n"));
            out.push_str("}\n");
            return;
        }
    };
    out.push_str(&format!("    return {construct};\n"));
    out.push_str("}\n");
}

/// Return the Zig-wrapper parameter type string for a function parameter.
fn format_param_wrapper(
    p: &ParamDef,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> String {
    let ty_str = zig_param_type(&p.ty, p.optional, struct_names, opaque_creator_map);
    format!("{}: {}", p.name, ty_str)
}

/// Zig type used at the wrapper boundary for a function parameter.
///
/// - `String`, `Path` → `[]const u8`  (body allocates null-terminated copy)
/// - `Bytes`          → `[]const u8`  (body passes `.ptr` + `.len`)
/// - `Vec`, `Map`     → `[]const u8`  (caller supplies JSON; body passes as C string)
/// - `Named` struct   → `[]const u8`  (caller supplies JSON; body converts to opaque
///   handle via the FFI `<prefix>_<snake>_from_json` helper)
/// - Everything else  → same as struct-field type
fn zig_param_type(
    ty: &TypeRef,
    optional: bool,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> String {
    if get_opaque_named(ty, opaque_creator_map).is_some() {
        return "?[]const u8".to_string();
    }
    let inner = match ty {
        TypeRef::String | TypeRef::Path | TypeRef::Bytes | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            "[]const u8".to_string()
        }
        TypeRef::Named(name) if struct_names.contains(name) => "[]const u8".to_string(),
        TypeRef::Optional(inner) => {
            let inner_str = zig_param_type(inner, false, struct_names, opaque_creator_map);
            return format!("?{inner_str}");
        }
        other => zig_field_type(other, false),
    };
    if optional { format!("?{inner}") } else { inner }
}

/// Emit the allocation / conversion lines needed before the C call for `p`.
///
/// String/Path: allocate a null-terminated copy via `std.heap.c_allocator`.
/// Vec/Map:     same — caller supplies a JSON `[]const u8`; we need a sentinel-
///              terminated copy to pass to `*const c_char` parameters.
/// Named struct (opt or required): caller supplies JSON `[]const u8`; we
///              allocate a sentinel-terminated copy and convert it to an
///              opaque FFI handle via `<prefix>_<snake>_from_json`. The
///              optional variant unwraps the optional first and substitutes
///              `null` for the C handle when the wrapper arg is `null`.
/// Bytes:       nothing needed; `.ptr` and `.len` are used directly in `c_arg_names`.
fn emit_param_conversion(
    p: &ParamDef,
    prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    json_error_return: &str,
    out: &mut String,
) {
    let name = &p.name;

    if let Some(opaque_name) = get_opaque_named(&p.ty, opaque_creator_map) {
        if let Some((creator_fn, config_snake)) = opaque_creator_map.get(opaque_name) {
            out.push_str(&crate::backends::zig::template_env::render(
                "param_opaque_config_from_json.jinja",
                minijinja::context! {
                    name => name,
                    prefix => prefix,
                    creator_fn => creator_fn,
                    config_snake => config_snake,
                    name_snake => &c_symbol_component(opaque_name),
                    json_error_return => json_error_return,
                },
            ));
        }
        return;
    }

    if let Some(inner_name) = struct_named_inner(&p.ty)
        && struct_names.contains(inner_name)
    {
        let snake = c_symbol_component(inner_name);
        let is_optional = p.optional || matches!(p.ty, TypeRef::Optional(_));
        if is_optional {
            out.push_str(&crate::backends::zig::template_env::render(
                "param_optional_string_alloc.jinja",
                minijinja::context! { name => name },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_optional_struct_handle.jinja",
                minijinja::context! {
                    name => name,
                    prefix => prefix,
                    snake => &snake,
                    json_error_return => json_error_return,
                },
            ));
        } else {
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line1.jinja",
                minijinja::context! { name => name },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line2.jinja",
                minijinja::context! { name => name },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_struct_handle.jinja",
                minijinja::context! {
                    name => name,
                    prefix => prefix,
                    snake => &snake,
                    json_error_return => json_error_return,
                },
            ));
        }
        return;
    }
    let is_optional_string = p.optional
        || matches!(
                &p.ty,
                TypeRef::Optional(inner)
                    if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        );
    if is_optional_string && matches!(unwrap_optional(&p.ty), TypeRef::String | TypeRef::Path) {
        out.push_str(&crate::backends::zig::template_env::render(
            "param_optional_string_alloc.jinja",
            minijinja::context! { name => name },
        ));
        return;
    }
    match &p.ty {
        TypeRef::String | TypeRef::Path => {
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line1.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line2.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
        }
        TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            out.push_str("    // Vec/Map parameters are passed as JSON strings across the FFI boundary.\n");
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line1.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
            out.push_str(&crate::backends::zig::template_env::render(
                "param_string_line2.jinja",
                minijinja::context! {
                    name => name,
                },
            ));
        }
        _ => {}
    }
}

/// Strip a single `Optional<>` layer if present.
fn unwrap_optional(ty: &TypeRef) -> &TypeRef {
    match ty {
        TypeRef::Optional(inner) => inner,
        other => other,
    }
}

/// Returns true when a return type crosses the C boundary via the byte-buffer out-param
/// convention (`&_out_ptr, &_out_len, &_out_cap` trailing args, `i32` status return)
/// instead of as a direct C return value.
///
/// `Optional<Bytes>` shares the convention with bare `Bytes`; `None` arrives as a null
/// `_out_ptr` with `_out_len == _out_cap == 0`, while `Some(&[])` arrives as a non-null
/// `_out_ptr` with `_out_len == 0`. This is why `Bytes` cannot use the `_len()` companion
/// the `Char`/`String` path uses: `Bytes` carries no NUL terminator, so its length only
/// ever exists in `*_out_len`.
///
/// Must mirror `crate::backends::ffi::gen_bindings::functions::returns_bytes_out_params`.
/// ~keep
pub(crate) fn return_uses_bytes_out_params(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Bytes => true,
        TypeRef::Optional(inner) => matches!(inner.as_ref(), TypeRef::Bytes),
        _ => false,
    }
}

/// Returns true when a return type maps to `*mut c_char` and therefore has a
/// matching `_len()` companion in alef-backend-ffi.
///
/// Must mirror `crate::backends::ffi::gen_bindings::functions::returns_c_char`.
pub(crate) fn return_uses_len_companion(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json => true,
        TypeRef::Vec(_) | TypeRef::Map(_, _) => true,
        TypeRef::Optional(inner) => matches!(
            inner.as_ref(),
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _)
        ),
        _ => false,
    }
}

/// Return the max-value sentinel literal for a primitive integer type, if one
/// is used by the C FFI to represent `None`.  The Rust FFI layer uses
/// `<Type>::MAX` as the sentinel for optional numeric primitives.
pub(super) fn optional_int_sentinel(prim: &PrimitiveType) -> Option<&'static str> {
    match prim {
        PrimitiveType::U8 => Some("std.math.maxInt(u8)"),
        PrimitiveType::U16 => Some("std.math.maxInt(u16)"),
        PrimitiveType::U32 => Some("std.math.maxInt(u32)"),
        PrimitiveType::U64 | PrimitiveType::Usize => Some("std.math.maxInt(u64)"),
        PrimitiveType::I8 => Some("std.math.maxInt(i8)"),
        PrimitiveType::I16 => Some("std.math.maxInt(i16)"),
        PrimitiveType::I32 => Some("std.math.maxInt(i32)"),
        PrimitiveType::I64 | PrimitiveType::Isize => Some("std.math.maxInt(i64)"),
        _ => None,
    }
}

/// Emit the deallocation lines for allocations made in `emit_param_conversion`.
///
/// These are emitted after the C call (and after the error check) so the
/// allocations are always freed even when an error is returned.
fn emit_param_free(
    p: &ParamDef,
    _prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
    out: &mut String,
) {
    let name = &p.name;

    if let Some(opaque_name) = get_opaque_named(&p.ty, opaque_creator_map) {
        if let Some((_, config_snake)) = opaque_creator_map.get(opaque_name) {
            let config_name = format!("{name}_config");
            out.push_str(&crate::backends::zig::template_env::render(
                "param_optional_free.jinja",
                minijinja::context! {
                    name => &config_name,
                },
            ));
            let _ = config_snake;
        }
        return;
    }

    if let Some(inner_name) = struct_named_inner(&p.ty)
        && struct_names.contains(inner_name)
    {
        let _ = inner_name;
    }
}

/// The C argument name(s) to use for a given wrapper parameter.
///
/// Bytes expand to two arguments: `.ptr` and `.len`.
/// String/Path/Vec/Map expand to the `_z` null-terminated copy.
/// Optional String/Path expand to a conditional unwrap of the optional slice
/// to its `.ptr`, substituting `null` when the wrapper arg was null — Zig
/// does not auto-coerce `?[:0]u8` into `?[*:0]const u8`.
/// Named structs expand to the `_handle` opaque pointer produced by the
/// JSON-to-handle helper in `emit_param_conversion`.
/// Everything else passes the parameter directly.
fn c_arg_names(
    p: &ParamDef,
    struct_names: &std::collections::HashSet<String>,
    opaque_creator_map: &std::collections::HashMap<String, (String, String)>,
) -> Vec<String> {
    if get_opaque_named(&p.ty, opaque_creator_map).is_some() {
        return vec![format!("{}_handle", p.name)];
    }
    if is_struct_named(&p.ty, struct_names) {
        return vec![format!("{}_handle", p.name)];
    }
    let is_optional_string = p.optional
        || matches!(
            &p.ty,
            TypeRef::Optional(inner)
                if matches!(inner.as_ref(), TypeRef::String | TypeRef::Path)
        );
    if is_optional_string && matches!(unwrap_optional(&p.ty), TypeRef::String | TypeRef::Path) {
        return vec![format!("if ({0}_z) |z| z.ptr else null", p.name)];
    }
    {
        let prim_opt = match &p.ty {
            TypeRef::Optional(inner) => {
                if let TypeRef::Primitive(prim) = inner.as_ref() {
                    Some(prim)
                } else {
                    None
                }
            }
            TypeRef::Primitive(prim) if p.optional => Some(prim),
            _ => None,
        };
        if let Some(prim) = prim_opt
            && let Some(sentinel) = optional_int_sentinel(prim)
        {
            return vec![format!("if ({name}) |v| v else {sentinel}", name = p.name)];
        }
    }
    match &p.ty {
        TypeRef::String | TypeRef::Path | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            vec![format!("{}_z", p.name)]
        }
        TypeRef::Bytes => {
            vec![format!("{}.ptr", p.name), format!("{}.len", p.name)]
        }
        _ => vec![p.name.clone()],
    }
}

/// Produce the Zig expression that converts a raw C return value (`raw`) to the
/// wrapper return type.
///
/// Bool: the C ABI represents `bool` as `i32`; Zig rejects an implicit `i32→bool`
/// coercion, so emit `_result != 0`.
/// String/Char/Path/Json/Vec/Map: copy the C string to an owned Zig slice, then free
/// the FFI allocation via `_free_string`. `Char` takes the same `*mut c_char` +
/// `_len()` convention as `String` on the FFI side, so it shares this arm.
/// Named struct (has_serde): serialize to JSON via `<prefix>_<snake>_to_json`,
/// copy the JSON string to an owned Zig slice, then free both the JSON string and
/// the opaque handle.
/// Everything else: pass through unchanged.
fn unwrap_return_expr(
    raw: &str,
    ty: &TypeRef,
    prefix: &str,
    struct_names: &std::collections::HashSet<String>,
    error_type: Option<&str>,
) -> String {
    match ty {
        TypeRef::Primitive(PrimitiveType::Bool) => format!("{raw} != 0"),
        TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
            crate::backends::zig::template_env::render(
                "return_owned_bytes_block.jinja",
                minijinja::context! {
                    raw => raw,
                    error_type => error_type,
                },
            )
        }
        TypeRef::Named(name) if struct_names.contains(name) => {
            let snake = c_symbol_component(name);
            crate::backends::zig::template_env::render(
                "return_named_json_block.jinja",
                minijinja::context! {
                    prefix => prefix,
                    snake => &snake,
                    raw => raw,
                    error_type => error_type,
                },
            )
        }
        TypeRef::Named(name) => {
            let fallback = error_type.map_or_else(
                || "error.OutOfMemory".to_string(),
                |error| format!("_first_error({error})"),
            );
            format!("blk: {{ if ({raw} == 0) return {fallback}; break :blk {name}{{ ._handle = {raw} }}; }}")
        }
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String | TypeRef::Char | TypeRef::Path | TypeRef::Json | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                crate::backends::zig::template_env::render(
                    "return_optional_owned_bytes_block.jinja",
                    minijinja::context! {
                        raw => raw,
                    },
                )
            }
            TypeRef::Named(name) if struct_names.contains(name) => {
                let snake = c_symbol_component(name);
                let inner_block = crate::backends::zig::template_env::render(
                    "return_named_json_block.jinja",
                    minijinja::context! {
                        prefix => prefix,
                        snake => &snake,
                        raw => raw,
                        error_type => error_type,
                    },
                );
                format!("if ({raw} == 0) null else {inner_block}")
            }
            TypeRef::Named(name) => {
                format!("if ({raw} == 0) null else {name}{{ ._handle = {raw} }}")
            }
            _ => raw.to_string(),
        },
        _ => raw.to_string(),
    }
}

/// Build the Zig return type for a function (not for struct fields).
///
/// Owned string/char/JSON/collection returns are `[]u8` (allocated slice) — `Char`
/// crosses the FFI exactly like `String` (see `returns_c_char` in the FFI backend),
/// so it takes the same owned-slice shape rather than the `[]const u8` a struct
/// field would get.
/// `Bytes` returns are `[]u8` — the FFI uses the out-param convention
/// (`uint8_t **out_ptr, uintptr_t *out_len, uintptr_t *out_cap`) and the
/// wrapper copies the bytes into a caller-owned heap allocation. `Optional<Bytes>`
/// is `?[]u8` (owned, not the `[]const u8` a struct field would get) and rides the
/// same out-param convention with a null `out_ptr` standing for `None`.
/// Named struct returns (opaque C handles) are also serialized to `[]u8` (JSON).
/// Everything else matches the struct-field mapping.
pub(crate) fn zig_return_type(ty: &TypeRef, struct_names: &std::collections::HashSet<String>) -> String {
    match ty {
        TypeRef::String
        | TypeRef::Char
        | TypeRef::Path
        | TypeRef::Json
        | TypeRef::Bytes
        | TypeRef::Vec(_)
        | TypeRef::Map(_, _) => "[]u8".to_string(),
        TypeRef::Named(name) if struct_names.contains(name) => "[]u8".to_string(),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::String
            | TypeRef::Char
            | TypeRef::Path
            | TypeRef::Json
            | TypeRef::Bytes
            | TypeRef::Vec(_)
            | TypeRef::Map(_, _) => "?[]u8".to_string(),
            TypeRef::Named(name) if struct_names.contains(name) => "?[]u8".to_string(),
            other => zig_field_type(other, true),
        },
        other => zig_field_type(other, false),
    }
}

#[cfg(test)]
mod capsule_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_opaque_handle_return_wraps_into_struct_instead_of_bare_raw_value() {
        let expr = unwrap_return_expr(
            "_result",
            &TypeRef::Optional(Box::new(TypeRef::Named("NodeHandle".to_string()))),
            "sample",
            &std::collections::HashSet::new(),
            None,
        );

        assert_eq!(expr, "if (_result == 0) null else NodeHandle{ ._handle = _result }");
    }

    /// Positive control: an `Optional<primitive>` has no null/zero sentinel to translate and
    /// must stay a bare passthrough. Guards against a fix that wraps every `Optional<_>`
    /// unconditionally, which would make the assertion above pass for the wrong reason.
    #[test]
    fn optional_primitive_return_stays_a_bare_passthrough() {
        let expr = unwrap_return_expr(
            "_result",
            &TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::I64))),
            "sample",
            &std::collections::HashSet::new(),
            None,
        );

        assert_eq!(expr, "_result");
    }

    /// Regression test: `Char` crosses the C ABI exactly like `String` (a `*mut c_char` +
    /// `_len()` companion, see `returns_c_char` in the FFI backend), so the wrapper must copy
    /// and free it the same way instead of falling through to a bare passthrough of a pointer
    /// the declared `[]u8` return type cannot represent. ~keep
    #[test]
    fn char_return_copies_and_frees_like_string_instead_of_bare_passthrough() {
        let expr = unwrap_return_expr(
            "_result",
            &TypeRef::Char,
            "sample",
            &std::collections::HashSet::new(),
            None,
        );

        assert!(expr.contains("_free_string(_result)"), "{expr}");
        assert!(expr.contains("std.heap.c_allocator.dupe(u8, slice)"), "{expr}");
        assert_ne!(expr, "_result");
    }

    #[test]
    fn optional_char_return_degrades_to_null_instead_of_bare_passthrough() {
        let expr = unwrap_return_expr(
            "_result",
            &TypeRef::Optional(Box::new(TypeRef::Char)),
            "sample",
            &std::collections::HashSet::new(),
            None,
        );

        assert!(expr.contains("if (_result == null) break :blk null;"), "{expr}");
        assert!(expr.contains("_free_string(_result)"), "{expr}");
        assert_ne!(expr, "_result");
    }

    #[test]
    fn zig_return_type_char_is_owned_slice_not_field_style_const_slice() {
        assert_eq!(
            zig_return_type(&TypeRef::Char, &std::collections::HashSet::new()),
            "[]u8"
        );
        assert_eq!(
            zig_return_type(
                &TypeRef::Optional(Box::new(TypeRef::Char)),
                &std::collections::HashSet::new()
            ),
            "?[]u8"
        );
    }

    #[test]
    fn return_uses_len_companion_covers_char_like_string() {
        assert!(return_uses_len_companion(&TypeRef::Char));
        assert!(return_uses_len_companion(&TypeRef::Optional(Box::new(TypeRef::Char))));
    }

    /// `Optional<Bytes>` gets no `_len()` companion (unlike Optional<String/Char/Path/Json/Vec/Map>)
    /// and its `None` case never sets the FFI last-error state, so treating its null as an
    /// FFI-call error here would misclassify a legitimate `None` as failure. This must stay
    /// `false` even though bare `Bytes` (routed entirely through the separate out-param
    /// convention) is `true`. ~keep
    #[test]
    fn return_type_can_be_null_excludes_optional_bytes_but_includes_optional_char() {
        assert!(!return_type_can_be_null(
            &TypeRef::Optional(Box::new(TypeRef::Bytes)),
            &std::collections::HashSet::new()
        ));
        assert!(return_type_can_be_null(
            &TypeRef::Optional(Box::new(TypeRef::Char)),
            &std::collections::HashSet::new()
        ));
    }

    fn char_return_fn() -> FunctionDef {
        FunctionDef {
            name: "first_char".to_string(),
            rust_path: "sample::first_char".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type: TypeRef::Char,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    /// End-to-end regression: every piece touched by the `Char` return-shape fix must agree —
    /// the `_len()` companion call, the `[]u8` declared type, the `OutOfMemory` error set (the
    /// generated body's `try std.heap.c_allocator.dupe` requires it even with no declared Rust
    /// error type), and the owned-copy-then-free body. Each was a separate missing arm before
    /// this fix; asserting them together in one generated function is what proves the whole
    /// pipeline — not just one function in isolation — now agrees for `Char`. ~keep
    #[test]
    fn emit_function_char_return_wires_len_companion_error_set_and_owned_copy_together() {
        let f = char_return_fn();
        let mut out = String::new();
        emit_function(
            &f,
            "sample",
            &[],
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &mut out,
        );

        assert!(
            out.contains("error{OutOfMemory}![]u8"),
            "Char return must declare an owned []u8 in an OutOfMemory-capable error set. Got:\n{out}"
        );
        assert!(
            out.contains("c.sample_first_char_len("),
            "Char return must call its FFI _len() companion. Got:\n{out}"
        );
        assert!(
            out.contains("_free_string(_result)"),
            "Char return must free the FFI allocation. Got:\n{out}"
        );
        assert!(
            out.contains("std.heap.c_allocator.dupe(u8, slice)"),
            "Char return must copy into an owned, caller-freed slice. Got:\n{out}"
        );
        assert!(
            !out.contains("return _result;"),
            "Char return must not fall through to a bare pointer passthrough. Got:\n{out}"
        );
    }

    fn bytes_return_fn(return_type: TypeRef) -> FunctionDef {
        FunctionDef {
            name: "maybe_thumbnail".to_string(),
            rust_path: "sample::maybe_thumbnail".to_string(),
            original_rust_path: String::new(),
            params: vec![],
            return_type,
            is_async: false,
            error_type: None,
            doc: String::new(),
            cfg: None,
            sanitized: false,
            return_sanitized: false,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn emit_bytes_fn(return_type: TypeRef) -> String {
        let f = bytes_return_fn(return_type);
        let mut out = String::new();
        emit_function(
            &f,
            "sample",
            &[],
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
            &mut out,
        );
        out
    }

    #[test]
    fn zig_return_type_optional_bytes_is_an_owned_optional_slice() {
        assert_eq!(
            zig_return_type(
                &TypeRef::Optional(Box::new(TypeRef::Bytes)),
                &std::collections::HashSet::new()
            ),
            "?[]u8"
        );
    }

    /// The `Char` fix routed through the `_len()` companion because a NUL-terminated
    /// `c_char` has a recoverable length. `Bytes` has no terminator, so `Optional<Bytes>`
    /// must take the out-param convention instead — and must specifically NOT acquire a
    /// `_len()` companion, which the FFI backend does not emit for it. ~keep
    #[test]
    fn optional_bytes_takes_out_params_not_the_len_companion_char_uses() {
        assert!(return_uses_bytes_out_params(&TypeRef::Bytes));
        assert!(return_uses_bytes_out_params(&TypeRef::Optional(Box::new(
            TypeRef::Bytes
        ))));
        assert!(!return_uses_bytes_out_params(&TypeRef::Optional(Box::new(
            TypeRef::Char
        ))));
        assert!(!return_uses_len_companion(&TypeRef::Optional(Box::new(TypeRef::Bytes))));
    }

    /// End-to-end: the declared `?[]u8` and the emitted body must agree. The body reads the
    /// length from `_out_len` (there is no `_result_len` to read — `Optional<Bytes>` has no
    /// `_len()` companion) and treats a null `_out_ptr` as the `None` encoding. ~keep
    #[test]
    fn emit_function_optional_bytes_return_reads_out_len_and_maps_null_ptr_to_null() {
        let out = emit_bytes_fn(TypeRef::Optional(Box::new(TypeRef::Bytes)));

        assert!(
            out.contains("error{OutOfMemory}!?[]u8"),
            "Optional<Bytes> must declare an owned ?[]u8. Got:\n{out}"
        );
        assert!(
            out.contains("    var _out_ptr: [*c]u8 = undefined;\n"),
            "Optional<Bytes> must declare the byte-buffer out-param locals. Got:\n{out}"
        );
        assert!(
            out.contains("    _ = c.sample_maybe_thumbnail(&_out_ptr, &_out_len, &_out_cap);\n"),
            "Optional<Bytes> must pass the three out-params and discard the status. Got:\n{out}"
        );
        assert!(
            out.contains("    if (_out_ptr == null) return null;\n"),
            "Optional<Bytes> must map a null out_ptr to Zig null. Got:\n{out}"
        );
        assert!(
            out.contains("    const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);\n"),
            "Optional<Bytes> must copy _out_len bytes into an owned slice. Got:\n{out}"
        );
        assert!(
            out.contains("    c.sample_free_bytes(_out_ptr, _out_len, _out_cap);\n"),
            "Optional<Bytes> must release the FFI buffer via free_bytes. Got:\n{out}"
        );
        assert!(
            !out.contains("_result_len"),
            "Optional<Bytes> must not reference the _len() companion's _result_len. Got:\n{out}"
        );
        assert!(
            !out.contains("c.sample_maybe_thumbnail_len("),
            "Optional<Bytes> must not call a _len() companion the FFI side never emits. Got:\n{out}"
        );
        assert!(
            !out.contains("return _result;"),
            "Optional<Bytes> must not fall through to a bare pointer passthrough. Got:\n{out}"
        );
    }

    /// Present-but-empty (`Some(&[])`) must not be read as absent. The only absence test in
    /// the emitted body is the null-pointer check, which runs before the slice is taken; a
    /// zero `_out_len` simply yields an empty owned slice. ~keep
    #[test]
    fn emit_function_optional_bytes_treats_zero_length_as_present_not_absent() {
        let out = emit_bytes_fn(TypeRef::Optional(Box::new(TypeRef::Bytes)));

        assert!(
            !out.contains("_out_len == 0"),
            "a zero length must never be read as absence — only a null pointer is. Got:\n{out}"
        );
        let null_check = out
            .find("if (_out_ptr == null) return null;")
            .expect("null check must be emitted");
        let slice_copy = out
            .find("std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len])")
            .expect("owned copy must be emitted");
        assert!(
            null_check < slice_copy,
            "the null check must precede the slice so an empty present value still copies. Got:\n{out}"
        );
    }

    /// Positive control: bare `Bytes` has no absent case, so it must NOT gain the null check.
    /// Without this, the assertions above would pass for a fix that emitted the check
    /// unconditionally for every byte-buffer return. ~keep
    #[test]
    fn emit_function_bare_bytes_return_keeps_no_absence_check() {
        let out = emit_bytes_fn(TypeRef::Bytes);

        assert!(
            out.contains("error{OutOfMemory}![]u8"),
            "bare Bytes must still declare a non-optional []u8. Got:\n{out}"
        );
        assert!(
            !out.contains("if (_out_ptr == null) return null;"),
            "bare Bytes has no absent case and must not emit the null check. Got:\n{out}"
        );
        assert!(
            out.contains("    const _owned = try std.heap.c_allocator.dupe(u8, _out_ptr[0.._out_len]);\n"),
            "bare Bytes must keep its owned copy. Got:\n{out}"
        );
    }
}
