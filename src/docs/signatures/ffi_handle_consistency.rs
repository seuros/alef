use super::*;

// ---------------------------------------------------------------------------
// ~keep Regression coverage for the handle-ABI migration (every `TypeRef::Named` crosses
// the C ABI as a scalar `AlefHandle` token, not a pointer to a per-type struct). See
// `type_mapping.rs`'s `FFI_HANDLE_TYPE_NAME` for the source of the spelling.
// ---------------------------------------------------------------------------

#[test]
fn test_render_c_fn_sig_named_return_is_scalar_handle_not_pointer() {
    let func = make_function(
        "parse_document",
        vec![make_param("input", TypeRef::String, false)],
        TypeRef::Named("ConversionResult".to_string()),
        false,
        None,
    );
    let sig = render_c_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "HTMAlefHandle htm_parse_document(const char* input);");
    assert!(
        !sig.contains("HTMAlefHandle*"),
        "the handle itself must not be pointer-suffixed: {sig}"
    );
    assert!(
        !sig.contains("ConversionResult"),
        "must not name the concrete Rust type: {sig}"
    );
}

#[test]
fn test_render_c_fn_sig_named_param_is_scalar_handle_not_pointer() {
    let func = make_function(
        "attach",
        vec![make_param("config", TypeRef::Named("ClientConfig".to_string()), false)],
        TypeRef::Unit,
        false,
        None,
    );
    let sig = render_c_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "void htm_attach(HTMAlefHandle config);");
}

#[test]
fn test_render_c_fn_sig_optional_named_param_stays_a_bare_scalar_handle() {
    let func = make_function(
        "attach",
        vec![make_param("config", TypeRef::Named("ClientConfig".to_string()), true)],
        TypeRef::Unit,
        false,
        None,
    );
    let sig = render_c_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "void htm_attach(HTMAlefHandle config);");
}

#[test]
fn test_render_method_signature_ffi_named_return_and_param_use_scalar_handle() {
    let method = make_method(
        "convert",
        vec![make_param("options", TypeRef::Named("ParseOptions".to_string()), false)],
        TypeRef::Named("ConversionResult".to_string()),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Converter", Language::Ffi, TEST_PREFIX);
    assert!(
        sig.contains("HTMAlefHandle"),
        "return and param must use the handle token: {sig}"
    );
    assert!(
        !sig.contains("ConversionResult") && !sig.contains("ParseOptions"),
        "{sig}"
    );
}

/// ~keep The whole point of the handle-ABI migration bug was that two published surfaces
/// for the same function -- the rendered signature and the rendered worked example --
/// disagreed about the C ABI's handle type. A test that only checks `signatures.rs` in
/// isolation re-encodes the same assumption the bug had; it cannot fail when the two
/// halves drift apart again. This test renders both from the same `FunctionDef` and
/// asserts they agree.
#[test]
fn test_c_signature_and_example_agree_on_named_return_handle_type() {
    let func = make_function(
        "parse_document",
        vec![make_param("input", TypeRef::String, false)],
        TypeRef::Named("ConversionResult".to_string()),
        false,
        None,
    );
    let signature = render_function_signature(&func, Language::C, TEST_PREFIX);
    let example = crate::docs::examples::render_function_example(&func, Language::C, TEST_PREFIX);

    assert_eq!(signature, "HTMAlefHandle htm_parse_document(const char* input);");
    assert!(
        example.contains("HTMAlefHandle result = htm_parse_document(\"value\");"),
        "example must declare `result` as the same scalar handle type the signature returns: {example}"
    );
    assert!(
        !signature.contains("ConversionResult") && !example.contains("ConversionResult"),
        "neither surface may name the concrete Rust type -- it isn't a struct in the C ABI:\n\
         signature: {signature}\nexample: {example}"
    );
}

// ---------------------------------------------------------------------------
// ~keep The C doc emitter (`render_c_fn_sig`, defined in signatures.rs rather than in this
// test module) builds a signature purely from `FunctionDef.params`/`return_type` via
// `doc_type`, one C parameter per IR parameter.
// The real FFI backend does not: `orchestration.rs`'s free-function codegen (mirrored in
// its method-codegen sibling ~570 lines earlier in the same file) appends a `<name>_len:
// usize` companion parameter for every `TypeRef::Bytes` parameter, and three extra
// `out_ptr`/`out_len`/`out_cap` out-parameters whenever the return type resolves to bytes
// -- neither is IR-visible to `doc_type`, so no formula fix in `type_mapping.rs` alone can
// close this gap. This test pins the correct, ABI-faithful shape; it is expected to fail
// until `render_c_fn_sig` reads the emitted binding (cbindgen header) or the same
// param-shaping logic `orchestration.rs` uses, instead of the bare IR.
//
// Two qualifications, established while auditing task #67:
//
// 1. "Read the cbindgen header" is only conditionally available. alef never runs cbindgen --
//    it emits `cbindgen.toml` plus a `build.rs` calling `cbindgen::generate`
//    (backends/ffi/templates/build_rs.jinja), so the header is produced by the *consumer's*
//    `cargo build`. `alef all` does build the FFI crate before the docs stage
//    (bin_cli/all_commands.rs:265 precedes :513, via `complete_generated_artifacts` ->
//    `ensure_ffi_header_freshness`), so a fresh header exists there. A bare `alef docs`
//    (bin_cli/core_commands.rs:584-622) never calls it, so in a fresh tree there is no header
//    at all. Any header-reading fix must therefore also decide what `alef docs` does when the
//    header is missing -- emitting a fabricated signature is the current, wrong answer;
//    omitting the signature is the honest one.
//
// 2. Not every gap needs the header. Two of the largest mismatches are plain IR and fixable
//    here: the C arm of `render_method_signature_with_override` (signatures.rs:675-700) omits
//    the leading `this` receiver that `orchestration.rs:141-147` always emits for a non-static
//    method, and it names the symbol `{prefix}_{method}` via `func_name` while the backend
//    emits `{prefix}_{type_snake}_{method}` (orchestration.rs:45,78) -- it already receives
//    `type_name_str` and simply does not use it. Both are deferred with the rest of #67, but
//    they are not blocked on the header.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pins the target shape for task #67 (render_c_fn_sig must read the cbindgen header); \
            C doc signatures are a documentation defect, deferred as non-blocking for this release"]
fn test_render_c_fn_sig_bytes_param_gets_length_companion_parameter() {
    let func = make_function(
        "ingest",
        vec![make_param("data", TypeRef::Bytes, false)],
        TypeRef::Unit,
        false,
        None,
    );
    let sig = render_c_fn_sig(&func, TEST_PREFIX);
    assert!(
        sig.contains("data_len"),
        "a `Bytes` parameter crosses the C ABI as a pointer plus an explicit length -- the \
         real FFI backend (orchestration.rs) appends `data_len: usize`, which this \
         IR-only signature omits entirely: {sig}"
    );
}

/// ~keep Same cross-check for a Named parameter, not just a Named return.
#[test]
fn test_c_signature_and_example_agree_on_named_param_handle_type() {
    let func = make_function(
        "attach",
        vec![make_param("config", TypeRef::Named("ClientConfig".to_string()), false)],
        TypeRef::Unit,
        false,
        None,
    );
    let signature = render_function_signature(&func, Language::C, TEST_PREFIX);
    let example = crate::docs::examples::render_function_example(&func, Language::C, TEST_PREFIX);

    assert_eq!(signature, "void htm_attach(HTMAlefHandle config);");
    assert!(
        example.contains("htm_attach(0);"),
        "example must pass the scalar sentinel for a by-value Named param: {example}"
    );
    assert!(
        !signature.contains("ClientConfig") && !example.contains("ClientConfig"),
        "{signature}\n{example}"
    );
}
