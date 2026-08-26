//! C#'s consumption of the FFI result-presence channel.
//!
//! A Rust `Option<i64>` return crosses the C ABI as a bare `i64`, so `None` and a legitimate
//! `Some(0)` arrive at the P/Invoke stub as the same bits. C# did not merely fail to
//! disambiguate them: `returns_ptr` classified every `Optional` as pointer-shaped, so the wrapper
//! emitted `if (nativeResult == 0) { return null; }` and reported a real `Some(0)` as absent —
//! and shadowed the `else if error_type.is_some()` arm, so a genuine FFI failure surfaced as
//! `null` too.
//!
//! These tests assert against the **rendered C#**, never against a host-side predicate: a test
//! that compared C#'s own notion of eligibility with the FFI backend's would pass even if the
//! emitter never consulted either.

use alef::backends::csharp::CsharpBackend;
use alef::backends::ffi::type_map::result_presence_companion_exists;
use alef::core::backend::Backend;
use alef::core::config::{NewAlefConfig, ResolvedCrateConfig};
use alef::core::ir::{
    ApiSurface, CoreWrapper, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef,
};

/// The substring every generated reference to a presence companion contains — the C symbol
/// suffix in the `[DllImport]` and the `HasResult` stub it binds. Parity assertions search the
/// rendered C# for it. ~keep
const COMPANION_MARKER: &str = "HasResult";

fn config() -> ResolvedCrateConfig {
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["csharp", "ffi"]

[[crates]]
name = "test_lib"
sources = ["src/lib.rs"]

[crates.ffi]
prefix = "test"

[crates.csharp]
namespace = "Test"
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

/// Every generated C# file joined, so an assertion can span the `NativeMethods` declarations and
/// the wrapper that calls them without caring which file each landed in.
fn render(api: &ApiSurface) -> String {
    CsharpBackend
        .generate_bindings(api, &config())
        .expect("csharp bindings")
        .iter()
        .map(|file| file.content.as_str())
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

fn scalar_param(name: &str, ty: TypeRef) -> ParamDef {
    ParamDef {
        name: name.to_string(),
        ty,
        optional: false,
        default: None,
        sanitized: false,
        typed_default: None,
        is_ref: false,
        is_mut: false,
        newtype_wrapper: None,
        original_type: None,
        map_is_ahash: false,
        map_key_is_cow: false,
        vec_inner_is_ref: false,
        map_is_btree: false,
        core_wrapper: CoreWrapper::None,
    }
}

#[test]
fn should_declare_a_presence_stub_when_a_free_function_returns_an_optional_scalar() {
    let generated = render(&surface(vec![], vec![free_function("port", i64_option())]));

    assert!(
        generated.contains(r#"EntryPoint = "test_port_has_result")]"#),
        "the companion must bind the FFI crate's exported symbol; got:\n{generated}"
    );
    assert!(
        generated.contains("internal static extern int PortHasResult();"),
        "the companion always returns i32 regardless of the primary's shape; got:\n{generated}"
    );
}

/// The value this whole channel exists for. `Some(0)` and `None` are the same bits at the C
/// boundary, and the old `if (nativeResult == 0) { return null; }` resolved them the wrong way:
/// it reported a legitimate zero as absent. ~keep
#[test]
fn should_report_absent_as_null_and_keep_a_zero_valued_result_present() {
    let generated = render(&surface(vec![], vec![free_function("port", i64_option())]));

    assert!(
        generated.contains("public static long? Port()"),
        "expected a nullable C# return type; got:\n{generated}"
    );
    assert!(
        generated.contains(concat!(
            "        if (NativeMethods.PortHasResult() != 1)\n",
            "        {\n",
            "            return (long?)null;\n",
            "        }\n",
        )),
        "absence must come from the companion, not from the returned value; got:\n{generated}"
    );
    assert!(
        !generated.contains("if (nativeResult == 0)"),
        "a zero result must stay present — testing the value itself is the defect being fixed; got:\n{generated}"
    );
    assert!(
        generated.contains("var returnValue = nativeResult;"),
        "a present result, zero included, must be returned unchanged; got:\n{generated}"
    );
}

/// The companion clears the FFI crate's last-error slot on entry. Calling it after the primary
/// would erase an error the primary had just recorded. ~keep
#[test]
fn should_call_the_companion_before_the_primary_stub() {
    let generated = render(&surface(vec![], vec![free_function("port", i64_option())]));

    let companion_at = generated
        .find("NativeMethods.PortHasResult(")
        .expect("companion call site");
    let primary_at = generated
        .find("var nativeResult = NativeMethods.Port(")
        .expect("primary call site");
    assert!(
        companion_at < primary_at,
        "the presence companion must run before the primary call; got:\n{generated}"
    );
}

/// Before the fix `returns_ptr` claimed every `Optional`, which made the wrapper's
/// `else if error_type.is_some()` arm unreachable — a real FFI failure on an optional-returning
/// call was reported to the caller as a plain `null`. ~keep
#[test]
fn should_throw_rather_than_report_absence_when_a_fallible_optional_call_fails() {
    let mut func = free_function("port", i64_option());
    func.error_type = Some("test_lib::Error".to_string());

    let generated = render(&surface(vec![], vec![func]));

    assert!(
        generated.contains(
            concat!(
                "        if (NativeMethods.PortHasResult() != 1)\n",
                "        {\n",
                "            if (NativeMethods.LastErrorCode() != 0)\n",
            )
        ),
        "the companion's own failure must surface as an exception, not as absence; got:\n{generated}"
    );
    let primary_at = generated
        .find("var nativeResult = NativeMethods.Port(")
        .expect("primary call site");
    assert!(
        generated[primary_at..].contains("if (NativeMethods.LastErrorCode() != 0)"),
        "a failure recorded by the primary call must still be checked after it; got:\n{generated}"
    );
}

/// The comma-separated argument text of the first `call_prefix(...)` occurrence.
fn call_arguments<'a>(generated: &'a str, call_prefix: &str) -> &'a str {
    let start = generated
        .find(call_prefix)
        .unwrap_or_else(|| panic!("no `{call_prefix}` call in:\n{generated}"))
        + call_prefix.len();
    let end = start
        + generated[start..]
            .find(')')
            .unwrap_or_else(|| panic!("unterminated `{call_prefix}` call in:\n{generated}"));
    &generated[start..end]
}

/// The companion's C signature *is* the primary export's parameter list, receiver included. This
/// reads both argument lists out of the rendered C# rather than restating what they should be, so
/// it fails if either side is built from its own construction of the list. ~keep
#[test]
fn should_pass_the_same_arguments_to_the_companion_as_to_the_primary_call() {
    let mut timeout = method_def("timeout", i64_option(), Some(ReceiverKind::Ref));
    timeout.params = vec![scalar_param("scale", TypeRef::Primitive(PrimitiveType::U32))];
    let generated = render(&surface(vec![opaque_type("Settings", vec![timeout])], vec![]));

    let companion = call_arguments(&generated, "NativeMethods.SettingsTimeoutHasResult(");
    let primary = call_arguments(&generated, "var nativeResult = NativeMethods.SettingsTimeout(")
        .replace(['\n', ','], " ");

    assert_eq!(
        companion.split_whitespace().collect::<Vec<_>>(),
        primary.split_whitespace().collect::<Vec<_>>(),
        "the companion and the primary must pass the same arguments; got:\n{generated}"
    );
    assert!(
        generated.contains("internal static extern int SettingsTimeoutHasResult("),
        "the companion needs its own P/Invoke declaration; got:\n{generated}"
    );
}

#[test]
fn should_not_reference_a_companion_for_a_pointer_shaped_optional_return() {
    let generated = render(&surface(
        vec![],
        vec![free_function("label", optional(TypeRef::String))],
    ));

    assert!(
        !generated.contains(COMPANION_MARKER),
        "`Option<String>` already carries a real null pointer; got:\n{generated}"
    );
    assert!(
        generated.contains("if (nativeResult == IntPtr.Zero)"),
        "a pointer-shaped optional must keep testing its own sentinel; got:\n{generated}"
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

/// C#'s decision to declare and call `{fn}_has_result` must equal the FFI backend's decision to
/// export it, for every return shape and receiver. A `[DllImport]` for a symbol the FFI crate
/// never exported throws `EntryPointNotFoundException` at the first call, so this compares the
/// rendered C# against the authority rather than restating the eligibility rule. ~keep
#[test]
fn csharp_references_a_companion_exactly_when_the_ffi_backend_exports_one() {
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
