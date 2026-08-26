use std::collections::HashSet;

use crate::backends::ffi::type_map::result_presence_companion_exists;
use crate::backends::go::gen_bindings::functions::gen_function_wrapper;
use crate::backends::go::gen_bindings::methods::gen_method_wrapper;
use crate::core::ir::{CoreWrapper, FunctionDef, MethodDef, ParamDef, PrimitiveType, ReceiverKind, TypeDef, TypeRef};

const PREFIX: &str = "sample";

/// The substring every generated reference to a presence companion contains. Parity assertions
/// search the *rendered Go*, not the eligibility predicate, so an emitter that forgets to
/// consult the predicate fails them instead of agreeing with itself. ~keep
const COMPANION_MARKER: &str = "_has_result";

fn optional(inner: TypeRef) -> TypeRef {
    TypeRef::Optional(Box::new(inner))
}

fn i64_option() -> TypeRef {
    optional(TypeRef::Primitive(PrimitiveType::I64))
}

fn free_function(name: &str, return_type: TypeRef) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        return_type,
        ..Default::default()
    }
}

fn opaque_type(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        is_opaque: true,
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

fn render_free_function(func: &FunctionDef) -> String {
    gen_function_wrapper(
        func,
        PREFIX,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn render_method(typ: &TypeDef, method: &MethodDef) -> String {
    let opaque_names: HashSet<&str> = if typ.is_opaque {
        [typ.name.as_str()].into()
    } else {
        HashSet::new()
    };
    gen_method_wrapper(
        typ,
        method,
        PREFIX,
        &opaque_names,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
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

#[test]
fn should_gate_on_the_presence_companion_when_a_free_function_returns_optional_scalar() {
    let generated = render_free_function(&free_function("port", i64_option()));

    assert!(
        generated.contains("\tif C.sample_port_has_result() != 1 {\n\t\treturn nil\n\t}\n"),
        "expected a presence gate; got:\n{generated}"
    );
    let gate_at = generated.find("sample_port_has_result").expect("presence gate");
    let call_at = generated.find("ptr := C.sample_port(").expect("primary call");
    assert!(
        gate_at < call_at,
        "the presence gate must run before the primary call; got:\n{generated}"
    );
}

#[test]
fn should_report_absent_as_nil_and_keep_a_zero_valued_result_non_nil() {
    let generated = render_free_function(&free_function("port", i64_option()));

    assert!(
        generated.contains("func Port() *int64 {"),
        "expected a nullable Go return type; got:\n{generated}"
    );
    assert!(
        generated.contains("\t\treturn nil\n"),
        "absent must be reported as a nil pointer; got:\n{generated}"
    );
    assert!(
        generated.contains("return func() *int64 { v := int64(ptr); return &v }()"),
        "a present result must still be wrapped in a pointer, zero included; got:\n{generated}"
    );
    assert_go_syntax_is_valid(&generated);
}

#[test]
fn should_surface_a_companion_failure_through_last_error_when_the_wrapper_returns_an_error() {
    let mut func = free_function("port", i64_option());
    func.error_type = Some("sample_core::Error".to_string());

    let generated = render_free_function(&func);

    assert!(
        generated.contains("\tif C.sample_port_has_result() != 1 {\n\t\treturn nil, lastError()\n\t}\n"),
        "a fallible wrapper must report the companion's own failure; got:\n{generated}"
    );
}

fn timeout_method(receiver: Option<ReceiverKind>, is_static: bool) -> MethodDef {
    let mut timeout = method_def("timeout", i64_option(), receiver);
    timeout.is_static = is_static;
    timeout.params = vec![scalar_param("scale", TypeRef::Primitive(PrimitiveType::U32))];
    timeout
}

/// Asserts the companion call and the primary call agree on their argument list, and returns it.
fn assert_companion_matches_primary_arguments(generated: &str) -> String {
    let primary = call_arguments(generated, "C.sample_settings_timeout(");
    let companion = call_arguments(generated, "C.sample_settings_timeout_has_result(");
    assert_eq!(
        companion, primary,
        "the companion's C signature is the primary export's parameter list; got:\n{generated}"
    );
    primary.to_string()
}

#[test]
fn should_pass_an_opaque_receivers_handle_and_params_to_the_companion() {
    let generated = render_method(&opaque_type("Settings"), &timeout_method(Some(ReceiverKind::Ref), false));

    assert_eq!(assert_companion_matches_primary_arguments(&generated), "h.ptr, cScale");
}

#[test]
fn should_pass_a_value_receivers_marshalled_handle_to_the_companion() {
    let mut typ = opaque_type("Settings");
    typ.is_opaque = false;

    let generated = render_method(&typ, &timeout_method(Some(ReceiverKind::Ref), false));

    assert_eq!(assert_companion_matches_primary_arguments(&generated), "cRecv, cScale");
    assert!(
        generated.find("defer C.sample_settings_free(cRecv)").expect("receiver free")
            < generated.find(COMPANION_MARKER).expect("presence gate"),
        "the receiver handle must already be scheduled for release when the gate returns early; \
         got:\n{generated}"
    );
}

#[test]
fn should_omit_a_receiver_argument_for_a_static_method() {
    let generated = render_method(&opaque_type("Settings"), &timeout_method(None, true));

    assert_eq!(assert_companion_matches_primary_arguments(&generated), "cScale");
}

#[test]
fn should_not_call_a_companion_for_an_owned_receiver_because_ffi_exports_none() {
    let mut typ = opaque_type("Settings");
    typ.is_opaque = false;
    let consumed = method_def("into_timeout", i64_option(), Some(ReceiverKind::Owned));

    assert!(
        !result_presence_companion_exists(&consumed.return_type, consumed.receiver.as_ref()),
        "the FFI backend must not export a companion for an owned receiver"
    );

    let generated = render_method(&typ, &consumed);

    assert!(
        !generated.contains(COMPANION_MARKER),
        "an owned receiver's first call consumes the handle, so no companion exists to call; got:\n{generated}"
    );
}

#[test]
fn should_not_call_a_companion_for_a_pointer_shaped_optional_return() {
    let generated = render_free_function(&free_function("label", optional(TypeRef::String)));

    assert!(
        !generated.contains(COMPANION_MARKER),
        "`Option<String>` already carries a real null pointer; got:\n{generated}"
    );
}

/// Go's decision to call `{fn}_has_result` must equal the FFI backend's decision to export it,
/// for every shape and receiver. Emitting a call the FFI crate never exported is a link error in
/// consumer code, not a wrong value, so this compares the rendered Go against the authority
/// rather than restating the eligibility rule. ~keep
#[test]
fn go_emits_a_companion_call_exactly_when_the_ffi_backend_exports_one() {
    let shapes: Vec<TypeRef> = vec![
        i64_option(),
        optional(TypeRef::Primitive(PrimitiveType::U64)),
        optional(TypeRef::Primitive(PrimitiveType::Bool)),
        optional(TypeRef::Primitive(PrimitiveType::F64)),
        optional(TypeRef::Duration),
        optional(TypeRef::String),
        optional(TypeRef::Path),
        optional(TypeRef::Json),
        optional(TypeRef::Bytes),
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

    let typ = opaque_type("Settings");
    for shape in &shapes {
        let generated = render_free_function(&free_function("probe", shape.clone()));
        assert_eq!(
            generated.contains(COMPANION_MARKER),
            result_presence_companion_exists(shape, None),
            "free function returning {shape:?} disagreed with the FFI backend; got:\n{generated}"
        );

        for receiver in &receivers {
            let generated = render_method(&typ, &method_def("probe", shape.clone(), receiver.clone()));
            assert_eq!(
                generated.contains(COMPANION_MARKER),
                result_presence_companion_exists(shape, receiver.as_ref()),
                "method returning {shape:?} with receiver {receiver:?} disagreed with the FFI backend; \
                 got:\n{generated}"
            );
        }
    }
}

fn assert_go_syntax_is_valid(generated: &str) {
    use std::io::Write as _;

    let Ok(mut child) = crate::test_support::spawn_from_stable_dir("gofmt")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    else {
        return;
    };
    let source = format!("package sample\n\n{generated}");
    child
        .stdin
        .take()
        .expect("gofmt stdin")
        .write_all(source.as_bytes())
        .expect("write generated Go source");
    let output = child.wait_with_output().expect("wait for gofmt");
    assert!(
        output.status.success(),
        "generated Go syntax is invalid: {}\n{generated}",
        String::from_utf8_lossy(&output.stderr)
    );
}
