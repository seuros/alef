use crate::backends::ffi::type_map::{c_return_type_with_paths, is_passthrough_return, is_void_return};
use crate::codegen::conversions::core_type_path;
use crate::core::ir::{CoreWrapper, FunctionDef, MethodDef, ReceiverKind, TypeDef, TypeRef};
use ahash::{AHashMap, AHashSet};
use minijinja::context;

use super::super::helpers::{
    ffi_null_return_value, gen_ffi_unimplemented_body, gen_owned_value_to_c, null_return_value,
};
use super::params::{ParamConversionContext, gen_param_conversion_with_enums};
use super::return_handling::{gen_owned_c_char_to_c_with_len, return_type_needs_non_serde_named, returns_c_char};
use super::signatures::{c_symbol_component, internal_class_component, is_owned_default_constructor};
use super::support::{ffi_doxygen_block, method_sanitized_recoverable, sanitized_recoverable};

fn named_handle_type(ty: &TypeRef) -> Option<&str> {
    match ty {
        TypeRef::Named(name) => Some(name),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(name) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

fn named_type_path(type_name: &str, core_import: &str, path_map: &AHashMap<String, String>) -> String {
    path_map
        .get(type_name)
        .filter(|path| !path.is_empty())
        .cloned()
        .unwrap_or_else(|| format!("{core_import}::{type_name}"))
}

pub(in crate::backends::ffi::gen_bindings) fn gen_streaming_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    prefix: &str,
    core_import: &str,
    body: &str,
) -> String {
    let type_snake = c_symbol_component(&typ.name);
    let method_name = &method.name;
    let fn_name = format!("{prefix}_{type_snake}_{method_name}");
    let qualified = core_type_path(typ, core_import);
    let callback_type = format!("{}StreamCallback", internal_class_component(prefix));

    let doc_comment = ffi_doxygen_block(&method.doc);

    let body_indented = format!(" {}", body.replace('\n', "\n "));

    crate::backends::ffi::template_env::render(
        "streaming_method_wrapper.jinja",
        minijinja::context! {
            doc_comment => doc_comment.trim_end(),
            fn_name => fn_name,
            qualified => qualified,
            callback_type => callback_type,
            body_indented => body_indented,
        },
    )
}

pub(in crate::backends::ffi::gen_bindings) fn gen_method_wrapper(
    typ: &TypeDef,
    method: &MethodDef,
    prefix: &str,
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
    serde_names: &AHashSet<String>,
) -> String {
    let returns_ref = method.returns_ref && !is_owned_default_constructor(method, typ);
    let type_snake = c_symbol_component(&typ.name);
    let type_name = &typ.name;
    let method_name = &method.name;
    let fn_name = format!("{prefix}_{type_snake}_{method_name}");

    let doc_comment = ffi_doxygen_block(&method.doc);

    let has_error = method.error_type.is_some();

    let is_bytes_result = matches!(method.return_type, TypeRef::Bytes);

    let ffi_param_count = (if method.is_static { 0 } else { 1 })
        + method.params.len()
        + method.params.iter().filter(|p| matches!(p.ty, TypeRef::Bytes)).count()
        + if is_bytes_result { 3 } else { 0 };
    let allow_clippy = if ffi_param_count > 7 {
        Some("clippy::too_many_arguments".to_string())
    } else {
        None
    };

    let qualified = core_type_path(typ, core_import);
    let qualified_with_lifetime = if typ.has_lifetime_params {
        format!("{qualified}<'static>")
    } else {
        qualified.clone()
    };
    let handle_qualified = if typ.has_lifetime_params {
        format!("SerializedHandle<{qualified_with_lifetime}>")
    } else {
        qualified.clone()
    };

    let mut ret_type = if is_bytes_result {
        "i32".to_string()
    } else if has_error && is_void_return(&method.return_type) {
        "i32".to_string()
    } else if has_error {
        match &method.return_type {
            TypeRef::Primitive(_) => c_return_type_with_paths(&method.return_type, core_import, path_map).into_owned(),
            _ => c_return_type_with_paths(&method.return_type, core_import, path_map).into_owned(),
        }
    } else {
        c_return_type_with_paths(&method.return_type, core_import, path_map).into_owned()
    };

    if ret_type.contains("Self") {
        ret_type = ret_type.replace("Self", &qualified);
    }

    if typ.has_lifetime_params
        && let TypeRef::Named(n) = &method.return_type
        && n == type_name
    {
        let bare = format!("*mut {qualified}");
        if ret_type == bare {
            ret_type = format!("*mut {qualified}<'static>");
        }
    }

    let return_needs_non_serde_named_method = return_type_needs_non_serde_named(&method.return_type, serde_names);
    let will_be_unimplemented =
        (method.sanitized && !method_sanitized_recoverable(method)) || return_needs_non_serde_named_method;

    let mut params = Vec::new();
    if !method.is_static {
        let receiver_ty = match method.receiver.as_ref().unwrap_or(&ReceiverKind::Ref) {
            ReceiverKind::Ref | ReceiverKind::RefMut | ReceiverKind::Owned => "AlefHandle".to_string(),
        };
        let param_name = if will_be_unimplemented { "_this" } else { "this" };
        params.push(format!("    {param_name}: {receiver_ty}"));
    }
    for p in &method.params {
        let param_name = if will_be_unimplemented {
            format!("_{}", p.name)
        } else {
            p.name.clone()
        };
        params.push(format!(
            "    {}: {}",
            param_name,
            crate::backends::ffi::type_map::c_param_type_with_paths_and_enums(
                &p.ty,
                core_import,
                path_map,
                enum_names,
                p.is_mut,
            )
        ));
        if matches!(p.ty, TypeRef::Bytes) {
            let len_param_name = if will_be_unimplemented {
                format!("_{}_len", p.name)
            } else {
                format!("{}_len", p.name)
            };
            params.push(format!("    {}: usize", len_param_name));
        }
    }
    if is_bytes_result {
        let pfx = if will_be_unimplemented { "_" } else { "" };
        params.push(format!("    {pfx}out_ptr: *mut *mut u8"));
        params.push(format!("    {pfx}out_len: *mut usize"));
        params.push(format!("    {pfx}out_cap: *mut usize"));
    }

    let return_type = if is_void_return(&method.return_type) && !has_error {
        None
    } else {
        Some(ret_type.clone())
    };

    let header = crate::backends::ffi::template_env::render(
        "method_wrapper_header.jinja",
        context! {
            doc_comment => doc_comment.trim_end(),
            allow_clippy => allow_clippy,
            fn_name => fn_name.clone(),
            params => params,
            return_type => return_type,
            source_cfg => typ.cfg.as_deref().unwrap_or(""),
        },
    );

    let mut out = header;

    if will_be_unimplemented {
        out.push_str(&gen_ffi_unimplemented_body(
            if is_bytes_result {
                &TypeRef::Unit
            } else {
                &method.return_type
            },
            &format!("{type_name}::{method_name}"),
            has_error || is_bytes_result,
        ));
        out.push_str(&gen_function_wrapper_footer(
            &return_type,
            &method.return_type,
            has_error || is_bytes_result,
        ));
        return out;
    }

    if is_bytes_result {
        out.push_str(&crate::backends::ffi::template_env::render(
            "bytes_result_null_check.jinja",
            context! {},
        ));
    }

    let fail_ret = if is_bytes_result || (has_error && is_void_return(&method.return_type)) {
        "return -1;".to_string()
    } else if is_void_return(&method.return_type) {
        "return;".to_string()
    } else {
        format!("return {};", null_return_value(&method.return_type))
    };

    let mut handle_requests = Vec::new();
    let is_owned_receiver = method.receiver.as_ref() == Some(&ReceiverKind::Owned);
    if !method.is_static && !is_owned_receiver {
        handle_requests.push(format!(
            "    __alef_requests.push(HandleRequest {{ handle: this, expected_type: std::any::TypeId::of::<{handle_qualified}>() }});"
        ));
    }
    for parameter in &method.params {
        let Some(type_name) = named_handle_type(&parameter.ty) else {
            continue;
        };
        if enum_names.contains(type_name) {
            continue;
        }
        let request = format!(
            "__alef_requests.push(HandleRequest {{ handle: {}, expected_type: std::any::TypeId::of::<{}>() }});",
            parameter.name,
            named_type_path(type_name, core_import, path_map)
        );
        if parameter.optional {
            handle_requests.push(format!("    if {} != 0 {{ {request} }}", parameter.name));
        } else {
            handle_requests.push(format!("    {request}"));
        }
    }
    if !handle_requests.is_empty() || method.receiver.as_ref() == Some(&ReceiverKind::Owned) {
        out.push_str(&crate::backends::ffi::template_env::render(
            "handle_acquisition.rs.jinja",
            context! {
                requests => handle_requests.join("\n"),
                request_count => handle_requests.len(),
                fail_ret => fail_ret.clone(),
                owned_handle => is_owned_receiver.then_some("this"),
            },
        ));
    }

    if !method.is_static {
        let receiver_kind = method.receiver.as_ref().unwrap_or(&ReceiverKind::Ref);
        let null_check = if typ.has_lifetime_params {
            match receiver_kind {
                ReceiverKind::Ref | ReceiverKind::RefMut => crate::backends::ffi::template_env::render(
                    "snapshot_handle_self_ref.jinja",
                    context! {
                        fail_ret => fail_ret,
                        qualified => qualified_with_lifetime.clone(),
                        handle_qualified => handle_qualified.clone(),
                    },
                ),
                ReceiverKind::Owned => crate::backends::ffi::template_env::render(
                    "snapshot_handle_self_owned.jinja",
                    context! {
                        fail_ret => fail_ret,
                        qualified => qualified_with_lifetime.clone(),
                        handle_qualified => handle_qualified.clone(),
                    },
                ),
            }
        } else {
            match receiver_kind {
                ReceiverKind::Ref => crate::backends::ffi::template_env::render(
                    "null_check_self_ref.jinja",
                    context! { fail_ret => fail_ret, qualified => qualified.clone() },
                ),
                ReceiverKind::RefMut => crate::backends::ffi::template_env::render(
                    "null_check_self_mut.jinja",
                    context! { fail_ret => fail_ret, qualified => qualified.clone() },
                ),
                ReceiverKind::Owned => crate::backends::ffi::template_env::render(
                    "null_check_self_owned.jinja",
                    context! { fail_ret => fail_ret, qualified => qualified.clone() },
                ),
            }
        };
        out.push_str(&crate::backends::ffi::template_env::render(
            "code_line.jinja",
            context! { content => null_check },
        ));
    }

    for p in &method.params {
        out.push_str(&crate::backends::ffi::template_env::render(
            "emitted_code_block.jinja",
            context! {
                content => gen_param_conversion_with_enums(p, &ParamConversionContext {
                    has_error,
                    is_bytes_result,
                    return_type: &method.return_type,
                    ffi_return_type: return_type.as_deref(),
                    core_import,
                    path_map,
                    enum_names,
                }),
            },
        ));
    }

    for p in &method.params {
        if matches!(p.ty, TypeRef::Map(_, _)) && !p.optional && p.is_ref && p.map_is_btree {
            let rs = format!("{}_rs", p.name);
            let btree = format!("{}_btree", p.name);
            out.push_str(&crate::backends::ffi::template_env::render(
                "ffi_btree_binding.jinja",
                context! {
                    btree => btree,
                    rs => rs,
                },
            ));
        }
    }

    let arg_names: Vec<String> = method
        .params
        .iter()
        .map(|p| {
            let rs = format!("{}_rs", p.name);
            match &p.ty {
                TypeRef::Path if !p.optional => {
                    if p.is_ref {
                        format!("{rs}.as_path()")
                    } else {
                        rs
                    }
                }
                TypeRef::Named(_) if !p.optional => {
                    if p.is_mut || is_owned_receiver || !p.is_ref {
                        rs
                    } else {
                        format!("&{rs}")
                    }
                }
                TypeRef::String | TypeRef::Char if !p.optional => {
                    if p.is_ref {
                        format!("&{rs}")
                    } else if p.core_wrapper == CoreWrapper::Cow {
                        format!("{rs}.into()")
                    } else {
                        rs
                    }
                }
                TypeRef::Bytes if !p.optional => {
                    if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::String | TypeRef::Char | TypeRef::Bytes if p.optional => {
                    if p.is_ref {
                        format!("{rs}.as_deref()")
                    } else if p.core_wrapper == CoreWrapper::Cow {
                        format!("{rs}.map(std::borrow::Cow::Owned)")
                    } else {
                        rs
                    }
                }
                TypeRef::Path if p.optional => {
                    if p.is_ref {
                        format!("{rs}.as_ref().map(|s| std::path::Path::new(s.as_str()))")
                    } else {
                        rs
                    }
                }
                TypeRef::Named(_) if p.optional => {
                    if p.is_ref {
                        format!("{rs}.as_ref()")
                    } else {
                        rs
                    }
                }
                TypeRef::Json if !p.optional => {
                    if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::Json if p.optional => {
                    if p.is_ref {
                        format!("{rs}.as_ref()")
                    } else {
                        rs
                    }
                }
                TypeRef::Vec(_inner) if !p.optional => {
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref && p.vec_inner_is_ref {
                        format!("&{rs}.iter().map(|s| s.as_str()).collect::<Vec<&str>>()")
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::Map(_, _) if !p.optional => {
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref && p.map_is_btree {
                        format!("&{}_btree", p.name)
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else if p.map_is_btree {
                        format!("{rs}.into_iter().collect::<std::collections::BTreeMap<_, _>>()")
                    } else {
                        rs
                    }
                }
                TypeRef::Vec(_) if p.optional => {
                    if p.is_mut {
                        format!("{rs}.as_deref_mut()")
                    } else if p.is_ref {
                        format!("{rs}.as_deref()")
                    } else {
                        rs
                    }
                }
                TypeRef::Map(_, _) if p.optional => {
                    if p.is_mut {
                        format!("{rs}.as_deref_mut()")
                    } else if p.is_ref {
                        format!("{rs}.as_ref()")
                    } else {
                        rs
                    }
                }
                _ => rs,
            }
        })
        .collect();
    let call_args = arg_names.join(", ");

    let can_inline = is_passthrough_return(&method.return_type)
        && !is_bytes_result
        && !has_error
        && !returns_ref
        && !method.returns_cow
        && method.return_newtype_wrapper.is_none();

    if method.is_async {
        let call = if method.is_static {
            format!("get_ffi_runtime().block_on(async {{ {qualified}::{method_name}({call_args}).await }})")
        } else {
            format!("get_ffi_runtime().block_on(async {{ obj.{method_name}({call_args}).await }})")
        };
        if can_inline {
            out.push_str(&crate::backends::ffi::template_env::render(
                "call_inline.jinja",
                context! { call => call },
            ));
        } else {
            out.push_str(&crate::backends::ffi::template_env::render(
                "call_with_result.jinja",
                context! { call => call },
            ));
        }
    } else if method.is_static {
        if can_inline {
            out.push_str(&crate::backends::ffi::template_env::render("static_method_call.jinja", context! { qualified => qualified.clone(), method_name => method_name.clone(), call_args => call_args.clone() }));
        } else {
            out.push_str(&crate::backends::ffi::template_env::render("static_method_call_result.jinja", context! { qualified => qualified.clone(), method_name => method_name.clone(), call_args => call_args.clone() }));
        }
    } else if method_name == "drop" {
        out.push_str("    std::mem::drop(obj);\n");
    } else if can_inline {
        out.push_str(&crate::backends::ffi::template_env::render(
            "instance_method_call.jinja",
            context! { method_name => method_name.clone(), call_args => call_args.clone() },
        ));
    } else {
        out.push_str(&crate::backends::ffi::template_env::render(
            "instance_method_call_result.jinja",
            context! { method_name => method_name.clone(), call_args => call_args.clone() },
        ));
    }

    if is_bytes_result {
        out.push_str(&crate::backends::ffi::template_env::render(
            "bytes_result_match.jinja",
            context! { has_error },
        ));
    } else {
        let result_expr =
            if method.return_newtype_wrapper.is_some() && matches!(method.return_type, TypeRef::Primitive(_)) {
                "result.0"
            } else {
                "result"
            };
        if returns_ref && !has_error {
            match &method.return_type {
                TypeRef::String => {
                    out.push_str("    let result = result.to_owned();\n");
                }
                TypeRef::Char => {
                    // `char: Copy` — `.clone()` on `&char` triggers clippy::noop_method_call.
                    out.push_str("    let result = *result;\n");
                }
                TypeRef::Vec(_) => {
                    out.push_str("    let result = result.to_vec();\n");
                }
                TypeRef::Map(_, _) => {
                    out.push_str("    let result = result.clone();\n");
                }
                TypeRef::Named(_) => {
                    out.push_str("    let result = result.clone();\n");
                }
                TypeRef::Optional(inner) => match inner.as_ref() {
                    TypeRef::String => {
                        out.push_str("    let result = result.map(str::to_owned);\n");
                    }
                    TypeRef::Named(_) | TypeRef::Char | TypeRef::Vec(_) | TypeRef::Map(_, _) => {
                        out.push_str("    let result = result.cloned();\n");
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if method.returns_cow && !has_error {
            out.push_str("    let result = result.into_owned();\n");
        }
        let returns_serialized_self =
            typ.has_lifetime_params && matches!(&method.return_type, TypeRef::Named(name) if name == type_name);
        if has_error {
            if is_void_return(&method.return_type) {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "error_match_void.jinja",
                    context! {},
                ));
            } else {
                let val_expr =
                    if method.return_newtype_wrapper.is_some() && matches!(method.return_type, TypeRef::Primitive(_)) {
                        "val.0"
                    } else {
                        "val"
                    };
                let ok_body = if returns_serialized_self {
                    crate::backends::ffi::template_env::render(
                        "serialized_value_to_c.jinja",
                        context! { value => val_expr, indent => "            " },
                    )
                } else {
                    gen_owned_value_to_c(val_expr, &method.return_type, "            ", enum_names)
                };
                out.push_str(&crate::backends::ffi::template_env::render(
                    "error_match_non_void.jinja",
                    context! {
                        ok_body => ok_body,
                        null_ret => null_return_value(&method.return_type),
                    },
                ));
            }
        } else if is_void_return(&method.return_type) {
        } else if can_inline {
        } else if returns_serialized_self {
            out.push_str(&crate::backends::ffi::template_env::render(
                "serialized_value_to_c.jinja",
                context! { value => result_expr, indent => "    " },
            ));
        } else {
            out.push_str(&crate::backends::ffi::template_env::render(
                "emitted_code_block.jinja",
                context! {
                    content => gen_owned_value_to_c(result_expr, &method.return_type, "    ", enum_names),
                },
            ));
        }
    }

    out.push_str(&gen_function_wrapper_footer(
        &return_type,
        &method.return_type,
        has_error || is_bytes_result,
    ));
    out
}

pub(super) fn gen_function_wrapper_footer(
    return_type: &Option<String>,
    rust_return_type: &TypeRef,
    has_status_return: bool,
) -> String {
    let panic_return = if has_status_return && matches!(rust_return_type, TypeRef::Unit | TypeRef::Bytes) {
        "-1".to_string()
    } else {
        return_type.as_ref().map_or_else(
            || "()".to_string(),
            |ffi_return_type| ffi_null_return_value(rust_return_type, Some(ffi_return_type)).to_string(),
        )
    };
    crate::backends::ffi::template_env::render(
        "function_wrapper_footer.jinja",
        context! { panic_return => panic_return },
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::backends::ffi::gen_bindings) fn gen_free_function(
    func: &FunctionDef,
    prefix: &str,
    core_import: &str,
    path_map: &AHashMap<String, String>,
    enum_names: &AHashSet<String>,
    serde_names: &AHashSet<String>,
    capsule_cfg: Option<&crate::core::config::FfiCapsuleTypeConfig>,
    returns_serialized_handle: bool,
) -> String {
    let fn_name_snake = c_symbol_component(&func.name);
    let ffi_name = format!("{prefix}_{fn_name_snake}");
    let core_fn_path = {
        let path = func.rust_path.replace('-', "_");
        if path.starts_with(core_import) {
            path
        } else {
            format!("{core_import}::{}", func.name)
        }
    };
    let func_name = &func.name;

    let doc_comment = ffi_doxygen_block(&func.doc);

    let has_error = func.error_type.is_some();

    let is_bytes_result = matches!(func.return_type, TypeRef::Bytes);

    let ffi_param_count = func.params.len()
        + func.params.iter().filter(|p| matches!(p.ty, TypeRef::Bytes)).count()
        + if is_bytes_result { 3 } else { 0 };
    let allow_clippy = if ffi_param_count > 7 {
        Some("clippy::too_many_arguments".to_string())
    } else {
        None
    };

    let ret_type = if is_bytes_result {
        "i32".to_string()
    } else if has_error && is_void_return(&func.return_type) {
        "i32".to_string()
    } else if let Some(cfg) = capsule_cfg {
        super::super::capsule::capsule_c_return_type(cfg)
    } else {
        c_return_type_with_paths(&func.return_type, core_import, path_map).into_owned()
    };

    let return_needs_non_serde_named = return_type_needs_non_serde_named(&func.return_type, serde_names);
    let will_be_unimplemented = (func.sanitized && !sanitized_recoverable(func)) || return_needs_non_serde_named;

    let mut params = Vec::new();
    for p in &func.params {
        let param_name = if will_be_unimplemented {
            format!("_{}", p.name)
        } else {
            p.name.clone()
        };
        params.push(format!(
            "    {}: {}",
            param_name,
            crate::backends::ffi::type_map::c_param_type_with_paths_and_enums(
                &p.ty,
                core_import,
                path_map,
                enum_names,
                p.is_mut,
            )
        ));
        if matches!(p.ty, TypeRef::Bytes) {
            let len_param_name = if will_be_unimplemented {
                format!("_{}_len", p.name)
            } else {
                format!("{}_len", p.name)
            };
            params.push(format!("    {}: usize", len_param_name));
        }
    }
    if is_bytes_result {
        let pfx = if will_be_unimplemented { "_" } else { "" };
        params.push(format!("    {pfx}out_ptr: *mut *mut u8"));
        params.push(format!("    {pfx}out_len: *mut usize"));
        params.push(format!("    {pfx}out_cap: *mut usize"));
    }

    let return_type = if is_void_return(&func.return_type) && !has_error {
        None
    } else {
        Some(ret_type.clone())
    };

    let header = crate::backends::ffi::template_env::render(
        "free_function_header.jinja",
        context! {
            doc_comment => doc_comment.trim_end(),
            allow_clippy => allow_clippy,
            fn_name => ffi_name.clone(),
            params => params,
            return_type => return_type,
            source_cfg => func.cfg.as_deref().unwrap_or(""),
            return_len_key => returns_c_char(&func.return_type).then_some(ffi_name.as_str()),
        },
    );

    let mut out = header;

    if will_be_unimplemented {
        out.push_str(&gen_ffi_unimplemented_body(
            if is_bytes_result {
                &TypeRef::Unit
            } else {
                &func.return_type
            },
            func_name,
            has_error || is_bytes_result,
        ));
        out.push_str(&gen_function_wrapper_footer(
            &return_type,
            &func.return_type,
            has_error || is_bytes_result,
        ));
        return out;
    }

    if is_bytes_result {
        out.push_str(&crate::backends::ffi::template_env::render(
            "bytes_result_null_check.jinja",
            context! {},
        ));
    }

    let fail_ret = if is_bytes_result || (has_error && is_void_return(&func.return_type)) {
        "return -1;".to_string()
    } else if is_void_return(&func.return_type) {
        "return;".to_string()
    } else {
        format!("return {};", null_return_value(&func.return_type))
    };
    let mut handle_requests = Vec::new();
    for parameter in &func.params {
        let Some(type_name) = named_handle_type(&parameter.ty) else {
            continue;
        };
        if enum_names.contains(type_name) {
            continue;
        }
        let request = format!(
            "__alef_requests.push(HandleRequest {{ handle: {}, expected_type: std::any::TypeId::of::<{}>() }});",
            parameter.name,
            named_type_path(type_name, core_import, path_map)
        );
        if parameter.optional {
            handle_requests.push(format!("    if {} != 0 {{ {request} }}", parameter.name));
        } else {
            handle_requests.push(format!("    {request}"));
        }
    }
    if !handle_requests.is_empty() {
        out.push_str(&crate::backends::ffi::template_env::render(
            "handle_acquisition.rs.jinja",
            context! {
                requests => handle_requests.join("\n"),
                request_count => handle_requests.len(),
                fail_ret => fail_ret,
                owned_handle => Option::<&str>::None,
            },
        ));
    }

    for p in &func.params {
        out.push_str(&crate::backends::ffi::template_env::render(
            "emitted_code_block.jinja",
            context! {
                content => gen_param_conversion_with_enums(p, &ParamConversionContext {
                    has_error,
                    is_bytes_result,
                    return_type: &func.return_type,
                    ffi_return_type: return_type.as_deref(),
                    core_import,
                    path_map,
                    enum_names,
                }),
            },
        ));
    }

    for p in &func.params {
        if matches!(p.ty, TypeRef::Map(_, _)) && !p.optional && p.is_ref && p.map_is_btree {
            let rs = format!("{}_rs", p.name);
            let btree = format!("{}_btree", p.name);
            out.push_str(&crate::backends::ffi::template_env::render(
                "ffi_btree_binding.jinja",
                context! {
                    btree => btree,
                    rs => rs,
                },
            ));
        }
    }

    let arg_names: Vec<String> = func
        .params
        .iter()
        .map(|p| {
            let rs = format!("{}_rs", p.name);
            match &p.ty {
                TypeRef::Path if !p.optional => {
                    if p.is_ref {
                        format!("{rs}.as_path()")
                    } else {
                        rs
                    }
                }
                TypeRef::String | TypeRef::Char if !p.optional => {
                    if p.is_ref {
                        format!("&{rs}")
                    } else if p.core_wrapper == CoreWrapper::Cow {
                        format!("{rs}.into()")
                    } else {
                        rs
                    }
                }
                TypeRef::Bytes if !p.optional => {
                    if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::Named(_) if !p.optional => {
                    if p.is_mut || !p.is_ref {
                        rs
                    } else {
                        format!("&{rs}")
                    }
                }
                TypeRef::String | TypeRef::Char | TypeRef::Bytes if p.optional => {
                    if p.is_ref {
                        format!("{rs}.as_deref()")
                    } else if p.core_wrapper == CoreWrapper::Cow {
                        format!("{rs}.map(std::borrow::Cow::Owned)")
                    } else {
                        rs
                    }
                }
                TypeRef::Path if p.optional => {
                    if p.is_ref {
                        format!("{rs}.as_ref().map(|s| std::path::Path::new(s.as_str()))")
                    } else {
                        rs
                    }
                }
                TypeRef::Named(_) if p.optional => {
                    if p.is_ref {
                        format!("{rs}.as_ref()")
                    } else {
                        rs
                    }
                }
                TypeRef::Json if !p.optional => {
                    if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::Json if p.optional => {
                    if p.is_ref {
                        format!("{rs}.as_ref()")
                    } else {
                        rs
                    }
                }
                TypeRef::Vec(_inner) if !p.optional => {
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref && p.vec_inner_is_ref {
                        format!("&{rs}.iter().map(|s| s.as_str()).collect::<Vec<&str>>()")
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else {
                        rs
                    }
                }
                TypeRef::Map(_, _) if !p.optional => {
                    if p.is_mut {
                        format!("&mut {rs}")
                    } else if p.is_ref && p.map_is_btree {
                        format!("&{}_btree", p.name)
                    } else if p.is_ref {
                        format!("&{rs}")
                    } else if p.map_is_btree {
                        format!("{rs}.into_iter().collect::<std::collections::BTreeMap<_, _>>()")
                    } else {
                        rs
                    }
                }
                TypeRef::Vec(_) if p.optional => {
                    if p.is_mut {
                        format!("{rs}.as_deref_mut()")
                    } else if p.is_ref {
                        format!("{rs}.as_deref()")
                    } else {
                        rs
                    }
                }
                TypeRef::Map(_, _) if p.optional => {
                    if p.is_mut {
                        format!("{rs}.as_deref_mut()")
                    } else if p.is_ref {
                        format!("{rs}.as_ref()")
                    } else {
                        rs
                    }
                }
                _ => rs,
            }
        })
        .collect();
    let call_args = arg_names.join(", ");

    let can_inline_fn = is_passthrough_return(&func.return_type)
        && !is_bytes_result
        && !has_error
        && !func.returns_ref
        && !func.returns_cow
        && func.return_newtype_wrapper.is_none();

    if func.is_async {
        let call = format!("get_ffi_runtime().block_on(async {{ {core_fn_path}({call_args}).await }})");
        if can_inline_fn {
            out.push_str(&crate::backends::ffi::template_env::render(
                "call_inline.jinja",
                context! { call => call },
            ));
        } else {
            out.push_str(&crate::backends::ffi::template_env::render(
                "call_with_result.jinja",
                context! { call => call },
            ));
        }
    } else if can_inline_fn {
        out.push_str(&crate::backends::ffi::template_env::render(
            "call_inline.jinja",
            context! { call => format!("{core_fn_path}({call_args})") },
        ));
    } else {
        out.push_str(&crate::backends::ffi::template_env::render(
            "call_with_result.jinja",
            context! { call => format!("{core_fn_path}({call_args})") },
        ));
    }

    if is_bytes_result {
        out.push_str(&crate::backends::ffi::template_env::render(
            "bytes_result_match.jinja",
            context! { has_error },
        ));
    } else {
        let result_expr = if func.return_newtype_wrapper.is_some() && matches!(func.return_type, TypeRef::Primitive(_))
        {
            "result.0"
        } else {
            "result"
        };
        if func.returns_ref
            && !has_error
            && matches!(&func.return_type, TypeRef::Optional(inner) if matches!(inner.as_ref(), TypeRef::Named(_)))
        {
            out.push_str("    let result = result.cloned();\n");
        }
        if func.returns_cow && !has_error {
            out.push_str("    let result = result.into_owned();\n");
        }
        if has_error {
            if is_void_return(&func.return_type) {
                out.push_str(&crate::backends::ffi::template_env::render(
                    "error_match_void.jinja",
                    context! {},
                ));
            } else {
                let val_expr =
                    if func.return_newtype_wrapper.is_some() && matches!(func.return_type, TypeRef::Primitive(_)) {
                        "val.0"
                    } else {
                        "val"
                    };
                let ok_body = if returns_serialized_handle {
                    crate::backends::ffi::template_env::render(
                        "serialized_value_to_c.jinja",
                        context! { value => val_expr, indent => "            " },
                    )
                } else if let Some(cfg) = capsule_cfg {
                    format!(
                        "            {}",
                        super::super::capsule::capsule_into_raw_expr(val_expr, cfg)
                    )
                } else if returns_c_char(&func.return_type) {
                    gen_owned_c_char_to_c_with_len(val_expr, &func.return_type, "            ", &ffi_name)
                } else {
                    gen_owned_value_to_c(val_expr, &func.return_type, "            ", enum_names)
                };
                let null_ret = if capsule_cfg.is_some() {
                    "std::ptr::null()"
                } else {
                    null_return_value(&func.return_type)
                };
                out.push_str(&crate::backends::ffi::template_env::render(
                    "error_match_non_void.jinja",
                    context! {
                        ok_body => ok_body,
                        null_ret => null_ret,
                    },
                ));
            }
        } else if is_void_return(&func.return_type) {
        } else if can_inline_fn {
        } else {
            let content = if returns_serialized_handle {
                crate::backends::ffi::template_env::render(
                    "serialized_value_to_c.jinja",
                    context! { value => result_expr, indent => "    " },
                )
            } else if let Some(cfg) = capsule_cfg {
                format!("    {}", super::super::capsule::capsule_into_raw_expr(result_expr, cfg))
            } else if returns_c_char(&func.return_type) {
                gen_owned_c_char_to_c_with_len(result_expr, &func.return_type, "    ", &ffi_name)
            } else {
                gen_owned_value_to_c(result_expr, &func.return_type, "    ", enum_names)
            };
            out.push_str(&crate::backends::ffi::template_env::render(
                "emitted_code_block.jinja",
                context! {
                    content => content,
                },
            ));
        }
    }

    out.push_str(&gen_function_wrapper_footer(
        &return_type,
        &func.return_type,
        has_error || is_bytes_result,
    ));
    out
}
