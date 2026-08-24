//! Compiles the generated Java options-field visitor path with a real `javac`.
//!
//! Substring assertions cannot see whether a generated method body is *type-correct*. The
//! convert-with-visitor body is the densest exception-flow shape the Java backend emits — nested
//! `try`/`catch`/`finally`, a captured `operationFailure` slot, a typed rethrow chain and a
//! resource `finally` that suppresses cleanup failures onto the primary — and every one of those
//! interacts with `javac`'s checked-exception analysis. This test extracts the generated method
//! together with the generated helpers it calls, compiles them against the real generated
//! `NativeLib` and exception classes, and fails on any `javac` diagnostic. ~keep

#[path = "backends_java_blocker_regressions/support.rs"]
mod support;

use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, EnumDef, EnumVariant, FieldDef, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef,
    TypeRef,
};
use support::{compile_java, extract_java_method, java_available, write_file};

/// Generated files that carry no Jackson references and can therefore be compiled as emitted.
const REAL_DEPENDENCIES: &[&str] = &[
    "NativeLib.java",
    "TestLibRsException.java",
    "ConversionErrorException.java",
    "CoreErrorException.java",
    "PanicException.java",
    "Callback.java",
    "FlowDecision.java",
];

/// Generated members the convert-with-visitor body calls, lifted into the probe alongside it.
const PROBE_MEMBERS: &[&str] = &[
    "private interface NativeReleaser",
    "static final class NativeResources implements AutoCloseable",
    "private static void checkLastError()",
];

const VISITOR_METHOD: &str = "private static WorkResult processHtmlWithVisitorInternal";

fn visitor_config() -> ResolvedCrateConfig {
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

[[crates.trait_bridges]]
trait_name = "Callback"
type_alias = "CallbackHandle"
bind_via = "options_field"
options_type = "WorkConfig"
options_field = "hook"
context_type = "VisitContext"
result_type = "FlowDecision"
"#,
    )
    .expect("valid Java visitor config");
    config.resolve().expect("resolved Java visitor config").remove(0)
}

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_owned(),
        ty,
        ..Default::default()
    }
}

fn record(name: &str, fields: Vec<FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_owned(),
        rust_path: format!("test_lib::{name}"),
        fields,
        is_clone: true,
        has_serde: true,
        ..Default::default()
    }
}

fn callback_trait() -> TypeDef {
    TypeDef {
        name: "Callback".to_owned(),
        rust_path: "test_lib::Callback".to_owned(),
        is_trait: true,
        methods: vec![MethodDef {
            name: "inspect".to_owned(),
            params: vec![ParamDef {
                name: "context".to_owned(),
                ty: TypeRef::Named("VisitContext".to_owned()),
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::Named("FlowDecision".to_owned()),
            receiver: Some(ReceiverKind::RefMut),
            has_default_impl: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn visitor_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_owned(),
        version: "0.1.0".to_owned(),
        types: vec![
            record("VisitContext", vec![field("path", TypeRef::String)]),
            record(
                "WorkConfig",
                vec![
                    field("hook", TypeRef::Named("CallbackHandle".to_owned())),
                    field("mode", TypeRef::String),
                ],
            ),
            record("WorkResult", vec![field("text", TypeRef::String)]),
            callback_trait(),
        ],
        functions: vec![FunctionDef {
            name: "process_html".to_owned(),
            rust_path: "test_lib::process_html".to_owned(),
            params: vec![
                ParamDef {
                    name: "html".to_owned(),
                    ty: TypeRef::String,
                    ..Default::default()
                },
                ParamDef {
                    name: "config".to_owned(),
                    ty: TypeRef::Named("WorkConfig".to_owned()),
                    ..Default::default()
                },
            ],
            return_type: TypeRef::Named("WorkResult".to_owned()),
            ..Default::default()
        }],
        enums: vec![EnumDef {
            name: "FlowDecision".to_owned(),
            rust_path: "test_lib::FlowDecision".to_owned(),
            variants: vec![
                EnumVariant {
                    name: "Proceed".to_owned(),
                    is_default: true,
                    ..Default::default()
                },
                EnumVariant {
                    name: "DropNode".to_owned(),
                    ..Default::default()
                },
            ],
            has_serde: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn generated_files() -> Vec<(String, String)> {
    generate(&visitor_api(), &visitor_config())
}

fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> Vec<(String, String)> {
    JavaBackend
        .generate_bindings(api, config)
        .expect("java visitor generation must succeed")
        .into_iter()
        .map(|file| {
            let name = file
                .path
                .file_name()
                .expect("generated file name")
                .to_string_lossy()
                .into_owned();
            (name, file.content)
        })
        .collect()
}

fn facade_source(files: &[(String, String)]) -> String {
    files
        .iter()
        .find(|(_, content)| content.contains(VISITOR_METHOD))
        .unwrap_or_else(|| panic!("no generated file declares `{VISITOR_METHOD}`"))
        .1
        .clone()
}

/// Wraps the generated convert-with-visitor method and the generated helpers it calls in a probe
/// class, supplying only the object mapper — the one collaborator that would otherwise drag the
/// Jackson jars onto the classpath.
fn probe_source(facade: &str) -> String {
    let mut members = vec![extract_java_method(facade, VISITOR_METHOD)];
    members.extend(PROBE_MEMBERS.iter().map(|member| extract_java_method(facade, member)));
    format!(
        "package com.test;\n\n\
         import java.lang.foreign.Arena;\n\
         import java.lang.foreign.MemorySegment;\n\
         import java.util.List;\n\n\
         final class VisitorProbe {{\n{}\n\
         \x20   private static final ProbeMapper MAPPER = new ProbeMapper();\n\n\
         \x20   static final class ProbeMapper {{\n\
         \x20       String writeValueAsString(final Object value) {{\n\
         \x20           return \"{{}}\";\n\
         \x20       }}\n\n\
         \x20       <T> T readValue(final String json, final Class<T> type) {{\n\
         \x20           return null;\n\
         \x20       }}\n\
         \x20   }}\n\
         }}\n",
        members.join("\n")
    )
}

/// The `operationFailure` slot must be typed as the crate exception, not `Throwable`.
///
/// Every value assigned to it is already that exception, and the `Throwable` clause of the catch
/// chain rethrows the slot itself. Typing the slot `Throwable` compiles only because the whole
/// body sits inside an outer `catch (Throwable)`; the moment that outer chain changes shape the
/// rethrow becomes `unreported exception Throwable`. This assertion runs without a JDK. ~keep
#[test]
fn visitor_operation_failure_slot_is_typed_as_the_crate_exception() {
    let facade = facade_source(&generated_files());
    let method = extract_java_method(&facade, VISITOR_METHOD);
    assert!(
        method.contains("TestLibRsException operationFailure = null;"),
        "{method}"
    );
    assert!(!method.contains("Throwable operationFailure"), "{method}");
    assert!(method.contains("throw operationFailure;"), "{method}");
}

#[test]
fn generated_visitor_method_compiles_under_javac() {
    if !java_available() {
        return;
    }
    let files = generated_files();
    let directory = tempfile::tempdir().expect("temporary Java visitor directory");
    let mut sources: Vec<String> = Vec::new();
    for name in REAL_DEPENDENCIES {
        let content = files
            .iter()
            .find(|(file_name, _)| file_name == name)
            .unwrap_or_else(|| panic!("generated {name} must be emitted"))
            .1
            .clone();
        write_file(directory.path(), &format!("com/test/{name}"), &content);
        sources.push(format!("com/test/{name}"));
    }
    write_file(
        directory.path(),
        "com/test/Stubs.java",
        include_str!("fixtures/java_visitor_stubs.java"),
    );
    sources.push("com/test/Stubs.java".to_owned());
    write_file(
        directory.path(),
        "com/test/VisitorProbe.java",
        &probe_source(&facade_source(&files)),
    );
    sources.push("com/test/VisitorProbe.java".to_owned());

    let arguments: Vec<&str> = sources.iter().map(String::as_str).collect();
    compile_java(directory.path(), &arguments);
}

/// Generated files the bridge compile needs verbatim; the context record is stubbed for Jackson.
const BRIDGE_DEPENDENCIES: &[&str] = &["VisitorBridge.java", "Callback.java", "FlowDecision.java"];

fn bridge_config() -> ResolvedCrateConfig {
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

[[crates.trait_bridges]]
trait_name = "Callback"
type_alias = "CallbackHandle"
bind_via = "options_field"
options_type = "WorkConfig"
options_field = "hook"
context_type = "NodeContext"
result_type = "FlowDecision"
"#,
    )
    .expect("valid Java bridge config");
    config.resolve().expect("resolved Java bridge config").remove(0)
}

fn bridge_api() -> ApiSurface {
    let mut api = visitor_api();
    api.types[0] = record(
        "NodeContext",
        vec![
            field("kind", TypeRef::Named("NodeKind".to_owned())),
            field("name", TypeRef::String),
            field("depth", TypeRef::Primitive(PrimitiveType::U64)),
            field("position", TypeRef::Primitive(PrimitiveType::U64)),
            field("parent", TypeRef::String),
            field("inline", TypeRef::Primitive(PrimitiveType::Bool)),
        ],
    );
    api.types[3] = TypeDef {
        methods: vec![MethodDef {
            name: "inspect".to_owned(),
            params: vec![ParamDef {
                name: "context".to_owned(),
                ty: TypeRef::Named("NodeContext".to_owned()),
                is_ref: true,
                ..Default::default()
            }],
            return_type: TypeRef::Named("FlowDecision".to_owned()),
            receiver: Some(ReceiverKind::RefMut),
            has_default_impl: true,
            ..Default::default()
        }],
        ..callback_trait()
    };
    api.enums.push(EnumDef {
        name: "NodeKind".to_owned(),
        rust_path: "test_lib::NodeKind".to_owned(),
        variants: vec![
            EnumVariant {
                name: "Element".to_owned(),
                is_default: true,
                ..Default::default()
            },
            EnumVariant {
                name: "Text".to_owned(),
                ..Default::default()
            },
        ],
        has_serde: true,
        ..Default::default()
    });
    api
}

/// The visitor bridge is the other half of the visitor path and had no compile coverage either.
///
/// Its upcall handlers must return a discriminant the native side understands when the host
/// callback throws, and its `decodeContext` must construct the generated context record. Both are
/// invisible to substring assertions and both are `javac` errors when wrong. ~keep
#[test]
fn generated_visitor_bridge_compiles_under_javac() {
    if !java_available() {
        return;
    }
    let files = generate(&bridge_api(), &bridge_config());
    let directory = tempfile::tempdir().expect("temporary Java bridge directory");
    let mut sources: Vec<String> = Vec::new();
    for name in BRIDGE_DEPENDENCIES {
        let content = files
            .iter()
            .find(|(file_name, _)| file_name == name)
            .unwrap_or_else(|| panic!("generated {name} must be emitted"))
            .1
            .clone();
        write_file(directory.path(), &format!("com/test/{name}"), &content);
        sources.push(format!("com/test/{name}"));
    }
    write_file(
        directory.path(),
        "com/test/NodeContext.java",
        include_str!("fixtures/java_visitor_bridge_context.java"),
    );
    sources.push("com/test/NodeContext.java".to_owned());

    let arguments: Vec<&str> = sources.iter().map(String::as_str).collect();
    compile_java(directory.path(), &arguments);
}
