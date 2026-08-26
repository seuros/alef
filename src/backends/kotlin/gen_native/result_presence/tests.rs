use crate::backends::ffi::type_map::result_presence_companion_exists;
use crate::backends::kotlin::gen_native::emit_native_function_pub;
use crate::core::ir::{FunctionDef, PrimitiveType, TypeRef};

const PREFIX: &str = "sample";

/// The substring every generated reference to a presence companion contains. Parity assertions
/// search the *rendered Kotlin*, not the eligibility predicate, so an emitter that forgets to
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

fn render(func: &FunctionDef) -> String {
    let mut out = String::new();
    emit_native_function_pub(func, PREFIX, &[], &[], &mut out);
    out
}

/// Kotlin/JVM reaches the core crate through the Java facade, which already consults the
/// companion. Kotlin/Native calls the cbindgen export directly, so `Long` widened into `Long?` as
/// non-null and every `None` arrived as `0`. ~keep
#[test]
fn should_report_absent_as_null_and_keep_a_zero_valued_result_present() {
    let generated = render(&free_function("port", i64_option()));

    assert!(
        generated.contains("    fun port(): Long? {"),
        "expected a nullable Kotlin return type; got:\n{generated}"
    );
    assert!(
        generated.contains("            val _present = sample_port_has_result()\n"),
        "the companion's answer must be captured; got:\n{generated}"
    );
    assert!(
        generated.contains("            if (_present != 1) null else _result\n"),
        "absence must come from the companion and a present zero must stay present; \
         got:\n{generated}"
    );
}

/// The companion clears the FFI crate's last-error slot on entry. Capturing it after the primary
/// would erase an error the primary had just recorded, leaving the wrapper's own error check
/// looking at a clean slate. ~keep
#[test]
fn should_capture_the_companion_before_the_primary_call() {
    let generated = render(&free_function("port", i64_option()));

    let companion_at = generated.find("sample_port_has_result(").expect("companion call");
    let primary_at = generated
        .find("val _result = sample_port(")
        .expect("primary call");
    assert!(
        companion_at < primary_at,
        "the companion must run before the primary call; got:\n{generated}"
    );
}

/// The presence test sits after the wrapper's existing error handling, so a genuine FFI failure
/// still throws rather than being reported as a plain absent result. ~keep
#[test]
fn should_keep_the_error_check_ahead_of_the_presence_test_when_fallible() {
    let mut func = free_function("port", i64_option());
    func.error_type = Some("sample::Error".to_string());

    let generated = render(&func);

    let throw_at = generated.find("throw RuntimeException(").expect("error throw");
    let presence_at = generated.find("if (_present != 1)").expect("presence test");
    assert!(
        throw_at < presence_at,
        "a recorded FFI error must throw before absence is reported; got:\n{generated}"
    );
}

#[test]
fn should_not_call_a_companion_for_a_pointer_shaped_optional_return() {
    let generated = render(&free_function("label", optional(TypeRef::String)));

    assert!(
        !generated.contains(COMPANION_MARKER),
        "`Option<String>` already carries a real null pointer; got:\n{generated}"
    );
}

/// Kotlin/Native's decision to call `{fn}_has_result` must equal the FFI backend's decision to
/// export it. cinterop resolves the name against the generated header, so calling a symbol the
/// crate never exported fails the consumer's build rather than returning a wrong value. This
/// compares the rendered Kotlin against the authority rather than restating the rule. ~keep
#[test]
fn kotlin_native_calls_a_companion_exactly_when_the_ffi_backend_exports_one() {
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

    for shape in &shapes {
        let generated = render(&free_function("probe", shape.clone()));
        assert_eq!(
            generated.contains(COMPANION_MARKER),
            result_presence_companion_exists(shape, None),
            "free function returning {shape:?} disagreed with the FFI backend; got:\n{generated}"
        );
    }
}
