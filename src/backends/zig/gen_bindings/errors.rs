use crate::core::ir::ErrorDef;

fn zig_error_variant_component(name: &str) -> String {
    crate::codegen::naming::public_host_identifier(
        crate::core::config::Language::Zig,
        crate::codegen::naming::PublicIdentifierKind::Type,
        name,
    )
}

pub(crate) fn emit_error_set(error: &ErrorDef, out: &mut String) {
    if !error.doc.is_empty() {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_doc_block.jinja",
            minijinja::context! {
                error_doc_lines => error.doc.lines().collect::<Vec<_>>(),
            },
        ));
    }
    out.push_str(&crate::backends::zig::template_env::render(
        "error_set_header.jinja",
        minijinja::context! {
            error_name => &error.name,
        },
    ));
    for variant in &error.variants {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_set_variant.jinja",
            minijinja::context! {
                variant_name => zig_error_variant_component(&variant.name),
            },
        ));
    }
    if !error
        .variants
        .iter()
        .any(|v| zig_error_variant_component(&v.name) == "OutOfMemory")
    {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_set_variant.jinja",
            minijinja::context! {
                variant_name => "OutOfMemory",
            },
        ));
    }
    // The generated helpers return `error.UnknownFfiError` whenever the FFI layer reports a
    // failure that no declared `#[alef(error_code = N)]` substantiates (the Zig mirror of
    // `ALEF_FFI_UNKNOWN_ERROR`). Zig coerces that only into a set that declares the member, and
    // the helpers are instantiated with every declared set, so it must be injected here. ~keep
    if !error
        .variants
        .iter()
        .any(|v| zig_error_variant_component(&v.name) == "UnknownFfiError")
    {
        out.push_str(&crate::backends::zig::template_env::render(
            "error_set_variant.jinja",
            minijinja::context! {
                variant_name => "UnknownFfiError",
            },
        ));
    }
    out.push_str("};\n");
}

/// Map a Rust error_type (e.g. `"anyhow::Error"`, `"SampleCrateError"`) to a
/// Zig error-set identifier. If the path's last segment matches a declared
/// error set, use it; otherwise fall back to the first declared error set
/// (the project's main error type).
pub(crate) fn resolve_zig_error_type(error_type: &str, declared: &[String]) -> String {
    let last = error_type.rsplit("::").next().unwrap_or(error_type);
    if declared.iter().any(|d| d == last) {
        return last.to_string();
    }
    declared.first().cloned().unwrap_or_else(|| "anyerror".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::ErrorVariant;

    fn variant(name: &str, template: Option<&str>) -> ErrorVariant {
        ErrorVariant {
            error_code: None,
            name: name.to_string(),
            message_template: template.map(str::to_string),
            ..ErrorVariant::default()
        }
    }

    // `emit_error_set` stopped emitting the `_from_ffi_msg_*` prefix-matcher in
    // a6f094df5 ("feat(go-zig): map typed errors by native code"); dispatch now
    // goes exclusively through the numeric FFI taxonomy code in
    // `helpers::emit_helpers`/`_error_with_message` (see
    // `error_with_message_dispatches_to_each_declared_error` in helpers.rs). ~keep
    #[test]
    fn emit_error_set_emits_only_the_error_set() {
        let error = ErrorDef {
            name: "MyError".into(),
            rust_path: "x::MyError".into(),
            original_rust_path: String::new(),
            variants: vec![variant("Boom", Some("Boom happened: {0}"))],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let mut out = String::new();
        emit_error_set(&error, &mut out);

        assert!(
            out.contains("pub const MyError = error{"),
            "expected error set header:\n{out}"
        );
        assert!(out.contains("Boom,"), "expected Boom variant:\n{out}");
        assert!(
            out.contains("OutOfMemory,"),
            "expected implicit OutOfMemory variant:\n{out}"
        );
        assert!(
            out.trim_end().ends_with("};"),
            "expected closing brace of error set:\n{out}"
        );
        assert!(
            !out.contains("_from_ffi_msg_"),
            "message-prefix dispatch was replaced by numeric taxonomy-code dispatch \
             in helpers::_error_with_message and must not be emitted here:\n{out}"
        );
        assert!(
            out.contains("UnknownFfiError,"),
            "expected implicit UnknownFfiError variant:\n{out}"
        );
    }

    /// `helpers::emit_helpers` emits `return error.UnknownFfiError;` inside a function whose
    /// return type is a caller-supplied `comptime E`. Zig only coerces that literal into `E`
    /// when `E` declares the member, and `E` is instantiated with every generated error set —
    /// so a set missing the member is a compile error in the emitted binding, not a runtime
    /// nicety. Injected exactly once even if the Rust enum already spells it. ~keep
    #[test]
    fn emit_error_set_never_duplicates_an_explicit_unknown_ffi_error_variant() {
        let error = ErrorDef {
            name: "MyError".into(),
            rust_path: "x::MyError".into(),
            original_rust_path: String::new(),
            variants: vec![variant("UnknownFfiError", None), variant("Boom", None)],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        };

        let mut out = String::new();
        emit_error_set(&error, &mut out);

        assert_eq!(
            out.matches("UnknownFfiError,").count(),
            1,
            "the unknown-error member must be declared exactly once:\n{out}"
        );
    }
}
