//! Security control (task #558): `kotlin_field_default` is what `dto.rs` calls to render a
//! `data class` constructor parameter's default suffix. An `Option<Enum>` field with no explicit
//! default resolves to `typed_default = Some(DefaultValue::Empty)` after extraction (see
//! `extract::extractor::postprocess::resolve_enum_field_defaults`), and the emitted Kotlin
//! default must stay `= null` — never the enum's own `#[default]` variant. Forwarding a
//! materialized variant here is indistinguishable from a caller's explicit choice once it
//! crosses the JNI boundary, and can silently override a stricter policy the caller set
//! elsewhere (e.g. a global content-only mode).
//!
//! `tests.rs` in this same directory is already at this repo's 1,000-line file-size cap
//! (grandfathered), so this coverage lives in its own module instead of growing that file. ~keep

use crate::core::ir::DefaultValue;
use std::collections::{HashMap, HashSet};

fn detection_policy_enum_defaults() -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("DetectionPolicy".to_string(), "PreferContent".to_string());
    map
}

#[test]
fn optional_enum_field_with_no_explicit_default_renders_null_not_the_variant() {
    let ty = crate::core::ir::TypeRef::Named("DetectionPolicy".to_string());
    let enum_defaults = detection_policy_enum_defaults();
    let constructible = HashSet::new();

    let rendered =
        super::types::kotlin_field_default(&ty, true, Some(&DefaultValue::Empty), &enum_defaults, &constructible);

    assert_eq!(
        rendered, " = null",
        "an Option<Enum> field with no explicit default must render `null`, not a materialized \
         variant like `DetectionPolicy.PREFER_CONTENT`; got `{rendered}`"
    );
}

/// Negative control: a *required* (non-optional) `Enum` field with the same `Empty` typed
/// default legitimately does resolve to the enum's own default variant. A fix that suppressed
/// every `Empty`-on-`Named` default (rather than only the optional-field case) would pass the
/// positive test above while silently breaking this legitimate one.
#[test]
fn required_enum_field_with_empty_default_still_renders_the_default_variant() {
    let ty = crate::core::ir::TypeRef::Named("DetectionPolicy".to_string());
    let enum_defaults = detection_policy_enum_defaults();
    let constructible = HashSet::new();

    let rendered =
        super::types::kotlin_field_default(&ty, false, Some(&DefaultValue::Empty), &enum_defaults, &constructible);

    assert_eq!(
        rendered, " = DetectionPolicy.PREFER_CONTENT",
        "a required enum field must still render its own `#[default]` variant"
    );
}

/// Negative control: an `Option<Enum>` field that genuinely does have an explicit default naming
/// a concrete variant (`typed_default = Some(EnumVariant(..))`, not narrowed from `Empty`) must
/// still render that real value.
#[test]
fn optional_enum_field_with_explicit_variant_default_still_renders_it() {
    let ty = crate::core::ir::TypeRef::Named("DetectionPolicy".to_string());
    let enum_defaults = detection_policy_enum_defaults();
    let constructible = HashSet::new();

    let rendered = super::types::kotlin_field_default(
        &ty,
        true,
        Some(&DefaultValue::EnumVariant("ContentOnly".to_string())),
        &enum_defaults,
        &constructible,
    );

    assert_eq!(
        rendered, " = DetectionPolicy.CONTENT_ONLY",
        "an explicit variant default on an optional field must still be rendered"
    );
}
