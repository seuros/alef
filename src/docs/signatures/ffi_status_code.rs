use super::*;

// ---------------------------------------------------------------------------
// ~keep A fallible function/method whose logical return type is `()` reports failure
// through the return slot itself: the FFI backend (`gen_function_wrapper_footer` /
// `gen_free_function` in backends/ffi/gen_bindings/functions/orchestration.rs) emits
// `i32` -- not `void` -- whenever `has_error && is_void_return(&return_type)`, which
// cbindgen renders as `int32_t`. Documenting `void` there lets a caller believe the call
// cannot fail and skip the status check entirely.
// ---------------------------------------------------------------------------

#[test]
fn test_render_c_fn_sig_fallible_void_return_is_int32_t_not_void() {
    let func = make_function(
        "init",
        vec![make_param("config", TypeRef::Named("ClientConfig".to_string()), false)],
        TypeRef::Unit,
        false,
        Some("InitError"),
    );
    let sig = render_c_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "int32_t htm_init(HTMAlefHandle config);");
    assert!(
        !sig.starts_with("void"),
        "fallible void-return functions must report a status code: {sig}"
    );
}

#[test]
fn test_render_c_fn_sig_infallible_void_return_stays_void() {
    let func = make_function("touch", vec![], TypeRef::Unit, false, None);
    let sig = render_c_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "void htm_touch();");
}

#[test]
fn test_render_c_fn_sig_fallible_non_void_return_is_unaffected() {
    // Only the void-return case is repurposed as a status code; a fallible function that
    // already returns a value keeps that value's own type (the handle's 0 sentinel or a
    // primitive's own encoding communicates failure instead).
    let func = make_function(
        "parse_document",
        vec![make_param("input", TypeRef::String, false)],
        TypeRef::Named("ConversionResult".to_string()),
        false,
        Some("ParseError"),
    );
    let sig = render_c_fn_sig(&func, TEST_PREFIX);
    assert_eq!(sig, "HTMAlefHandle htm_parse_document(const char* input);");
}

#[test]
fn test_render_method_signature_ffi_fallible_void_return_is_int32_t() {
    let method = make_method(
        "attach",
        vec![make_param("config", TypeRef::Named("ClientConfig".to_string()), false)],
        TypeRef::Unit,
        false,
        false,
        Some("AttachError"),
    );
    let sig = render_method_signature(&method, "Client", Language::Ffi, TEST_PREFIX);
    assert_eq!(
        sig,
        "int32_t htm_client_attach(HTMAlefHandle this, HTMAlefHandle config);"
    );
}

#[test]
fn test_render_method_signature_ffi_infallible_void_return_stays_void() {
    let method = make_method("reset", vec![], TypeRef::Unit, false, false, None);
    let sig = render_method_signature(&method, "Client", Language::Ffi, TEST_PREFIX);
    assert_eq!(sig, "void htm_client_reset(HTMAlefHandle this);");
}

#[test]
fn test_render_method_signature_ffi_status_code_fix_does_not_leak_to_other_languages() {
    // The status-code substitution is a C-ABI-only concept; other languages keep
    // reporting failure their own idiomatic way (exceptions, Result, etc.) and must not
    // see their return type silently swapped to `int32_t`.
    let method = make_method("attach", vec![], TypeRef::Unit, false, false, Some("AttachError"));
    let python_sig = render_method_signature(&method, "Client", Language::Python, TEST_PREFIX);
    assert!(!python_sig.contains("int32_t"), "{python_sig}");
}

/// ~keep A curated signature override (used by streaming.rs today) is trusted verbatim --
/// the status-code substitution must not second-guess an explicitly authored return type.
#[test]
fn test_render_method_signature_ffi_respects_explicit_return_type_override() {
    let method = make_method("attach", vec![], TypeRef::Unit, false, false, Some("AttachError"));
    let override_ = MethodSignatureOverride {
        return_type: Some("void".to_string()),
        ..Default::default()
    };
    let sig = render_method_signature_with_override(
        &method,
        "Client",
        Language::Ffi,
        TEST_PREFIX,
        TEST_CRATE_NAME,
        Some(&override_),
    );
    assert!(
        sig.starts_with("void"),
        "an explicit override must win over the status-code inference: {sig}"
    );
}

// ---------------------------------------------------------------------------
// ~keep The signature line and the "Errors:" phrase (`formatting.rs::format_error_phrase`)
// are two independent renderers describing the same function's failure behavior on the
// same page. Asserting each only against a literal re-encodes the assumption a prior bug
// had; these render both from the same `FunctionDef` and assert they agree, the same
// pattern as `ffi_handle_consistency.rs`'s signature-vs-example cross-check and
// `function_render.rs`'s signature-vs-returns-prose cross-check.
// ---------------------------------------------------------------------------

#[test]
fn test_c_signature_and_error_phrase_agree_on_fallible_void_status_value() {
    let func = make_function(
        "init",
        vec![make_param("config", TypeRef::Named("ClientConfig".to_string()), false)],
        TypeRef::Unit,
        false,
        Some("InitError"),
    );
    let signature = render_function_signature(&func, Language::C, TEST_PREFIX, TEST_CRATE_NAME);
    let error_phrase = crate::docs::formatting::format_error_phrase(
        func.error_type.as_deref().expect("fallible"),
        &func.return_type,
        Language::C,
        TEST_CRATE_NAME,
    );

    assert!(signature.starts_with("int32_t "), "signature: {signature}");
    // The emitter always returns exactly `-1`, never some other non-zero value -- both
    // surfaces must name that exact value, not a vaguer "non-zero" on one side only.
    assert_eq!(error_phrase, "Returns `-1` on error.");
}

#[test]
fn test_c_signature_and_error_phrase_agree_named_return_uses_integer_not_null() {
    let func = make_function(
        "parse_document",
        vec![make_param("input", TypeRef::String, false)],
        TypeRef::Named("ConversionResult".to_string()),
        false,
        Some("ParseError"),
    );
    let signature = render_function_signature(&func, Language::C, TEST_PREFIX, TEST_CRATE_NAME);
    let error_phrase = crate::docs::formatting::format_error_phrase(
        func.error_type.as_deref().expect("fallible"),
        &func.return_type,
        Language::C,
        TEST_CRATE_NAME,
    );

    assert!(signature.contains("HTMAlefHandle"), "signature: {signature}");
    assert!(
        !error_phrase.contains("NULL"),
        "a scalar handle has no pointer to compare against NULL, so the error phrase must \
         not claim one exists just because the signature declares a handle return: {error_phrase}"
    );
    assert_eq!(error_phrase, "Returns the sentinel handle `0` on error.");
}
