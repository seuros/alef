//! The funnel for "this assertion's TYPE (not its field) could not be rendered here" skip markers.
//!
//! ~keep [`field_skip::FieldSkip`]'s own doc declares assertion-*type* skips out of scope by
//! design: "a bad assertion shape, not an unreachable field." That line is correct as a
//! description of what `FieldSkip` recognises, but it also meant `ALEF_E2E_STRICT_ASSERTIONS`
//! examined nothing on this axis — a backend could render `// skipped: ...` for an assertion
//! type it cannot express, and the strict gate would walk right past it, because nothing in
//! `fail_on_unavailable_field_markers` was looking for that wording. A clean gate run was
//! therefore indistinguishable from a run that dropped whole categories of assertions.
//!
//! This module is the parallel funnel for that axis. It deliberately does NOT extend
//! [`field_skip::FieldSkip`] or feed [`super::fail_on_unavailable_field_markers`]: the two axes
//! answer different questions ("does this field exist on the result?" vs. "can this backend
//! express this assertion shape at all?") and merging them would either force a reclassification
//! of existing `FieldSkip` variants or silently invalidate
//! `field_skip::tests::unsupported_assertion_type_wordings_stay_uncounted` /
//! `mod::unavailable_field_marker_tests::unsupported_assertion_type_comments_are_not_recorded`,
//! both of which pin that separation as a deliberate negative control. A type-driven skip is
//! also never something a fixture edit can fix — no `FieldSkip` variant is ever
//! [`field_skip::SkipClass::AuthoringGap`]-classified here, unlike the field axis, so recording it
//! never needs to consult a fixture's `skip` acknowledgement. The verdict follows straight from
//! the class.
//!
//! Same shape/recognition design as `field_skip.rs`: a variant's rendered wording and its
//! recognition read the same [`Shape`], and [`AssertionTypeSkip::ALL`] is generated from the
//! same macro arm as the variant list, so a variant cannot exist without the strict gate
//! counting it.

use super::field_skip::SkipClass;

/// The rendered text on either side of the quoted assertion-type (or, where the existing wording
/// never named the type, the field) token for one registered wording.
struct Shape {
    before: &'static str,
    after: &'static str,
}

macro_rules! assertion_type_skip_variants {
    ($($(#[$meta:meta])* $variant:ident : $class:ident => ($before:expr, $after:expr $(,)?)),+ $(,)?) => {
        /// A registered reason an assertion's *type* — not its field — could not be rendered.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum AssertionTypeSkip {
            $($(#[$meta])* $variant,)+
        }

        impl AssertionTypeSkip {
            /// Every variant, generated from the same macro arm as the variant list. ~keep
            const ALL: &'static [Self] = &[$(Self::$variant,)+];

            const fn shape(self) -> Shape {
                match self {
                    $(Self::$variant => Shape { before: $before, after: $after },)+
                }
            }

            /// ~keep No variant here is ever [`SkipClass::AuthoringGap`]: a bad assertion *shape*
            /// is never something a fixture edit fixes, so that branch is handled defensively in
            /// [`super::fail_on_unsupported_assertion_type_markers`] rather than reachable from
            /// any registered variant.
            pub(crate) const fn class(self) -> SkipClass {
                match self {
                    $(Self::$variant => SkipClass::$class,)+
                }
            }
        }
    };
}

assertion_type_skip_variants! {
    /// ~keep Emitted by `go`/`csharp`/`ruby`/`zig` when a synthetic-field's assertion type falls
    /// through their `match assertion.assertion_type.as_str()`'s default arm. The wording never
    /// names which type was unsupported (a pre-existing gap in the message itself, out of scope
    /// here), so this variant's captured token is the synthetic field, not the type.
    UnsupportedAssertionTypeOnSyntheticField: GeneratorGap => (
        "unsupported assertion type on synthetic field ",
        "",
    ),
    /// ~keep Emitted by most backends (dart/elixir/go/java/kotlin/php/python/r/ruby/rust/swift/
    /// typescript/csharp) when a traversal-shaped assertion (`equals`/`contains`/... against a
    /// `foo[].bar` path) names an assertion type their traversal renderer does not implement.
    UnsupportedTraversalAssertion: GeneratorGap => (
        "unsupported traversal assertion ",
        " on '",
    ),
    /// ~keep Emitted by `csharp/streaming.rs` when a streaming assertion names a type its
    /// aggregator-variable renderer does not implement.
    StreamingAssertionTypeNotSupported: GeneratorGap => (
        "assertion type ",
        " on field '",
    ),
    /// ~keep Emitted by `swift/assertions.rs` for `not_empty`/`is_empty`/`count_equals` against a
    /// field marked as an array in config but which is actually a scalar `String` — `.count` has
    /// no meaningful reading there. A property of the field's real Swift type, not alef's debt.
    ScalarWithoutMeaningfulCountInSwift: LanguageLimitation => (
        "field ",
        " is a scalar String without meaningful .count",
    ),
    /// ~keep The proof-of-concept wording this change adds: `python/test_function/
    /// error_assertions.rs` renders this for every fixture assertion beyond the one `"error"`-type
    /// check `emit_error_assertion` already handles — most commonly an `equals` assertion against
    /// an `error.<field>` path, which only the `rust` backend can resolve (see
    /// `rust/assertions.rs`'s `accessor_for_error`). Before this change python rendered nothing at
    /// all for these assertions: not even a skip comment, so the strict gate had no line to see.
    EqualsOnErrorFieldNotSupported: GeneratorGap => (
        "assertion type ",
        " has no accessor for error field ",
    ),
}

impl AssertionTypeSkip {
    /// The token — an assertion type, except for
    /// [`AssertionTypeSkip::UnsupportedAssertionTypeOnSyntheticField`] where the field name is
    /// the only thing the wording names — plus the variant that named it.
    pub(crate) fn extract_classified(line: &str) -> Option<(&str, Self)> {
        Self::ALL
            .iter()
            .find_map(|variant| variant.token_in(line).map(|token| (token, *variant)))
    }

    /// Every occurrence of `before` is tried, mirroring [`super::field_skip::FieldSkip::field_in`]:
    /// `before` is sometimes a substring of an unrelated longer phrase, so the first hit is not
    /// necessarily the quoted token this variant means to recognise.
    fn token_in(self, line: &str) -> Option<&str> {
        let Shape { before, after } = self.shape();
        for (start, _) in line.match_indices(before) {
            let rest = &line[start + before.len()..];
            let Some(quoted) = rest.strip_prefix('\'') else {
                continue;
            };
            let Some(end) = quoted.find('\'') else {
                continue;
            };
            if quoted[end + 1..].starts_with(after) {
                return Some(&quoted[..end]);
            }
        }
        None
    }

    #[cfg(test)]
    pub(crate) fn extract(line: &str) -> Option<&str> {
        Self::extract_classified(line).map(|(token, _)| token)
    }
}

#[cfg(test)]
mod tests {
    use super::AssertionTypeSkip;
    use crate::e2e::codegen::field_skip::SkipClass;

    #[test]
    fn no_variant_is_classified_as_an_authoring_gap() {
        for variant in AssertionTypeSkip::ALL {
            assert_ne!(
                variant.class(),
                SkipClass::AuthoringGap,
                "{variant:?} must never be an AuthoringGap: a bad assertion shape is never \
                 fixable by a fixture edit"
            );
        }
    }

    #[test]
    fn synthetic_field_wording_is_recognised() {
        let line = "\t// skipped: unsupported assertion type on synthetic field 'embeddings'";
        assert_eq!(AssertionTypeSkip::extract(line), Some("embeddings"));
    }

    #[test]
    fn traversal_wording_captures_the_assertion_type_not_the_field() {
        let line = "    // skipped: unsupported traversal assertion 'equals' on 'pages[].url'";
        assert_eq!(AssertionTypeSkip::extract(line), Some("equals"));
    }

    #[test]
    fn streaming_wording_captures_the_assertion_type() {
        let line = "        // skipped: assertion type 'count_min' on field 'chunks' not yet supported for streaming";
        assert_eq!(AssertionTypeSkip::extract(line), Some("count_min"));
    }

    #[test]
    fn swift_scalar_count_wording_captures_the_field() {
        let line = "        // skipped: field 'content' is a scalar String without meaningful .count";
        assert_eq!(AssertionTypeSkip::extract(line), Some("content"));
    }

    #[test]
    fn error_field_wording_captures_the_assertion_type() {
        let line = "    # skipped: assertion type 'equals' has no accessor for error field status_code in this backend";
        assert_eq!(AssertionTypeSkip::extract(line), Some("equals"));
    }

    /// Negative control: an ordinary field-availability skip (the `FieldSkip` axis) must not be
    /// recognised here — the two funnels stay disjoint.
    #[test]
    fn field_availability_wordings_stay_uncounted() {
        let line = "    // skipped: field 'chunks' not available on result type";
        assert_eq!(AssertionTypeSkip::extract(line), None);
    }

    #[test]
    fn a_line_with_no_marker_is_not_recognised() {
        assert_eq!(AssertionTypeSkip::extract("    assert result.count == 1"), None);
    }
}
