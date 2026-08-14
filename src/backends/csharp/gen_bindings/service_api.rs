//! Service-API codegen for the C# backend.
//!
//! Generates two outputs per [`ServiceDef`]:
//!
//! 1. **P/Invoke declarations** — [`DllImport`] stubs matching the C FFI contract
//!    (handlers, registration, entrypoints).
//! 2. **Service class** — An idiomatic C# wrapper that uses P/Invoke to invoke
//!    the Rust service, with registration methods and run/finalize entrypoints.
//!
//! The C# service class exposes:
//! - A constructor mirroring [`ServiceDef::constructor`].
//! - Configurator methods from [`ServiceDef::configurators`].
//! - Registration methods from [`ServiceDef::registrations`] that accept C# delegates
//!   and marshal them via `[UnmanagedCallersOnly]` trampolines + `GCHandle`.
//! - Entrypoint methods (run/finalize) from [`ServiceDef::entrypoints`].
//!
//! All names and signatures are derived entirely from the [`ApiSurface`] IR — no
//! transport- or domain-specific assumptions are made anywhere in this module.

use crate::codegen::naming::{csharp_type_name, to_csharp_name};
use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EntrypointDef, ServiceDef, TypeRef};
use heck::{ToLowerCamelCase, ToSnakeCase, ToUpperCamelCase};
use std::path::PathBuf;

/// Whether an entrypoint's return type can be represented over the C ABI.
/// Opaque types are representable only when this surface wraps them.
/// Unit/primitive/string/bytes/Named opaques are representable;
/// everything else (foreign framework types) is not representable.
fn entrypoint_return_representable(ep: &EntrypointDef, api: &ApiSurface) -> bool {
    match &ep.return_type {
        TypeRef::Unit | TypeRef::String | TypeRef::Char | TypeRef::Primitive(_) | TypeRef::Bytes => true,
        TypeRef::Named(n) => api.types.iter().any(|t| t.name == *n && t.is_opaque),
        _ => false,
    }
}

/// Map TypeRef to C# type name for metadata parameters and return types.
/// For opaque types in this surface, returns the C# wrapper class name (e.g., "GraphQLRouteConfig").
fn csharp_type_for_metadata(ty: &TypeRef, api: &ApiSurface) -> String {
    match ty {
        TypeRef::String | TypeRef::Char => "string".to_owned(),
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "bool".to_owned(),
                PrimitiveType::U8 => "byte".to_owned(),
                PrimitiveType::U16 => "ushort".to_owned(),
                PrimitiveType::U32 => "uint".to_owned(),
                PrimitiveType::U64 => "ulong".to_owned(),
                PrimitiveType::I8 => "sbyte".to_owned(),
                PrimitiveType::I16 => "short".to_owned(),
                PrimitiveType::I32 => "int".to_owned(),
                PrimitiveType::I64 => "long".to_owned(),
                PrimitiveType::F32 => "float".to_owned(),
                PrimitiveType::F64 => "double".to_owned(),
                PrimitiveType::Usize => "nuint".to_owned(),
                PrimitiveType::Isize => "nint".to_owned(),
            }
        }
        TypeRef::Bytes => "byte[]".to_owned(),
        TypeRef::Unit => "void".to_owned(),
        TypeRef::Named(name) => {
            if api.types.iter().any(|t| t.name == *name && t.is_opaque) {
                csharp_type_name(name)
            } else {
                "string".to_owned()
            }
        }
        _ => "string".to_owned(),
    }
}

fn metadata_param_decl_list(params: &[crate::core::ir::ParamDef], api: &ApiSurface) -> String {
    params
        .iter()
        .map(|param| {
            let ty = csharp_type_for_metadata(&param.ty, api);
            let name = param.name.to_lower_camel_case();
            format!("{ty} {name}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn handle_aware_arg(ty: &TypeRef, name: &str, api: &ApiSurface) -> String {
    if matches!(ty, TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n && t.is_opaque)) {
        format!("{name}.Handle")
    } else {
        name.to_owned()
    }
}

fn handle_aware_arg_lines(params: &[crate::core::ir::ParamDef], api: &ApiSurface, indent: &str) -> String {
    params
        .iter()
        .map(|param| {
            let name = param.name.to_lower_camel_case();
            let arg = handle_aware_arg(&param.ty, &name, api);
            format!(",\n{indent}{arg}")
        })
        .collect::<String>()
}

fn native_type_for_metadata(ty: &TypeRef) -> &'static str {
    match ty {
        TypeRef::String | TypeRef::Char => "string",
        TypeRef::Primitive(p) => {
            use crate::core::ir::PrimitiveType;
            match p {
                PrimitiveType::Bool => "int",
                PrimitiveType::U8 => "byte",
                PrimitiveType::U16 => "ushort",
                PrimitiveType::U32 => "uint",
                PrimitiveType::U64 => "ulong",
                PrimitiveType::I8 => "sbyte",
                PrimitiveType::I16 => "short",
                PrimitiveType::I32 => "int",
                PrimitiveType::I64 => "long",
                PrimitiveType::F32 => "float",
                PrimitiveType::F64 => "double",
                PrimitiveType::Usize => "nuint",
                PrimitiveType::Isize => "nint",
            }
        }
        _ => "IntPtr",
    }
}

fn pinvoke_param_lines(params: &[crate::core::ir::ParamDef]) -> String {
    params
        .iter()
        .map(|param| {
            let c_type = native_type_for_metadata(&param.ty);
            format!(",\n        {c_type} {}", param.name)
        })
        .collect::<String>()
}

/// Generate the idiomatic C# service class wrapper.
///
/// The class exposes:
/// - Constructor reflecting the service's Rust constructor
/// - Configurators as fluent builder methods
/// - Registration methods that accept C# delegates
/// - Run/Finalize entrypoint methods
fn gen_service_cs(api: &ApiSurface, service: &ServiceDef, namespace: &str, prefix: &str) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::new();

    let class_name = to_csharp_name(&service.name);
    let service_snake = service.name.to_snake_case();
    let native_free = format!("{}_{}_free", prefix.to_lowercase(), service_snake);
    out.push_str(&render(
        "service_class_header.jinja",
        minijinja::context! {
            namespace,
            service_name => &service.name,
            class_name,
            native_free,
        },
    ));

    {
        let ctor = &service.constructor;
        let params_decl = metadata_param_decl_list(&ctor.params, api);
        let native_new = format!("{}_{}_new", prefix.to_lowercase(), service.name.to_snake_case());
        out.push_str(&render(
            "service_constructor.jinja",
            minijinja::context! {
                service_name => &service.name,
                class_name,
                params_decl,
                native_new,
            },
        ));
    }

    for method in &service.configurators {
        let method_name = &method.name;
        let params_decl = metadata_param_decl_list(&method.params, api);
        out.push_str(&render(
            "service_configurator_method.jinja",
            minijinja::context! {
                class_name,
                method_name,
                params_decl,
            },
        ));
    }

    for reg in &service.registrations {
        let reg_method = &reg.method;
        let metadata_params = metadata_param_decl_list(&reg.metadata_params, api);
        let native_method = format!(
            "{}_{}_register_{}",
            prefix.to_lowercase(),
            service_snake,
            reg_method.to_snake_case()
        );
        let arg_lines = handle_aware_arg_lines(&reg.metadata_params, api, "                ");
        out.push_str(&render(
            "service_registration_method.jinja",
            minijinja::context! {
                method_name => reg_method,
                metadata_params,
                native_method,
                arg_lines,
            },
        ));

        for variant in &reg.variants {
            let variant_method_name = variant.name.to_upper_camel_case();
            let variant_fn_name = variant.name.to_snake_case();
            let doc = variant
                .doc
                .clone()
                .unwrap_or_else(|| format!("Register a handler via the {} variant.", variant.name));
            let signature_params = metadata_param_decl_list(&variant.signature_params, api);
            let native_method = format!("{}_{}_{}", prefix.to_lowercase(), service_snake, variant_fn_name);
            let arg_lines = handle_aware_arg_lines(&variant.signature_params, api, "                ");
            out.push_str(&render(
                "service_variant_registration_method.jinja",
                minijinja::context! {
                    method_name => variant_method_name,
                    doc,
                    signature_params,
                    native_method,
                    arg_lines,
                },
            ));
        }
    }

    for ep in &service.entrypoints {
        let ep_method = &ep.method;
        if !entrypoint_return_representable(ep, api) {
            continue;
        }

        let returns_opaque =
            matches!(&ep.return_type, TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n && t.is_opaque));
        let return_type = if returns_opaque { "ulong" } else { "int" };
        let params_decl = metadata_param_decl_list(&ep.params, api);
        let native_method = format!(
            "{}_{}_ep_{}",
            prefix.to_lowercase(),
            service_snake,
            ep_method.to_snake_case()
        );
        let arg_lines = handle_aware_arg_lines(&ep.params, api, "                ");
        out.push_str(&render(
            "service_entrypoint_method.jinja",
            minijinja::context! {
                method_name => ep_method,
                return_type,
                params_decl,
                native_method,
                arg_lines,
            },
        ));
    }

    out.push_str(&render("service_dispose_method.jinja", minijinja::context! {}));
    out.push_str(&render("service_handler_trampoline.jinja", minijinja::context! {}));

    out.push_str("}\n\n");
    out.push_str("}\n");

    out
}

/// Generate P/Invoke declarations for native service functions.
///
/// Mirrors the C FFI contract exactly: constructor/destructor, registration functions,
/// and entrypoint functions with their exact signatures and names.
/// Opaque metadata parameters are marshalled as IntPtr (handle);
/// other Named/complex types are not expected in P/Invoke metadata.
fn gen_native_methods_cs(api: &ApiSurface, namespace: &str, prefix: &str) -> String {
    use crate::backends::csharp::template_env::render;

    let mut out = String::new();
    out.push_str(&render(
        "service_native_methods_header.jinja",
        minijinja::context! { namespace },
    ));

    for service in &api.services {
        let service_snake = service.name.to_snake_case();

        let dll_name = format!("{}_ffi", prefix.to_lowercase());
        out.push_str(&render(
            "service_native_ctor_free.jinja",
            minijinja::context! {
                dll_name,
                new_method => format!("{}_{}_new", prefix.to_lowercase(), service_snake),
                free_method => format!("{}_{}_free", prefix.to_lowercase(), service_snake),
            },
        ));

        for reg in &service.registrations {
            let reg_method_snake = reg.method.to_snake_case();
            out.push_str(&render(
                "service_pinvoke_declaration.jinja",
                minijinja::context! {
                    dll_name => format!("{}_ffi", prefix.to_lowercase()),
                    return_type => "int",
                    method_name => format!("{}_{}_register_{}", prefix.to_lowercase(), service_snake, reg_method_snake),
                    base_params => "        ulong owner,\n        HandlerCallback callback,\n        HandlerResponseFree responseFree,\n        IntPtr ctx",
                    param_lines => pinvoke_param_lines(&reg.metadata_params),
                },
            ));

            for variant in &reg.variants {
                let variant_fn_name = variant.name.to_snake_case();
                out.push_str(&render(
                    "service_pinvoke_declaration.jinja",
                    minijinja::context! {
                        dll_name => format!("{}_ffi", prefix.to_lowercase()),
                        return_type => "int",
                        method_name => format!("{}_{}_{}", prefix.to_lowercase(), service_snake, variant_fn_name),
                        base_params => "        ulong owner,\n        HandlerCallback callback,\n        HandlerResponseFree responseFree,\n        IntPtr ctx",
                        param_lines => pinvoke_param_lines(&variant.signature_params),
                    },
                ));
            }
        }

        for ep in &service.entrypoints {
            if !entrypoint_return_representable(ep, api) {
                continue;
            }

            let ep_method_snake = ep.method.to_snake_case();
            let returns_opaque =
                matches!(&ep.return_type, TypeRef::Named(n) if api.types.iter().any(|t| t.name == *n && t.is_opaque));
            let return_type = if returns_opaque { "ulong" } else { "int" };
            out.push_str(&render(
                "service_pinvoke_declaration.jinja",
                minijinja::context! {
                    dll_name => format!("{}_ffi", prefix.to_lowercase()),
                    return_type,
                    method_name => format!("{}_{}_ep_{}", prefix.to_lowercase(), service_snake, ep_method_snake),
                    base_params => "        ulong owner",
                    param_lines => pinvoke_param_lines(&ep.params),
                },
            ));
        }
    }

    out.push_str("}\n\n");
    out.push_str("}\n");

    out
}

/// Generate all service-API files for the C# backend.
///
/// Returns two `GeneratedFile`s per non-empty service list:
/// - One C# service class file per service
/// - One P/Invoke native methods file (shared across all services)
pub fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> anyhow::Result<Vec<GeneratedFile>> {
    if api.services.is_empty() {
        return Ok(vec![]);
    }

    let namespace = config.csharp_namespace();
    let prefix = config.ffi_prefix();

    let output_dir = config
        .output_paths
        .get("csharp")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "packages/csharp/".to_owned());

    let base_path = PathBuf::from(&output_dir).join(namespace.replace('.', "/"));

    let mut files = Vec::new();

    for service in &api.services {
        let service_cs = gen_service_cs(api, service, &namespace, &prefix);
        let class_name = to_csharp_name(&service.name);
        files.push(GeneratedFile {
            path: base_path.join(format!("{}.cs", class_name)),
            content: service_cs,
            generated_header: false,
        });
    }

    let native_methods = gen_native_methods_cs(api, &namespace, &prefix);
    files.push(GeneratedFile {
        path: base_path.join("ServiceNativeMethods.cs"),
        content: native_methods,
        generated_header: false,
    });

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{
        EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, RegistrationDef, ServiceDef, TypeRef,
    };

    fn make_fixture_surface() -> ApiSurface {
        let constructor = MethodDef {
            name: "new".to_owned(),
            params: vec![],
            return_type: TypeRef::Unit,
            is_async: false,
            is_static: true,
            error_type: None,
            doc: "Create a new service owner.".to_owned(),
            receiver: None,
            sanitized: false,
            trait_source: None,
            returns_ref: false,
            returns_cow: false,
            return_newtype_wrapper: None,
            has_default_impl: false,
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let registration = RegistrationDef {
            method: "add_handler".to_owned(),
            callback_param: "handler".to_owned(),
            callback_contract: "RequestHandler".to_owned(),
            metadata_params: vec![ParamDef {
                name: "path".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            receiver: Some(crate::core::ir::ReceiverKind::RefMut),
            return_type: TypeRef::Unit,
            error_type: Some("HandlerError".to_owned()),
            doc: "Register a request handler.".to_owned(),
            variants: vec![
                crate::core::ir::RegistrationVariant {
                    name: "get".to_owned(),
                    overrides: vec![],
                    wrapper_call: None,
                    signature_params: vec![ParamDef {
                        name: "path".to_owned(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        ..ParamDef::default()
                    }],
                    doc: Some("Register a GET handler.".to_owned()),
                    style: Default::default(),
                    ..Default::default()
                },
                crate::core::ir::RegistrationVariant {
                    name: "post".to_owned(),
                    overrides: vec![],
                    wrapper_call: None,
                    signature_params: vec![ParamDef {
                        name: "path".to_owned(),
                        ty: TypeRef::String,
                        optional: false,
                        default: None,
                        ..ParamDef::default()
                    }],
                    doc: None,
                    style: Default::default(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let run_entrypoint = EntrypointDef {
            method: "run".to_owned(),
            kind: EntrypointKind::Run,
            is_async: true,
            params: vec![ParamDef {
                name: "addr".to_owned(),
                ty: TypeRef::String,
                optional: false,
                default: None,
                ..ParamDef::default()
            }],
            return_type: TypeRef::Unit,
            error_type: Some("IoError".to_owned()),
            doc: "Start the service.".to_owned(),
        };

        let handler_contract = HandlerContractDef {
            trait_name: "RequestHandler".to_owned(),
            rust_path: "my_crate::RequestHandler".to_owned(),
            dispatch: MethodDef {
                name: "handle".to_owned(),
                params: vec![ParamDef {
                    name: "req".to_owned(),
                    ty: TypeRef::Named("RequestData".to_owned()),
                    optional: false,
                    default: None,
                    ..ParamDef::default()
                }],
                return_type: TypeRef::Named("Response".to_owned()),
                is_async: true,
                is_static: false,
                error_type: None,
                doc: "Handle a request.".to_owned(),
                receiver: Some(crate::core::ir::ReceiverKind::Ref),
                sanitized: false,
                trait_source: None,
                returns_ref: false,
                returns_cow: false,
                return_newtype_wrapper: None,
                has_default_impl: false,
                binding_excluded: false,
                binding_exclusion_reason: None,
                version: Default::default(),
            },
            optional_methods: vec![],
            wire_request_type: Some("RequestData".to_owned()),
            wire_response_type: Some("Response".to_owned()),
            dispatch_extra_params: vec![],
            wire_param_name: None,
            dispatch_return_type: None,
            response_adapter: None,
            doc: "Handler contract.".to_owned(),
        };

        ApiSurface {
            crate_name: "test_crate".to_owned(),
            version: "1.0.0".to_owned(),
            services: vec![ServiceDef {
                name: "TestService".to_owned(),
                rust_path: "my_crate::TestService".to_owned(),
                constructor,
                configurators: vec![],
                registrations: vec![registration],
                entrypoints: vec![run_entrypoint],
                doc: "Test service.".to_owned(),
                cfg: None,
            }],
            handler_contracts: vec![handler_contract],
            ..ApiSurface::default()
        }
    }

    #[test]
    fn test_gen_service_cs_contains_class() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        assert!(cs.contains("public class TestService"));
        assert!(cs.contains("internal sealed class TestServiceSafeHandle : SafeHandle"));
        assert!(cs.contains("private readonly TestServiceSafeHandle _safeHandle"));
        assert!(cs.contains("NativeMethods.test_test_service_free(_nativeHandle)"));
        assert!(cs.contains("public TestService()"));
    }

    #[test]
    fn test_gen_service_cs_contains_registration_method() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        assert!(cs.contains("public int add_handler("));
        assert!(cs.contains("GCHandle.Alloc(handler, GCHandleType.Normal)"));
        assert!(cs.contains("ArgumentNullException.ThrowIfNull(handler)"));
        assert!(cs.contains("_safeHandle.AlefHandle"));
        assert!(cs.contains("_safeHandle.DangerousAddRef(ref handleAdded)"));
        assert!(cs.contains("if (handleAdded) _safeHandle.DangerousRelease()"));
        assert!(cs.contains("catch {\n            handle.Free();"));
        assert!(cs.contains("_handlerCallback"));
        assert!(cs.contains("_registeredCallbacks[ctx] = handle"));
    }

    #[test]
    fn test_gen_service_cs_contains_run_method() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        assert!(cs.contains("public int run("));
        assert!(cs.contains("NativeMethods.test_test_service_ep_run"));
        assert!(cs.contains("_safeHandle.DangerousAddRef(ref handleAdded)"));
        assert!(cs.contains("if (handleAdded) _safeHandle.DangerousRelease()"));
    }

    #[test]
    fn test_gen_service_cs_contains_unmanaged_callback() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        assert!(cs.contains("public static IntPtr HandlerTrampoline"));
        assert!(cs.contains("_handlerCallback = HandlerTrampoline"));
        assert!(cs.contains("GCHandle.FromIntPtr(ctx)"));
        assert!(cs.contains("Marshal.PtrToStringUTF8"));
        assert!(cs.contains("_handlerResponseFree = FreeHandlerResponse"));
        assert!(cs.contains("Marshal.FreeCoTaskMem(responsePtr)"));
    }

    #[test]
    fn test_gen_service_cs_trampoline_invokes_delegate() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        assert!(
            cs.contains("if (handle.Target is Func<string, string> handler)"),
            "trampoline must cast to Func<string, string>"
        );

        assert!(
            cs.contains("handler(requestStr)"),
            "trampoline must invoke the handler with request string"
        );

        assert!(
            cs.contains("string responseStr = handler(requestStr);"),
            "trampoline must capture delegate result into responseStr"
        );

        assert!(
            !cs.contains("\"stub implementation\""),
            "trampoline must not have stub implementation comment"
        );
        assert!(
            !cs.contains("string responseStr = \"{}\""),
            "trampoline must not return hardcoded {{}} response"
        );

        assert!(
            cs.contains("Marshal.StringToCoTaskMemUTF8(responseStr)"),
            "trampoline must marshal the response back to native memory"
        );
    }

    #[test]
    fn test_gen_native_methods_cs_contains_callback_typedef() {
        let api = make_fixture_surface();
        let native = gen_native_methods_cs(&api, "MyNamespace", "test");

        assert!(native.contains("delegate IntPtr HandlerCallback"));
        assert!(native.contains("delegate void HandlerResponseFree(IntPtr responsePtr)"));
        assert!(native.contains("[UnmanagedFunctionPointer(CallingConvention.Cdecl)]"));
    }

    #[test]
    fn test_gen_native_methods_cs_contains_pinvoke_decls() {
        let api = make_fixture_surface();
        let native = gen_native_methods_cs(&api, "MyNamespace", "test");

        assert!(native.contains("[DllImport("));
        assert!(native.contains("test_test_service_new()"));
        assert!(native.contains("test_test_service_free"));
        assert!(native.contains("test_test_service_register_add_handler"));
        assert!(native.contains("test_test_service_ep_run"));
    }

    #[test]
    fn test_generate_returns_files() {
        let api = make_fixture_surface();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let files = generate(&api, &config).expect("generate should not fail");
        assert!(!files.is_empty(), "expected at least one file");

        let has_service_class = files
            .iter()
            .any(|f| f.path.to_string_lossy().contains("TestService.cs"));
        let has_native_methods = files
            .iter()
            .any(|f| f.path.to_string_lossy().contains("ServiceNativeMethods.cs"));

        assert!(has_service_class, "expected TestService.cs in output");
        assert!(has_native_methods, "expected ServiceNativeMethods.cs in output");
    }

    #[test]
    fn test_generate_returns_empty_for_no_services() {
        let api = ApiSurface::default();
        let config = ResolvedCrateConfig {
            name: "test_crate".to_owned(),
            ..ResolvedCrateConfig::default()
        };

        let files = generate(&api, &config).expect("generate should not fail");
        assert!(files.is_empty(), "expected no files for surface without services");
    }

    #[test]
    fn test_gen_service_cs_contains_variant_methods() {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let cs = gen_service_cs(&api, service, "MyNamespace", "test");

        assert!(
            cs.contains("public int Get("),
            "expected Get variant method in service class"
        );
        assert!(
            cs.contains("Register a GET handler"),
            "expected Get variant documentation"
        );

        assert!(
            cs.contains("public int Post("),
            "expected Post variant method in service class"
        );
        assert!(
            cs.contains("Register a handler via the post variant"),
            "expected Post variant auto-generated documentation"
        );

        assert!(
            cs.contains("NativeMethods.test_test_service_get("),
            "expected Get variant P/Invoke call"
        );
        assert!(
            cs.contains("NativeMethods.test_test_service_post("),
            "expected Post variant P/Invoke call"
        );
    }

    #[test]
    fn test_gen_native_methods_cs_contains_variant_pinvoke_decls() {
        let api = make_fixture_surface();
        let native = gen_native_methods_cs(&api, "MyNamespace", "test");

        assert!(
            native.contains("public static extern int test_test_service_get("),
            "expected Get variant P/Invoke declaration"
        );

        assert!(
            native.contains("public static extern int test_test_service_post("),
            "expected Post variant P/Invoke declaration"
        );

        assert!(
            native.contains("ulong owner,")
                && native.contains("HandlerCallback callback,")
                && native.contains("HandlerResponseFree responseFree,"),
            "expected variant P/Invoke to carry the callback's matching deallocator"
        );
    }

    // ~keep Regression coverage for 2cb44dc09, which reflowed the C# service templates onto
    // run-together ~118-column lines and, in service_constructor.jinja, split a
    // `{{ class_name }}` interpolation across three template lines. 647da87ea repaired four of
    // the six service templates; these checks assert the contract that repair restored, proving
    // each check fails on the corrupted template text (frozen below, not read via git at test
    // time) and passes on the repaired one.

    /// Every line that is, or belongs to, an XML doc-comment (`<summary>`/`<param>` tags) must
    /// keep its leading `///`, and must not carry a second, stray `///` later on the line.
    fn doc_comment_violations(rendered: &str) -> Vec<String> {
        let mut violations = Vec::new();
        let mut in_doc_block = false;
        for (index, line) in rendered.lines().enumerate() {
            let trimmed = line.trim();
            let line_number = index + 1;
            let is_doc_tag_line = trimmed.contains("<summary>")
                || trimmed.contains("</summary>")
                || trimmed.contains("<param")
                || trimmed.contains("</param>");
            if trimmed.contains("<summary>") && !trimmed.contains("</summary>") {
                in_doc_block = true;
            }
            if is_doc_tag_line || in_doc_block {
                if let Some(rest) = trimmed.strip_prefix("///") {
                    if rest.contains("///") {
                        violations.push(format!("line {line_number}: stray /// inside doc line: {line:?}"));
                    }
                } else {
                    violations.push(format!(
                        "line {line_number}: doc-comment line missing /// prefix: {line:?}"
                    ));
                }
            }
            if trimmed.contains("</summary>") {
                in_doc_block = false;
            }
        }
        violations
    }

    /// A statement-terminating `;` must be the last non-brace, non-whitespace content on its
    /// line — otherwise a second statement has been crammed onto the same rendered line.
    fn multi_statement_line_violations(rendered: &str) -> Vec<String> {
        let mut violations = Vec::new();
        for (index, line) in rendered.lines().enumerate() {
            let Some(semicolon_position) = line.find(';') else {
                continue;
            };
            let trailing = line[semicolon_position + 1..].trim_start_matches(|c: char| c == '}' || c.is_whitespace());
            if !trailing.is_empty() {
                violations.push(format!("line {}: statement continues after ';': {line:?}", index + 1));
            }
        }
        violations
    }

    /// A `{{ ... }}` interpolation tag's delimiters must both land on the same line of the
    /// **template source**. This deliberately does not check rendered output: a substituted
    /// value (like `arg_lines`) legitimately embeds newlines by design, so only a source-level
    /// split — the actual 2cb44dc09 defect — indicates corruption. ~keep
    fn split_interpolation_violations(template_source: &str) -> Vec<String> {
        let mut balance: i32 = 0;
        let mut violations = Vec::new();
        for (index, line) in template_source.lines().enumerate() {
            if balance > 0 {
                violations.push(format!(
                    "line {}: continues a `{{{{ }}}}` interpolation opened on an earlier line",
                    index + 1
                ));
            }
            balance += line.matches("{{").count() as i32 - line.matches("}}").count() as i32;
        }
        violations
    }

    /// A member-access chain must not be broken so that one line ends on an identifier/`)` and
    /// the next line opens with the `.` that continues it.
    fn split_member_access_violations(rendered: &str) -> Vec<String> {
        let lines: Vec<&str> = rendered.lines().collect();
        let mut violations = Vec::new();
        for index in 0..lines.len().saturating_sub(1) {
            let current_ends_identifier = lines[index]
                .trim_end()
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ')');
            let next_starts_dot = lines[index + 1].trim_start().starts_with('.');
            if current_ends_identifier && next_starts_dot {
                violations.push(format!(
                    "line {}: member-access chain broken across lines: {:?} / {:?}",
                    index + 1,
                    lines[index],
                    lines[index + 1]
                ));
            }
        }
        violations
    }

    /// Context values mirroring `gen_service_cs`'s own calls to `render`, so each of the six
    /// service templates is exercised the same way production code exercises it.
    fn service_template_render_cases() -> Vec<(&'static str, minijinja::Value)> {
        vec![
            (
                "service_constructor.jinja",
                minijinja::context! {
                    service_name => "TestService",
                    class_name => "TestService",
                    params_decl => "string path",
                    native_new => "my_lib_test_service_new",
                },
            ),
            (
                "service_entrypoint_method.jinja",
                minijinja::context! {
                    method_name => "run",
                    return_type => "int",
                    params_decl => "string addr",
                    native_method => "my_lib_test_service_ep_run",
                    arg_lines => ",\n                addr",
                },
            ),
            (
                "service_registration_method.jinja",
                minijinja::context! {
                    method_name => "add_handler",
                    metadata_params => "string path",
                    native_method => "my_lib_test_service_register_add_handler",
                    arg_lines => ",\n                path",
                },
            ),
            (
                "service_variant_registration_method.jinja",
                minijinja::context! {
                    method_name => "Get",
                    doc => "Register a GET handler.",
                    signature_params => "string path",
                    native_method => "my_lib_test_service_get",
                    arg_lines => ",\n                path",
                },
            ),
            (
                "service_class_header.jinja",
                minijinja::context! {
                    namespace => "MyNamespace",
                    service_name => "TestService",
                    class_name => "TestService",
                    native_free => "my_lib_test_service_free",
                },
            ),
            ("service_dispose_method.jinja", minijinja::context! {}),
        ]
    }

    const SERVICE_CONSTRUCTOR_SOURCE: &str = include_str!("../templates/service_constructor.jinja");
    const SERVICE_ENTRYPOINT_SOURCE: &str = include_str!("../templates/service_entrypoint_method.jinja");
    const SERVICE_REGISTRATION_SOURCE: &str = include_str!("../templates/service_registration_method.jinja");
    const SERVICE_VARIANT_REGISTRATION_SOURCE: &str =
        include_str!("../templates/service_variant_registration_method.jinja");
    const SERVICE_CLASS_HEADER_SOURCE: &str = include_str!("../templates/service_class_header.jinja");
    const SERVICE_DISPOSE_SOURCE: &str = include_str!("../templates/service_dispose_method.jinja");

    #[test]
    fn test_service_templates_retain_triple_slash_doc_comments() {
        use crate::backends::csharp::template_env::render;

        for (name, context) in service_template_render_cases() {
            let rendered = render(name, context);
            let violations = doc_comment_violations(&rendered);
            assert!(violations.is_empty(), "{name}: {violations:?}\n---\n{rendered}");
        }
    }

    #[test]
    fn test_service_templates_enforce_one_statement_per_line() {
        use crate::backends::csharp::template_env::render;

        for (name, context) in service_template_render_cases() {
            let rendered = render(name, context);
            let violations = multi_statement_line_violations(&rendered);
            assert!(violations.is_empty(), "{name}: {violations:?}\n---\n{rendered}");
        }
    }

    #[test]
    fn test_service_templates_no_split_member_access_chains() {
        use crate::backends::csharp::template_env::render;

        for (name, context) in service_template_render_cases() {
            let rendered = render(name, context);
            let violations = split_member_access_violations(&rendered);
            assert!(violations.is_empty(), "{name}: {violations:?}\n---\n{rendered}");
        }
    }

    #[test]
    fn test_service_templates_source_has_no_split_interpolation_tags() {
        for (name, source) in [
            ("service_constructor.jinja", SERVICE_CONSTRUCTOR_SOURCE),
            ("service_entrypoint_method.jinja", SERVICE_ENTRYPOINT_SOURCE),
            ("service_registration_method.jinja", SERVICE_REGISTRATION_SOURCE),
            (
                "service_variant_registration_method.jinja",
                SERVICE_VARIANT_REGISTRATION_SOURCE,
            ),
            ("service_class_header.jinja", SERVICE_CLASS_HEADER_SOURCE),
            ("service_dispose_method.jinja", SERVICE_DISPOSE_SOURCE),
        ] {
            let violations = split_interpolation_violations(source);
            assert!(violations.is_empty(), "{name}: {violations:?}");
        }
    }

    #[test]
    fn test_split_member_access_violations_detects_a_broken_chain() {
        // None of the four templates 2cb44dc09 corrupted happened to break a member-access
        // chain specifically, so this proves the checker itself works against a minimal
        // fixture shaped like the corruption's style (a call target left dangling at line end).
        let broken = "_safeHandle\n    .DangerousAddRef(ref handleAdded);";
        let violations = split_member_access_violations(broken);
        assert_eq!(violations.len(), 1, "{violations:?}");

        let fine = "_safeHandle.DangerousAddRef(ref handleAdded);";
        assert!(split_member_access_violations(fine).is_empty());
    }

    /// Mirrors `template_env::make_env`'s settings so the frozen corrupted-template literals
    /// below render exactly as the real 2cb44dc09 templates would have.
    fn render_frozen_template(source: &str, context: minijinja::Value) -> String {
        let mut env = minijinja::Environment::new();
        env.set_trim_blocks(true);
        env.set_lstrip_blocks(true);
        env.set_keep_trailing_newline(true);
        env.add_template("frozen", source)
            .expect("frozen template source is valid jinja");
        env.get_template("frozen")
            .expect("frozen template registered")
            .render(context)
            .expect("frozen template renders")
    }

    // Frozen at 2cb44dc09 via `git show 2cb44dc09:src/backends/csharp/templates/<name>.jinja`;
    // 647da87ea repaired the live templates, so these literals are the only remaining record of
    // the corruption these tests guard against. ~keep
    const CORRUPTED_SERVICE_CONSTRUCTOR: &str = "/// <summary>\n  /// Create a new {{ service_name }}. ///\n</summary>\npublic {{ class_name }}({{ params_decl }}) { var handle = NativeMethods.{{ native_new }}(); if (handle == IntPtr.Zero) {\nthrow new InvalidOperationException(\"Native service constructor returned a null handle\"); } _safeHandle = new {{\n  class_name\n}}SafeHandle(handle); }\n";

    const CORRUPTED_SERVICE_ENTRYPOINT: &str = "/// <summary>\n  /// {{ method_name }}. ///\n</summary>\npublic {{ return_type }} {{ method_name }}({{ params_decl }}) { bool handleAdded = false; try {\n_safeHandle.DangerousAddRef(ref handleAdded); return NativeMethods.{{ native_method }}(\n_safeHandle.DangerousGetHandle(){{ arg_lines }}\n); } finally { if (handleAdded) _safeHandle.DangerousRelease(); } }\n";

    const CORRUPTED_SERVICE_REGISTRATION: &str = "/// <summary>\n  /// Register a handler for {{ method_name }}. ///\n</summary>\npublic int {{ method_name }}({% if metadata_params %}{{ metadata_params }},\n{% endif %}Delegate handler) { ArgumentNullException.ThrowIfNull(handler); var handle = GCHandle.Alloc(handler,\nGCHandleType.Normal); IntPtr ctx = GCHandle.ToIntPtr(handle); bool handleAdded = false; int result; try {\n_safeHandle.DangerousAddRef(ref handleAdded); result = NativeMethods.{{ native_method }}(\n_safeHandle.DangerousGetHandle(), _handlerCallback, ctx{{ arg_lines }}\n); } catch { handle.Free(); throw; } finally { if (handleAdded) _safeHandle.DangerousRelease(); } if (result == 0) { //\nKeep the GCHandle alive for the lifetime of the registration lock (_registeredCallbacks) { _registeredCallbacks[ctx] =\nhandle; } } else { handle.Free(); } return result; }\n";

    const CORRUPTED_SERVICE_VARIANT_REGISTRATION: &str = "/// <summary>\n  /// {{ doc }}\n  ///\n</summary>\npublic int {{ method_name }}({% if signature_params %}{{ signature_params }},\n{% endif %}Delegate handler) { ArgumentNullException.ThrowIfNull(handler); var handle = GCHandle.Alloc(handler,\nGCHandleType.Normal); IntPtr ctx = GCHandle.ToIntPtr(handle); bool handleAdded = false; int result; try {\n_safeHandle.DangerousAddRef(ref handleAdded); result = NativeMethods.{{ native_method }}(\n_safeHandle.DangerousGetHandle(), _handlerCallback, ctx{{ arg_lines }}\n); } catch { handle.Free(); throw; } finally { if (handleAdded) _safeHandle.DangerousRelease(); } if (result == 0) {\nlock (_registeredCallbacks) { _registeredCallbacks[ctx] = handle; } } else { handle.Free(); } return result; }\n";

    #[test]
    fn test_corrupted_service_constructor_fails_all_three_contract_checks() {
        let rendered = render_frozen_template(
            CORRUPTED_SERVICE_CONSTRUCTOR,
            minijinja::context! {
                service_name => "TestService",
                class_name => "TestService",
                params_decl => "string path",
                native_new => "my_lib_test_service_new",
            },
        );
        assert!(
            !doc_comment_violations(&rendered).is_empty(),
            "expected the bare </summary> line to be caught"
        );
        assert!(
            !multi_statement_line_violations(&rendered).is_empty(),
            "expected the run-together statements to be caught"
        );
        assert!(
            !split_interpolation_violations(CORRUPTED_SERVICE_CONSTRUCTOR).is_empty(),
            "expected the {{ class_name }} split across three lines to be caught"
        );
    }

    #[test]
    fn test_corrupted_service_entrypoint_fails_contract_checks() {
        let rendered = render_frozen_template(
            CORRUPTED_SERVICE_ENTRYPOINT,
            minijinja::context! {
                method_name => "run",
                return_type => "int",
                params_decl => "string addr",
                native_method => "my_lib_test_service_ep_run",
                arg_lines => ",\n                addr",
            },
        );
        assert!(!doc_comment_violations(&rendered).is_empty());
        assert!(!multi_statement_line_violations(&rendered).is_empty());
    }

    #[test]
    fn test_corrupted_service_registration_fails_contract_checks() {
        let rendered = render_frozen_template(
            CORRUPTED_SERVICE_REGISTRATION,
            minijinja::context! {
                method_name => "add_handler",
                metadata_params => "string path",
                native_method => "my_lib_test_service_register_add_handler",
                arg_lines => ",\n                path",
            },
        );
        assert!(!doc_comment_violations(&rendered).is_empty());
        assert!(!multi_statement_line_violations(&rendered).is_empty());
    }

    #[test]
    fn test_corrupted_service_variant_registration_fails_contract_checks() {
        let rendered = render_frozen_template(
            CORRUPTED_SERVICE_VARIANT_REGISTRATION,
            minijinja::context! {
                method_name => "Get",
                doc => "Register a GET handler.",
                signature_params => "string path",
                native_method => "my_lib_test_service_get",
                arg_lines => ",\n                path",
            },
        );
        assert!(!doc_comment_violations(&rendered).is_empty());
        assert!(!multi_statement_line_violations(&rendered).is_empty());
    }

    fn write_dotnet_allocator_fixture(directory: &std::path::Path) {
        let api = make_fixture_surface();
        let service = &api.services[0];
        let service_cs = gen_service_cs(&api, service, "MyNamespace", "my_lib");
        let native_methods_cs = gen_native_methods_cs(&api, "MyNamespace", "my_lib");
        std::fs::write(directory.join("TestService.cs"), service_cs).expect("write service class");
        std::fs::write(directory.join("ServiceNativeMethods.cs"), native_methods_cs).expect("write native methods");
        let project = format!(
            "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>Exe</OutputType>\n    \
             <TargetFramework>{}</TargetFramework>\n    <Nullable>enable</Nullable>\n  \
             </PropertyGroup>\n</Project>\n",
            dotnet_target_framework()
        );
        std::fs::write(directory.join("Service.csproj"), project).expect("write project file");
        std::fs::write(directory.join("Program.cs"), DOTNET_ALLOCATOR_HARNESS).expect("write allocator harness");
    }

    fn dotnet_target_framework() -> String {
        let output = std::process::Command::new("dotnet")
            .arg("--version")
            .output()
            .expect("query dotnet SDK version");
        let version = String::from_utf8(output.stdout).expect("dotnet version is UTF-8");
        let major = version.trim().split('.').next().expect("dotnet major version");
        format!("net{major}.0")
    }

    fn run_dotnet_allocator_fixture(directory: &std::path::Path) -> std::process::Output {
        let dotnet_cli_home = directory.join(".dotnet");
        let nuget_packages = directory.join(".nuget/packages");
        std::fs::create_dir_all(&dotnet_cli_home).expect("isolated DOTNET_CLI_HOME");
        std::fs::create_dir_all(&nuget_packages).expect("isolated NUGET_PACKAGES");
        std::process::Command::new("dotnet")
            .args(["run", "--nologo", "-v", "quiet"])
            .current_dir(directory)
            .env("DOTNET_CLI_HOME", &dotnet_cli_home)
            .env("NUGET_PACKAGES", &nuget_packages)
            .output()
            .expect("run generated service allocator harness")
    }

    const DOTNET_ALLOCATOR_HARNESS: &str = r#"namespace MyNamespace;
using System;
using System.Runtime.InteropServices;
public static class Program {
    public static void Main() {
        var callback = GCHandle.Alloc(new Func<string, string>(_ => "{}"));
        IntPtr request = Marshal.StringToCoTaskMemUTF8("{}");
        IntPtr response = IntPtr.Zero;
        try {
            response = TestService.HandlerTrampoline(GCHandle.ToIntPtr(callback), request);
            if (Marshal.PtrToStringUTF8(response) != "{}") throw new InvalidOperationException();
        } finally {
            TestService.FreeHandlerResponse(response);
            Marshal.FreeCoTaskMem(request);
            callback.Free();
        }
    }
}
"#;

    #[test]
    fn test_service_callback_allocator_compiles_and_runs_when_dotnet_is_available() {
        if which::which("dotnet").is_err() {
            return;
        }
        let directory = tempfile::tempdir().expect("temp directory");
        write_dotnet_allocator_fixture(directory.path());
        let output = run_dotnet_allocator_fixture(directory.path());

        assert!(
            output.status.success(),
            "generated service allocator harness failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
