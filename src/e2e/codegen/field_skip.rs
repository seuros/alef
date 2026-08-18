//! The one funnel for "this assertion's field cannot be asserted here" skip markers.
//!
//! ~keep Every backend used to `writeln!` its own prose for a dropped field assertion and
//! [`super::fail_on_unavailable_field_markers`] used to recognise that prose with two hand-written
//! substring patterns. Backends that invented other wording (dart/swift's tagged-union boundary,
//! ruby's serialized-enum accessor, every `result_is_simple` branch, ...) were therefore emitted
//! but never counted, so arming `ALEF_E2E_STRICT_FIELD_AVAILABILITY` examined a fraction of the
//! skips it appeared to cover — a pass was indistinguishable from health.
//!
//! [`FieldSkip`] closes that by construction: the same per-variant [`Shape`] both renders the
//! human-readable message and recognises it, and `ALL` is generated from the same macro arm as
//! the variant list, so a variant cannot exist without the strict gate counting it. Adding a
//! backend wording means adding a variant here, which automatically extends the gate.
//!
//! Each variant keeps its backend's exact original wording — the reason text is useful to
//! consumers and is deliberately *not* unified. Only the recognition path is shared, and the
//! reason prose sits entirely outside it.
//!
//! The comment syntax and indentation stay at the call site (`// `, `# `, `/* */`), so a rendered
//! line is `<indent><comment-open> skipped: <FieldSkip::message(field)>`.

/// The rendered text on either side of the quoted field name for one registered wording.
struct Shape {
    before: &'static str,
    after: &'static str,
}

/// Why a field assertion was dropped, and therefore whether dropping it is defensible.
///
/// ~keep The axis is *not* "which backend emitted it" but "could an edit to the fixture or the
/// alef config have made this assertion run?".
///
/// [`SkipClass::LanguageLimitation`] names a property of the target language, ABI or declared
/// call shape. No fixture edit makes the assertion expressible, so rendering a comment is the
/// only honest option and the fixture author is not at fault.
///
/// [`SkipClass::AuthoringGap`] names a *lookup that failed*: a field path that did not resolve
/// against a result type or virtual-field set which is itself derived from the same IR the
/// fixture is written against. That is drift between fixture and config, it is fixable, and
/// rendering a green test for it is the defect this module exists to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipClass {
    /// A resolution failure a fixture or config edit can fix. Fatal unless explicitly opted out.
    AuthoringGap,
    /// ~keep alef itself cannot express this assertion yet — a missing generator feature, not a
    /// mistake in the fixture. Never fatal: a consumer cannot fix it from their own `alef.toml`,
    /// so failing their build on it would only force a blanket opt-out and rebuild the silent
    /// skip with extra steps. Counted in its own bucket instead, because this debt is alef's.
    GeneratorGap,
    /// A real property of the language, ABI or declared call shape. Counted, never fatal.
    LanguageLimitation,
}

macro_rules! field_skip_variants {
    ($($(#[$meta:meta])* $variant:ident : $class:ident => ($before:expr, $after:expr $(,)?)),+ $(,)?) => {
        /// A registered reason a field assertion was dropped from generated e2e code.
        ///
        /// Out of scope by design: skips whose cause is the *assertion type* rather than the
        /// field (`unsupported assertion type on synthetic field '<name>'`, `unsupported
        /// traversal assertion ...`, `'<name>' assertion missing value`). Those are a different
        /// defect — a bad assertion shape, not an unreachable field — and must not be conflated
        /// with this one. ~keep
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(crate) enum FieldSkip {
            $($(#[$meta])* $variant,)+
        }

        impl FieldSkip {
            /// Every variant. Generated from the same macro arm as the variant list so the
            /// recognition set can never fall behind the render set. ~keep
            const ALL: &'static [Self] = &[$(Self::$variant,)+];

            const fn shape(self) -> Shape {
                match self {
                    $(Self::$variant => Shape { before: $before, after: $after },)+
                }
            }

            /// Whether dropping this assertion is a defensible language limit or a fixable gap.
            ///
            /// Read from the same macro arm as the wording, so a new variant cannot be added
            /// without its author deciding — in the same edit — which side of the line it sits
            /// on. There is no default. ~keep
            pub(crate) const fn class(self) -> SkipClass {
                match self {
                    $(Self::$variant => SkipClass::$class,)+
                }
            }
        }
    };
}

field_skip_variants! {
    /// ~keep The canonical gap: `FieldResolver::is_valid_for_result` did not find the path. The
    /// resolver is fed from the same IR the binding types are generated from, so a miss means the
    /// fixture names a field that no longer exists (or never did), not that the language is
    /// incapable of asserting it.
    NotAvailableOnResultType: AuthoringGap => ("field ", " not available on result type"),
    /// ~keep zig's JSON-struct return shape, where the result is parsed JSON rather than a binding
    /// type. The struct is generated from the IR, so an unresolved path is drift, not a zig limit —
    /// parsed JSON can express any shape the fixture could name.
    NotAvailableOnJsonStructResult: AuthoringGap => ("field ", " not available on the JSON-struct result"),
    /// ~keep The call declares `result_is_simple`, so the binding returns a bare scalar and a dotted
    /// path has nowhere to live. A property of the declared call shape, not a fixture typo.
    NotAvailableWhenResultIsSimple: LanguageLimitation => ("field ", " not available when result_is_simple"),
    /// ~keep The C ABI flattens to primitives and opaque handles; deeply nested and richly typed
    /// fields are genuinely unreachable across it. No fixture edit changes that.
    NotAvailableInCFfi: LanguageLimitation => ("field ", " not available in C FFI"),
    NotAvailableOnGoProcessingResult: AuthoringGap => ("field ", " not available on Go ProcessingResult"),
    NotAvailableOnPythonProcessingResult: AuthoringGap => ("field ", " not available on Python ProcessingResult"),
    NotAvailableOnRubyProcessingResult: AuthoringGap => ("field ", " not available on Ruby ProcessingResult"),
    NotAvailableOnRProcessingResult: AuthoringGap => ("field ", " not available on R ProcessingResult"),
    NotAvailableOnNodeProcessingResult: AuthoringGap => ("field ", " not available on Node JsProcessingResult"),
    /// ~keep csharp builds its reason into a `skipped_reason` context variable that
    /// `templates/csharp/assertion.jinja` prefixes with `skipped: `, so this wording never appears
    /// on a source line next to the word `skipped:` — grepping for the marker text misses it.
    NotAvailableOnGeneratedCsharpResultType: AuthoringGap => (
        "field ",
        " not available on the generated C# result type",
    ),
    NotAvailableOnDartResultType: AuthoringGap => ("field ", " not available on dart result type"),
    NotAvailableOnElixirResultType: AuthoringGap => ("field ", " not available on Elixir result type"),
    /// ~keep A streaming call yields an event sequence, not a struct, so no field path can name a
    /// property of it. This is a missing generator feature (a first-class stream assertion), not a
    /// fixture mistake — see the `StreamingAssertionOnUnsupportedField` note.
    NotAvailableOnStreamingResultType: GeneratorGap => ("field ", " not available on streaming result type"),
    /// ~keep Simple-result sibling of `NotAvailableWhenResultIsSimple`; same declared-call-shape
    /// reason, different backend's wording.
    NotApplicableForSimpleResultType: LanguageLimitation => ("field ", " not applicable for simple result type"),
    NotAccessibleOnSimpleResultType: LanguageLimitation => ("field ", " not accessible on simple result type"),
    /// ~keep Despite the `result_is_simple` prefix this is the *resolver* rejecting the path, not
    /// the simple-result shape rejecting it — the wording pairs `result_is_simple for field` with
    /// `not available on result type`, which is the `NotAvailableOnResultType` oracle talking.
    ResultIsSimpleForFieldNotAvailable: AuthoringGap => (
        "result_is_simple for field ",
        " not available on result type",
    ),
    /// ~keep Reaching the field would require narrowing a tagged union to one variant first, which
    /// neither language's type system lets generated straight-line assertion code do.
    CrossesTaggedUnionBoundaryInDart: LanguageLimitation => (
        "field ",
        " crosses a tagged-union variant boundary (not expressible in Dart)",
    ),
    CrossesTaggedUnionBoundaryInSwift: LanguageLimitation => (
        "field ",
        " crosses a tagged-union variant boundary (not expressible in Swift)",
    ),
    /// ~keep The field or its type is excluded from the binding by explicit config. The exclusion
    /// is already a visible, deliberate decision, so the skip is its honest consequence.
    ExcludedFromSwiftBinding: LanguageLimitation => (
        "field ",
        " references a field or type excluded from the Swift binding",
    ),
    /// ~keep Was `NestedArrayWildcardNotSupportedInZig` (" not supported in zig"), emitted by the
    /// one backend that had ever guarded the case. Every other backend now refuses it too, so the
    /// wording is language-neutral and single-sourced through `nested_wildcard_skip_line`. The
    /// old text is a prefix of the new one, so anything grepping for it still matches.
    NestedArrayWildcardNotSupported: LanguageLimitation => (
        "nested array-wildcard field ",
        " not supported",
    ),
    ArrayElementNotSupportedInGleam: LanguageLimitation => (
        "array element field ",
        " not yet supported in Gleam e2e",
    ),
    /// ~keep Ruby serializes enums to a Hash, so there is no variant accessor to call. A property
    /// of the Ruby binding's representation choice.
    EnumVariantAccessorNotAvailableInRuby: LanguageLimitation => (
        "enum variant accessor ",
        " not available on Ruby (serialized to Hash)",
    ),
    /// ~keep Reworded from `metadata.format enum field serialization differs in Ruby`, which named
    /// no quoted field and so was structurally uncountable — the strict gate could never have seen
    /// it whatever patterns it matched. The reason is unchanged; only the field is now named.
    EnumSerializationDiffersInRuby: LanguageLimitation => ("field ", " enum serialization differs in Ruby"),
    /// ~keep The python streaming adapter has no accessor for this virtual field. Same missing
    /// feature as `StreamingAssertionOnUnsupportedField`, seen from the python side.
    NoPythonStreamingAccessor: GeneratorGap => ("streaming field ", ": no python accessor"),
    /// ~keep This is the variant that hid whole expected-event-sequence assertions — a fixture
    /// asserting a traversal order rendered a comment and the test still passed.
    ///
    /// It is nonetheless a `GeneratorGap`, not an `AuthoringGap`: a streaming call returns an
    /// event sequence rather than a struct, so *no* field mapping can ever express an assertion
    /// over that sequence. Consumers cannot fix these from their own config — the fix is a
    /// first-class streaming assertion type in alef. Failing their build on it would force a
    /// blanket opt-out, which is the silent skip again with more ceremony. Counted loudly instead,
    /// in a bucket that names alef as the owner.
    StreamingAssertionOnUnsupportedField: GeneratorGap => ("streaming assertion on unsupported field ", ""),
    /// Emitted by `templates/{java,php}/synthetic_assertion.jinja`, which cannot call into Rust;
    /// registered here so the strict gate still counts it. Declared-call-shape reason. ~keep
    ResultIsSimpleNotOnSimpleResultType: LanguageLimitation => (
        "result_is_simple, field ",
        " not on simple result type",
    ),
    /// Emitted by `templates/java/synthetic_assertion.jinja`. ~keep
    NotAvailableOnJavaResultType: AuthoringGap => ("field ", " not available on Java result type"),
    /// Emitted by `templates/php/synthetic_assertion.jinja`. ~keep
    NotAvailableOnPhpResultType: AuthoringGap => ("field ", " not available on PHP result type"),
    /// Emitted by `templates/r/synthetic_assertion.jinja`. ~keep
    NotAvailableOnRResultType: AuthoringGap => ("field ", " not available on R result type"),
}

impl FieldSkip {
    /// The human-readable marker body for `field`, to be written after a backend's own
    /// `<comment-open> skipped: ` prefix.
    pub(crate) fn message(self, field: &str) -> String {
        let Shape { before, after } = self.shape();
        format!("{before}'{field}'{after}")
    }

    /// The field name a single rendered line names, if the line carries any registered wording.
    ///
    /// Production code uses [`Self::extract_classified`]; this narrower form exists so the
    /// recognition tests below can assert *what* is matched without also restating every
    /// variant's classification. ~keep
    #[cfg(test)]
    pub(crate) fn extract(line: &str) -> Option<&str> {
        Self::extract_classified(line).map(|(field, _)| field)
    }

    /// The field name *and* the variant that named it, so a caller can tell an authoring gap
    /// from a language limitation without re-matching the wording itself.
    pub(crate) fn extract_classified(line: &str) -> Option<(&str, Self)> {
        Self::ALL
            .iter()
            .find_map(|variant| variant.field_in(line).map(|field| (field, *variant)))
    }

    /// ~keep Every occurrence of `before` is tried, not just the first: `before` is often the bare
    /// `"field "`, which also occurs inside longer phrases ("synthetic field ", "for field "), so
    /// stopping at the first hit would miss a line whose earlier `field ` is not the quoted one.
    fn field_in(self, line: &str) -> Option<&str> {
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
}

/// The `skipped:` line refusing a doubly-nested bracket-wildcard path, or `None` when
/// `element_sub_path` carries no second wildcard.
///
/// `FieldResolver::wildcard_split` consumes the FIRST `[].` only, so for `pages[].links[].url`
/// it hands back the element sub-path `links[].url`. Every backend then builds its per-element
/// accessor from that sub-path — and `parse_path` lowers the surviving `[]` to index 0, so the
/// emitted loop covers `pages` while the check inside it silently reads `links[0]`. A fixture
/// claiming "every element" then passes on element zero alone, in a language whose output is
/// green.
///
/// Refusing is the deliberate answer, not a placeholder: a nested quantifier is expressible in
/// most of these languages but not all (Go emits statements rather than an expression, R goes
/// through `vapply`, Dart's element accessor renders the wildcard as the syntactically invalid
/// `links![]`), so emitting it would land unevenly — and an index-0 fallback in the backends
/// that could not keep up is exactly the false green this function exists to remove. `zig` had
/// already made that call for itself; this is the same call for everyone, single-sourced so the
/// predicate and the wording cannot drift apart between backends.
///
/// The returned line carries no trailing newline: callers `writeln!` it, or return it where a
/// rendered line is expected. Indentation and comment syntax stay at the call site, matching the
/// rest of this module. ~keep
pub(crate) fn nested_wildcard_skip_line(
    indent: &str,
    comment_open: &str,
    field: &str,
    element_sub_path: &str,
) -> Option<String> {
    if !element_sub_path.contains("[]") {
        return None;
    }
    Some(format!(
        "{indent}{comment_open} skipped: {}",
        FieldSkip::NestedArrayWildcardNotSupported.message(field)
    ))
}

#[cfg(test)]
mod tests {
    use super::{FieldSkip, SkipClass, nested_wildcard_skip_line};

    /// ~keep The classification is what decides whose build breaks, so it is pinned here rather
    /// than left implicit in the variant list. These four are the load-bearing calls:
    ///
    /// - `NotAvailableOnResultType` is the canonical fixable gap and must stay fatal, or this
    ///   whole change is inert.
    /// - the streaming wordings must NOT be fatal: a stream is an event sequence, so no field
    ///   mapping can express an assertion over it and a consumer cannot fix it from their config.
    ///   Making them fatal would force a blanket opt-out and rebuild the silent skip.
    /// - a tagged-union boundary is a real type-system limit and was never anyone's mistake.
    #[test]
    fn the_load_bearing_classifications_are_pinned() {
        assert_eq!(FieldSkip::NotAvailableOnResultType.class(), SkipClass::AuthoringGap);
        assert_eq!(
            FieldSkip::StreamingAssertionOnUnsupportedField.class(),
            SkipClass::GeneratorGap
        );
        assert_eq!(FieldSkip::NoPythonStreamingAccessor.class(), SkipClass::GeneratorGap);
        assert_eq!(
            FieldSkip::NotAvailableOnStreamingResultType.class(),
            SkipClass::GeneratorGap
        );
        assert_eq!(
            FieldSkip::CrossesTaggedUnionBoundaryInDart.class(),
            SkipClass::LanguageLimitation
        );
        assert_eq!(FieldSkip::NotAvailableInCFfi.class(), SkipClass::LanguageLimitation);
    }

    /// Every class must actually be populated: a table where one arm went empty would silently
    /// collapse the three-way distinction back into a two-way one.
    #[test]
    fn every_class_is_represented() {
        for class in [
            SkipClass::AuthoringGap,
            SkipClass::GeneratorGap,
            SkipClass::LanguageLimitation,
        ] {
            assert!(
                FieldSkip::ALL.iter().any(|variant| variant.class() == class),
                "no variant is classified {class:?}"
            );
        }
    }

    /// The load-bearing invariant: render and recognition read the same `Shape`, so anything a
    /// backend can emit through the funnel is by construction something the strict gate counts.
    #[test]
    fn every_variant_round_trips_through_extract() {
        for variant in FieldSkip::ALL {
            let rendered = format!("    // skipped: {}", variant.message("metadata.format.excel"));
            assert_eq!(
                FieldSkip::extract(&rendered),
                Some("metadata.format.excel"),
                "variant {variant:?} rendered `{rendered}` but the gate did not recognise it"
            );
        }
    }

    #[test]
    fn tagged_union_boundary_wordings_are_recognised() {
        let dart = "    // skipped: field 'tags' crosses a tagged-union variant boundary (not expressible in Dart)";
        assert_eq!(FieldSkip::extract(dart), Some("tags"));
        let swift = "    // skipped: field 'tags' crosses a tagged-union variant boundary (not expressible in Swift)";
        assert_eq!(FieldSkip::extract(swift), Some("tags"));
    }

    #[test]
    fn ruby_serialized_enum_accessor_wording_is_recognised() {
        let line = "    # skipped: enum variant accessor 'format.excel' not available on Ruby (serialized to Hash)";
        assert_eq!(FieldSkip::extract(line), Some("format.excel"));
    }

    #[test]
    fn result_is_simple_template_wording_is_recognised() {
        let line = "        // skipped: result_is_simple, field 'metadata.title' not on simple result type";
        assert_eq!(FieldSkip::extract(line), Some("metadata.title"));
    }

    /// Negative control: an unsupported *assertion type* is a different defect and stays uncounted,
    /// even though the line contains `field '<name>'`.
    #[test]
    fn unsupported_assertion_type_wordings_stay_uncounted() {
        let synthetic = "\t// skipped: unsupported assertion type on synthetic field 'embeddings'";
        assert_eq!(FieldSkip::extract(synthetic), None);
        let traversal = "    // skipped: unsupported traversal assertion 'equals' on 'pages[].url'";
        assert_eq!(FieldSkip::extract(traversal), None);
        let streaming = "    // skipped: assertion type 'count_min' on field 'chunks' not yet supported for streaming";
        assert_eq!(FieldSkip::extract(streaming), None);
        let scalar = "        // skipped: field 'content' is a scalar String without meaningful .count";
        assert_eq!(FieldSkip::extract(scalar), None);
    }

    #[test]
    fn a_line_with_no_marker_is_not_recognised() {
        assert_eq!(FieldSkip::extract("    assert result.count == 1"), None);
        assert_eq!(FieldSkip::extract("        // skipped: field is a scalar String"), None);
    }

    /// An earlier `field ` inside a longer phrase must not shift which quote pair is read.
    #[test]
    fn extracts_the_quoted_name_not_a_later_phrase() {
        let line = "  # skipped: result_is_simple for field 'metadata' not available on result type";
        assert_eq!(FieldSkip::extract(line), Some("metadata"));
    }

    #[test]
    fn should_refuse_when_the_element_sub_path_still_carries_a_wildcard() {
        assert_eq!(
            nested_wildcard_skip_line("    ", "//", "pages[].links[].url", "links[].url").as_deref(),
            Some("    // skipped: nested array-wildcard field 'pages[].links[].url' not supported")
        );
    }

    /// A single wildcard leaves nothing behind on the element side, and that is the case every
    /// backend must keep quantifying over — refusing it would delete working coverage. ~keep
    #[test]
    fn should_not_refuse_a_single_wildcard() {
        assert_eq!(nested_wildcard_skip_line("    ", "#", "links[].url", "url"), None);
    }

    /// An explicit numeric index is a different, correct feature: `pages[].links[0].url` names
    /// element zero because the fixture author asked for it. ~keep
    #[test]
    fn should_not_refuse_an_explicit_inner_index() {
        assert_eq!(
            nested_wildcard_skip_line("    ", "#", "pages[].links[0].url", "links[0].url"),
            None
        );
    }

    /// The refusal must round-trip through the strict field-availability gate, or the honest
    /// gap is invisible to the very tooling that counts gaps. ~keep
    #[test]
    fn the_nested_wildcard_refusal_is_counted_by_the_strict_gate() {
        let line = nested_wildcard_skip_line("  ", "#", "pages[].links[].url", "links[].url").unwrap();
        assert_eq!(FieldSkip::extract(&line), Some("pages[].links[].url"));
    }
}
