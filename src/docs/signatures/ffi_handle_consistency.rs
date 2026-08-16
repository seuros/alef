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
