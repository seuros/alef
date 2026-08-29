//! Which Ruby shape `backends::magnus` lowers each IR enum to, and therefore whether a fixture
//! field path that steps *into* an enum's variant payload names anything the Ruby binding
//! actually exposes.
//!
//! ~keep This replaces two literal guards (`f.contains("metadata.format.")` and
//! `f == "metadata.format"`) that carried the reason "Magnus serializes FormatMetadata to JSON,
//! so variants are unavailable in Ruby". The path was one consumer crate's own field name
//! hard-coded into a project-agnostic generator: an identically shaped hash-serialized enum
//! under any other field name was not recognised at all (rendering a property chain against a
//! Ruby `Hash`, e.g. `NoMethodError`), while a field that merely happened to share the literal
//! name `metadata.format` was skipped whatever it actually resolved to.
//!
//! The real condition is the enum's Ruby *lowering* — a property of the enum's own IR shape
//! (`backends::magnus::gen_bindings::classes::gen_enum::gen_enum`'s `has_data` check) — resolved
//! by walking the fixture path through the crate's own IR, exactly as `php/enum_variant_access.rs`
//! does for PHP's three-way split. Magnus's lowering is simpler than PHP's: a data-carrying enum
//! (any variant with at least one field, tagged or not) always serializes through
//! `serde_json::to_value` into a plain Ruby `Hash` (see `enum_magnus.rs.jinja`'s `IntoValue` impl
//! for the `has_data` branch); a unit-variant-only enum always becomes a `Symbol`. There is no
//! third, member-ful shape the way PHP's flat `#[php_class]` is.

use std::collections::HashSet;

use crate::core::ir::EnumDef;
use crate::e2e::field_access::FieldResolver;

/// Names of every enum this crate declares that Magnus lowers to a plain Ruby `Hash` rather than
/// a `Symbol` — restated from `gen_enum`'s own `has_data` predicate
/// (`enum_def.variants.iter().any(|v| !v.fields.is_empty())`), which that module cannot export
/// because `backends::magnus::gen_bindings` is private to `backends::magnus`.
/// `should_match_the_binding_backends_partition` below pins the two agree.
pub(super) fn hash_serialized_enum_names(enums: &[EnumDef]) -> HashSet<String> {
    enums
        .iter()
        .filter(|enum_def| enum_def.variants.iter().any(|variant| !variant.fields.is_empty()))
        .map(|enum_def| enum_def.name.clone())
        .collect()
}

/// What the Ruby binding offers for a fixture field path that may cross a hash-serialized enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RubyEnumAccess {
    /// Nothing to refuse: the path crosses no enum, or every enum on it is a `Symbol`.
    Available,
    /// The path lands exactly on a hash-serialized enum. A real `Hash` value exists, but the
    /// serialization differs between languages and doesn't preserve `Display` formatting, so a
    /// plain string/equality comparison against it is not meaningful.
    SerializedAsHash,
    /// The path steps PAST a hash-serialized enum into a variant payload segment. No native
    /// accessor exists on the Ruby side for that segment; the whole subtree is a `Hash`.
    VariantAccessorUnavailable,
}

/// Classify `field` by walking each of its leading segments and asking the IR (via
/// `field_resolver.ruby_enum_serialized_as_hash`) whether that prefix resolves to a
/// hash-serialized enum. Entirely name-agnostic: it fires for any field path the crate's own IR
/// confirms crosses a hash-serialized enum, under whatever name that field happens to have, and
/// it does NOT fire for a field that merely shares a literal name with one but resolves to
/// something else (a plain scalar, a unit-variant/`Symbol` enum, or nothing the IR recognizes).
pub(super) fn classify(field_resolver: &FieldResolver, field: &str) -> RubyEnumAccess {
    if field_resolver.ruby_enum_serialized_as_hash(field) == Some(true) {
        return RubyEnumAccess::SerializedAsHash;
    }
    let segments: Vec<&str> = field.split('.').collect();
    for i in 1..segments.len() {
        let prefix = segments[..i].join(".");
        if field_resolver.ruby_enum_serialized_as_hash(&prefix) == Some(true) {
            return RubyEnumAccess::VariantAccessorUnavailable;
        }
    }
    RubyEnumAccess::Available
}

#[cfg(test)]
mod tests {
    use super::{RubyEnumAccess, classify, hash_serialized_enum_names};
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
    use crate::e2e::field_access::FieldResolver;

    fn field(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            ..FieldDef::default()
        }
    }

    fn named(name: &str) -> TypeRef {
        TypeRef::Named(name.to_string())
    }

    /// A neutral fixture crate carrying the two Ruby enum lowerings:
    ///
    /// * `EncodingDetails` — a data-carrying (internally tagged) enum, lowered to a `Hash`.
    /// * `DocumentKind` — unit variants only, lowered to a `Symbol`.
    ///
    /// `metadata` deliberately carries a plain `String` field named `format` — the exact literal
    /// spelling the old guard matched on — so a test can prove that name alone no longer triggers
    /// the skip.
    fn ir() -> (Vec<TypeDef>, Vec<EnumDef>) {
        let type_defs = vec![
            TypeDef {
                name: "ProcessingResult".to_string(),
                fields: vec![
                    field("summary", named("DocumentSummary")),
                    field("metadata", named("PageMetadata")),
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "DocumentSummary".to_string(),
                fields: vec![
                    field("encoding", named("EncodingDetails")),
                    field("kind", named("DocumentKind")),
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "PageMetadata".to_string(),
                fields: vec![field("format", TypeRef::String)],
                ..TypeDef::default()
            },
        ];
        let enums = vec![
            EnumDef {
                name: "EncodingDetails".to_string(),
                serde_tag: Some("type".to_string()),
                variants: vec![EnumVariant {
                    name: "Spreadsheet".to_string(),
                    is_tuple: true,
                    fields: vec![field("_0", TypeRef::Primitive(PrimitiveType::U32))],
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            },
            EnumDef {
                name: "DocumentKind".to_string(),
                variants: vec![EnumVariant {
                    name: "Report".to_string(),
                    ..EnumVariant::default()
                }],
                ..EnumDef::default()
            },
        ];
        (type_defs, enums)
    }

    fn resolver() -> FieldResolver {
        let (type_defs, enums) = ir();
        let map = FieldResolver::ir_enum_fields(&type_defs, &enums);
        FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        )
        .with_ir_enum_map(map, Some("ProcessingResult".to_string()))
        .with_ruby_hash_serialized_enum_names(hash_serialized_enum_names(&enums))
    }

    /// THE CANARY (positive control). A field path that crosses a hash-serialized enum, spelled
    /// under a name that is nothing like the old literal `metadata.format`, must still refuse —
    /// proving the condition is the enum's own IR-derived lowering, not the field's name.
    #[test]
    fn a_variant_path_through_a_hash_serialized_enum_is_refused_under_any_name() {
        let out = classify(&resolver(), "summary.encoding.sheet_count");
        assert_eq!(out, RubyEnumAccess::VariantAccessorUnavailable);
    }

    /// Same enum, path stops exactly on it: a `Hash` value does exist, but comparisons against
    /// it as a string/Display value are not meaningful.
    #[test]
    fn a_path_landing_exactly_on_a_hash_serialized_enum_is_flagged_as_hash() {
        let out = classify(&resolver(), "summary.encoding");
        assert_eq!(out, RubyEnumAccess::SerializedAsHash);
    }

    /// THE NEGATIVE CONTROL. `metadata.format` is the exact literal the old guard matched on —
    /// here it resolves through the IR to a plain `String`, not an enum at all, so it must NOT be
    /// refused. This is what tells a real condition apart from a renamed string match.
    #[test]
    fn a_field_that_merely_shares_the_old_literal_name_is_not_refused() {
        let out = classify(&resolver(), "metadata.format");
        assert_eq!(out, RubyEnumAccess::Available);
    }

    /// A `Symbol`-lowered (unit-variant-only) enum has no `Hash` shape to worry about, whatever
    /// the field is named.
    #[test]
    fn a_symbol_lowered_enum_field_is_available() {
        let out = classify(&resolver(), "summary.kind");
        assert_eq!(out, RubyEnumAccess::Available);
    }

    /// An unresolved resolver (no IR wired in at all — the state every resolver had before
    /// `with_ruby_hash_serialized_enum_names` existed) must never refuse: absence of IR data is
    /// "unknown", not "hash-serialized".
    #[test]
    fn with_no_ir_wired_in_nothing_is_ever_refused() {
        let resolver = FieldResolver::new(
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
            &std::collections::HashSet::new(),
        );
        assert_eq!(
            classify(&resolver, "summary.encoding.sheet_count"),
            RubyEnumAccess::Available
        );
        assert_eq!(classify(&resolver, "metadata.format"), RubyEnumAccess::Available);
    }

    /// The partition must reproduce the binding backend's `has_data` split, since it decides both
    /// this skip and the Rust-side `IntoValue` shape.
    #[test]
    fn should_match_the_binding_backends_partition() {
        let (_, enums) = ir();
        let names = hash_serialized_enum_names(&enums);
        assert!(
            names.contains("EncodingDetails"),
            "a data-carrying enum lowers to a Hash"
        );
        assert!(
            !names.contains("DocumentKind"),
            "a unit-variant-only enum lowers to a Symbol"
        );
    }

    fn render(field: &str) -> String {
        let assertion = crate::e2e::fixture::Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::json!("Excel")),
            ..Default::default()
        };
        let mut out = String::new();
        super::super::assertions::render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver(),
            false,
            &crate::e2e::config::E2eConfig::default(),
            &std::collections::HashSet::new(),
            &std::collections::HashMap::new(),
        );
        out
    }

    /// End to end through `render_assertion` itself (not just `classify`), under a field name
    /// that is nothing like the old literal `metadata.format` — proving the real entry point, not
    /// just the classifier in isolation, is name-agnostic.
    #[test]
    fn render_assertion_emits_the_variant_accessor_skip_for_a_neutral_field_name() {
        let out = render("summary.encoding.sheet_count");
        assert!(out.contains("# skipped:"), "got: {out}");
        assert!(
            out.contains(
                "enum variant accessor 'summary.encoding.sheet_count' not available on Ruby (serialized to Hash)"
            ),
            "got: {out}"
        );
    }

    /// The skip must classify as a `GeneratorGap`, not a `LanguageLimitation` — see
    /// `field_skip::FieldSkip::EnumVariantAccessorNotAvailableInRuby`'s doc for why the
    /// already-generated Rust-side accessor is dead code today rather than a real boundary Ruby
    /// or Magnus forbids crossing.
    #[test]
    fn render_assertion_skip_is_a_generator_gap_not_a_language_limitation() {
        use crate::e2e::codegen::field_skip::{FieldSkip, SkipClass};

        let out = render("summary.encoding.sheet_count");
        assert_eq!(
            FieldSkip::extract_classified(&out).map(|(field, skip)| (field, skip.class())),
            Some(("summary.encoding.sheet_count", SkipClass::GeneratorGap)),
            "got: {out}"
        );
    }
}
