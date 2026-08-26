//! Java's consumption of the FFI result-presence channel.
//!
//! A Rust `Option<i64>` return crosses the C ABI as a bare `i64`, so `None` and a legitimate
//! `Some(0)` arrive at the Panama downcall identically. Java used to wrap that with an
//! unconditional `Optional.of(...)`, which is never empty. These tests assert against the
//! **rendered Java**, never against a host-side predicate: a test that compared Java's own notion
//! of eligibility with the FFI backend's would pass even if the emitter never consulted either.

use alef::backends::ffi::type_map::result_presence_companion_exists;
use alef::backends::java::JavaBackend;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{ApiSurface, FunctionDef, MethodDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

/// The substring every generated reference to a presence companion contains — the Java
/// `MethodHandle` constant suffix. Parity assertions search the rendered Java for it. ~keep
const COMPANION_MARKER: &str = "_HAS_RESULT";

fn config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["java", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.java]
package = "com.example"
"#,
    )
    .unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn optional(inner: TypeRef) -> TypeRef {
    TypeRef::Optional(Box::new(inner))
}

fn i64_option() -> TypeRef {
    optional(TypeRef::Primitive(PrimitiveType::I64))
}

fn surface(types: Vec<TypeDef>, functions: Vec<FunctionDef>) -> ApiSurface {
    ApiSurface {
        crate_name: "test_lib".to_string(),
        version: "0.1.0".to_string(),
        types,
        functions,
        ..Default::default()
    }
}

/// Every generated Java file joined, so an assertion can span the `NativeLib` handle table and the
/// wrapper that invokes it without caring which file each landed in.
fn render(api: &ApiSurface) -> String {
    JavaBackend
        .generate_bindings(api, &config())
        .expect("java bindings")
        .iter()
        .map(|f| f.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn free_function(name: &str, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        return_type,
        ..Default::default()
    }
}

fn opaque_type(name: &str, methods: Vec<MethodDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        is_opaque: true,
        methods,
        ..Default::default()
    }
}

fn method_def(name: &str, return_type: TypeRef, receiver: Option<ReceiverKind>) -> MethodDef {
    MethodDef {
        name: name.to_string(),
        return_type,
        receiver,
        ..Default::default()
    }
}

#[test]
fn should_declare_a_presence_handle_when_a_free_function_returns_an_optional_scalar() {
    let generated = render(&surface(vec![], vec![free_function("port", i64_option())]));

    assert!(
        generated.contains(r#"LIB.find("test_port_has_result")"#),
        "the companion must be bound to the FFI crate's exported symbol; got:\n{generated}"
    );
    assert!(
        generated.contains("static final MethodHandle TEST_PORT_HAS_RESULT = LINKER.downcallHandle("),
        "the companion needs its own downcall handle; got:\n{generated}"
    );
    assert!(
        generated.contains("FunctionDescriptor.of(ValueLayout.JAVA_INT)"),
        "the companion always returns i32 regardless of the primary's shape; got:\n{generated}"
    );
}

#[test]
fn should_report_absent_as_an_empty_optional_and_keep_a_zero_valued_result_present() {
    let generated = render(&surface(vec![], vec![free_function("port", i64_option())]));

    assert!(
        generated.contains("var presenceResult = (int) NativeLib.TEST_PORT_HAS_RESULT.invoke();"),
        "the presence answer must be captured; got:\n{generated}"
    );
    assert!(
        generated.contains("return presenceResult == 1 ? Optional.of(primitiveResult) : Optional.empty();"),
        "absent must be an empty Optional and a present zero must stay present; got:\n{generated}"
    );
    assert!(
        !generated.contains("return Optional.of(primitiveResult);"),
        "the unconditional wrap is the defect being fixed; got:\n{generated}"
    );
}

/// The companion clears the FFI crate's last-error slot on entry. Invoking it after the primary
/// would erase an error the primary had just recorded, so `checkLastError()` would see a clean
/// slate and a real failure would surface as a plain absent result. ~keep
#[test]
fn should_invoke_the_companion_before_the_primary_downcall() {
    let generated = render(&surface(vec![], vec![free_function("port", i64_option())]));

    let companion_at = generated.find("TEST_PORT_HAS_RESULT.invoke(").expect("companion invocation");
    let primary_at = generated
        .find("var primitiveResult = (long) NativeLib.TEST_PORT.invoke(")
        .expect("primary invocation");
    assert!(
        companion_at < primary_at,
        "the presence companion must run before the primary downcall; got:\n{generated}"
    );
}

#[test]
fn should_not_reference_a_companion_for_a_pointer_shaped_optional_return() {
    let generated = render(&surface(vec![], vec![free_function("label", optional(TypeRef::String))]));

    assert!(
        !generated.contains(COMPANION_MARKER),
        "`Option<String>` already carries a real null pointer; got:\n{generated}"
    );
}

#[test]
fn should_not_reference_a_companion_for_an_owned_receiver_because_ffi_exports_none() {
    let consumed = method_def("into_timeout", i64_option(), Some(ReceiverKind::Owned));
    assert!(
        !result_presence_companion_exists(&consumed.return_type, consumed.receiver.as_ref()),
        "the FFI backend must not export a companion for an owned receiver"
    );

    let generated = render(&surface(vec![opaque_type("Settings", vec![consumed])], vec![]));

    assert!(
        !generated.contains(COMPANION_MARKER),
        "an owned receiver's first call consumes the handle, so no companion exists; got:\n{generated}"
    );
}

/// Java's decision to reference `{fn}_has_result` must equal the FFI backend's decision to export
/// it, for every return shape and receiver. Referencing a symbol the FFI crate never exported
/// fails at class-initialization time and takes the whole binding down, so this compares the
/// rendered Java against the authority rather than restating the eligibility rule. ~keep
#[test]
fn java_references_a_companion_exactly_when_the_ffi_backend_exports_one() {
    let shapes: Vec<TypeRef> = vec![
        i64_option(),
        optional(TypeRef::Primitive(PrimitiveType::U64)),
        optional(TypeRef::Primitive(PrimitiveType::Bool)),
        optional(TypeRef::Primitive(PrimitiveType::F64)),
        optional(TypeRef::Duration),
        optional(TypeRef::String),
        optional(TypeRef::Path),
        optional(TypeRef::Json),
        optional(TypeRef::Named("Settings".to_string())),
        optional(TypeRef::Vec(Box::new(TypeRef::String))),
        TypeRef::Primitive(PrimitiveType::I64),
        TypeRef::String,
        TypeRef::Unit,
    ];
    let receivers = [
        None,
        Some(ReceiverKind::Ref),
        Some(ReceiverKind::RefMut),
        Some(ReceiverKind::Owned),
    ];

    for shape in &shapes {
        let generated = render(&surface(vec![], vec![free_function("probe", shape.clone())]));
        assert_eq!(
            generated.contains(COMPANION_MARKER),
            result_presence_companion_exists(shape, None),
            "free function returning {shape:?} disagreed with the FFI backend; got:\n{generated}"
        );

        for receiver in &receivers {
            let method = method_def("probe", shape.clone(), receiver.clone());
            let generated = render(&surface(vec![opaque_type("Settings", vec![method])], vec![]));
            assert_eq!(
                generated.contains(COMPANION_MARKER),
                result_presence_companion_exists(shape, receiver.as_ref()),
                "method returning {shape:?} with receiver {receiver:?} disagreed with the FFI \
                 backend; got:\n{generated}"
            );
        }
    }
}
