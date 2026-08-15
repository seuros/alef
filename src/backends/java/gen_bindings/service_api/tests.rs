use super::*;
use crate::core::ir::{
    EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, PrimitiveType, RegistrationDef, ServiceDef,
    TypeDef, TypeRef, WrapperConstructorCall,
};

/// Construct a minimal but realistic [`ApiSurface`] that exercises:
/// - A service with a constructor, one registration, and Run entrypoint
/// - One [`HandlerContractDef`] with wire request/response DTO names
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
        error_type: None,
        doc: "Register a request handler.".to_owned(),
        variants: vec![
            crate::core::ir::RegistrationVariant {
                name: "get".to_owned(),
                overrides: vec![],
                wrapper_call: Some(WrapperConstructorCall {
                    metadata_param: "path".into(),
                    wrapper_type_path: "my_crate::Route".into(),
                    wrapper_type_name: "Route".into(),
                    constructor_method: "get".into(),
                    args: vec![],
                }),
                signature_params: vec![ParamDef {
                    name: "path".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                doc: Some("Register a GET handler.".to_owned()),
                style: Default::default(),
                ..Default::default()
            },
            crate::core::ir::RegistrationVariant {
                name: "post".to_owned(),
                overrides: vec![],
                wrapper_call: Some(WrapperConstructorCall {
                    metadata_param: "path".into(),
                    wrapper_type_path: "my_crate::Route".into(),
                    wrapper_type_name: "Route".into(),
                    constructor_method: "post".into(),
                    args: vec![],
                }),
                signature_params: vec![ParamDef {
                    name: "path".into(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                doc: Some("Register a POST handler.".to_owned()),
                style: Default::default(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let run_ep = EntrypointDef {
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
        error_type: Some("ServiceError".to_owned()),
        doc: "Run the service.".to_owned(),
    };

    let service = ServiceDef {
        name: "TestService".to_owned(),
        rust_path: "my_crate::TestService".to_owned(),
        constructor,
        configurators: vec![],
        registrations: vec![registration],
        entrypoints: vec![run_ep],
        doc: "A test service owner.".to_owned(),
        cfg: None,
    };

    let dispatch_method = MethodDef {
        name: "handle".to_owned(),
        params: vec![ParamDef {
            name: "request".to_owned(),
            ty: TypeRef::Named("RequestData".to_owned()),
            optional: false,
            default: None,
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("ResponseData".to_owned()),
        is_async: true,
        is_static: false,
        error_type: Some("HandlerError".to_owned()),
        doc: "Dispatch a request.".to_owned(),
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
    };

    let contract = HandlerContractDef {
        trait_name: "RequestHandler".to_owned(),
        rust_path: "my_crate::RequestHandler".to_owned(),
        dispatch: dispatch_method,
        optional_methods: vec![],
        wire_request_type: Some("RequestData".to_owned()),
        wire_response_type: Some("ResponseData".to_owned()),
        dispatch_extra_params: vec![],
        wire_param_name: None,
        dispatch_return_type: None,
        response_adapter: None,
        doc: "Async trait for handling requests.".to_owned(),
    };

    ApiSurface {
        crate_name: "my_crate".to_owned(),
        version: "0.1.0".to_owned(),
        types: vec![
            TypeDef {
                name: "RequestData".into(),
                has_serde: true,
                ..Default::default()
            },
            TypeDef {
                name: "ResponseData".into(),
                has_serde: true,
                ..Default::default()
            },
        ],
        services: vec![service],
        handler_contracts: vec![contract],
        ..ApiSurface::default()
    }
}

fn make_test_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "test-crate".to_owned(),
        ..ResolvedCrateConfig::default()
    }
}

#[test]
fn java_class_uses_panama_ffm() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(java.contains("import java.lang.foreign.*;"), "should import Panama FFM");
    assert!(java.contains("Linker.nativeLinker()"), "should use Linker");
    assert!(java.contains("downcallHandle"), "should use downcalls");
    assert!(java.contains("SymbolLookup"), "should lookup C symbols");
    assert!(java.contains("FunctionDescriptor"), "should build function descriptors");
    assert!(java.contains("MemorySegment"), "should use MemorySegment");
    assert!(java.contains("Arena"), "should use Arena for lifetime management");
}

#[test]
fn java_class_contains_service_class() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(java.contains("public class TestService"));
    assert!(java.contains("implements AutoCloseable"));
    assert!(java.contains("private MemorySegment ownerHandle"));
}

#[test]
fn java_class_constructor_uses_downcall() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(java.contains("public TestService()"));
    assert!(
        java.contains("test_crate_test_service_new"),
        "constructor should bind to C symbol"
    );
    assert!(
        java.contains("LINKER.downcallHandle"),
        "constructor should use downcall"
    );
}

#[test]
fn java_class_contains_upcall_stub_for_handler() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(
        java.contains("LINKER.upcallStub"),
        "registration should build upcall stub for handler"
    );
    assert!(java.contains("MethodHandle"), "should use MethodHandle to wrap handler");
}

#[test]
fn java_class_registration_binds_to_c_symbol() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(
        java.contains("test_crate_test_service_register_add_handler"),
        "registration should bind to exact C FFI symbol"
    );
}

#[test]
fn java_class_entrypoint_uses_downcall() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(java.contains("public void run(String addr)"));
    assert!(
        java.contains("test_crate_test_service_ep_run"),
        "entrypoint should bind to C symbol"
    );
    assert!(java.contains("LINKER.downcallHandle"), "entrypoint should use downcall");
}

#[test]
fn java_class_close_frees_via_downcall() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(java.contains("@Override"));
    assert!(java.contains("public void close()"));
    assert!(
        java.contains("test_crate_test_service_free"),
        "close should bind to C symbol"
    );
    assert!(java.contains("LINKER.downcallHandle"), "close should use downcall");
    assert!(java.contains("arena.close()"), "arena lifetime should be managed");
    assert!(
        java.contains("private synchronized OwnerHandleLease borrowOwnerHandle()"),
        "{java}"
    );
    assert!(
        java.contains("MemorySegment detached = takeOwnerHandleForClose()"),
        "{java}"
    );
    assert!(java.contains("if (!detached.equals(MemorySegment.NULL))"), "{java}");
    assert!(java.contains("if (markServiceArenaClosed()) arena.close()"), "{java}");
    assert!(java.contains("freeHandle.invoke(detached)"), "{java}");
    assert!(!java.contains("freeHandle.invoke(ownerHandle)"), "{java}");
    assert!(java.contains("while (activeOwnerBorrows != 0)"), "{java}");
    assert!(java.contains("Thread.currentThread().interrupt()"), "{java}");
    assert!(
        java.contains("public void close() {\n        synchronized (ownerMutationLock)"),
        "{java}"
    );
}

#[test]
fn java_class_leases_owner_for_every_service_downcall() {
    let surface = make_fixture_surface();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &make_test_config());

    assert!(java.contains("try (var ownerLease = borrowOwnerHandle()"), "{java}");
    assert!(java.contains("ownerLease.handle(),     // owner"), "{java}");
    assert!(java.contains("try (var ownerTransfer = takeOwnerHandle())"), "{java}");
    assert!(java.contains("epHandle.invoke(ownerTransfer.handle()"), "{java}");
    assert!(java.contains("ownerTransfer.commit();"), "{java}");
    assert!(java.find("ownerTransfer.handle()").unwrap() < java.find("ownerTransfer.commit()").unwrap());
    assert!(!java.contains("epHandle.invoke(ownerHandle"), "{java}");
    assert!(
        !java.contains("applyHandle.invoke"),
        "unsupported configurators must not be emitted:\n{java}"
    );
}

#[test]
fn java_finalize_returns_opaque_wrapper_and_consumes_owner() {
    let mut surface = make_fixture_surface();
    surface.types.push(crate::core::ir::TypeDef {
        name: "Router".into(),
        is_opaque: true,
        ..Default::default()
    });
    let entrypoint = &mut surface.services[0].entrypoints[0];
    entrypoint.kind = EntrypointKind::Finalize;
    entrypoint.return_type = TypeRef::Named("Router".into());

    let java = gen_service_class(&surface, &surface.services[0], "com.example", &make_test_config());

    assert!(java.contains("public Router run("), "{java}");
    assert!(
        java.contains("MemorySegment result = (MemorySegment) epHandle.invoke(ownerTransfer.handle()"),
        "{java}"
    );
    assert!(java.contains("return new Router(result)"), "{java}");
}

#[test]
fn generated_service_owner_gate_compiles_and_blocks_close_until_release() {
    if std::process::Command::new("javac").arg("-version").output().is_err() {
        return;
    }
    let surface = make_fixture_surface();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &make_test_config());
    let directory = tempfile::tempdir().expect("temporary service directory");
    let package_directory = directory.path().join("com/example");
    std::fs::create_dir_all(&package_directory).expect("service package directory");
    std::fs::write(package_directory.join("TestService.java"), java).expect("generated service");
    std::fs::write(
        package_directory.join("NativeLib.java"),
        "package com.example; final class NativeLib {}\n",
    )
    .expect("NativeLib stub");
    std::fs::write(
        package_directory.join("Callable.java"),
        "package com.example; interface Callable { String handle(String request); }\n",
    )
    .expect("Callable stub");
    std::fs::write(
        package_directory.join("ServiceGateMain.java"),
        include_str!("../../../../../tests/fixtures/java_service_gate_main.java"),
    )
    .expect("service gate runtime");

    let compile = std::process::Command::new("javac")
        .args([
            "com/example/NativeLib.java",
            "com/example/Callable.java",
            "com/example/TestService.java",
            "com/example/ServiceGateMain.java",
        ])
        .current_dir(directory.path())
        .output()
        .expect("javac service gate");
    assert!(
        compile.status.success(),
        "generated service must compile:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new("java")
        .args(["-cp", ".", "com.example.ServiceGateMain"])
        .current_dir(directory.path())
        .output()
        .expect("run service gate");
    assert!(
        run.status.success(),
        "service owner gate runtime must pass:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn java_class_no_native_method_declarations() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(
        !java.contains("public native ")
            && !java.contains("private native ")
            && !java.contains("protected native ")
            && !java.contains("static native "),
        "should not contain JNI native method declarations:\n{java}"
    );
    assert!(
        !java.contains("System.loadLibrary"),
        "should not load library (Panama manages it)"
    );
    assert!(!java.contains("Java_"), "should not contain Java_ JNI symbols");
}

#[test]
fn callable_interface_is_functional() {
    let iface = gen_callable_interface("com.example");

    assert!(iface.contains("@FunctionalInterface"));
    assert!(iface.contains("public interface Callable"));
    assert!(iface.contains("String handle(String request)"));
}

#[test]
fn generate_returns_service_and_callable() {
    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    let config = make_test_config();

    let files = generate(&surface, &config).expect("generate should not fail");
    assert!(files.len() >= 2, "expected at least service class + Callable interface");

    let has_service_class = files
        .iter()
        .any(|f| f.path.to_string_lossy().contains("TestService.java"));
    let has_callable = files.iter().any(|f| f.path.to_string_lossy().contains("Callable.java"));

    assert!(has_service_class, "expected TestService.java");
    assert!(has_callable, "expected Callable.java");
}

#[test]
fn generate_returns_empty_for_no_services() {
    let surface = ApiSurface::default();
    let config = make_test_config();

    let files = generate(&surface, &config).expect("generate should not fail");
    assert!(files.is_empty(), "expected no files for surface without services");
}

#[test]
fn generate_rejects_unsupported_service_abi_shapes() {
    let config = make_test_config();

    let mut optional = make_fixture_surface();
    optional.services[0].registrations[0].variants.clear();
    optional.services[0].registrations[0].metadata_params[0].optional = true;
    let error = generate(&optional, &config).expect_err("optional service metadata must be rejected");
    assert!(error.to_string().contains("TestService.add_handler.path"), "{error:#}");

    let mut bytes = make_fixture_surface();
    bytes.services[0].registrations[0].variants.clear();
    bytes.services[0].entrypoints[0].params[0].ty = TypeRef::Bytes;
    let error = generate(&bytes, &config).expect_err("bytes without a length carrier must be rejected");
    assert!(error.to_string().contains("TestService.run.addr"), "{error:#}");

    let mut variants = make_fixture_surface();
    variants.services[0].registrations[0].variants[0].wrapper_call = None;
    let error = generate(&variants, &config).expect_err("variants without FFI wrapper calls must be rejected");
    assert!(error.to_string().contains("TestService.add_handler.get"), "{error:#}");
}

#[test]
fn generate_rejects_nonopaque_finalize_results() {
    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    surface.services[0].entrypoints[0].kind = EntrypointKind::Finalize;
    surface.services[0].entrypoints[0].return_type = TypeRef::Primitive(crate::core::ir::PrimitiveType::I32);

    let error = generate(&surface, &make_test_config()).expect_err("nonopaque finalize must be rejected");
    assert!(error.to_string().contains("TestService.run return"), "{error:#}");
}

#[test]
fn generated_service_uses_shared_java_primitive_carriers() {
    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    surface.services[0].registrations[0].metadata_params = [
        ("flag", PrimitiveType::Bool),
        ("u8_value", PrimitiveType::U8),
        ("u16_value", PrimitiveType::U16),
        ("u32_value", PrimitiveType::U32),
        ("u64_value", PrimitiveType::U64),
        ("i8_value", PrimitiveType::I8),
        ("i16_value", PrimitiveType::I16),
        ("i32_value", PrimitiveType::I32),
        ("i64_value", PrimitiveType::I64),
        ("usize_value", PrimitiveType::Usize),
        ("isize_value", PrimitiveType::Isize),
        ("f32_value", PrimitiveType::F32),
        ("f64_value", PrimitiveType::F64),
    ]
    .into_iter()
    .map(|(name, primitive)| ParamDef {
        name: name.into(),
        ty: TypeRef::Primitive(primitive),
        ..Default::default()
    })
    .collect();

    let files = generate(&surface, &make_test_config()).expect("supported primitive service carriers");
    let java = &files
        .iter()
        .find(|file| file.path.ends_with("TestService.java"))
        .expect("service class")
        .content;

    assert!(java.matches("ValueLayout.JAVA_LONG").count() >= 11, "{java}");
    assert!(java.contains("ValueLayout.JAVA_FLOAT"), "{java}");
    assert!(java.contains("ValueLayout.JAVA_DOUBLE"), "{java}");
    assert!(java.contains("(long) ((flag ? 1 : 0))"), "{java}");
    assert!(java.contains("(long) usizeValue"), "{java}");
    assert!(java.contains("(long) isizeValue"), "{java}");
}

#[test]
fn generated_java_service_rejects_named_params_until_header_matches_runtime() {
    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    surface.services[0].registrations[0].metadata_params[0].ty = TypeRef::Named("RequestData".into());

    let error = generate(&surface, &make_test_config()).expect_err("named header mismatch must be rejected");

    assert!(
        error.to_string().contains("named parameters are unsupported"),
        "{error:#}"
    );
}

#[test]
fn java_backend_service_generation_suppresses_lifetime_bound_types() {
    use crate::core::backend::Backend;

    let mut surface = make_fixture_surface();
    surface.types.push(TypeDef {
        name: "BorrowedConfig".into(),
        has_lifetime_params: true,
        ..Default::default()
    });
    surface.services[0].entrypoints[0].params[0].ty = TypeRef::Named("BorrowedConfig".into());

    let files = crate::backends::java::JavaBackend
        .generate_service_api(&surface, &make_test_config())
        .expect("filtered Java service generation");

    assert!(files.is_empty(), "lifetime-bound service signatures must be suppressed");
}

#[test]
fn java_service_rejects_missing_or_nonserde_callback_wire_types() {
    let mut surface = make_fixture_surface();
    surface.services[0].registrations[0].variants.clear();
    surface
        .types
        .iter_mut()
        .find(|typ| typ.name == "RequestData")
        .unwrap()
        .has_serde = false;

    let error = generate(&surface, &make_test_config()).expect_err("nonserde callback request");

    assert!(
        error.to_string().contains("callback request type RequestData"),
        "{error:#}"
    );
}

#[test]
fn java_class_passes_all_metadata_params() {
    let mut surface = make_fixture_surface();
    let reg = &mut surface.services[0].registrations[0];

    reg.metadata_params.push(ParamDef {
        name: "method".to_owned(),
        ty: TypeRef::String,
        optional: false,
        default: None,
        ..ParamDef::default()
    });
    reg.metadata_params.push(ParamDef {
        name: "priority".to_owned(),
        ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
        optional: false,
        default: None,
        ..ParamDef::default()
    });

    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(
        java.contains("public int registerTestServiceAddHandler(Callable handler, String path"),
        "registration method must include all metadata parameters"
    );

    assert!(
        java.contains("ValueLayout.ADDRESS") || java.contains("ValueLayout.JAVA_INT"),
        "registration should build FunctionDescriptor with metadata param layouts"
    );
}

#[test]
fn java_class_does_not_borrow_record_metadata_as_an_opaque_handle() {
    let mut surface = make_fixture_surface();
    surface.types.push(TypeDef {
        name: "RequestOptions".to_owned(),
        rust_path: "test_crate::RequestOptions".to_owned(),
        is_opaque: false,
        ..TypeDef::default()
    });
    surface.services[0].registrations[0].metadata_params.push(ParamDef {
        name: "options".to_owned(),
        ty: TypeRef::Named("RequestOptions".to_owned()),
        ..ParamDef::default()
    });

    let java = gen_service_class(&surface, &surface.services[0], "com.example", &make_test_config());

    assert!(!java.contains("RequestOptions.HandleLease"), "{java}");
    assert!(!java.contains("options.borrowHandle()"), "{java}");
}

#[test]
fn java_class_marshals_service_metadata_to_ffi_carriers() {
    let mut surface = make_fixture_surface();
    surface.types.push(TypeDef {
        name: "RequestOptions".to_owned(),
        rust_path: "test_crate::RequestOptions".to_owned(),
        ..TypeDef::default()
    });
    surface.services[0].registrations[0].metadata_params.extend([
        ParamDef {
            name: "options".to_owned(),
            ty: TypeRef::Named("RequestOptions".to_owned()),
            ..ParamDef::default()
        },
        ParamDef {
            name: "priority".to_owned(),
            ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::I32),
            ..ParamDef::default()
        },
    ]);

    let java = gen_service_class(&surface, &surface.services[0], "com.example", &make_test_config());

    assert!(
        java.contains("MemorySegment cPath = callArena.allocateFrom(path)"),
        "{java}"
    );
    assert!(java.contains("TEST_CRATE_REQUEST_OPTIONS_FROM_JSON.invoke"), "{java}");
    assert!(java.contains("nativeResources.register(cOptions"), "{java}");
    assert!(java.contains(", (long) priority    // metadata"), "{java}");
    assert!(java.contains("varHandle.invokeWithArguments("), "{java}");
    assert!(java.contains(", cPath    // variant metadata"), "{java}");
    assert!(!java.contains("invokeExact(args)"), "{java}");
}

#[test]
fn java_class_emits_registration_variants() {
    let surface = make_fixture_surface();
    let config = make_test_config();
    let java = gen_service_class(&surface, &surface.services[0], "com.example", &config);

    assert!(
        java.contains("public int get(Callable handler, String path)"),
        "should emit get variant method"
    );
    assert!(
        java.contains("public int post(Callable handler, String path)"),
        "should emit post variant method"
    );

    assert!(
        java.contains("test_crate_test_service_get"),
        "should bind get variant to correct C symbol"
    );
    assert!(
        java.contains("test_crate_test_service_post"),
        "should bind post variant to correct C symbol"
    );

    assert!(
        java.contains("LINKER.downcallHandle"),
        "variant methods should use Panama downcalls"
    );
    assert!(
        java.contains("LINKER.upcallStub"),
        "variant methods should create upcall stubs"
    );
    assert!(
        java.contains("FunctionDescriptor.of"),
        "variant methods should build function descriptors"
    );
}
