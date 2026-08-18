use crate::core::config::{Language, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, FunctionDef, ParamDef, TypeRef, VersionAnnotation};
use crate::docs::descriptions::generate_param_description;
use crate::docs::doc_cleaning::{clean_doc_inline, demote_headings, extract_param_docs};
use crate::docs::examples::render_function_example;
use crate::docs::formatting::{doc_type_with_optional, escape_table_cell, format_error_phrase};
use crate::docs::naming::{field_name, func_name, lang_code_fence};
use crate::docs::signatures::render_function_signature;
use crate::docs::{clean_doc, doc_type, template_env, version_labels};

pub(super) fn push_version_annotation(out: &mut String, version: &VersionAnnotation) {
    if let Some(ref since) = version.since {
        let since = version_labels::major_minor(since);
        out.push_str(&template_env::render(
            "since_badge.jinja",
            minijinja::context! { since => since },
        ));
        out.push('\n');
        out.push('\n');
    }
    if let Some(ref dep) = version.deprecated {
        let since = dep
            .since
            .as_deref()
            .map(version_labels::major_minor)
            .unwrap_or_default();
        out.push_str(&template_env::render(
            "deprecated_notice.jinja",
            minijinja::context! {
                since => since,
                note => dep.note.as_deref().unwrap_or(""),
            },
        ));
        out.push('\n');
        out.push('\n');
    }
}

pub(super) fn render_function(
    func: &FunctionDef,
    lang: Language,
    _config: &ResolvedCrateConfig,
    api: &ApiSurface,
    ffi_prefix: &str,
) -> String {
    let mut out = String::new();
    let fn_name = func_name(&func.name, lang, ffi_prefix);

    out.push_str(&template_env::render(
        "heading.jinja",
        minijinja::context! { marker => "####", title => format!("{fn_name}()") },
    ));

    push_version_annotation(&mut out, &func.version);

    let param_docs = extract_param_docs(&func.doc);

    if !func.doc.is_empty() {
        let doc = clean_doc(&func.doc, lang);
        let doc = demote_headings(&doc, 2);
        out.push_str(&doc);
        out.push('\n');
        out.push('\n');
    }

    out.push_str("**Signature:**\n\n");
    let lang_code = lang_code_fence(lang);
    let sig = render_function_signature(func, lang, ffi_prefix, &api.crate_name);
    out.push_str(&template_env::render(
        "code_block.jinja",
        minijinja::context! { lang_code => lang_code, body => sig },
    ));
    out.push('\n');

    out.push_str(&render_function_example(func, lang, ffi_prefix));

    push_parameters_table(&mut out, &func.params, &param_docs, lang, ffi_prefix);

    push_returns(
        &mut out,
        &func.return_type,
        func.error_type.as_deref(),
        lang,
        ffi_prefix,
    );
    push_errors(&mut out, func.error_type.as_deref(), &func.return_type, lang);

    let _ = api;
    out
}

pub(super) fn push_parameters_table(
    out: &mut String,
    params: &[ParamDef],
    param_docs: &std::collections::HashMap<String, String>,
    lang: Language,
    ffi_prefix: &str,
) {
    if params.is_empty() {
        return;
    }
    out.push_str("**Parameters:**\n\n");
    out.push_str("| Name | Type | Required | Description |\n");
    out.push_str("|------|------|----------|-------------|\n");
    for param in params {
        let pname = field_name(&param.name, lang);
        let pty = doc_type_with_optional(&param.ty, lang, param.optional, ffi_prefix);
        let required = if param.optional { "No" } else { "Yes" };
        let pdoc = param_docs
            .get(param.name.as_str())
            .map(|s| clean_doc_inline(s, lang))
            .unwrap_or_else(|| generate_param_description(&param.name, &param.ty));
        out.push_str(&template_env::render(
            "param_row.jinja",
            minijinja::context! {
                name => escape_table_cell(&pname),
                ty => escape_table_cell(&pty),
                required => required,
                doc => escape_table_cell(&pdoc),
            },
        ));
    }
    out.push('\n');
}

pub(super) fn push_returns(
    out: &mut String,
    return_type: &TypeRef,
    error_type: Option<&str>,
    lang: Language,
    ffi_prefix: &str,
) {
    push_returns_with_override(out, return_type, None, error_type, lang, ffi_prefix);
}

pub(super) fn push_returns_with_override(
    out: &mut String,
    return_type: &TypeRef,
    return_type_override: Option<&str>,
    error_type: Option<&str>,
    lang: Language,
    ffi_prefix: &str,
) {
    if matches!(return_type, TypeRef::Unit) {
        if let Some(override_ty) = return_type_override {
            out.push_str(&template_env::render(
                "returns.jinja",
                minijinja::context! { ty => override_ty },
            ));
            out.push('\n');
            return;
        }
        // ~keep A fallible Ffi/C function/method whose logical return is `()` reports
        // failure through the return itself: `signatures.rs`'s `render_c_fn_sig` /
        // `render_method_signature_with_override` document that return as `int32_t`
        // (see `backends/ffi/gen_bindings/functions/orchestration.rs`'s
        // `has_error && is_void_return -> i32`), so this prose must say the same thing.
        // Printing "No return value" here while the signature line says `int32_t` two
        // lines above is worse than not knowing the ABI at all -- the page contradicts
        // itself and a reader can't tell which half to trust. The emitter always returns
        // exactly `-1` on failure (`orchestration.rs:224`), never some other non-zero
        // value, so say `-1` here -- the same value `formatting.rs`'s
        // `ffi_error_return_phrase` names in the Errors section below.
        if matches!(lang, Language::Ffi | Language::C) && error_type.is_some() {
            out.push_str("**Returns:** `int32_t` status code -- `0` on success, `-1` on error.\n");
        } else {
            out.push_str("**Returns:** No return value.\n");
        }
        out.push('\n');
        return;
    }

    let ret_ty = return_type_override
        .map(str::to_string)
        .unwrap_or_else(|| doc_type(return_type, lang, ffi_prefix));
    if ret_ty.is_empty() {
        out.push_str("**Returns:** No return value.\n");
        out.push('\n');
    } else {
        out.push_str(&template_env::render(
            "returns.jinja",
            minijinja::context! { ty => ret_ty },
        ));
        out.push('\n');
    }
}

pub(super) fn push_errors(out: &mut String, error_type: Option<&str>, return_type: &TypeRef, lang: Language) {
    if let Some(err) = error_type {
        let error_phrase = format_error_phrase(err, return_type, lang);
        out.push_str(&template_env::render(
            "errors_phrase.jinja",
            minijinja::context! { phrase => error_phrase },
        ));
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::test_helpers::{TEST_CRATE_NAME, TEST_PREFIX, make_function, make_param};

    /// ~keep The signature line and the "Returns:" prose are two independent renderers
    /// describing the same function on the same generated page. For a fallible Ffi/C
    /// function whose logical return is `()`, the ABI repurposes the return as an
    /// `int32_t` status code (see signatures.rs). If the two renderers drift -- one says
    /// `int32_t`, the other still says "No return value" -- the page contradicts itself,
    /// which is worse than the original bug because a reader can no longer tell which half
    /// to trust. This renders both from the same `FunctionDef` and asserts they agree,
    /// rather than asserting each against a literal that could independently go stale.
    #[test]
    fn test_c_signature_and_returns_prose_agree_on_fallible_void_status_code() {
        let func = make_function(
            "init",
            vec![make_param("config", TypeRef::Named("ClientConfig".to_string()), false)],
            TypeRef::Unit,
            false,
            Some("InitError"),
        );

        let signature = render_function_signature(&func, Language::C, TEST_PREFIX, TEST_CRATE_NAME);

        let mut returns_prose = String::new();
        push_returns(
            &mut returns_prose,
            &func.return_type,
            func.error_type.as_deref(),
            Language::C,
            TEST_PREFIX,
        );

        assert!(signature.starts_with("int32_t "), "signature: {signature}");
        assert!(
            returns_prose.contains("int32_t"),
            "returns prose must mention the same status-code type the signature declares: {returns_prose}"
        );
        assert!(
            !returns_prose.contains("No return value"),
            "must not still claim there's no return value once the signature says int32_t: {returns_prose}"
        );
    }

    #[test]
    fn test_c_signature_and_returns_prose_agree_infallible_void_stays_silent() {
        let func = make_function("touch", vec![], TypeRef::Unit, false, None);

        let signature = render_function_signature(&func, Language::C, TEST_PREFIX, TEST_CRATE_NAME);

        let mut returns_prose = String::new();
        push_returns(
            &mut returns_prose,
            &func.return_type,
            func.error_type.as_deref(),
            Language::C,
            TEST_PREFIX,
        );

        assert!(signature.starts_with("void "), "signature: {signature}");
        assert!(returns_prose.contains("No return value"), "{returns_prose}");
        assert!(!returns_prose.contains("int32_t"), "{returns_prose}");
    }

    #[test]
    fn test_returns_with_override_wins_over_status_code_inference_for_unit_return() {
        // ~keep A curated return-type override (streaming's use case) is trusted verbatim even
        // when the logical return type is `()` -- the status-code inference must not
        // second-guess it.
        let mut out = String::new();
        push_returns_with_override(
            &mut out,
            &TypeRef::Unit,
            Some("StreamHandle"),
            Some("StreamError"),
            Language::C,
            TEST_PREFIX,
        );
        assert!(out.contains("StreamHandle"), "{out}");
        assert!(!out.contains("int32_t"), "{out}");
    }
}
