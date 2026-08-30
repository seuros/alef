//! Hostile-input coverage for the Java literal escaper.
//!
//! Each case is a wire name a consumer can legally write in `#[serde(rename = "...")]` and that
//! breaks emitted Java source when pasted between two quote characters. The `javac` oracle in
//! `tests/backends_java_hostile_serde_name_compile_test.rs` proves the same inputs against a real
//! compiler, including the negative control that the unescaped emission is rejected. ~keep
//!
//! Java Unicode escapes are spelled `concat!("\\", "uXXXX")` throughout rather than written out,
//! so that no source line of this file contains a bare backslash-`u` sequence for a tool to
//! reinterpret on its way here. ~keep

use super::{escape_java_comment_text, escape_java_string_literal};

/// A consumer-authored wire name that literally contains backslash, `u`, `0041` — the sequence
/// `javac` substitutes before lexing.
const RAW_UNICODE_ESCAPE: &str = concat!("\\", "u0041");

/// The same name after escaping: the backslash is doubled, so the `\` in front of `u` is
/// odd-preceded and no longer eligible to start a Unicode escape.
const ESCAPED_UNICODE_ESCAPE: &str = concat!("\\\\", "u0041");

/// Every hostile shape the escaper has to survive, reused by the invariant tests below.
const HOSTILE_NAMES: &[&str] = &[
    r#"quote"inside"#,
    r"back\slash",
    RAW_UNICODE_ESCAPE,
    r"\u{1F600}",
    r"trailing\",
    "new\nline",
    "carriage\rreturn",
    "tab\there",
    "nul\0byte",
    "bell\u{7}here",
    "escape\u{1b}[0m",
    "delete\u{7f}here",
    "caf\u{e9}",
    "emoji\u{1F600}name",
    "line\u{2028}separator",
    r#"breakout") String x; //"#,
    "comment*/close",
    "angle<bracket>and&amp",
];

#[test]
fn escapes_the_double_quote_that_would_close_the_literal() {
    assert_eq!(escape_java_string_literal(r#"quote"inside"#), r#"quote\"inside"#);
}

#[test]
fn escapes_the_backslash_that_would_consume_the_next_character() {
    assert_eq!(escape_java_string_literal(r"back\slash"), r"back\\slash");
    assert_eq!(escape_java_string_literal(r"trailing\"), r"trailing\\");
}

/// The subtle one: JLS §3.3 runs Unicode-escape substitution *before* lexing, so a raw
/// backslash-`u` in emitted source is rewritten (or rejected as `illegal unicode escape`) even
/// inside a string literal. Doubling the backslash is what makes it inert.
#[test]
fn neutralizes_a_raw_unicode_escape_sequence() {
    assert_eq!(escape_java_string_literal(RAW_UNICODE_ESCAPE), ESCAPED_UNICODE_ESCAPE);
    assert_eq!(escape_java_string_literal(r"\u{1F600}"), r"\\u{1F600}");
    // A malformed sequence is the one javac rejects outright rather than silently rewriting.
    assert_eq!(escape_java_string_literal(r"\uZZZZ"), r"\\uZZZZ");
}

#[test]
fn escapes_line_terminators_with_named_escapes_never_unicode_escapes() {
    assert_eq!(escape_java_string_literal("new\nline"), r"new\nline");
    assert_eq!(escape_java_string_literal("carriage\rreturn"), r"carriage\rreturn");
    assert_eq!(escape_java_string_literal("tab\there"), r"tab\there");
    assert_eq!(escape_java_string_literal("back\u{8}space"), r"back\bspace");
    assert_eq!(escape_java_string_literal("form\u{c}feed"), r"form\ffeed");
}

/// Control characters with no named escape use Java's lexer-level octal form. A Unicode escape
/// would be substituted before lexing and could reintroduce the very line terminator it hid.
#[test]
fn escapes_remaining_control_characters_as_three_digit_octal() {
    assert_eq!(escape_java_string_literal("nul\0byte"), r"nul\000byte");
    assert_eq!(escape_java_string_literal("bell\u{7}here"), r"bell\007here");
    assert_eq!(escape_java_string_literal("escape\u{1b}[0m"), r"escape\033[0m");
    assert_eq!(escape_java_string_literal("delete\u{7f}here"), r"delete\177here");
    assert_eq!(escape_java_string_literal("c1\u{9f}here"), r"c1\237here");
}

/// Three octal digits always: `\0` beside a following digit would otherwise be read as one
/// longer escape and swallow the digit.
#[test]
fn octal_escapes_do_not_swallow_a_following_digit() {
    assert_eq!(escape_java_string_literal("\u{7}7"), r"\0077");
}

#[test]
fn escapes_non_ascii_as_utf16_unicode_escapes() {
    assert_eq!(escape_java_string_literal("caf\u{e9}"), concat!("caf", "\\", "u00E9"));
    assert_eq!(
        escape_java_string_literal("line\u{2028}separator"),
        concat!("line", "\\", "u2028", "separator")
    );
}

/// An astral code point needs a surrogate *pair*: one Java Unicode escape names one UTF-16 code
/// unit, so a single escape would name a lone high surrogate followed by stray text.
#[test]
fn escapes_astral_code_points_as_a_surrogate_pair() {
    assert_eq!(
        escape_java_string_literal("emoji\u{1F600}name"),
        concat!("emoji", "\\", "uD83D", "\\", "uDE00", "name")
    );
}

#[test]
fn leaves_ordinary_wire_names_untouched() {
    for name in ["max_tokens", "top_p", "type", "SCREAMING_NAME", "a-b.c:d"] {
        assert_eq!(escape_java_string_literal(name), name, "{name} must pass through");
    }
}

#[test]
fn escapes_an_annotation_breakout_payload() {
    assert_eq!(
        escape_java_string_literal(r#"breakout") String x; //"#),
        r#"breakout\") String x; //"#
    );
}

/// Whatever the input, the escaped form must be pure ASCII, single-line, carry no unescaped
/// quote, and contain only well-formed Unicode escapes — the four properties `javac` checks.
#[test]
fn escaped_output_is_always_a_well_formed_java_literal_body() {
    for name in HOSTILE_NAMES {
        let escaped = escape_java_string_literal(name);
        assert!(escaped.is_ascii(), "{name:?} escaped to non-ASCII: {escaped:?}");
        assert!(
            !escaped.chars().any(char::is_control),
            "{name:?} escaped to a raw control character: {escaped:?}"
        );
        assert_eq!(
            unescaped_quote_count(&escaped),
            0,
            "{name:?} escaped to an unescaped quote: {escaped:?}"
        );
        assert!(
            unicode_escapes_are_well_formed(&escaped),
            "{name:?} escaped to a malformed unicode escape: {escaped:?}"
        );
    }
}

#[test]
fn comment_escaping_cannot_close_the_comment_or_inject_markup() {
    for name in HOSTILE_NAMES {
        let escaped = escape_java_comment_text(name);
        assert!(!escaped.contains("*/"), "{name:?} can close a comment: {escaped:?}");
        assert!(
            !escaped.contains('<') && !escaped.contains('>'),
            "{name:?} leaks javadoc markup: {escaped:?}"
        );
        assert!(
            !escaped.chars().any(char::is_control),
            "{name:?} escapes to a control character inside a comment: {escaped:?}"
        );
    }
    assert_eq!(escape_java_comment_text("comment*/close"), "comment*&#47;close");
    assert_eq!(
        escape_java_comment_text("angle<bracket>and&amp"),
        "angle&lt;bracket&gt;and&amp;amp"
    );
}

/// Counts `"` characters that are *not* preceded by an odd-length run of backslashes, i.e. the
/// ones that would actually terminate a Java string literal.
fn unescaped_quote_count(escaped: &str) -> usize {
    let bytes = escaped.as_bytes();
    let mut count = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'"' {
            continue;
        }
        let preceding_backslashes = bytes[..index].iter().rev().take_while(|b| **b == b'\\').count();
        if preceding_backslashes.is_multiple_of(2) {
            count += 1;
        }
    }
    count
}

/// True when every odd-length backslash run followed by `u` introduces exactly four hexadecimal
/// digits — the only Unicode-escape shape `javac`'s pre-lexer accepts.
fn unicode_escapes_are_well_formed(escaped: &str) -> bool {
    let bytes = escaped.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'u' || index == 0 {
            continue;
        }
        let preceding_backslashes = bytes[..index].iter().rev().take_while(|b| **b == b'\\').count();
        if preceding_backslashes.is_multiple_of(2) {
            continue;
        }
        let digits = &bytes[index + 1..];
        if digits.len() < 4 || !digits[..4].iter().all(u8::is_ascii_hexdigit) {
            return false;
        }
    }
    true
}
