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
    /// ~keep No longer emitted: `swift/assertions.rs`'s count arms now render
    /// `FieldSkip::CountOnJsonBridgedLeafInSwift`, which names the real cause (a JSON-bridged
    /// getter) and files it under the field axis where it belongs. Kept so the recognition set
    /// still matches suites generated before that change; do not add new emitters.
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
    /// ~keep No existing variant fits, so this one is new. The streaming renderers in
    /// `{dart,go,java,kotlin,php,swift,typescript,zig}/assertions.rs` implement each assertion
    /// type against a value they first narrow (`value.as_u64()`, `Value::String(..)`); when the
    /// fixture's value does not survive that narrowing the arm produced an empty string and the
    /// assertion vanished with no line for any funnel to see.
    ///
    /// `GeneratorGap`, not `AuthoringGap`: the narrowing is alef's, not the fixture's — a
    /// `greater_than` against a float is a legitimate assertion that no streaming renderer
    /// currently emits — and [`AssertionTypeSkip`] is pinned never to classify a variant as an
    /// authoring gap (see `no_variant_is_classified_as_an_authoring_gap`). Failing a consumer's
    /// build on alef's own numeric narrowing would only buy a blanket opt-out.
    StreamingAssertionValueNotRenderable: GeneratorGap => (
        "assertion type ",
        " has no renderable value for streaming field ",
    ),
    /// ~keep Emitted by `declared_error_variant::skip_line` for a fixture's declared `error`
    /// value that names a real `ErrorVariant` a backend's generated binding cannot
    /// differentiate from any other variant of the same error type, where the reason is a true
    /// ABI/toolchain property (`c`'s numeric-only error surface; `dart`'s third-party
    /// flutter_rust_bridge decode step; an uncoded `go`/`java`/`zig` variant, since carrying a
    /// stable ABI taxonomy code is a property of the crate's declared error shape). See
    /// `declared_error_variant::substantiates_variant_identity` for the per-backend evidence.
    /// `LanguageLimitation`, not `AuthoringGap`: no fixture edit changes any of this, the same
    /// axis `NotAvailableInCFfi` already occupies.
    DeclaredErrorVariantNotSubstantiated: LanguageLimitation => (
        "declared error variant ",
        " not substantiated by this backend's generated error type",
    ),
    /// ~keep The `GeneratorGap` sibling of [`Self::DeclaredErrorVariantNotSubstantiated`]: the
    /// backend's own runtime supports distinct per-variant error identities (a class, an atom, a
    /// condition) — `php`, `csharp`, `swift`, `ruby`, `elixir`, `gleam`, `r` and `typescript` all
    /// do — but alef's generator for that backend does not build one yet. Fixable in a future
    /// alef release, not a permanent limit, so it is counted separately from
    /// [`Self::DeclaredErrorVariantNotSubstantiated`] the same way every other `GeneratorGap`
    /// here is kept apart from a `LanguageLimitation`.
    DeclaredErrorVariantNotYetPreservedByGenerator: GeneratorGap => (
        "declared error variant ",
        " not yet preserved as a distinct identity by this backend's generator",
    ),
    /// ~keep A fixture commonly declares its error expectation as two `"error"`-type assertions
    /// — a bare `{"type": "error"}` plus one carrying a message/type-name `value` — and every
    /// backend's message check (via `declared_error_value`) renders exactly one such value. Both
    /// of *those* assertions are fully rendered and neither is this variant. This variant is for
    /// the shape no observed fixture actually uses: a THIRD `"error"` assertion (or a second one
    /// also carrying a value), where the additional declared value has no second check to land
    /// in. `GeneratorGap`, not `AuthoringGap`: a backend that checked every declared value instead
    /// of only the first would render this fixture correctly, so a future alef release — not a
    /// fixture edit — is what closes this gap.
    AdditionalDeclaredErrorValueNotChecked: GeneratorGap => (
        "assertion type ",
        " has an additional declared value not checked by this backend",
    ),
    /// ~keep Emitted verbatim by `csharp/discriminated.rs` and `kotlin/discriminated.rs` (which
    /// `kotlin_android` reuses) when a discriminated-union field carries an assertion type their
    /// union renderer does not implement. It went unregistered until 0.77.0, so
    /// `ALEF_E2E_STRICT_ASSERTIONS` walked past every instance -- the gate reported clean on a
    /// line it could not classify, which is the exact failure this registry exists to prevent.
    ///
    /// `GeneratorGap`: both renderers already narrow the union correctly and implement
    /// `equals`/ordering/`contains`/`contains_all`/`not_empty`/`is_empty`. The gap is an
    /// unimplemented match arm, not a property of C# or Kotlin.
    DiscriminatedUnionAssertionTypeNotSupported: GeneratorGap => (
        "assertion type ",
        " not yet supported for discriminated union fields",
    ),
}

/// The `skipped:` line for a streaming assertion whose *type* this backend's streaming renderer
/// does not implement.
///
/// ~keep Single-sources the wording [`AssertionTypeSkip::StreamingAssertionTypeNotSupported`]
/// recognises, which `csharp/streaming.rs` already emits by hand. Six backends
/// (`go`/`typescript`/`java`/`kotlin`/`php`/`swift`) instead wrote `// streaming field '<f>':
/// assertion type '<t>' not rendered` and `zig` a third spelling — neither matched any registered
/// shape, and neither carried the `skipped:` prefix, so a grep census missed them too.
///
/// Indentation and comment syntax stay at the call site, matching
/// [`super::field_skip::nested_wildcard_skip_line`]. The returned line carries no trailing
/// newline.
pub(crate) fn streaming_assertion_type_skip_line(
    indent: &str,
    comment_open: &str,
    field: &str,
    assertion_type: &str,
) -> String {
    format!(
        "{indent}{comment_open} skipped: assertion type '{assertion_type}' on field '{field}' \
         not yet supported for streaming"
    )
}

/// The `skipped:` line for a streaming assertion whose type IS implemented but whose fixture value
/// does not fit the shape the renderer narrows to.
///
/// ~keep Distinct from [`streaming_assertion_type_skip_line`] on purpose: conflating them would
/// make a census unable to tell "alef never implemented this assertion type for streaming" from
/// "alef implemented it for `u64` only and this fixture passed a float", which have different
/// fixes. The returned line carries no trailing newline.
pub(crate) fn streaming_assertion_value_skip_line(
    indent: &str,
    comment_open: &str,
    field: &str,
    assertion_type: &str,
) -> String {
    format!(
        "{indent}{comment_open} skipped: assertion type '{assertion_type}' has no renderable value \
         for streaming field '{field}'"
    )
}

impl AssertionTypeSkip {
    /// The rendered marker body for `token`, to be written after a backend's own
    /// `<comment-open> skipped: ` prefix. Mirrors `field_skip::FieldSkip::message`. Only correct
    /// for a variant whose `Shape` wraps a single quoted token (`before` + `'token'` + `after`,
    /// with no quote characters embedded in either half) — the multi-token variants above
    /// (`UnsupportedTraversalAssertion` and friends) keep rendering by hand at their call sites.
    pub(crate) fn message(self, token: &str) -> String {
        let Shape { before, after } = self.shape();
        format!("{before}'{token}'{after}")
    }

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
    use super::{AssertionTypeSkip, streaming_assertion_type_skip_line, streaming_assertion_value_skip_line};
    use crate::e2e::codegen::field_skip::SkipClass;

    /// The load-bearing invariant for both helpers: the line they render is recognised as the
    /// variant they claim, not merely as "some registered shape" and not merely as text a grep
    /// would find. A helper that emitted an unregistered wording would still look right in a
    /// generated file — that is the exact bug these replace. ~keep
    #[test]
    fn the_streaming_type_helper_renders_the_registered_variant() {
        let line = streaming_assertion_type_skip_line("        ", "//", "chunks", "matches_regex");
        assert_eq!(
            AssertionTypeSkip::extract_classified(&line),
            Some(("matches_regex", AssertionTypeSkip::StreamingAssertionTypeNotSupported)),
            "got: {line}"
        );
        assert!(line.contains("// skipped: "), "the census prefix must survive: {line}");
    }

    #[test]
    fn the_streaming_value_helper_renders_the_registered_variant() {
        let line = streaming_assertion_value_skip_line("    ", "#", "chunks", "count_min");
        assert_eq!(
            AssertionTypeSkip::extract_classified(&line),
            Some(("count_min", AssertionTypeSkip::StreamingAssertionValueNotRenderable)),
            "got: {line}"
        );
        assert!(line.contains("# skipped: "), "the census prefix must survive: {line}");
    }

    /// The two `discriminated.rs` renderers build their line by hand rather than through a
    /// helper, so nothing but this test couples their wording to the registry. Both literals are
    /// copied from the call sites (`csharp/discriminated.rs`, `kotlin/discriminated.rs`, the
    /// latter also serving `kotlin_android`); if either is reworded, this fails instead of the
    /// strict gate silently going blind to it again. ~keep
    #[test]
    fn the_discriminated_union_wording_is_recognised_at_both_call_sites() {
        for (indent, comment_open) in [("            ", "//"), ("                ", "//")] {
            let line = format!(
                "{indent}{comment_open} skipped: assertion type 'matches_regex' \
                 not yet supported for discriminated union fields"
            );
            let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
            assert_eq!(
                AssertionTypeSkip::extract_classified(&line),
                Some((
                    "matches_regex",
                    AssertionTypeSkip::DiscriminatedUnionAssertionTypeNotSupported
                )),
                "got: {line}"
            );
        }
    }

    /// The two helpers must stay distinguishable: each line may only match its own variant, or the
    /// axis collapses back into one undifferentiated bucket. ~keep
    #[test]
    fn the_two_streaming_helpers_do_not_match_each_other() {
        let type_line = streaming_assertion_type_skip_line("    ", "//", "chunks", "matches_regex");
        let value_line = streaming_assertion_value_skip_line("    ", "//", "chunks", "count_min");
        assert_ne!(
            AssertionTypeSkip::extract_classified(&type_line).map(|(_, variant)| variant),
            AssertionTypeSkip::extract_classified(&value_line).map(|(_, variant)| variant)
        );
    }

    /// Neither helper's line may be picked up by the *field* funnel — the two axes stay disjoint,
    /// and every backend body these are emitted into is already scanned by that funnel. ~keep
    #[test]
    fn neither_streaming_helper_is_recognised_by_the_field_funnel() {
        use crate::e2e::codegen::field_skip::FieldSkip;
        let type_line = streaming_assertion_type_skip_line("    ", "//", "chunks", "matches_regex");
        let value_line = streaming_assertion_value_skip_line("    ", "//", "chunks", "count_min");
        assert_eq!(FieldSkip::extract(&type_line), None, "got: {type_line}");
        assert_eq!(FieldSkip::extract(&value_line), None, "got: {value_line}");
    }

    /// Every variant's rendered wording must round-trip, mirroring
    /// `field_skip::tests::every_variant_round_trips_through_extract`. Two of the shapes end
    /// mid-quote (`" on field '"`, `" has no renderable value for streaming field "`), so the
    /// probe supplies the trailing token those need rather than assuming a one-token wording. ~keep
    #[test]
    fn every_variant_round_trips_through_extract() {
        for variant in AssertionTypeSkip::ALL {
            let shape = variant.shape();
            let tail = if shape.after.ends_with('\'') {
                "chunks'"
            } else {
                "'chunks'"
            };
            let rendered = format!("    // skipped: {}'count_min'{}{tail}", shape.before, shape.after);
            assert_eq!(
                AssertionTypeSkip::extract(&rendered),
                Some("count_min"),
                "variant {variant:?} rendered `{rendered}` but the gate did not recognise it"
            );
        }
    }

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
    fn declared_error_variant_wording_round_trips_through_message() {
        let rendered = AssertionTypeSkip::DeclaredErrorVariantNotSubstantiated.message("Authentication");
        assert_eq!(
            rendered,
            "declared error variant 'Authentication' not substantiated by this backend's generated error type"
        );
        let line = format!("    // skipped: {rendered}");
        assert_eq!(
            AssertionTypeSkip::extract_classified(&line),
            Some((
                "Authentication",
                AssertionTypeSkip::DeclaredErrorVariantNotSubstantiated
            )),
            "got: {line}"
        );
    }

    #[test]
    fn declared_error_variant_generator_gap_wording_round_trips_through_message() {
        let rendered = AssertionTypeSkip::DeclaredErrorVariantNotYetPreservedByGenerator.message("BadRequest");
        assert_eq!(
            rendered,
            "declared error variant 'BadRequest' not yet preserved as a distinct identity by this backend's \
             generator"
        );
        let line = format!("    # skipped: {rendered}");
        assert_eq!(
            AssertionTypeSkip::extract_classified(&line),
            Some((
                "BadRequest",
                AssertionTypeSkip::DeclaredErrorVariantNotYetPreservedByGenerator
            )),
            "got: {line}"
        );
    }

    /// The two declared-error-variant wordings must stay distinguishable, mirroring
    /// `the_two_streaming_helpers_do_not_match_each_other`.
    #[test]
    fn the_two_declared_error_variant_wordings_do_not_match_each_other() {
        let limitation = AssertionTypeSkip::DeclaredErrorVariantNotSubstantiated.message("X");
        let gap = AssertionTypeSkip::DeclaredErrorVariantNotYetPreservedByGenerator.message("X");
        assert_ne!(
            AssertionTypeSkip::extract_classified(&format!("// skipped: {limitation}")).map(|(_, v)| v),
            AssertionTypeSkip::extract_classified(&format!("// skipped: {gap}")).map(|(_, v)| v),
        );
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
