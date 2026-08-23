//! Kotlin JSON value helpers.

use crate::e2e::escape::escape_kotlin;

/// The JVM's `CONSTANT_Utf8` constant-pool entry -- and `kotlinc`'s own enforcement of the same
/// cap on any single string literal, since Kotlin compiles to the same bytecode -- tops out at
/// 65535 bytes of modified UTF-8. No amount of escaping raises that ceiling; a value long enough
/// to threaten it has to stop being one literal. See the identical constraint and rationale on
/// the Java backend's `java_string_literal` (`src/e2e/codegen/java/values.rs`), which this
/// mirrors.
///
/// The budget is counted in raw characters, not bytes, because a single non-BMP character costs
/// up to 6 bytes in modified UTF-8 (a CESU-8 surrogate pair) while counting as one `char` here --
/// so the budget has to assume every character could be that expensive. `8_000 * 6 = 48_000`,
/// comfortably under the 65535 cap even before the escaping `kotlin_string_literal` already
/// performs on top of it.
const KOTLIN_STRING_LITERAL_CHUNK_CHARS: usize = 8_000;

/// Render `s` as a Kotlin string-literal expression that compiles regardless of length.
///
/// A single `"..."` literal cannot exceed the JVM's per-constant byte cap -- see
/// [`KOTLIN_STRING_LITERAL_CHUNK_CHARS`]. Values under the safe budget (the overwhelming
/// majority) render exactly as before, one `"..."` literal. A value long enough to threaten the
/// cap -- e.g. a large fixture body inlined into a generated doc snippet or e2e test -- is split
/// into `+`-concatenated literal chunks, each small enough that no single chunk can approach the
/// limit, and the whole expression is parenthesized so a caller can chain a method off it
/// (`.replace(...)`) without knowing whether it rendered one literal or several.
pub(super) fn kotlin_string_literal(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= KOTLIN_STRING_LITERAL_CHUNK_CHARS {
        return format!("\"{}\"", escape_kotlin(s));
    }
    let joined = chars
        .chunks(KOTLIN_STRING_LITERAL_CHUNK_CHARS)
        .map(|chunk| format!("\"{}\"", escape_kotlin(&chunk.iter().collect::<String>())))
        .collect::<Vec<_>>()
        .join(" + ");
    format!("({joined})")
}

/// Convert a `serde_json::Value` to a Kotlin literal string.
pub(super) fn json_to_kotlin(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => kotlin_string_literal(s),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => {
            if n.is_f64() {
                // Kotlin Double literals use no suffix (or `.0` if integer-shaped).
                // `0.9d` would parse as identifier `d` following a malformed literal.
                let s = n.to_string();
                if s.contains('.') || s.contains('e') || s.contains('E') {
                    s
                } else {
                    format!("{s}.0")
                }
            } else {
                n.to_string()
            }
        }
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_kotlin).collect();
            format!("listOf({})", items.join(", "))
        }
        serde_json::Value::Object(_) => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            kotlin_string_literal(&json_str)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The JVM's `CONSTANT_Utf8` constant-pool cap is exactly 65535 bytes. Values comfortably
    /// under it must render exactly as before: one plain `"..."` literal, no parentheses, no
    /// concatenation.
    #[test]
    fn a_short_value_stays_a_single_quoted_literal() {
        assert_eq!(kotlin_string_literal("hello world"), "\"hello world\"");
    }

    /// Neutral synthetic payload, well over the JVM's 65535-byte `CONSTANT_Utf8` cap. Not any
    /// real consumer's data -- chosen only to be unambiguously larger than the limit, per
    /// `project-agnostic-codegen`.
    fn oversized_payload() -> String {
        "abcdefghij".repeat(10_000) // 100,000 bytes
    }

    /// Regression twin of the Java fix (alef task #180): Kotlin compiles to the same JVM
    /// bytecode and inherits the identical 65535-byte `CONSTANT_Utf8` cap, so a value long
    /// enough to threaten it must never render as a single literal segment. ~keep
    #[test]
    fn a_value_over_the_jvm_constant_cap_is_never_a_single_literal_segment() {
        let literal = kotlin_string_literal(&oversized_payload());
        assert!(
            literal.contains(" + "),
            "an oversized value must be split into multiple concatenated literals: {literal}"
        );
        let inner = literal
            .strip_prefix('(')
            .and_then(|rest| rest.strip_suffix(')'))
            .unwrap_or(&literal);
        for segment in inner.split(" + ") {
            assert!(
                segment.len() <= 65_535,
                "a single Kotlin string literal segment must never exceed the JVM's 65535-byte \
                 CONSTANT_Utf8 cap: got {} bytes in {segment:?}",
                segment.len()
            );
        }
    }

    #[test]
    fn an_oversized_literal_is_parenthesized_so_a_caller_can_chain_a_method_onto_it() {
        let literal = kotlin_string_literal(&oversized_payload());
        assert!(
            literal.starts_with('(') && literal.ends_with(')'),
            "expected a parenthesized concatenation: {literal}"
        );
    }
}
