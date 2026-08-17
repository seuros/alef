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

macro_rules! field_skip_variants {
    ($($(#[$meta:meta])* $variant:ident => ($before:expr, $after:expr $(,)?)),+ $(,)?) => {
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
        }
    };
}

field_skip_variants! {
    NotAvailableOnResultType => ("field ", " not available on result type"),
    /// zig's JSON-struct return shape, where the result is parsed JSON rather than a binding type.
    NotAvailableOnJsonStructResult => ("field ", " not available on the JSON-struct result"),
    NotAvailableWhenResultIsSimple => ("field ", " not available when result_is_simple"),
    NotAvailableInCFfi => ("field ", " not available in C FFI"),
    NotAvailableOnGoProcessingResult => ("field ", " not available on Go ProcessingResult"),
    NotAvailableOnPythonProcessingResult => ("field ", " not available on Python ProcessingResult"),
    NotAvailableOnRubyProcessingResult => ("field ", " not available on Ruby ProcessingResult"),
    NotAvailableOnRProcessingResult => ("field ", " not available on R ProcessingResult"),
    NotAvailableOnNodeProcessingResult => ("field ", " not available on Node JsProcessingResult"),
    /// ~keep csharp builds its reason into a `skipped_reason` context variable that
    /// `templates/csharp/assertion.jinja` prefixes with `skipped: `, so this wording never appears
    /// on a source line next to the word `skipped:` — grepping for the marker text misses it.
    NotAvailableOnGeneratedCsharpResultType => ("field ", " not available on the generated C# result type"),
    NotAvailableOnDartResultType => ("field ", " not available on dart result type"),
    NotAvailableOnElixirResultType => ("field ", " not available on Elixir result type"),
    NotAvailableOnStreamingResultType => ("field ", " not available on streaming result type"),
    NotApplicableForSimpleResultType => ("field ", " not applicable for simple result type"),
    NotAccessibleOnSimpleResultType => ("field ", " not accessible on simple result type"),
    ResultIsSimpleForFieldNotAvailable => ("result_is_simple for field ", " not available on result type"),
    CrossesTaggedUnionBoundaryInDart => (
        "field ",
        " crosses a tagged-union variant boundary (not expressible in Dart)",
    ),
    CrossesTaggedUnionBoundaryInSwift => (
        "field ",
        " crosses a tagged-union variant boundary (not expressible in Swift)",
    ),
    ExcludedFromSwiftBinding => ("field ", " references a field or type excluded from the Swift binding"),
    NestedArrayWildcardNotSupportedInZig => ("nested array-wildcard field ", " not supported in zig"),
    ArrayElementNotSupportedInGleam => ("array element field ", " not yet supported in Gleam e2e"),
    EnumVariantAccessorNotAvailableInRuby => (
        "enum variant accessor ",
        " not available on Ruby (serialized to Hash)",
    ),
    /// ~keep Reworded from `metadata.format enum field serialization differs in Ruby`, which named
    /// no quoted field and so was structurally uncountable — the strict gate could never have seen
    /// it whatever patterns it matched. The reason is unchanged; only the field is now named.
    EnumSerializationDiffersInRuby => ("field ", " enum serialization differs in Ruby"),
    NoPythonStreamingAccessor => ("streaming field ", ": no python accessor"),
    StreamingAssertionOnUnsupportedField => ("streaming assertion on unsupported field ", ""),
    /// Emitted by `templates/{java,php}/synthetic_assertion.jinja`, which cannot call into Rust;
    /// registered here so the strict gate still counts it. ~keep
    ResultIsSimpleNotOnSimpleResultType => ("result_is_simple, field ", " not on simple result type"),
    /// Emitted by `templates/java/synthetic_assertion.jinja`. ~keep
    NotAvailableOnJavaResultType => ("field ", " not available on Java result type"),
    /// Emitted by `templates/php/synthetic_assertion.jinja`. ~keep
    NotAvailableOnPhpResultType => ("field ", " not available on PHP result type"),
    /// Emitted by `templates/r/synthetic_assertion.jinja`. ~keep
    NotAvailableOnRResultType => ("field ", " not available on R result type"),
}

impl FieldSkip {
    /// The human-readable marker body for `field`, to be written after a backend's own
    /// `<comment-open> skipped: ` prefix.
    pub(crate) fn message(self, field: &str) -> String {
        let Shape { before, after } = self.shape();
        format!("{before}'{field}'{after}")
    }

    /// The field name a single rendered line names, if the line carries any registered wording.
    pub(crate) fn extract(line: &str) -> Option<&str> {
        Self::ALL.iter().find_map(|variant| variant.field_in(line))
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

#[cfg(test)]
mod tests {
    use super::FieldSkip;

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
}
