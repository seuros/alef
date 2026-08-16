//! C# NativeMethods (P/Invoke) code generation.

use super::{StreamingMethodMeta, is_bridge_param, pinvoke_param_type, pinvoke_return_type};
use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::config::TraitBridgeConfig;
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::ir::{ApiSurface, FunctionDef, MethodDef, PrimitiveType, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};

fn emits_registered_trait_bridge(has_visitor_callbacks: bool, config: &TraitBridgeConfig) -> bool {
    !has_visitor_callbacks
        || config.bind_via != crate::core::config::BridgeBinding::OptionsField
        || config.register_fn.is_some()
}

fn ffi_handle_type_names(api: &ApiSurface) -> HashSet<&str> {
    let mut names: HashSet<&str> = api
        .types
        .iter()
        .filter(|typ| !typ.is_trait)
        .map(|typ| typ.name.as_str())
        .collect();
    names.extend(
        api.enums
            .iter()
            .filter(|enum_def| enum_def.variants.iter().any(|variant| !variant.fields.is_empty()))
            .map(|enum_def| enum_def.name.as_str()),
    );
    names
}

/// Map a Rust FFI type string to the C# P/Invoke parameter declaration.
///
/// String parameters use explicit UTF-8 marshalling to match the C `const char*` ABI.
fn ffi_ty_to_pinvoke_param(rust_ty: &str, param_name: &str) -> String {
    let normalized = rust_ty.trim();
    let cs_name = param_name.to_lower_camel_case();
    if normalized.contains("c_char") || normalized.contains("CStr") {
        return format!("[MarshalAs(UnmanagedType.LPUTF8Str)] string {cs_name}");
    }
    let cs_type = match normalized {
        "bool" => "bool",
        "u8" | "uint8_t" => "byte",
        "u16" | "uint16_t" => "ushort",
        "u32" | "uint32_t" => "uint",
        "u64" | "uint64_t" | "usize" => "ulong",
        "i8" | "int8_t" => "sbyte",
        "i16" | "int16_t" => "short",
        "i32" | "int32_t" | "c_int" => "int",
        "i64" | "int64_t" | "isize" => "long",
        "f32" | "float" => "float",
        "f64" | "double" => "double",
        _ => "IntPtr",
    };
    format!("{cs_type} {cs_name}")
}

#[allow(clippy::too_many_arguments)]
pub(super) fn gen_native_methods(
    api: &ApiSurface,
    namespace: &str,
    lib_name: &str,
    prefix: &str,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
    has_visitor_callbacks: bool,
    trait_bridges: &[TraitBridgeConfig],
    streaming_methods: &HashSet<String>,
    streaming_methods_meta: &HashMap<String, StreamingMethodMeta>,
    exclude_functions: &HashSet<String>,
    client_constructors: &HashMap<String, ClientConstructorConfig>,
    adapters: &[crate::core::config::AdapterConfig],
) -> anyhow::Result<String> {
    use crate::backends::csharp::template_env::render;
    use minijinja::Value;

    let mut out = render(
        "native_methods_header.jinja",
        Value::from_serialize(serde_json::json!({
            "namespace": namespace,
            "lib_name": lib_name,
        })),
    );
    out.push('\n');

    let mut emitted: HashSet<String> = HashSet::new();

    let enum_names: HashSet<String> = api.enums.iter().map(|e| e.name.clone()).collect();

    let mut opaque_param_types: HashSet<String> = HashSet::new();
    let mut opaque_return_types: HashSet<String> = HashSet::new();

    fn inner_named(ty: &TypeRef) -> Option<&str> {
        match ty {
            TypeRef::Named(n) => Some(n.as_str()),
            TypeRef::Optional(inner) | TypeRef::Vec(inner) => inner_named(inner),
            _ => None,
        }
    }
    for func in api.functions.iter().filter(|f| !exclude_functions.contains(&f.name)) {
        for param in &func.params {
            if let TypeRef::Named(name) = &param.ty {
                opaque_param_types.insert(name.clone());
            }
        }
        if let Some(name) = inner_named(&func.return_type)
            && !enum_names.contains(name)
        {
            opaque_return_types.insert(name.to_string());
        }
    }
    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        for method in &typ.methods {
            if streaming_methods.contains(&method.name) {
                for param in &method.params {
                    if let TypeRef::Named(name) = &param.ty {
                        opaque_param_types.insert(name.clone());
                    }
                }
                if let Some(meta) = streaming_methods_meta.get(&method.name) {
                    opaque_return_types.insert(meta.item_type.clone());
                }
                continue;
            }
            for param in &method.params {
                if let TypeRef::Named(name) = &param.ty {
                    opaque_param_types.insert(name.clone());
                }
            }
            if let Some(name) = inner_named(&method.return_type)
                && !enum_names.contains(name)
            {
                opaque_return_types.insert(name.to_string());
            }
            if method.receiver.is_some() {
                opaque_param_types.insert(typ.name.clone());
                if !enum_names.contains(&typ.name) {
                    opaque_return_types.insert(typ.name.clone());
                }
            }
        }
    }

    let true_opaque_types: HashSet<String> = api
        .types
        .iter()
        .filter(|typ| typ.is_opaque && !typ.is_trait)
        .map(|t| t.name.clone())
        .collect();
    let ffi_handle_type_names = ffi_handle_type_names(api);
    opaque_param_types.retain(|name| ffi_handle_type_names.contains(name.as_str()));
    opaque_return_types.retain(|name| ffi_handle_type_names.contains(name.as_str()));
    opaque_param_types.retain(|name| !bridge_type_aliases.contains(name));
    opaque_return_types.retain(|name| !bridge_type_aliases.contains(name));

    let mut sorted_true_opaque_types: Vec<&String> = true_opaque_types.iter().collect();
    sorted_true_opaque_types.sort();
    for type_name in sorted_true_opaque_types {
        let snake = type_name.to_snake_case();
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", csharp_type_name(type_name));
        if emitted.insert(free_entry.clone()) {
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &free_entry },
            ));
            out.push_str(&render(
                "extern_void_ptr.jinja",
                minijinja::context! { cs_name => &free_cs },
            ));
            out.push('\n');
        }
    }

    let mut sorted_ctor_types: Vec<&String> = client_constructors.keys().collect();
    sorted_ctor_types.sort();
    for type_name in sorted_ctor_types {
        let ctor = &client_constructors[type_name];
        let snake = type_name.to_snake_case();
        let new_entry = format!("{prefix}_{snake}_new");
        let new_cs = format!("{}New", csharp_type_name(type_name));
        if emitted.insert(new_entry.clone()) {
            let params_str: String = ctor
                .params
                .iter()
                .map(|p| ffi_ty_to_pinvoke_param(&p.ty, &p.name))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &new_entry },
            ));
            out.push_str(&render(
                "client_constructor_pinvoke.jinja",
                minijinja::context! { new_cs, params_str },
            ));
            out.push('\n');
        }
    }

    let mut sorted_param_types: Vec<&String> = opaque_param_types.iter().collect();
    sorted_param_types.sort();
    for type_name in sorted_param_types {
        let snake = type_name.to_snake_case();
        if !true_opaque_types.contains(type_name) {
            let from_json_entry = format!("{prefix}_{snake}_from_json");
            let from_json_cs = format!("{}FromJson", csharp_type_name(type_name));
            if emitted.insert(from_json_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &from_json_entry },
                ));
                out.push_str(&render(
                    "extern_ptr_from_json.jinja",
                    minijinja::context! { cs_name => &from_json_cs },
                ));
                out.push('\n');
            }
        }
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", csharp_type_name(type_name));
        if emitted.insert(free_entry.clone()) {
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &free_entry },
            ));
            out.push_str(&render(
                "extern_void_ptr.jinja",
                minijinja::context! { cs_name => &free_cs },
            ));
            out.push('\n');
        }
    }

    let mut sorted_return_types: Vec<&String> = opaque_return_types.iter().collect();
    sorted_return_types.sort();
    for type_name in sorted_return_types {
        let snake = type_name.to_snake_case();
        if !true_opaque_types.contains(type_name) {
            let to_json_entry = format!("{prefix}_{snake}_to_json");
            let to_json_cs = format!("{}ToJson", csharp_type_name(type_name));
            if emitted.insert(to_json_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &to_json_entry },
                ));
                out.push_str(&render(
                    "extern_ptr_to_json.jinja",
                    minijinja::context! { cs_name => &to_json_cs },
                ));
                out.push('\n');
            }
        }
        let free_entry = format!("{prefix}_{snake}_free");
        let free_cs = format!("{}Free", csharp_type_name(type_name));
        if emitted.insert(free_entry.clone()) {
            out.push_str(&render(
                "dll_import_attr.jinja",
                minijinja::context! { entry_point => &free_entry },
            ));
            out.push_str(&render(
                "extern_void_ptr.jinja",
                minijinja::context! { cs_name => &free_cs },
            ));
            out.push('\n');
        }
    }

    for func in api.functions.iter().filter(|f| {
        !exclude_functions.contains(&f.name)
            && !crate::codegen::generators::trait_bridge::is_trait_bridge_managed_fn(&f.name, trait_bridges)
    }) {
        let c_func_name = format!("{}_{}", prefix, func.name.to_lowercase());
        if emitted.insert(c_func_name.clone()) {
            out.push_str(&gen_pinvoke_for_func(
                &c_func_name,
                func,
                bridge_param_names,
                bridge_type_aliases,
            ));
        }
    }

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        let type_snake = typ.name.to_snake_case();
        for method in &typ.methods {
            if streaming_methods.contains(&method.name) {
                continue;
            }
            if method.returns_ref_to_owner(&typ.name) {
                continue;
            }
            let c_method_name = format!("{}_{}_{}", prefix, type_snake, method.name.to_lowercase());
            let cs_method_name = format!("{}{}", csharp_type_name(&typ.name), to_csharp_name(&method.name));
            if emitted.insert(c_method_name.clone()) {
                out.push_str(&gen_pinvoke_for_method(&c_method_name, &cs_method_name, method));
            }
        }
    }

    for typ in api.types.iter().filter(|typ| !typ.is_trait) {
        let type_snake = typ.name.to_snake_case();
        for method in &typ.methods {
            if !streaming_methods.contains(&method.name) {
                continue;
            }
            let cs_type = csharp_type_name(&typ.name);
            let cs_method = to_csharp_name(&method.name);

            let start_entry = format!("{}_{}_{}_start", prefix, type_snake, method.name.to_lowercase());
            let start_cs = format!("{cs_type}{cs_method}Start");
            if emitted.insert(start_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &start_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "IntPtr",
                        cs_name => &start_cs,
                        params => "IntPtr client, IntPtr req",
                    },
                ));
                out.push('\n');
            }

            let next_entry = format!("{}_{}_{}_next", prefix, type_snake, method.name.to_lowercase());
            let next_cs = format!("{cs_type}{cs_method}Next");
            if emitted.insert(next_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &next_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "IntPtr",
                        cs_name => &next_cs,
                        params => "IntPtr handle",
                    },
                ));
                out.push('\n');
            }

            let free_entry = format!("{}_{}_{}_free", prefix, type_snake, method.name.to_lowercase());
            let free_cs = format!("{cs_type}{cs_method}Free");
            if emitted.insert(free_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &free_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "void",
                        cs_name => &free_cs,
                        params => "IntPtr handle",
                    },
                ));
                out.push('\n');
            }
        }
    }

    for adapter in adapters {
        if matches!(adapter.pattern, crate::core::config::AdapterPattern::Streaming) {
            let Some(owner_type) = adapter.owner_type.as_deref() else {
                continue;
            };
            let owner_snake = owner_type.to_snake_case();
            let owner_cs = csharp_type_name(owner_type);
            let adapter_snake = adapter.name.to_snake_case();
            let adapter_cs = to_csharp_name(&adapter.name);

            let start_entry = format!("{}_{}_{}_start", prefix, owner_snake, adapter_snake);
            let start_cs = format!("{owner_cs}{adapter_cs}Start");
            if emitted.insert(start_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &start_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "IntPtr",
                        cs_name => &start_cs,
                        params => "IntPtr engine, IntPtr req",
                    },
                ));
                out.push('\n');
            }

            let next_entry = format!("{}_{}_{}_next", prefix, owner_snake, adapter_snake);
            let next_cs = format!("{owner_cs}{adapter_cs}Next");
            if emitted.insert(next_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &next_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "IntPtr",
                        cs_name => &next_cs,
                        params => "IntPtr handle",
                    },
                ));
                out.push('\n');
            }

            let free_entry = format!("{}_{}_{}_free", prefix, owner_snake, adapter_snake);
            let free_cs = format!("{owner_cs}{adapter_cs}Free");
            if emitted.insert(free_entry.clone()) {
                out.push_str(&render(
                    "dll_import_attr.jinja",
                    minijinja::context! { entry_point => &free_entry },
                ));
                out.push_str(&render(
                    "streaming_pinvoke_declaration.jinja",
                    minijinja::context! {
                        return_type => "void",
                        cs_name => &free_cs,
                        params => "IntPtr handle",
                    },
                ));
                out.push('\n');
            }
        }
    }

    let last_error_code_entry = format!("{prefix}_last_error_code");
    out.push_str(&render(
        "dll_import_attr.jinja",
        minijinja::context! { entry_point => &last_error_code_entry },
    ));
    out.push_str("    internal static extern int LastErrorCode();\n\n");

    let last_error_context_entry = format!("{prefix}_last_error_context");
    out.push_str(&render(
        "dll_import_attr.jinja",
        minijinja::context! { entry_point => &last_error_context_entry },
    ));
    out.push_str("    internal static extern IntPtr LastErrorContext();\n\n");

    let free_string_entry = format!("{prefix}_free_string");
    out.push_str(&render(
        "dll_import_attr.jinja",
        minijinja::context! { entry_point => &free_string_entry },
    ));
    out.push_str("    internal static extern void FreeString(IntPtr ptr);\n\n");

    let has_bytes_results = api.functions.iter().any(is_bytes_result_func)
        || api
            .types
            .iter()
            .any(|typ| typ.methods.iter().any(is_bytes_result_method));
    if has_bytes_results {
        let free_bytes_entry = format!("{prefix}_free_bytes");
        out.push_str(&render(
            "dll_import_attr.jinja",
            minijinja::context! { entry_point => &free_bytes_entry },
        ));
        out.push_str("    internal static extern void FreeBytes(IntPtr ptr, UIntPtr len, UIntPtr cap);\n");
    }

    if has_visitor_callbacks {
        out.push('\n');
        let visitor_bridge = trait_bridges.iter().find(|b| {
            b.bind_via == crate::core::config::BridgeBinding::OptionsField
                && b.is_active_for(&crate::core::config::Language::Csharp.to_string())
        });

        if let Some(bridge) = visitor_bridge {
            // Both names below are load-bearing ABI, not cosmetics: the emitted declaration is
            // `[DllImport(EntryPoint = "{prefix}_options_set_{field}")] ... {OptionsType}Set{Field}`,
            // and the FFI crate emits that setter ONLY for a bridge that resolves both keys
            // (`backends::ffi::gen_bindings::lib_rs` skips the bridge when either is `None`).
            // Guessing `options_type` declares a P/Invoke for a symbol the native library never
            // exports; guessing the field name additionally desyncs the declaration from the call
            // `bridge_field_inject.jinja` emits off the real IR field, which is a C# compile error.
            // `options_type` is documented as required under `bind_via = "options_field"`, so
            // absence is a config defect. `function_param` bridges are filtered out above and are
            // unaffected; `[crates.<name>.ffi] visitor_callbacks = false` skips this block whole. ~keep
            let Some(options_type) = bridge.options_type.as_deref() else {
                anyhow::bail!(
                    "csharp NativeMethods: trait bridge `{trait_name}` declares `bind_via = \"options_field\"` \
                     but no `options_type`; the FFI crate emits no `{prefix}_options_set_*` entry point for such \
                     a bridge and the generated wrapper has no options type to attach the visitor to, so the \
                     visitor P/Invoke block cannot be generated. Set `options_type` on the \
                     `[[crates.trait_bridges]]` entry for `{trait_name}`, or set \
                     `[crates.<name>.ffi] visitor_callbacks = false` to drop visitor support from this backend",
                    trait_name = bridge.trait_name,
                );
            };
            let Some(options_field) = bridge.resolved_options_field() else {
                anyhow::bail!(
                    "csharp NativeMethods: trait bridge `{trait_name}` declares `bind_via = \"options_field\"` \
                     but neither `options_field` nor `param_name`; the setter entry point is named \
                     `{prefix}_options_set_<field>` and cannot be derived. Set `options_field` (or `param_name`) \
                     on the `[[crates.trait_bridges]]` entry for `{trait_name}`, or set \
                     `[crates.<name>.ffi] visitor_callbacks = false` to drop visitor support from this backend",
                    trait_name = bridge.trait_name,
                );
            };
            out.push_str(&crate::backends::csharp::gen_visitor::gen_native_methods_visitor(
                namespace,
                lib_name,
                prefix,
                &bridge.trait_name,
                options_type,
                options_field,
            ));
        }
    }

    if !trait_bridges.is_empty() {
        let trait_defs: Vec<_> = api.types.iter().filter(|t| t.is_trait).collect();

        let bridges: Vec<_> = trait_bridges
            .iter()
            .filter(|config| emits_registered_trait_bridge(has_visitor_callbacks, config))
            .filter_map(|config| {
                let trait_name = config.trait_name.clone();
                trait_defs
                    .iter()
                    .find(|t| t.name == trait_name)
                    .map(|trait_def| (trait_name, config, *trait_def))
            })
            .collect();

        if !bridges.is_empty() {
            let visible_type_names: std::collections::HashSet<&str> = api
                .types
                .iter()
                .filter(|t| !t.is_trait)
                .map(|t| t.name.as_str())
                .collect();
            out.push('\n');
            out.push_str(
                &crate::backends::csharp::trait_bridge::gen_native_methods_trait_bridges(
                    namespace,
                    prefix,
                    &bridges,
                    &visible_type_names,
                ),
            );
        }
    }

    out.push_str("}\n");

    // `NativeMethods.cs` spells the opaque handle as `IntPtr` by hand — nothing here reads the
    // cbindgen header — so a pointer-vs-`uint64_t` straddle against the FFI crate compiles
    // cleanly and only misbehaves at runtime. Stamped inside the backend body so the marker is
    // part of the content `finalize_hashes` hashes; stamping after `inject_hash_line` would
    // leave every generated C# file permanently stale. ~keep
    Ok(crate::core::hash::inject_stamp_line(
        &out,
        crate::core::hash::HANDLE_ABI_STAMP_KEY,
        crate::core::template_versions::abi::HANDLE_ABI_VERSION,
    ))
}

/// Returns true when a function returns bytes — uses the owned out-param convention:
/// `(args..., out IntPtr, out UIntPtr, out UIntPtr) -> int`.
pub(super) fn is_bytes_result_func(func: &FunctionDef) -> bool {
    matches!(func.return_type, TypeRef::Bytes)
}

/// Same check for MethodDef.
pub(super) fn is_bytes_result_method(method: &MethodDef) -> bool {
    matches!(method.return_type, TypeRef::Bytes)
}

pub(super) fn gen_pinvoke_for_func(
    c_name: &str,
    func: &FunctionDef,
    bridge_param_names: &HashSet<String>,
    bridge_type_aliases: &HashSet<String>,
) -> String {
    use crate::backends::csharp::template_env::render;

    let cs_name = to_csharp_name(&func.name);
    let is_bytes_result = is_bytes_result_func(func);

    let mut out = render("dll_import_attr.jinja", minijinja::context! { entry_point => c_name });
    out.push_str("    internal static extern ");

    if is_bytes_result {
        out.push_str("int");
    } else {
        out.push_str(pinvoke_return_type(&func.return_type));
    }

    out.push(' ');
    out.push_str(&cs_name);
    out.push('(');

    let visible_params: Vec<_> = func
        .params
        .iter()
        .filter(|p| !is_bridge_param(p, bridge_param_names, bridge_type_aliases))
        .collect();

    if visible_params.is_empty() && !is_bytes_result {
        out.push_str(");\n\n");
    } else {
        out.push('\n');
        for param in visible_params.iter() {
            out.push_str("        ");
            let pinvoke_ty = pinvoke_param_type(&param.ty);
            if pinvoke_ty == "string" {
                out.push_str("[MarshalAs(UnmanagedType.LPUTF8Str)] ");
            } else if pinvoke_ty == "int" && matches!(param.ty, TypeRef::Primitive(PrimitiveType::Bool)) {
                out.push_str("[MarshalAs(UnmanagedType.U1)] bool ");
                let param_name = param.name.to_lower_camel_case();
                out.push_str(&param_name);
                out.push_str(",\n");
                continue;
            }
            let param_name = param.name.to_lower_camel_case();
            out.push_str(
                render("pinvoke_param.jinja", minijinja::context! { pinvoke_ty, param_name }).trim_end_matches('\n'),
            );
            out.push_str(",\n");
            if matches!(param.ty, TypeRef::Bytes) {
                let len_param_name = format!("{param_name}Len");
                out.push_str(&render(
                    "pinvoke_bytes_len_param.jinja",
                    minijinja::context! { len_param_name },
                ));
            }
        }
        if is_bytes_result {
            out.push_str("        out IntPtr outPtr,\n");
            out.push_str("        out UIntPtr outLen,\n");
            out.push_str("        out UIntPtr outCap\n");
        } else {
            let trim_len = ",\n".len();
            out.truncate(out.len() - trim_len);
            out.push('\n');
        }
        out.push_str("    );\n\n");
    }

    out
}

pub(super) fn gen_pinvoke_for_method(c_name: &str, cs_name: &str, method: &MethodDef) -> String {
    use crate::backends::csharp::template_env::render;

    let is_bytes_result = is_bytes_result_method(method);

    let mut out = render("dll_import_attr.jinja", minijinja::context! { entry_point => c_name });
    out.push_str("    internal static extern ");

    if is_bytes_result {
        out.push_str("int");
    } else {
        out.push_str(pinvoke_return_type(&method.return_type));
    }

    out.push(' ');
    out.push_str(cs_name);
    out.push('(');

    let has_receiver = !method.is_static && method.receiver.is_some();

    let needs_params = has_receiver || !method.params.is_empty() || is_bytes_result;
    if !needs_params {
        out.push_str(");\n\n");
    } else {
        out.push('\n');
        if has_receiver {
            out.push_str("        IntPtr handle,\n");
        }
        for param in method.params.iter() {
            out.push_str("        ");
            let pinvoke_ty = pinvoke_param_type(&param.ty);
            if pinvoke_ty == "string" {
                out.push_str("[MarshalAs(UnmanagedType.LPUTF8Str)] ");
            } else if pinvoke_ty == "int" && matches!(param.ty, TypeRef::Primitive(PrimitiveType::Bool)) {
                out.push_str("[MarshalAs(UnmanagedType.U1)] bool ");
                let param_name = param.name.to_lower_camel_case();
                out.push_str(&param_name);
                out.push_str(",\n");
                continue;
            }
            let param_name = param.name.to_lower_camel_case();
            out.push_str(
                render("pinvoke_param.jinja", minijinja::context! { pinvoke_ty, param_name }).trim_end_matches('\n'),
            );
            out.push_str(",\n");
            if matches!(param.ty, TypeRef::Bytes) {
                let len_param_name = format!("{param_name}Len");
                out.push_str(&render(
                    "pinvoke_bytes_len_param.jinja",
                    minijinja::context! { len_param_name },
                ));
            }
        }
        if is_bytes_result {
            out.push_str("        out IntPtr outPtr,\n");
            out.push_str("        out UIntPtr outLen,\n");
            out.push_str("        out UIntPtr outCap\n");
        } else {
            let trim_len = ",\n".len();
            out.truncate(out.len() - trim_len);
            out.push('\n');
        }
        out.push_str("    );\n\n");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{emits_registered_trait_bridge, ffi_handle_type_names};
    use crate::core::config::NewAlefConfig;
    use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef};

    #[test]
    fn ffi_handle_types_exclude_traits_and_enums() {
        let api = ApiSurface {
            types: vec![type_def("RenderOptions", false), type_def("MarkupVisitor", true)],
            enums: vec![
                EnumDef {
                    name: "NodeKind".to_string(),
                    ..EnumDef::default()
                },
                EnumDef {
                    name: "CrawlEvent".to_string(),
                    variants: vec![EnumVariant {
                        name: "Progress".to_string(),
                        fields: vec![FieldDef::default()],
                        ..EnumVariant::default()
                    }],
                    ..EnumDef::default()
                },
            ],
            ..ApiSurface::default()
        };

        let names = ffi_handle_type_names(&api);

        assert!(names.contains("RenderOptions"));
        assert!(!names.contains("MarkupVisitor"));
        assert!(!names.contains("NodeKind"));
        assert!(names.contains("CrawlEvent"));
    }

    #[test]
    fn visitor_callbacks_preserve_registered_options_field_pinvoke() {
        let config: NewAlefConfig = toml::from_str(
            r#"
[workspace]
languages = ["csharp"]

[[crates]]
name = "sample"
sources = ["src/lib.rs"]

[[crates.trait_bridges]]
trait_name = "MarkupVisitor"
bind_via = "options_field"
options_type = "RenderOptions"
options_field = "visitor"
register_fn = "register_markup_visitor"
registry_getter = "markup_visitor_registry"
"#,
        )
        .unwrap();
        let resolved = config.resolve().unwrap();
        let bridge = &resolved[0].trait_bridges[0];

        assert!(emits_registered_trait_bridge(true, bridge));
    }

    fn type_def(name: &str, is_trait: bool) -> TypeDef {
        TypeDef {
            name: name.to_string(),
            rust_path: format!("sample::{name}"),
            original_rust_path: format!("sample::{name}"),
            is_trait,
            ..TypeDef::default()
        }
    }

    #[test]
    fn native_methods_carry_the_handle_abi_stamp() {
        use crate::core::ir::{FunctionDef, TypeRef};
        use std::collections::{HashMap, HashSet};

        let api = ApiSurface {
            crate_name: "sample".to_string(),
            types: vec![TypeDef {
                is_opaque: true,
                ..type_def("Thing", false)
            }],
            functions: vec![FunctionDef {
                name: "make_thing".to_string(),
                rust_path: "sample::make_thing".to_string(),
                return_type: TypeRef::Named("Thing".to_string()),
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        };

        let native_methods = super::gen_native_methods(
            &api,
            "Sample",
            "sample",
            "sample",
            &HashSet::new(),
            &HashSet::new(),
            false,
            &[],
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
            &[],
        )
        .expect("no trait bridges configured, so generation cannot fail");

        assert!(
            native_methods.contains("IntPtr"),
            "fixture must actually emit the hand-declared handle type this stamp guards:\n{native_methods}"
        );
        crate::backends::ffi::handle_abi_stamp::assert_stamped_before_hashing(
            &native_methods,
            "csharp NativeMethods.cs",
        );
    }

    fn visitor_api() -> ApiSurface {
        use crate::core::ir::{FunctionDef, ParamDef, TypeRef};

        ApiSurface {
            crate_name: "htm".to_string(),
            types: vec![type_def("ConversionOptions", false), type_def("HtmlVisitor", true)],
            functions: vec![FunctionDef {
                name: "convert".to_string(),
                rust_path: "htm::convert".to_string(),
                params: vec![ParamDef {
                    name: "options".to_string(),
                    ty: TypeRef::Named("ConversionOptions".to_string()),
                    ..ParamDef::default()
                }],
                return_type: TypeRef::String,
                ..FunctionDef::default()
            }],
            ..ApiSurface::default()
        }
    }

    fn native_methods_with_visitor_bridge(bridge: crate::core::config::TraitBridgeConfig) -> anyhow::Result<String> {
        use std::collections::{HashMap, HashSet};

        let api = visitor_api();
        super::gen_native_methods(
            &api,
            "Htm",
            "htm_ffi",
            "htm",
            &HashSet::new(),
            &HashSet::new(),
            true,
            std::slice::from_ref(&bridge),
            &HashSet::new(),
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
            &[],
        )
    }

    fn options_field_bridge() -> crate::core::config::TraitBridgeConfig {
        crate::core::config::TraitBridgeConfig {
            trait_name: "HtmlVisitor".to_string(),
            type_alias: Some("VisitorHandle".to_string()),
            param_name: Some("visitor".to_string()),
            bind_via: crate::core::config::BridgeBinding::OptionsField,
            options_type: Some("ConversionOptions".to_string()),
            ..crate::core::config::TraitBridgeConfig::default()
        }
    }

    #[test]
    fn options_field_bridge_without_options_type_fails_generation() {
        let bridge = crate::core::config::TraitBridgeConfig {
            options_type: None,
            ..options_field_bridge()
        };

        let error = native_methods_with_visitor_bridge(bridge)
            .expect_err("a visitor bridge with no `options_type` must not silently fabricate one");
        let message = error.to_string();

        assert!(
            message.contains("HtmlVisitor") && message.contains("no `options_type`"),
            "error must name the offending bridge and the missing key: {message}"
        );
        assert!(
            message.contains("visitor_callbacks = false"),
            "error must name the sanctioned opt-out: {message}"
        );
    }

    #[test]
    fn options_field_bridge_without_a_resolvable_field_fails_generation() {
        let bridge = crate::core::config::TraitBridgeConfig {
            param_name: None,
            options_field: None,
            ..options_field_bridge()
        };

        let error = native_methods_with_visitor_bridge(bridge)
            .expect_err("a visitor bridge with no resolvable options field must not default to `visitor`");
        let message = error.to_string();

        assert!(
            message.contains("neither `options_field` nor `param_name`"),
            "error must name both keys that can supply the field: {message}"
        );
        assert!(
            message.contains("htm_options_set_<field>"),
            "error must show the entry point that cannot be derived: {message}"
        );
    }

    #[test]
    fn options_field_bridge_with_options_type_emits_the_configured_names() {
        let native_methods = native_methods_with_visitor_bridge(options_field_bridge())
            .expect("a fully configured options-field bridge generates");

        let declaration = "internal static extern void ConversionOptionsSetVisitor(IntPtr options, IntPtr visitor);";
        assert!(
            native_methods.contains(declaration),
            "the declaration must carry the configured `options_type`:\n{native_methods}"
        );
        assert!(
            native_methods.contains(r#"EntryPoint = "htm_options_set_visitor""#),
            "the entry point must be derived from the resolved options field:\n{native_methods}"
        );
        // `ConversionOptionsSetVisitor` contains the fabricated `Options` as a substring, so the
        // regression guard has to match the whole declaration, not the bare type name. ~keep
        assert!(
            !native_methods.contains("void OptionsSetVisitor("),
            "the fabricated `Options` type name must not reach the emitted declaration:\n{native_methods}"
        );
    }

    #[test]
    fn function_param_bridge_without_options_type_still_generates() {
        // Absence of `options_type` is legitimate — and the documented default — for
        // `bind_via = "function_param"`; only the options-field shape needs it. ~keep
        let bridge = crate::core::config::TraitBridgeConfig {
            trait_name: "HtmlVisitor".to_string(),
            param_name: Some("visitor".to_string()),
            bind_via: crate::core::config::BridgeBinding::FunctionParam,
            options_type: None,
            ..crate::core::config::TraitBridgeConfig::default()
        };

        let native_methods =
            native_methods_with_visitor_bridge(bridge).expect("function-param bridges never need `options_type`");

        assert!(
            !native_methods.contains("htm_options_set_") && !native_methods.contains("OptionsSetVisitor("),
            "no options setter belongs in a function-param bridge's declarations:\n{native_methods}"
        );
    }
}
