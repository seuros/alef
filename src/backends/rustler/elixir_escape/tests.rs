//! Table-driven expectations for [`super::escape_elixir_string_literal`] and
//! [`super::elixir_atom_body`].
//!
//! Every row's `escaped` and `atom_body` column was produced by this module and then fed to the
//! REAL `elixir` interpreter (1.20.4), which compiled `:<atom_body>` and `"<escaped>"` and
//! confirmed `Atom.to_string/1` and the string itself both return the `input` column byte for
//! byte, with no interpolation side effect. These are the pinned results of that run; the
//! executable oracle that re-derives them from a live toolchain is
//! `tests/backends_rustler_elixir_escaping_oracle.rs`. ~keep

use super::{elixir_atom_body, escape_elixir_string_literal};

/// `input`, the escaped string-literal body, and the atom body (bare or quoted).
const CASES: &[(&str, &str, &str)] = &[
    ("plain", "plain", "plain"),
    ("og:image", "og:image", "\"og:image\""),
    ("123", "123", "\"123\""),
    ("1foo", "1foo", "\"1foo\""),
    ("", "", "\"\""),
    ("café", "café", "\"café\""),
    ("has space", "has space", "\"has space\""),
    ("quote\"inside", "quote\\\"inside", "\"quote\\\"inside\""),
    ("back\\slash", "back\\\\slash", "\"back\\\\slash\""),
    ("interp#{1 + 1}end", "interp\\#{1 + 1}end", "\"interp\\#{1 + 1}end\""),
    ("hash#nointerp", "hash\\#nointerp", "\"hash\\#nointerp\""),
    ("ctrl\u{0}nul", "ctrl\\u{0}nul", "\"ctrl\\u{0}nul\""),
    ("nl\nline", "nl\\u{a}line", "\"nl\\u{a}line\""),
    ("tab\there", "tab\\u{9}here", "\"tab\\u{9}here\""),
    ("del\u{7f}", "del\\u{7f}", "\"del\\u{7f}\""),
    ("valid?", "valid?", "valid?"),
    ("valid!", "valid!", "valid!"),
    ("end", "end", "end"),
    ("a?b", "a?b", "\"a?b\""),
];

#[test]
fn escape_and_atom_body_agree_with_the_elixir_verified_table() {
    for (input, expected_escaped, expected_atom_body) in CASES {
        assert_eq!(
            &escape_elixir_string_literal(input),
            expected_escaped,
            "string-literal escaping of {input:?}"
        );
        assert_eq!(
            &elixir_atom_body(input),
            expected_atom_body,
            "atom body for {input:?}"
        );
    }
}

/// The one character an Elixir escaper is most likely to omit, called out on its own so a
/// regression names itself. `#` alone is inert; `#{` is an interpolation, and an interpolation in
/// a generated literal is code execution at the generated module's compile time — not a syntax
/// error a compiler would catch. Both forms parse, so only escaping prevents it.
#[test]
fn interpolation_openers_are_neutralised_in_both_literal_forms() {
    let payload = "x#{System.halt(1)}y";
    assert!(
        !escape_elixir_string_literal(payload).contains("#{"),
        "an unescaped `#{{` in a generated string literal executes when the module compiles"
    );
    assert!(
        !elixir_atom_body(payload).contains("#{"),
        "an unescaped `#{{` in a generated quoted atom executes when the module compiles"
    );
}

/// A name that is already a bare identifier must not acquire quotes: `:"foo"` and `:foo` are the
/// same atom, but the quoted spelling would churn every generated file and every snapshot.
#[test]
fn bare_identifiers_stay_unquoted() {
    for name in ["foo", "_private", "a1", "snake_case_name", "q?", "bang!"] {
        assert_eq!(&elixir_atom_body(name), name, "{name} must stay a bare atom");
    }
}

/// The whole invalid-atom space in one place, as a checklist: whatever else changes, none of
/// these may come back as a bare (unquoted) atom, because each is a `SyntaxError` after a `:`.
#[test]
fn every_invalid_atom_shape_comes_back_quoted() {
    for name in ["", "123", "1foo", "café", "has space", "og:image", "a?b", "-lead", "no!bang"] {
        let body = elixir_atom_body(name);
        assert!(
            body.starts_with('"') && body.ends_with('"'),
            "{name:?} is not a bare Elixir identifier, so `:{name}` would be a SyntaxError; \
             it must be rendered as a quoted atom, got {body}"
        );
    }
}
