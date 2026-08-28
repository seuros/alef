//! Security control (task #558): generated reference docs must not advertise a default for an
//! `Option<Enum>` field the Rust source never actually gives one. `format_field_default` is the
//! single function every per-language reference page (`docs-site/.../reference/types.md` and the
//! per-language variants) calls to render a field's "Default" table cell.
//!
//! Before the fix, `extract::extractor::postprocess::resolve_enum_field_defaults` narrowed a
//! field's `Empty` typed default to the enum's own `#[default]` variant without checking
//! `field.optional`, so an `Option<Enum>` field with no explicit default reached
//! `format_field_default` as `typed_default = Some(DefaultValue::EnumVariant(..))` — a state
//! this function's `EnumVariant` branch renders unconditionally, without ever consulting
//! `optional`. The fix keeps that field `Empty`, which this function's `Empty` branch already
//! renders correctly as `None`/`null`/etc. per language.

use super::*;
use crate::core::ir::{DefaultValue, EnumDef, EnumVariant};
use crate::docs::formatting::format_field_default;

fn detection_policy_enum() -> EnumDef {
    EnumDef {
        name: "DetectionPolicy".to_string(),
        has_default: true,
        variants: vec![
            EnumVariant {
                name: "PreferContent".to_string(),
                is_default: true,
                ..Default::default()
            },
            EnumVariant {
                name: "ContentOnly".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

const NULLISH_BY_LANGUAGE: &[(Language, &str)] = &[
    (Language::Python, "`None`"),
    (Language::Node, "`null`"),
    (Language::Kotlin, "`null`"),
    (Language::KotlinAndroid, "`null`"),
    (Language::Swift, "`null`"),
    (Language::Dart, "`null`"),
    (Language::Csharp, "`null`"),
    (Language::Ruby, "`nil`"),
    (Language::Go, "`nil`"),
    (Language::Php, "`null`"),
];

#[test]
fn optional_enum_field_with_no_explicit_default_documents_absence_not_the_variant() {
    let api = ApiSurface {
        enums: vec![detection_policy_enum()],
        ..make_minimal_api("0.1.0")
    };
    let field = make_field(
        "detection_policy",
        TypeRef::Named("DetectionPolicy".to_string()),
        true,
        Some(DefaultValue::Empty),
    );

    for (lang, expected) in NULLISH_BY_LANGUAGE {
        let rendered = format_field_default(&field, *lang, &api, "Htm");
        assert_eq!(
            rendered, *expected,
            "docs for {lang:?} must document an Option<Enum> field with no explicit default as \
             absent (`{expected}`), not as the enum's `#[default]` variant `PreferContent`; got \
             `{rendered}`"
        );
    }
}

/// Negative control: a *required* (non-optional) `Enum` field with the same `Empty` typed
/// default legitimately documents the enum's own default variant. A fix that suppressed every
/// `Empty`-on-`Named` default (rather than only the optional-field case) would pass the positive
/// test above while silently breaking this legitimate one.
#[test]
fn required_enum_field_with_empty_default_still_documents_the_default_variant() {
    let api = ApiSurface {
        enums: vec![detection_policy_enum()],
        ..make_minimal_api("0.1.0")
    };
    let field = make_field(
        "detection_policy",
        TypeRef::Named("DetectionPolicy".to_string()),
        false,
        Some(DefaultValue::Empty),
    );

    let rendered = format_field_default(&field, Language::Python, &api, "Htm");

    assert_eq!(
        rendered, "`DetectionPolicy.PREFER_CONTENT`",
        "a required enum field must still document its own `#[default]` variant"
    );
}

/// Negative control: an `Option<Enum>` field that genuinely does have an explicit default naming
/// a concrete variant (`typed_default = Some(EnumVariant(..))`, not narrowed from `Empty`) must
/// still document that real value.
#[test]
fn optional_enum_field_with_explicit_variant_default_still_documents_it() {
    let api = ApiSurface {
        enums: vec![detection_policy_enum()],
        ..make_minimal_api("0.1.0")
    };
    let field = make_field(
        "detection_policy",
        TypeRef::Named("DetectionPolicy".to_string()),
        true,
        Some(DefaultValue::EnumVariant("ContentOnly".to_string())),
    );

    let rendered = format_field_default(&field, Language::Python, &api, "Htm");

    assert_eq!(
        rendered, "`DetectionPolicy.CONTENT_ONLY`",
        "an explicit variant default on an optional field must still be documented"
    );
}
