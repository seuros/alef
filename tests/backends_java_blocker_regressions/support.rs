#![allow(dead_code)]

use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, EntrypointDef, EntrypointKind, ErrorDef, ErrorVariant, HandlerContractDef, MethodDef, ParamDef,
    PrimitiveType, ReceiverKind, RegistrationDef, ServiceDef, TypeDef, TypeRef,
};
use std::path::Path;

pub fn test_config() -> ResolvedCrateConfig {
    let config: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "com.test"
"#,
    )
    .expect("valid Java blocker config");
    config.resolve().expect("resolved Java blocker config").remove(0)
}

fn handler_contract() -> HandlerContractDef {
    HandlerContractDef {
        trait_name: "RequestHandler".into(),
        rust_path: "test_lib::RequestHandler".into(),
        dispatch: MethodDef {
            name: "handle".into(),
            receiver: Some(ReceiverKind::Ref),
            cfg: None,
            params: vec![ParamDef {
                name: "request".into(),
                ty: TypeRef::Named("RequestData".into()),
                ..Default::default()
            }],
            return_type: TypeRef::Named("ResponseData".into()),
            error_type: Some("HandlerError".into()),
            ..Default::default()
        },
        wire_request_type: Some("RequestData".into()),
        wire_response_type: Some("ResponseData".into()),
        optional_methods: Vec::new(),
        dispatch_extra_params: Vec::new(),
        wire_param_name: None,
        dispatch_return_type: None,
        response_adapter: None,
        doc: String::new(),
    }
}

fn registration(metadata_params: Vec<ParamDef>) -> RegistrationDef {
    RegistrationDef {
        method: "add_handler".into(),
        callback_param: "handler".into(),
        callback_contract: "RequestHandler".into(),
        metadata_params,
        receiver: Some(ReceiverKind::RefMut),
        return_type: TypeRef::Unit,
        ..Default::default()
    }
}

fn service(metadata_params: Vec<ParamDef>) -> ServiceDef {
    ServiceDef {
        name: "TestService".into(),
        rust_path: "test_lib::TestService".into(),
        constructor: MethodDef {
            name: "new".into(),
            is_static: true,
            ..Default::default()
        },
        registrations: vec![registration(metadata_params)],
        entrypoints: vec![EntrypointDef {
            method: "run".into(),
            kind: EntrypointKind::Run,
            is_async: false,
            params: Vec::new(),
            return_type: TypeRef::Unit,
            error_type: None,
            doc: String::new(),
        }],
        configurators: Vec::new(),
        doc: String::new(),
        cfg: None,
    }
}

pub fn service_api(metadata_params: Vec<ParamDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".into(),
        version: "0.1.0".into(),
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
        services: vec![service(metadata_params)],
        handler_contracts: vec![handler_contract()],
        ..Default::default()
    }
}

pub fn service_source(metadata_params: Vec<ParamDef>) -> String {
    let api = service_api(metadata_params);
    JavaBackend
        .generate_service_api(&api, &test_config())
        .expect("Java service generation")
        .into_iter()
        .find(|file| file.path.to_string_lossy().ends_with("TestService.java"))
        .expect("generated TestService.java")
        .content
}

pub fn primitive_param(name: &str, primitive: PrimitiveType) -> ParamDef {
    ParamDef {
        name: name.into(),
        ty: TypeRef::Primitive(primitive),
        ..Default::default()
    }
}

pub fn opaque_source() -> String {
    let api = ApiSurface {
        crate_name: "test_lib".into(),
        version: "0.1.0".into(),
        types: vec![TypeDef {
            name: "Resource".into(),
            rust_path: "test_lib::Resource".into(),
            is_opaque: true,
            methods: vec![MethodDef {
                name: "consume".into(),
                receiver: Some(ReceiverKind::Owned),
                cfg: None,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    generated_source(&api, "Resource.java")
}

pub fn error_facade() -> (String, u32) {
    let error = ErrorDef {
        name: "RequestError".into(),
        rust_path: "test_lib::RequestError".into(),
        variants: vec![ErrorVariant {
            name: "Rejected".into(),
            is_unit: true,
            ..Default::default()
        }],
        original_rust_path: String::new(),
        doc: String::new(),
        methods: Vec::new(),
        binding_excluded: false,
        binding_exclusion_reason: None,
        version: Default::default(),
    };
    let api = error_api(error);
    let taxonomy_code = api.error_taxonomy()[0].code;
    (generated_source_containing(&api, "checkLastError()"), taxonomy_code)
}

pub fn extract_java_method(source: &str, signature: &str) -> String {
    let start = source.find(signature).expect("generated Java method");
    let open = source[start..].find('{').expect("generated Java method body") + start;
    let mut depth = 0;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' if depth == 1 => return source[start..=open + offset].to_string(),
            '}' => depth -= 1,
            _ => {}
        }
    }
    panic!("unterminated generated Java method")
}

fn error_api(error: ErrorDef) -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".into(),
        version: "0.1.0".into(),
        functions: vec![alef::core::ir::FunctionDef {
            name: "fallible".into(),
            rust_path: "test_lib::fallible".into(),
            error_type: Some("RequestError".into()),
            ..Default::default()
        }],
        errors: vec![error],
        ..Default::default()
    }
}

fn generated_source(api: &ApiSurface, suffix: &str) -> String {
    let files = JavaBackend
        .generate_bindings(api, &test_config())
        .expect("Java blocker generation");
    files
        .into_iter()
        .find(|file| file.path.to_string_lossy().ends_with(suffix))
        .unwrap_or_else(|| panic!("missing generated {suffix}"))
        .content
}

fn generated_source_containing(api: &ApiSurface, needle: &str) -> String {
    JavaBackend
        .generate_bindings(api, &test_config())
        .expect("Java error generation")
        .into_iter()
        .find(|file| file.content.contains(needle))
        .unwrap_or_else(|| panic!("missing generated source containing {needle}"))
        .content
}

pub fn java_available() -> bool {
    std::process::Command::new("javac").arg("-version").output().is_ok()
}

pub fn write_file(directory: &Path, relative: &str, contents: &str) {
    let path = directory.join(relative);
    std::fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture directory");
    std::fs::write(path, contents).expect("write Java blocker fixture");
}

pub fn compile_java(directory: &Path, files: &[&str]) {
    let output = std::process::Command::new("javac")
        .args(files)
        .current_dir(directory)
        .output()
        .expect("run javac");
    assert!(
        output.status.success(),
        "generated Java blocker fixture must compile:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn run_java(directory: &Path, main_class: &str) {
    run_java_args(directory, &["-cp", ".", main_class]);
}

pub fn run_java_args(directory: &Path, args: &[&str]) {
    let output = std::process::Command::new("java")
        .args(args)
        .current_dir(directory)
        .output()
        .expect("run Java blocker fixture");
    assert!(
        output.status.success(),
        "Java blocker fixture failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
