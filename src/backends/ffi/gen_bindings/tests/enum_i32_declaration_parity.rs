// Separate file rather than adding to `cfg_gated_variants.rs`, which already covers the
// reconstruction helper `gen_enum_from_i32_rs_helper`; this file covers the paired C-facing
// validation surface `gen_enum_from_i32`/`gen_enum_to_i32` that must agree with it. ~keep
use super::super::types::{gen_enum_from_i32, gen_enum_from_i32_rs_helper, gen_enum_to_i32};
use crate::core::ir::*;
use std::collections::HashSet;

const HOST_CRATE: &str = "my_lib";

/// `Base`/`Middle` are always reachable; `Extra` carries `cfg`. `owner_crate` selects whether the
/// whole enum -- and therefore `Extra`'s cfg -- is host- or foreign-owned relative to
/// [`HOST_CRATE`].
fn sample_enum(owner_crate: &str, extra_cfg: &str) -> EnumDef {
    EnumDef {
        name: "SampleMode".to_string(),
        rust_path: format!("{owner_crate}::SampleMode"),
        variants: vec![
            EnumVariant {
                name: "Base".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Middle".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Extra".to_string(),
                cfg: Some(extra_cfg.to_string()),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

/// The regression this task fixes: before it, `gen_enum_from_i32`/`gen_enum_to_i32` validated
/// every discriminant in `0..variants.len()` unconditionally, regardless of cfg -- while
/// `gen_enum_from_i32_rs_helper` already drops the reconstruction arm for any foreign cfg-gated
/// variant unconditionally. A FOREIGN variant this binding's configured feature set proves
/// unreachable must be rejected by both validation functions too, or a C caller is told
/// discriminant 2 is valid and then gets `None` back from the paired reconstruction.
#[test]
fn foreign_variant_proven_unreachable_is_rejected_by_from_i32_and_to_i32() {
    let en = sample_enum("dep_crate", r#"feature = "testkit""#);
    let configured: HashSet<&str> = HashSet::new();

    let from_i32 = gen_enum_from_i32(&en, "alef", HOST_CRATE, Some(&configured));
    assert!(
        from_i32.contains("0 => 0, // Base") && from_i32.contains("1 => 1, // Middle"),
        "the two reachable variants must still validate, got:\n{from_i32}"
    );
    assert!(
        !from_i32.contains("2 =>"),
        "the proven-unreachable discriminant must not validate as accepted input, got:\n{from_i32}"
    );

    let to_i32 = gen_enum_to_i32(&en, "alef", HOST_CRATE, Some(&configured));
    assert!(
        to_i32.contains("\"Base\" => 0,") && to_i32.contains("\"Middle\" => 1,"),
        "the two reachable variants' wire names must still validate, got:\n{to_i32}"
    );
    assert!(
        !to_i32.contains("\"Extra\""),
        "a proven-unreachable variant's wire name must not be advertised as accepted input, got:\n{to_i32}"
    );

    // Cross-check against the paired reconstruction helper: it already drops this arm
    // unconditionally (regardless of provenance), so `from_i32`/`to_i32` accepting discriminant 2
    // would have disagreed with it even before this fix proved the variant unreachable.
    let helper = gen_enum_from_i32_rs_helper(&en, "dep_crate", HOST_CRATE);
    assert!(
        !helper.contains("2 => Some"),
        "apparatus check: the reconstruction helper must have no arm for discriminant 2 either, got:\n{helper}"
    );
}

/// Same foreign variant, but `configured_features` is `None` -- "unknown", not proven absent,
/// since Cargo feature unification could still enable it -- so both validation functions must
/// keep accepting it, unchanged from before this fix.
#[test]
fn foreign_variant_not_proven_unreachable_is_accepted_by_from_i32_and_to_i32() {
    let en = sample_enum("dep_crate", r#"feature = "testkit""#);

    let from_i32 = gen_enum_from_i32(&en, "alef", HOST_CRATE, None);
    assert!(
        from_i32.contains("2 => 2, // Extra"),
        "an unproven foreign variant must still validate, got:\n{from_i32}"
    );

    let to_i32 = gen_enum_to_i32(&en, "alef", HOST_CRATE, None);
    assert!(
        to_i32.contains("\"Extra\" => 2,"),
        "an unproven foreign variant's wire name must still validate, got:\n{to_i32}"
    );
}

/// A host-owned cfg-gated variant must never be rejected by either validation function --
/// existing behavior, unchanged by this fix -- regardless of `configured_features`.
#[test]
fn host_owned_cfg_gated_variant_is_accepted_by_from_i32_and_to_i32_regardless_of_configured_features() {
    let en = sample_enum(HOST_CRATE, r#"feature = "extra_feature""#);
    let configured: HashSet<&str> = HashSet::new();

    let from_i32 = gen_enum_from_i32(&en, "alef", HOST_CRATE, Some(&configured));
    assert!(
        from_i32.contains("2 => 2, // Extra"),
        "a host-owned cfg-gated variant must stay accepted, got:\n{from_i32}"
    );

    let to_i32 = gen_enum_to_i32(&en, "alef", HOST_CRATE, Some(&configured));
    assert!(
        to_i32.contains("\"Extra\" => 2,"),
        "a host-owned cfg-gated variant's wire name must stay accepted, got:\n{to_i32}"
    );
}

/// Discriminant numbering must never re-sequence after a variant is dropped: `Extra` (index 2)
/// being proven unreachable must not shift a later variant down to fill the gap, or the C
/// caller's understanding of a discriminant would silently disagree with
/// `gen_enum_from_i32_rs_helper`, which always reserves by original position.
#[test]
fn dropping_a_variant_does_not_renumber_the_variants_that_follow_it() {
    let mut en = sample_enum("dep_crate", r#"feature = "testkit""#);
    en.variants.push(EnumVariant {
        name: "Trailing".to_string(),
        ..EnumVariant::default()
    });
    let configured: HashSet<&str> = HashSet::new();

    let from_i32 = gen_enum_from_i32(&en, "alef", HOST_CRATE, Some(&configured));
    assert!(
        from_i32.contains("3 => 3, // Trailing"),
        "the variant after the dropped one must keep its ORIGINAL discriminant (3), not be \
         renumbered down to 2, got:\n{from_i32}"
    );
}
