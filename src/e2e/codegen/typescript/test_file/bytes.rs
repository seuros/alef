//! Bytes-field value classification and TypeScript expression generation.
//!
//! `TypeRef::Bytes` fields are lowered to a real `Uint8Array` in the generated binding
//! (`Uint8Array<ArrayBufferLike>`). A fixture supplies that field's sample value as either a
//! JSON array of numbers or a JSON string (a relative file path, inline text, or base64 blob —
//! the same three-way convention `classify_bytes_value` in
//! `src/e2e/codegen/python/helpers.rs` and `is_file_path`/`is_base64` in
//! `src/e2e/codegen/ruby/values.rs` already use for their languages). Two TypeScript call
//! sites used to guess this independently: the napi object-literal builder
//! (`node_value_expression`) unconditionally wrapped any string in `Uint8Array.from(...)`
//! (producing a `string` argument where `Uint8Array.from` expects `Iterable<number>` — a type
//! error), and the WASM `default()`+setter builder had no string handling at all, emitting the
//! raw fixture string as the field value. Both now ask this single classifier instead of
//! carrying their own copy of the rule. ~keep

use super::*;

/// How to represent a fixture `type = "bytes"` JSON string value in generated TypeScript.
enum BytesKind {
    /// A relative file path like `"pdf/fake_memo.pdf"` — read via `node:fs/promises`.
    FilePath,
    /// Inline text content like `"<!DOCTYPE html>..."` — encode via `TextEncoder`.
    InlineText,
    /// A base64-encoded blob like `"/9j/4AAQ"` — decode via `Buffer.from(..., "base64")`.
    Base64,
}

/// Classify a fixture string value that maps to a `bytes` argument.
///
/// Mirrors `classify_bytes_value` in `src/e2e/codegen/python/helpers.rs` — the fixture
/// convention (bare relative path vs. inline text vs. base64) is shared across every backend
/// that reads a `bytes` fixture value from a JSON string.
fn classify_bytes_value(s: &str) -> BytesKind {
    if s.starts_with('<') || s.starts_with('{') || s.starts_with('[') || s.contains(' ') {
        return BytesKind::InlineText;
    }

    let first = s.chars().next().unwrap_or('\0');
    if (first.is_ascii_alphanumeric() || first == '_')
        && let Some(slash_pos) = s.find('/')
        && slash_pos > 0
    {
        let after_slash = &s[slash_pos + 1..];
        if after_slash.contains('.') && !after_slash.is_empty() {
            return BytesKind::FilePath;
        }
    }

    BytesKind::Base64
}

/// Build the TypeScript expression that lowers a fixture `bytes` field's JSON value to a real
/// `Uint8Array`, plus whether that expression contains an `await` — the caller's enclosing
/// scope (an IIFE, for the WASM setter builder) must be declared `async` when it does.
///
/// - A JSON array of numbers becomes `Uint8Array.from([...])`.
/// - A JSON string is classified via [`classify_bytes_value`] and becomes a file read (returns
///   a `Buffer`, which is a `Uint8Array` subtype), a `TextEncoder`-encoded literal, or a
///   base64-decoded `Buffer`.
/// - Any other JSON shape (malformed fixture data) falls back to `Uint8Array.from(...)` so the
///   error surfaces as a `tsc` diagnostic pointing at the right expression, rather than a panic
///   here.
pub(in crate::e2e::codegen::typescript::test_file) fn ts_bytes_value_expression(
    value: &serde_json::Value,
) -> (String, bool) {
    match value {
        serde_json::Value::String(s) => match classify_bytes_value(s) {
            BytesKind::FilePath => (
                format!(
                    "await (await import(\"node:fs/promises\")).readFile(\"{}\")",
                    escape_js(s)
                ),
                true,
            ),
            BytesKind::InlineText => (format!("new TextEncoder().encode(\"{}\")", escape_js(s)), false),
            BytesKind::Base64 => (format!("Buffer.from(\"{}\", \"base64\")", escape_js(s)), false),
        },
        other => (format!("Uint8Array.from({})", json_to_js(other)), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_bytes_value_pdf_path_is_file_path() {
        assert!(matches!(classify_bytes_value("pdf/fake_memo.pdf"), BytesKind::FilePath));
    }

    #[test]
    fn classify_bytes_value_html_is_inline() {
        assert!(matches!(classify_bytes_value("<!DOCTYPE html>"), BytesKind::InlineText));
    }

    #[test]
    fn classify_bytes_value_base64_is_base64() {
        assert!(matches!(
            classify_bytes_value("/9j/4AAQSkZJRgABAQEASABIAAD"),
            BytesKind::Base64
        ));
    }

    #[test]
    fn file_path_string_reads_the_file_and_needs_async() {
        let (expr, needs_await) = ts_bytes_value_expression(&serde_json::json!("pdf/fake_memo.pdf"));
        assert_eq!(
            expr,
            "await (await import(\"node:fs/promises\")).readFile(\"pdf/fake_memo.pdf\")"
        );
        assert!(needs_await);
    }

    #[test]
    fn inline_text_string_uses_text_encoder_and_stays_sync() {
        let (expr, needs_await) = ts_bytes_value_expression(&serde_json::json!("<!DOCTYPE html>"));
        assert_eq!(expr, "new TextEncoder().encode(\"<!DOCTYPE html>\")");
        assert!(!needs_await);
    }

    #[test]
    fn number_array_uses_uint8array_from_and_stays_sync() {
        let (expr, needs_await) = ts_bytes_value_expression(&serde_json::json!([72, 105]));
        assert_eq!(expr, "Uint8Array.from([72, 105])");
        assert!(!needs_await);
    }
}
