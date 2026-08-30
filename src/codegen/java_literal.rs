//! The single Java source-literal escaper.
//!
//! Wire names (`#[serde(rename)]`, `#[serde(tag)]`, `#[serde(content)]` and `rename_all` output)
//! are arbitrary Rust string literals. Every one of them is interpolated into Java *source* —
//! `@JsonProperty("...")`, `@JsonSubTypes.Type(name = "...")`, `case "..." ->`,
//! `gen.writeStringField("...", tag)` — so a name carrying a double quote, a backslash or a
//! newline terminates the literal it was pasted into and the generated package stops compiling.
//! Backends must route every such interpolation through this module instead of pasting the raw
//! name between two quote characters.
//!
//! # Why a backslash may never survive unchanged
//!
//! JLS §3.3 processes Unicode escapes in the *first* translation step, before the source is even
//! lexed. A Unicode escape naming U+0022 is therefore a real double quote *everywhere* — inside
//! `//` comments, inside `/** */` javadoc — and a `\u` followed by anything that is not four hex
//! digits is `illegal unicode escape` rather than four harmless characters. A wire name
//! containing a literal backslash-`u` is consequently a compile error in emitted source that a
//! naive "escape the quotes" pass would call safe.
//!
//! JLS §3.3 also says a backslash is only *eligible* to start a Unicode escape when preceded by
//! an even number of backslashes. Doubling every backslash therefore disarms the pre-lexer: the
//! `\` in front of a `u` always has an odd-count run of backslashes before it and is read as
//! data. ~keep
//!
//! # Why control characters use octal, not `\uXXXX`
//!
//! `\uXXXX` is substituted before lexing, so a Unicode escape naming U+000A becomes a *real*
//! line terminator and ends the literal. Java's octal escapes (`\000`–`\377`) are lexer-level
//! and cannot do that, so control characters without a named escape use the three-digit octal
//! form. Non-ASCII characters have no such hazard and are emitted as `\uXXXX` UTF-16 code units
//! (an astral character becomes a surrogate pair) to keep generated sources pure ASCII: `javac`
//! decodes source with the platform charset unless told otherwise, and a raw UTF-8 byte sequence
//! read as a single-byte charset is silently mangled rather than rejected. ~keep

/// The highest code point Java's octal escape (`\000`–`\377`) can express.
const MAX_OCTAL_ESCAPE: u32 = 0o377;

/// First code point that is not plain ASCII.
const FIRST_NON_ASCII: u32 = 0x80;

/// Escape `value` for embedding between the double quotes of a Java string literal.
///
/// The result is pure ASCII, contains no line terminator, and contains no backslash that is
/// eligible to start a Unicode escape. It is therefore also safe inside a Java comment, except
/// that a comment additionally has to survive `*/` — use [`escape_java_comment_text`] there.
///
/// Callers supply their own surrounding quotes; this escapes the literal *content* only.
pub fn escape_java_string_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            // Must come first: doubling the backslash is what disarms JLS §3.3 pre-lexing, and
            // every later arm depends on its own output no longer being reachable as data. ~keep
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            other => push_escaped_scalar(&mut out, other),
        }
    }
    out
}

/// Escape `value` for embedding into Java comment text (`//`, `/* */`, `/** */`).
///
/// Everything [`escape_java_string_literal`] guarantees, plus: no `*/` that would close the
/// comment early, and no bare `<`, `>` or `&` that javadoc's HTML reader (or a doclint run)
/// would interpret as markup.
pub fn escape_java_comment_text(value: &str) -> String {
    let escaped = escape_java_string_literal(value);
    let mut out = String::with_capacity(escaped.len());
    let mut characters = escaped.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '*' if characters.peek() == Some(&'/') => {
                characters.next();
                out.push_str("*&#47;");
            }
            other => out.push(other),
        }
    }
    out
}

/// Emit one scalar that has no named escape: printable ASCII verbatim, any remaining control
/// character as three-digit octal, everything else as `\uXXXX` UTF-16 code units.
fn push_escaped_scalar(out: &mut String, character: char) {
    let code_point = character as u32;
    if code_point < FIRST_NON_ASCII && !character.is_control() {
        out.push(character);
        return;
    }
    if character.is_control() && code_point <= MAX_OCTAL_ESCAPE {
        out.push_str(&format!("\\{code_point:03o}"));
        return;
    }
    let mut units = [0u16; 2];
    for unit in character.encode_utf16(&mut units) {
        out.push_str(&format!("\\u{unit:04X}"));
    }
}

#[cfg(test)]
mod tests;
