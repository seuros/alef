//! How the Java binding backend lowered an assertion's leaf field, as far as the assertion
//! generator needs to know: whether the field is a sealed-interface type rendered through a
//! generated `{TypeName}Display` helper, and whether it is a plain Java `enum` whose access may
//! have `.getValue()` appended.
//!
//! ~keep Extracted from `assertions.rs`, which is over the repo's 1,000-line
//! file-modularization cap and therefore may not grow. The two computations moved here are
//! byte-identical to the `let` bindings they replace, and the caller destructures
//! [`EnumLowering`] back into the same three local names, so every downstream use site in
//! `render_assertion` reads exactly as it did before.

use std::collections::{HashMap, HashSet};

use crate::e2e::field_access::FieldResolver;

/// The three facts `render_assertion` derives about its leaf field's Java lowering.
pub(super) struct EnumLowering {
    /// The sealed-interface type name declared for this field in `assert_enum_types` (e.g.
    /// `"FormatMetadata"`), whose generated `{TypeName}Display` helper produces the display
    /// string the assertion compares against. `None` when the field declares no such type.
    pub(super) sealed_display_type: Option<String>,
    /// Whether [`Self::sealed_display_type`] is set.
    pub(super) is_sealed_display_field: bool,
    /// Whether the field is a Java `enum` whose access may have `.getValue()` appended.
    pub(super) field_is_enum: bool,
}

/// Classify `field`'s Java lowering from the hand-maintained `enum_fields` / `assert_enum_types`
/// config and the IR-derived facts `field_resolver` carries.
pub(super) fn classify_enum_lowering(
    field: Option<&str>,
    field_resolver: &FieldResolver,
    enum_fields: &HashSet<String>,
    assert_enum_types: &HashMap<String, String>,
) -> EnumLowering {
    let sealed_display_type: Option<String> = field.and_then(|f| {
        let resolved = field_resolver.resolve(f);
        assert_enum_types
            .get(f)
            .or_else(|| assert_enum_types.get(resolved))
            .cloned()
    });
    let is_sealed_display_field = sealed_display_type.is_some();

    // Determine if this field is an enum type (no `.contains()` on enums in Java).
    // Check both the raw fixture field path and the resolved (aliased) path so that
    // `fields_enum` entries can use either form (e.g., `"assets[].category"` or the
    // resolved `"assets[].asset_category"`). The hand-maintained `enum_fields` config is
    // checked first, but a field it never lists (e.g. a recursive struct's own enum field,
    // reached only through its parent's path — `data.kind` on a self-referential
    // `Option<Box<DataNode>>`) must still be rescued from the IR-derived classification, the
    // same way `field_resolver.is_enum` already backs every other backend's equivalent check
    // (csharp/kotlin/dart/gleam/swift/...). Without it, such a field silently falls through to
    // a plain `assertEquals(String, EnumType)` that can never pass, since a `String` is never
    // `.equals()` an enum constant. ~keep
    // NOTE: Sealed-interface types (those in assert_enum_types) are not Java enums
    // and do not have a .getValue() method — exclude them from enum field treatment.
    //
    // A third shape needs the same exclusion: an IR enum with data-carrying variants (e.g. a
    // `#[serde(untagged)]` union) is still an "enum" in the IR, but the Java binding backend
    // renders it as a wrapper class (`gen_java_tagged_union` / `gen_java_untagged_wrapper`),
    // neither of which declares `getValue()`. `java_enum_emits_get_value` answers from the
    // exact predicate the binding backend itself branches on
    // (`backends::java::gen_bindings::emits_get_value`), so this can never disagree with what
    // was actually emitted; it answers `None` when the IR doesn't resolve a concrete enum type
    // for the field (e.g. a `fields_enum`-only config entry), in which case the pre-existing
    // behaviour (assume `getValue()` is available) is kept. ~keep
    let field_is_enum = field.is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        let in_enum_fields = enum_fields.get(f).is_some()
            || enum_fields.get(resolved).is_some()
            || field_resolver.is_enum(f)
            || field_resolver.is_enum(resolved);
        let emits_get_value = field_resolver
            .java_enum_emits_get_value(f)
            .or_else(|| field_resolver.java_enum_emits_get_value(resolved))
            .unwrap_or(true);
        in_enum_fields && !is_sealed_display_field && emits_get_value
    });

    EnumLowering {
        sealed_display_type,
        is_sealed_display_field,
        field_is_enum,
    }
}
