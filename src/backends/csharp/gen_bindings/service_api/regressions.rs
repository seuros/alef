use super::{gen_native_methods_cs, gen_service_cs};
use crate::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, MethodDef, ParamDef, ReceiverKind, RegistrationDef, ServiceDef, TypeDef,
    TypeRef,
};

fn named_config_param() -> ParamDef {
    ParamDef {
        name: "config".to_owned(),
        ty: TypeRef::Named("ServerConfig".to_owned()),
        ..ParamDef::default()
    }
}

fn surface_with_named_service_calls() -> ApiSurface {
    let configurator = MethodDef {
        name: "config".to_owned(),
        params: vec![named_config_param()],
        return_type: TypeRef::Named("App".to_owned()),
        receiver: Some(ReceiverKind::Owned),
        cfg: None,
        ..MethodDef::default()
    };
    let registration = RegistrationDef {
        method: "add_handler".to_owned(),
        metadata_params: vec![named_config_param()],
        ..RegistrationDef::default()
    };
    let entrypoint = EntrypointDef {
        method: "run".to_owned(),
        kind: EntrypointKind::Run,
        is_async: false,
        params: vec![named_config_param()],
        return_type: TypeRef::Unit,
        error_type: None,
        doc: String::new(),
    };
    ApiSurface {
        crate_name: "demo".to_owned(),
        types: vec![TypeDef {
            name: "ServerConfig".to_owned(),
            ..TypeDef::default()
        }],
        services: vec![ServiceDef {
            name: "App".to_owned(),
            configurators: vec![configurator],
            registrations: vec![registration],
            entrypoints: vec![entrypoint],
            rust_path: "demo::App".to_owned(),
            constructor: MethodDef::default(),
            doc: String::new(),
            cfg: None,
        }],
        ..ApiSurface::default()
    }
}

#[test]
fn all_service_calls_marshal_named_records_through_owned_ffi_handles() {
    let api = surface_with_named_service_calls();
    let output = gen_service_cs(&api, &api.services[0], "Demo", "demo");

    assert_eq!(output.matches("ServerConfig config").count(), 3, "{output}");
    assert_eq!(
        output.matches("NativeMethods.ServerConfigFromJson(configJson)").count(),
        3,
        "{output}"
    );
    assert_eq!(output.matches("ulong configHandle = 0;").count(), 3, "{output}");
    assert_eq!(output.matches("if (configHandle == 0) {").count(), 3, "{output}");
    assert_eq!(
        output
            .matches("if (configHandle != 0) NativeMethods.ServerConfigFree(configHandle);")
            .count(),
        3,
        "{output}"
    );
    assert!(
        !output.contains("IntPtr configHandle") && !output.contains("configHandle.ToInt64()"),
        "a handle local fed to a `ulong` P/Invoke must never be declared or narrowed as a \
         pointer:\n{output}"
    );
    assert_eq!(
        output.matches("NativeMethods.ServerConfigFree(configHandle)").count(),
        3,
        "{output}"
    );
    assert!(output.contains("throw ResolveLastError();"), "{output}");
    assert!(output.contains("new DemoException(code, message)"), "{output}");
}

#[test]
fn configurator_calls_ffi_and_preserves_the_stable_owner() {
    let api = surface_with_named_service_calls();
    let output = gen_service_cs(&api, &api.services[0], "Demo", "demo");

    assert!(output.contains("NativeMethods.demo_app_config("), "{output}");
    assert!(
        output.contains("using var ownerLease = BorrowOwnerHandle()"),
        "{output}"
    );
    assert!(output.contains("configuredHandle != ownerLease.Handle"), "{output}");
    assert!(!output.contains("// Store configuration if needed"), "{output}");
}

#[test]
fn named_service_parameters_use_canonical_scalar_pinvoke_handles() {
    let api = surface_with_named_service_calls();
    let output = gen_native_methods_cs(&api, "Demo", "demo");

    assert!(output.contains("extern ulong demo_app_config("), "{output}");
    assert_eq!(output.matches("ulong config").count(), 3, "{output}");

    // The declaration side alone is vacuous: it passed while the caller in
    // `all_service_calls_marshal_named_records_through_owned_ffi_handles` narrowed an `IntPtr`
    // local into these very parameters. Pin the pair together so the two sides cannot drift. ~keep
    let service = gen_service_cs(&api, &api.services[0], "Demo", "demo");
    assert!(service.contains("ulong configHandle = 0;"), "{service}");
}

#[test]
fn callback_roots_are_scoped_to_each_service_instance() {
    let api = surface_with_named_service_calls();
    let output = gen_service_cs(&api, &api.services[0], "Demo", "demo");

    assert!(
        output.contains("private readonly Dictionary<IntPtr, GCHandle> _registeredCallbacks"),
        "{output}"
    );
    assert!(
        !output.contains("private static readonly Dictionary<IntPtr, GCHandle>"),
        "{output}"
    );
}
