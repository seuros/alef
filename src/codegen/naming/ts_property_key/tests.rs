use super::*;

/// One row: the case being pinned, the serde wire name, the exact key token that must be emitted.
type KeyCase<'a> = (&'a str, &'a str, &'a str);

#[test]
fn should_emit_a_bare_key_only_when_the_wire_name_is_a_legal_identifier() {
    let cases: [KeyCase<'_>; 8] = [
        ("a snake_case core field name is already legal", "max_chars", "max_chars"),
        ("a camelCase wire name is already legal", "maxChars", "maxChars"),
        ("a leading underscore is legal", "_internal", "_internal"),
        ("a leading dollar is legal", "$ref", "$ref"),
        ("a dollar inside the name is legal", "a$b", "a$b"),
        ("digits after the first character are legal", "sha256", "sha256"),
        ("a single letter is legal", "x", "x"),
        (
            "an all-caps SCREAMING wire name is legal",
            "CONTENT_TYPE",
            "CONTENT_TYPE",
        ),
    ];

    for (case, wire_name, expected) in cases {
        assert_eq!(ts_property_key(wire_name), expected, "{case}");
    }
}

/// The defect this module exists to close. `#[serde(rename = "content-type")]` and
/// `#[serde(rename_all = "kebab-case")]` are ordinary serde, and the wire name they produce is
/// not an identifier: interpolated bare into a `.d.ts` member it parses as a subtraction, so the
/// whole declaration -- and with it the emitted `typescript_custom_section` -- is a syntax error.
#[test]
fn should_quote_a_wire_name_that_is_not_a_legal_identifier() {
    let cases: [KeyCase<'_>; 9] = [
        ("kebab-case from #[serde(rename)]", "content-type", "\"content-type\""),
        (
            "kebab-case from #[serde(rename_all = \"kebab-case\")]",
            "max-chars",
            "\"max-chars\"",
        ),
        ("a space cannot appear in an identifier", "content type", "\"content type\""),
        ("a leading digit cannot start an identifier", "2fa", "\"2fa\""),
        ("a bare digit run cannot start an identifier", "0", "\"0\""),
        ("a dot would read as member access", "a.b", "\"a.b\""),
        ("an at-sign is not an identifier character", "@type", "\"@type\""),
        ("a colon is not an identifier character", "xml:lang", "\"xml:lang\""),
        ("the empty wire name is not an identifier", "", "\"\""),
    ];

    for (case, wire_name, expected) in cases {
        assert_eq!(ts_property_key(wire_name), expected, "{case}");
    }
}

/// Non-ASCII is a legal ECMAScript `IdentifierName`, so quoting it is conservative rather than
/// required -- but it must be quoted *consistently*, never half-escaped into mojibake.
#[test]
fn should_quote_a_non_ascii_wire_name_without_mangling_it() {
    assert_eq!(ts_property_key("café"), "\"café\"");
    assert_eq!(ts_property_key("naïve_field"), "\"naïve_field\"");
}

/// Quoting alone is not enough: a wire name carrying a quote, a backslash, or a line terminator
/// would close or break the string literal it was pasted into. A `#[serde(rename)]` value is an
/// arbitrary Rust string literal, so all three are reachable from consumer source.
#[test]
fn should_escape_characters_that_would_break_the_quoted_key() {
    let cases: [KeyCase<'_>; 8] = [
        (
            "a double quote would close the key early",
            r#"he said "hi""#,
            r#""he said \"hi\"""#,
        ),
        ("a backslash must not escape the next character", r"back\slash", r#""back\\slash""#),
        (
            "a backslash before a quote must not be swallowed",
            r#"trailing\"#,
            r#""trailing\\""#,
        ),
        ("a newline cannot appear raw in a string literal", "line\nbreak", r#""line\nbreak""#),
        ("a carriage return cannot appear raw", "line\rbreak", r#""line\rbreak""#),
        ("a tab is escaped for legibility", "col\tumn", r#""col\tumn""#),
        (
            "U+2028 terminates a line even inside a string literal",
            "a\u{2028}b",
            r#""a\u2028b""#,
        ),
        (
            "an unnamed control character falls back to a \\u escape",
            "a\u{1}b",
            r#""a\u0001b""#,
        ),
    ];

    for (case, wire_name, expected) in cases {
        assert_eq!(ts_property_key(wire_name), expected, "{case}");
    }
}

/// Reserved words are quoted. This is conservative, not mandatory -- see the function's own note
/// -- but it must be *decided in one place*, so pin it rather than leaving it to be rediscovered.
#[test]
fn should_quote_a_reserved_word_used_as_a_wire_name() {
    for reserved in ["default", "class", "new", "function", "in", "typeof", "null", "true"] {
        assert_eq!(
            ts_property_key(reserved),
            format!("\"{reserved}\""),
            "reserved word {reserved} must be quoted"
        );
    }
}

/// `type` and `kind` are the two discriminator keys this repo's backends synthesize (see
/// `codegen::serde_enum_repr::tagged_object_tag_key`). Neither is an ECMAScript reserved word, so
/// both stay bare -- if they ever started coming back quoted, every tagged-union `.d.ts`
/// assertion in the napi and wasm suites would be describing a different declaration.
#[test]
fn should_leave_the_discriminator_keys_bare() {
    assert_eq!(ts_property_key("type"), "type");
    assert_eq!(ts_property_key("kind"), "kind");
}
