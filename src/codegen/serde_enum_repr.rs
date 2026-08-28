//! The single place that answers "how does serde put this enum on the JSON wire?".
//!
//! serde has exactly four enum representations and the choice between them is made by the
//! `tag`/`content`/`untagged` container attributes alone — never by the shape of the variants.
//! Backends that re-derive the answer drift apart from each other and from Rust: the Swift
//! trait-bridge result encoder assumed serde's *external* default for every enum, so an
//! adjacently tagged enum went out as the bare string `"Variant"` and Rust rejected every
//! callback with `invalid type: string "...", expected adjacently tagged enum ...`, while Go
//! (which did consult `serde_content`) stayed correct.
//!
//! Every backend that emits or parses the JSON form of an IR enum must classify it through
//! [`serde_enum_repr`] so a future edit cannot reintroduce that divergence.

use crate::core::ir::EnumDef;

/// serde's four enum representations, carrying the key names each one needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerdeEnumRepr {
    /// serde's default. Unit variants are the bare string `"Variant"`; data variants are
    /// `{"Variant": payload}`.
    External,
    /// `#[serde(tag = "t")]`. The variant name is a field of the payload object:
    /// `{"t":"variant", ...payload fields}`.
    Internal { tag: String },
    /// `#[serde(tag = "t", content = "c")]`. The payload sits under its own key:
    /// `{"t":"variant","c":payload}`, and unit variants are just `{"t":"variant"}`.
    Adjacent { tag: String, content: String },
    /// `#[serde(untagged)]`. The payload alone, with no variant name anywhere; unit variants
    /// are `null`.
    Untagged,
}

impl SerdeEnumRepr {
    /// A stable lower-case discriminator suitable for a template context value.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::External => "external",
            Self::Internal { .. } => "internal",
            Self::Adjacent { .. } => "adjacent",
            Self::Untagged => "untagged",
        }
    }

    /// The wire key holding the variant name, for the representations that have one.
    pub fn tag(&self) -> Option<&str> {
        match self {
            Self::Internal { tag } | Self::Adjacent { tag, .. } => Some(tag),
            Self::External | Self::Untagged => None,
        }
    }

    /// The wire key holding the payload — adjacent tagging only.
    pub fn content(&self) -> Option<&str> {
        match self {
            Self::Adjacent { content, .. } => Some(content),
            Self::External | Self::Internal { .. } | Self::Untagged => None,
        }
    }
}

/// Classify an IR enum's serde JSON representation.
///
/// `serde_untagged` wins over a stray tag because `#[serde(untagged)]` and `#[serde(tag = ...)]`
/// are mutually exclusive in serde, and the absence of `serde_tag` means *external*, not
/// untagged — the two are different wire forms and conflating them is exactly the bug this
/// module exists to prevent.
pub fn serde_enum_repr(enum_def: &EnumDef) -> SerdeEnumRepr {
    if enum_def.serde_untagged {
        return SerdeEnumRepr::Untagged;
    }
    match (enum_def.serde_tag.as_deref(), enum_def.serde_content.as_deref()) {
        (Some(tag), Some(content)) => SerdeEnumRepr::Adjacent {
            tag: tag.to_string(),
            content: content.to_string(),
        },
        (Some(tag), None) => SerdeEnumRepr::Internal { tag: tag.to_string() },
        (None, _) => SerdeEnumRepr::External,
    }
}

/// The inverse of [`crate::codegen::naming::wire_variant_value`]: given a value read off the
/// JSON/wire surface (an e2e fixture's enum-typed input, a recorded response body), return the
/// Rust variant name that produces it — which is also the public member identifier every binding
/// declares for that variant.
///
/// A caller that instead re-cases the wire value (`wire.to_upper_camel_case()`) is using a wire
/// name as a host-language identifier casing rule, which the `centralized-naming` rule forbids.
/// The two agree only for variants whose wire spelling happens to round-trip, and diverge
/// silently for `#[serde(rename = "...")]` (`"md"` -> `Md`, declared `Markdown`), for
/// `rename_all = "lowercase"` over a multi-word variant (`PlainText` -> `"plaintext"` ->
/// `Plaintext`), and for any acronym-cased variant (`HTML` -> `"html"` -> `Html`).
///
/// Both spellings are accepted, mirroring the field-side precedent in the TypeScript e2e
/// generator's `refuse_undeclared_json_keys`: a fixture may legitimately name an enum value by
/// its Rust identifier or by its wire value, and rejecting the former would fail a correctly
/// authored fixture. `None` means no declared variant carries that wire value at all, which a
/// caller must not paper over by inventing an identifier. ~keep
#[must_use]
pub fn variant_name_for_wire<'a>(enum_def: &'a EnumDef, wire: &str) -> Option<&'a str> {
    let rename_all = enum_def.serde_rename_all.as_deref();
    enum_def
        .variants
        .iter()
        .find(|variant| {
            variant.name == wire
                || crate::codegen::naming::wire_variant_value(
                    &variant.name,
                    variant.serde_rename.as_deref(),
                    rename_all,
                ) == wire
        })
        .map(|variant| variant.name.as_str())
}

#[cfg(test)]
mod variant_name_for_wire_tests {
    use super::*;
    use crate::core::ir::EnumVariant;

    fn output_format(rename_all: Option<&str>, variants: &[(&str, Option<&str>)]) -> EnumDef {
        EnumDef {
            name: "OutputFormat".to_string(),
            serde_rename_all: rename_all.map(str::to_string),
            variants: variants
                .iter()
                .map(|(name, rename)| EnumVariant {
                    name: (*name).to_string(),
                    serde_rename: rename.map(str::to_string),
                    ..EnumVariant::default()
                })
                .collect(),
            ..EnumDef::default()
        }
    }

    /// One row of the rename-resolution table: the case's name, the enum's `rename_all`, its
    /// variants as `(name, serde_rename)`, the wire value to resolve, and the variant that must
    /// come back (`None` for "no variant matches").
    type RenameCase<'a> = (
        &'a str,
        Option<&'a str>,
        &'a [(&'a str, Option<&'a str>)],
        &'a str,
        Option<&'a str>,
    );

    #[test]
    fn should_resolve_every_rename_strategy_back_to_the_declared_variant() {
        let cases: [RenameCase<'_>; 7] = [
            (
                "single word, no rename",
                None,
                &[("Markdown", None)],
                "Markdown",
                Some("Markdown"),
            ),
            (
                "single word under lowercase",
                Some("lowercase"),
                &[("Markdown", None)],
                "markdown",
                Some("Markdown"),
            ),
            (
                "multi word under snake_case",
                Some("snake_case"),
                &[("PlainText", None)],
                "plain_text",
                Some("PlainText"),
            ),
            (
                "multi word under lowercase collapses, so re-casing cannot recover it",
                Some("lowercase"),
                &[("PlainText", None)],
                "plaintext",
                Some("PlainText"),
            ),
            (
                "explicit rename wins over rename_all",
                Some("snake_case"),
                &[("Markdown", Some("md"))],
                "md",
                Some("Markdown"),
            ),
            (
                "acronym variant under lowercase",
                Some("lowercase"),
                &[("HTML", None)],
                "html",
                Some("HTML"),
            ),
            ("value no variant declares", None, &[("Markdown", None)], "pdf", None),
        ];

        for (case, rename_all, variants, wire, expected) in cases {
            let enum_def = output_format(rename_all, variants);
            assert_eq!(variant_name_for_wire(&enum_def, wire), expected, "{case}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enum_def(tag: Option<&str>, content: Option<&str>, untagged: bool) -> EnumDef {
        EnumDef {
            name: "Sample".to_string(),
            serde_tag: tag.map(str::to_string),
            serde_content: content.map(str::to_string),
            serde_untagged: untagged,
            ..EnumDef::default()
        }
    }

    #[test]
    fn should_be_external_when_no_serde_container_attribute_is_present() {
        let repr = serde_enum_repr(&enum_def(None, None, false));
        assert_eq!(repr, SerdeEnumRepr::External);
        assert_eq!(repr.kind(), "external");
        assert_eq!(repr.tag(), None);
        assert_eq!(repr.content(), None);
    }

    #[test]
    fn should_be_internal_when_only_tag_is_present() {
        let repr = serde_enum_repr(&enum_def(Some("type"), None, false));
        assert_eq!(
            repr,
            SerdeEnumRepr::Internal {
                tag: "type".to_string()
            }
        );
        assert_eq!(repr.kind(), "internal");
        assert_eq!(repr.tag(), Some("type"));
        assert_eq!(repr.content(), None);
    }

    #[test]
    fn should_be_adjacent_when_tag_and_content_are_both_present() {
        let repr = serde_enum_repr(&enum_def(Some("kind"), Some("payload"), false));
        assert_eq!(
            repr,
            SerdeEnumRepr::Adjacent {
                tag: "kind".to_string(),
                content: "payload".to_string()
            }
        );
        assert_eq!(repr.kind(), "adjacent");
        assert_eq!(repr.tag(), Some("kind"));
        assert_eq!(repr.content(), Some("payload"));
    }

    #[test]
    fn should_be_untagged_when_untagged_is_set() {
        let repr = serde_enum_repr(&enum_def(None, None, true));
        assert_eq!(repr, SerdeEnumRepr::Untagged);
        assert_eq!(repr.kind(), "untagged");
        assert_eq!(repr.tag(), None);
    }

    #[test]
    fn should_prefer_untagged_over_a_stray_tag() {
        assert_eq!(
            serde_enum_repr(&enum_def(Some("type"), None, true)),
            SerdeEnumRepr::Untagged
        );
    }

    #[test]
    fn should_ignore_content_without_a_tag() {
        assert_eq!(
            serde_enum_repr(&enum_def(None, Some("payload"), false)),
            SerdeEnumRepr::External
        );
    }
}
