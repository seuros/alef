//! Parity between the service-API C symbols Go *calls* and the ones the FFI backend *exports*.
//!
//! Go and the FFI backend used to compose these names independently from the same IR. The
//! formulas happened to agree, so this file is a guard rather than a bug fix: it asserts against
//! the FFI backend's *emitted output* — the `extern "C" fn` items cbindgen turns into the header
//! — so a future edit to either derivation is caught the moment the two stop matching.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, HandlerContractDef, MethodDef, ParamDef, ReceiverKind, RegistrationDef,
    ServiceDef, TypeRef,
};

const PREFIX: &str = "demo";

/// Service, registration and entrypoint names whose snake spelling is not their own — an
/// embedded acronym, consecutive capitals, a digit boundary, a leading underscore. Ordinary
/// `snake_case` fixture names would spell the same string under any casing helper, so a table of
/// those would pass no matter which derivation either side used. ~keep
const ADVERSARIAL_SERVICE_NAMES: &[&str] = &["HTTPRouter", "UTF8Gateway", "Base64Relay", "_InternalBus", "AHub"];

const ADVERSARIAL_METHOD_NAMES: &[&str] = &["addURLHandler", "utf8Route", "Base64Sink", "_hidden", "run"];

fn param(name: &str) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty: TypeRef::String,
        ..ParamDef::default()
    }
}

fn dispatch_method() -> MethodDef {
    MethodDef {
        name: "handle".to_string(),
        params: vec![ParamDef {
            name: "req".to_string(),
            ty: TypeRef::Named("RequestData".to_string()),
            ..ParamDef::default()
        }],
        return_type: TypeRef::Named("ResponseData".to_string()),
        is_async: true,
        receiver: Some(ReceiverKind::Ref),
        ..MethodDef::default()
    }
}

fn surface(service_name: &str, method_name: &str) -> ApiSurface {
    let registration = RegistrationDef {
        method: method_name.to_string(),
        callback_param: "handler".to_string(),
        callback_contract: "RequestHandler".to_string(),
        metadata_params: vec![param("path")],
        receiver: Some(ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        error_type: Some("HandlerError".to_string()),
        ..RegistrationDef::default()
    };
    let entrypoint = EntrypointDef {
        method: method_name.to_string(),
        kind: EntrypointKind::Run,
        is_async: true,
        params: vec![param("addr")],
        return_type: TypeRef::Unit,
        error_type: Some("IoError".to_string()),
        doc: String::new(),
    };
    let contract = HandlerContractDef {
        trait_name: "RequestHandler".to_string(),
        rust_path: "demo_core::RequestHandler".to_string(),
        dispatch: dispatch_method(),
        optional_methods: vec![],
        wire_request_type: Some("RequestData".to_string()),
        wire_response_type: Some("ResponseData".to_string()),
        dispatch_extra_params: vec![],
        wire_param_name: None,
        dispatch_return_type: None,
        response_adapter: None,
        doc: String::new(),
    };

    ApiSurface {
        crate_name: PREFIX.to_string(),
        version: "1.0.0".to_string(),
        services: vec![ServiceDef {
            name: service_name.to_string(),
            rust_path: format!("demo_core::{service_name}"),
            constructor: MethodDef {
                name: "new".to_string(),
                return_type: TypeRef::Unit,
                is_static: true,
                ..MethodDef::default()
            },
            configurators: vec![],
            registrations: vec![registration],
            entrypoints: vec![entrypoint],
            doc: String::new(),
            cfg: None,
        }],
        handler_contracts: vec![contract],
        ..ApiSurface::default()
    }
}

fn config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: PREFIX.to_string(),
        ..ResolvedCrateConfig::default()
    }
}

fn generated_go(api: &ApiSurface) -> String {
    super::service_api::generate(api, &config(), "demo", PREFIX)
        .expect("go service generation")
        .into_iter()
        .next()
        .expect("a service surface produces one Go file")
        .content
}

fn generated_ffi(api: &ApiSurface) -> String {
    crate::backends::ffi::gen_bindings::service_api::generate(api, &config())
        .expect("ffi service generation")
        .into_iter()
        .next()
        .expect("a service surface produces one Rust file")
        .content
}

/// Every `C.<symbol>(` call site in the generated Go source, excluding the cgo preamble comment
/// block (which documents the same symbols but is not a call site).
fn called_c_symbols(generated: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    for line in generated.lines().filter(|line| !line.trim_start().starts_with("//")) {
        let mut rest = line;
        while let Some(at) = rest.find("C.") {
            rest = &rest[at + 2..];
            let end = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            let (symbol, tail) = rest.split_at(end);
            if tail.starts_with('(') && !symbol.is_empty() {
                symbols.push(symbol.to_string());
            }
            rest = tail;
        }
    }
    symbols
}

/// Every `extern "C" fn <symbol>(` item the FFI backend emits.
fn exported_c_symbols(generated: &str) -> Vec<String> {
    const MARKER: &str = "extern \"C\" fn ";
    let mut symbols = Vec::new();
    let mut rest = generated;
    while let Some(at) = rest.find(MARKER) {
        rest = &rest[at + MARKER.len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let (symbol, tail) = rest.split_at(end);
        if !symbol.is_empty() {
            symbols.push(symbol.to_string());
        }
        rest = tail;
    }
    symbols
}

#[test]
fn should_only_call_service_symbols_the_ffi_backend_exports() {
    for service_name in ADVERSARIAL_SERVICE_NAMES {
        for method_name in ADVERSARIAL_METHOD_NAMES {
            let api = surface(service_name, method_name);
            let exported = exported_c_symbols(&generated_ffi(&api));
            let called = called_c_symbols(&generated_go(&api));

            let service_calls: Vec<&String> = called
                .iter()
                .filter(|symbol| symbol.starts_with(&format!("{PREFIX}_")))
                .collect();
            assert!(
                !service_calls.is_empty(),
                "`{service_name}::{method_name}` produced no prefixed cgo call to check"
            );
            for symbol in service_calls {
                assert!(
                    exported.contains(symbol),
                    "`{service_name}::{method_name}` calls `{symbol}`, but the FFI backend exports {exported:?}"
                );
            }
        }
    }
}

/// The four service symbol shapes are distinct: `_new`, `_free`, a `_register_` infix and an
/// `_ep_` infix. A test that only checked "some prefixed symbol is exported" would pass even if
/// Go called the constructor where the entrypoint belongs. ~keep
#[test]
fn should_emit_each_service_symbol_shape_with_its_own_infix() {
    let api = surface("HTTPRouter", "addURLHandler");
    let called = called_c_symbols(&generated_go(&api));

    for expected in [
        "demo_http_router_new",
        "demo_http_router_free",
        "demo_http_router_register_add_url_handler",
        "demo_http_router_ep_add_url_handler",
    ] {
        assert!(
            called.contains(&expected.to_string()),
            "expected a call to `{expected}`, got {called:?}"
        );
    }
}
