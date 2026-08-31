//! Exact Java and C# identifier character classes, resolved from a pinned Unicode table.
//!
//! Java and C# both define "identifier" in terms of Unicode general categories, but they do not
//! define it the *same* way, and neither matches Rust's `char::is_alphabetic` /
//! `char::is_alphanumeric`. Approximating either with the Rust predicates rejects legal
//! identifiers (a currency-symbol start such as `€`, a combining-mark continuation such as
//! `café`) and accepts illegal ones (`Other_Number`, e.g. `a²`), so each grammar is spelled out
//! here against the general-category table instead.
//!
//! # Pinned Unicode version
//!
//! Every predicate here reads [`unicode_general_category`], which is pinned to `=1.1.0` and
//! carries Unicode [`IDENTIFIER_UNICODE_VERSION`] data. The pin is load-bearing, not hygiene: an
//! unpinned table silently changes which identifiers this module accepts on a dependency bump.
//! `tests/unicode_dependency_pins.rs` guards the pin.
//!
//! # How these sets were established
//!
//! Not from the prose of either specification alone -- both were derived from the categories and
//! then checked against the shipping compilers, since the specification text and the compiler
//! disagree in more than one place (see `is_csharp_lexable` and `JAVA_RESERVED`'s `_` entry):
//!
//! - Every one of the 1_114_112 code points was compared between the pinned table and both
//!   `java.lang.Character.getType` (JDK 25.0.2) and `System.Globalization.CharUnicodeInfo`
//!   (.NET 10.0.0). All three agree on every code point, with zero mismatches.
//! - `Character.isJavaIdentifierStart` / `isJavaIdentifierPart` were then compared against the
//!   category sets below over the same full range, again with zero mismatches in either
//!   direction.
//! - 93 code points drawn from all 23 general categories that can appear literally in source
//!   were then compiled by `javac` 25.0.2 and `dotnet` 10.0.100, in both identifier-start and
//!   identifier-continuation position, and the compilers agreed with the sets below on all 186
//!   cases. The separator and control categories are excluded from that round on purpose: a
//!   compile probe cannot distinguish "this character is an identifier character" from "this
//!   character separated two tokens", so they rest on the exhaustive comparison instead.
//!
//! The first two rounds are one-off derivations and are recorded here as provenance. The
//! standing, re-runnable check is `tests/identifier_grammar_compiler_oracle.rs`: it writes probe
//! sources, invokes `javac` and `dotnet` as subprocesses, and compares their verdicts against
//! these predicates and against `codegen::coordinates` on every run. A transcribed table would
//! only prove the implementation equals itself, so the compiler is the authority at test time,
//! not a constant.

use unicode_general_category::{GeneralCategory, get_general_category};

/// Unicode version of the exactly pinned `unicode-general-category` table every predicate in
/// this module reads. Stated rather than inferred: which identifiers Java and C# accept is a
/// function of the Unicode version, so a table bump is a behaviour change that must be
/// deliberate. Verified equal to the Unicode version behind JDK 25 and .NET 10 (see module
/// docs).
pub const IDENTIFIER_UNICODE_VERSION: &str = "16.0.0";

/// `Lu | Ll | Lt | Lm | Lo | Nl` -- `Character.isLetter` plus `LETTER_NUMBER`, which is also
/// exactly the C# specification's `letter_character` (ECMA-334 6.4.3).
fn is_letter_or_letter_number(character: char) -> bool {
    matches!(
        get_general_category(character),
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
            | GeneralCategory::LetterNumber
    )
}

/// The non-whitespace ISO control characters that `Character.isIdentifierIgnorable` covers on
/// top of category `Cf`, and which are therefore legal *inside* a Java identifier. Verified
/// exhaustively: on JDK 25.0.2, `isIdentifierIgnorable` is exactly `Cf` united with these three
/// ranges over all 1_114_112 code points, with an empty symmetric difference. ~keep
fn is_java_identifier_ignorable_control(character: char) -> bool {
    matches!(character, '\u{0}'..='\u{8}' | '\u{e}'..='\u{1b}' | '\u{7f}'..='\u{9f}')
}

/// `Character.isJavaIdentifierStart`: `Lu | Ll | Lt | Lm | Lo | Nl | Sc | Pc`.
///
/// `Sc` is why `$` starts a Java identifier -- but so does every other currency symbol, `€`
/// included, and `Pc` admits connector punctuation beyond `_`. Both are routinely lost when this
/// is approximated as "alphabetic, `_`, or `$`".
pub fn is_java_identifier_start(character: char) -> bool {
    is_letter_or_letter_number(character)
        || matches!(
            get_general_category(character),
            GeneralCategory::CurrencySymbol | GeneralCategory::ConnectorPunctuation
        )
}

/// `Character.isJavaIdentifierPart`: every start category plus `Nd | Mn | Mc | Cf`, plus the
/// identifier-ignorable ISO controls.
///
/// The mark categories are the other half of the "alphabetic" approximation's error: `Mn`
/// carries combining accents (`café` written as `cafe` + U+0301) and `Cf` carries the zero-width
/// joiners that Indic and Arabic identifiers need.
pub fn is_java_identifier_part(character: char) -> bool {
    is_java_identifier_start(character)
        || matches!(
            get_general_category(character),
            GeneralCategory::DecimalNumber
                | GeneralCategory::NonspacingMark
                | GeneralCategory::SpacingMark
                | GeneralCategory::Format
        )
        || is_java_identifier_ignorable_control(character)
}

/// Whether a scalar can reach the C# lexer intact.
///
/// Roslyn classifies UTF-16 code units, not scalars. A supplementary-plane scalar arrives as two
/// surrogate halves, whose general category is `Cs`, which is in no identifier category -- so no
/// supplementary character is legal anywhere in a C# identifier regardless of its own category.
/// `javac` lexes code points and accepts the very same character, which is why Java and C#
/// cannot share one predicate. Confirmed against both compilers: `class P { int 𐐀z = 0; }`
/// compiles under `javac` and fails under `dotnet` with `CS1056: Unexpected character '𐐀'`.
/// ~keep
fn is_csharp_lexable(character: char) -> bool {
    character.len_utf16() == 1
}

/// C# `identifier_start_character` (ECMA-334 6.4.3): `letter_character | '_'`.
///
/// Only the literal U+005F is admitted, not category `Pc` at large -- `⁔` (U+2054, `Pc`) is a
/// legal C# identifier *part* but not a legal start, a distinction Java does not make.
pub fn is_csharp_identifier_start(character: char) -> bool {
    is_csharp_lexable(character) && (is_letter_or_letter_number(character) || character == '_')
}

/// C# `identifier_part_character` (ECMA-334 6.4.3): `letter_character | decimal_digit_character |
/// connecting_character | combining_character | formatting_character`, i.e. the start categories
/// plus `Nd | Pc | Mn | Mc | Cf`.
pub fn is_csharp_identifier_part(character: char) -> bool {
    is_csharp_lexable(character)
        && (is_letter_or_letter_number(character)
            || matches!(
                get_general_category(character),
                GeneralCategory::DecimalNumber
                    | GeneralCategory::ConnectorPunctuation
                    | GeneralCategory::NonspacingMark
                    | GeneralCategory::SpacingMark
                    | GeneralCategory::Format
            ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The authority for these predicates is `tests/identifier_grammar_compiler_oracle.rs`, which
    /// invokes `javac` and `dotnet` as subprocesses and compares their verdicts against these
    /// functions. The tests here deliberately do not restate the category sets: a table
    /// transcribed from the implementation proves only that it equals itself.
    ///
    /// What they do cover is the specific way the previous approximation was wrong, so a
    /// regression back to `char::is_alphabetic` / `char::is_alphanumeric` fails here even if no
    /// JDK or .NET SDK is installed on the machine running the suite.
    #[test]
    fn rust_alphabetic_disagrees_with_both_grammars_on_starts() {
        for character in ['$', '€', '\u{20a3}', '\u{203f}'] {
            let label = format!("U+{:04X}", character as u32);
            assert!(!character.is_alphabetic(), "{label} is not Rust-alphabetic");
            assert!(is_java_identifier_start(character), "{label} starts a Java identifier");
        }
        // ... and C# admits none of them, so one shared predicate cannot serve both.
        for character in ['$', '€', '\u{20a3}', '\u{203f}'] {
            assert!(!is_csharp_identifier_start(character), "U+{:04X}", character as u32);
        }
    }

    #[test]
    fn rust_alphanumeric_disagrees_with_both_grammars_on_continuations() {
        // False negatives: legal continuations Rust calls non-alphanumeric.
        for character in ['\u{301}', '\u{200c}', '\u{203f}'] {
            let label = format!("U+{:04X}", character as u32);
            assert!(!character.is_alphanumeric(), "{label}");
            assert!(is_java_identifier_part(character), "java {label}");
            assert!(is_csharp_identifier_part(character), "c# {label}");
        }
        // False positives: Other_Number is alphanumeric to Rust and illegal in both languages.
        for character in ['\u{b2}', '\u{2160}'] {
            assert!(character.is_alphanumeric(), "U+{:04X}", character as u32);
        }
        assert!(!is_java_identifier_part('\u{b2}'));
        assert!(!is_csharp_identifier_part('\u{b2}'));
    }

    #[test]
    fn csharp_is_bmp_only_because_roslyn_lexes_utf16_code_units() {
        // A supplementary letter is one scalar but two UTF-16 code units, each a lone surrogate
        // (category Cs) to Roslyn. Java's lexer works on code points and accepts it.
        assert_eq!('\u{10400}'.len_utf16(), 2);
        assert!(is_java_identifier_start('\u{10400}'));
        assert!(is_java_identifier_part('\u{10400}'));
        assert!(!is_csharp_identifier_start('\u{10400}'));
        assert!(!is_csharp_identifier_part('\u{10400}'));
        // The rule keys on encoded width, not on category: the BMP letter still passes.
        assert_eq!('A'.len_utf16(), 1);
        assert!(is_csharp_identifier_start('A'));
    }

    #[test]
    fn only_the_literal_underscore_starts_a_csharp_identifier() {
        // `_` is category Pc, but C# admits the character, not the category. Java admits both.
        assert!(is_csharp_identifier_start('_'));
        assert!(!is_csharp_identifier_start('\u{203f}'));
        assert!(is_csharp_identifier_part('\u{203f}'));
        assert!(is_java_identifier_start('\u{203f}'));
    }

    #[test]
    fn java_admits_identifier_ignorable_controls_that_coordinates_reject_by_policy() {
        // `Character.isJavaIdentifierPart` is true for these; `validate_java_package` still
        // rejects them, and that divergence is a deliberate coordinate policy, not a grammar bug.
        for character in ['\u{0}', '\u{8}', '\u{e}', '\u{1b}', '\u{7f}', '\u{9f}'] {
            assert!(is_java_identifier_part(character), "U+{:04X}", character as u32);
            assert!(character.is_control(), "U+{:04X}", character as u32);
        }
        for character in ['\u{9}', '\u{a}', '\u{d}', '\u{1c}'] {
            assert!(
                !is_java_identifier_part(character),
                "whitespace U+{:04X}",
                character as u32
            );
        }
    }

    #[test]
    fn separator_categories_are_not_identifier_characters() {
        for character in [' ', '\u{a0}', '\u{2002}', '\u{3000}', '\u{2028}', '\u{2029}'] {
            let label = format!("U+{:04X}", character as u32);
            assert!(!is_java_identifier_part(character), "java {label}");
            assert!(!is_csharp_identifier_part(character), "c# {label}");
        }
    }

    #[test]
    fn unicode_version_is_pinned_and_stated() {
        assert_eq!(IDENTIFIER_UNICODE_VERSION, "16.0.0");
    }
}
