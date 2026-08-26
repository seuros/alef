// Separate file rather than adding to `regressions.rs`, which is already well over the
// 1,000-line file-modularization cap. ~keep
use super::super::types::gen_enum_from_i32_rs_helper;
use crate::core::ir::*;

const HOST_CRATE: &str = "my_lib";

/// A variant behind `#[cfg(feature = "...")]` does not exist in a build without that feature.
/// The `from_i32` reconstruction helper used to emit an ungated arm naming it, which is a hard
/// compile error in the consumer's crate — observed as a generated
/// `2 => Some(dep::TierStrategy::Tier1)` against a dependency whose `Tier1` is testkit-only.
/// The discriminant stays reserved so numbering is stable across feature subsets. ~keep
///
/// `rust_path` is rooted in a crate other than `HOST_CRATE`, so this enum is merged in from a
/// foreign `[[crates.source_crates]]` crate: its cfg is not a feature the FFI crate declares.
fn foreign_tiered_enum() -> EnumDef {
    EnumDef {
        name: "TierStrategy".to_string(),
        rust_path: "dep_crate::TierStrategy".to_string(),
        variants: vec![
            EnumVariant {
                name: "Auto".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Tier2".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Tier1".to_string(),
                cfg: Some("any(test, feature = \"testkit\")".to_string()),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

/// Same shape as [`foreign_tiered_enum`], but `rust_path` is rooted in `HOST_CRATE` itself: the
/// cfg names a feature the host crate (and therefore this FFI crate, via feature forwarding)
/// actually declares, so the arm and its `#[cfg(...)]` must be kept unchanged.
fn host_tiered_enum() -> EnumDef {
    EnumDef {
        name: "TierStrategy".to_string(),
        rust_path: format!("{HOST_CRATE}::TierStrategy"),
        variants: vec![
            EnumVariant {
                name: "Auto".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Tier2".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Tier1".to_string(),
                cfg: Some("feature = \"tier1\"".to_string()),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

#[test]
fn from_i32_helper_gates_a_host_cfg_variant_arm() {
    let rendered = gen_enum_from_i32_rs_helper(&host_tiered_enum(), HOST_CRATE, HOST_CRATE);

    assert!(
        rendered.contains("#[cfg(feature = \"tier1\")]"),
        "the host-owned cfg-gated variant's arm must carry its #[cfg], got:\n{rendered}"
    );

    let gated_arm = rendered
        .lines()
        .position(|line| line.contains("Tier1"))
        .expect("the Tier1 arm should still be emitted");
    let attribute = rendered
        .lines()
        .position(|line| line.contains("#[cfg("))
        .expect("a #[cfg] attribute should be emitted");
    assert_eq!(
        attribute + 1,
        gated_arm,
        "the #[cfg] must sit immediately above the arm it gates, got:\n{rendered}"
    );
    assert!(
        rendered.contains(&format!("2 => Some({HOST_CRATE}::TierStrategy::Tier1),")),
        "the host-owned gated variant keeps discriminant 2 so numbering stays stable, got:\n{rendered}"
    );
}

#[test]
fn from_i32_helper_leaves_ungated_variants_alone() {
    let rendered = gen_enum_from_i32_rs_helper(&host_tiered_enum(), HOST_CRATE, HOST_CRATE);

    for (index, variant) in [(0, "Auto"), (1, "Tier2")] {
        let arm = format!("{index} => Some({HOST_CRATE}::TierStrategy::{variant}),");
        assert!(
            rendered.contains(&arm),
            "expected an ungated arm `{arm}`, got:\n{rendered}"
        );
    }
    assert_eq!(
        rendered.matches("#[cfg(").count(),
        1,
        "only the gated variant may carry a #[cfg], got:\n{rendered}"
    );
}

/// The regression this task fixes: a variant merged in from a foreign crate carries that
/// crate's own cfg, which this FFI crate's `Cargo.toml` never declares as a feature. Emitting
/// it verbatim (`#[cfg(any(test, feature = "testkit"))]`) is an `unexpected cfg condition
/// value` error, so the arm referencing `Tier1` must be dropped entirely rather than gated.
#[test]
fn from_i32_helper_drops_foreign_cfg_variant_arm_instead_of_gating_it() {
    let rendered = gen_enum_from_i32_rs_helper(&foreign_tiered_enum(), "dep_crate", HOST_CRATE);

    assert!(
        !rendered.contains("Tier1"),
        "a foreign-crate cfg-gated variant must not be referenced at all, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("#[cfg("),
        "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{rendered}"
    );
}

/// Ungated variants of a foreign-owned enum are unaffected: only a variant that itself carries
/// a foreign cfg is dropped, per [`from_i32_helper_drops_foreign_cfg_variant_arm_instead_of_gating_it`].
#[test]
fn from_i32_helper_keeps_ungated_variants_of_a_foreign_enum() {
    let rendered = gen_enum_from_i32_rs_helper(&foreign_tiered_enum(), "dep_crate", HOST_CRATE);

    for (index, variant) in [(0, "Auto"), (1, "Tier2")] {
        let arm = format!("{index} => Some(dep_crate::TierStrategy::{variant}),");
        assert!(
            rendered.contains(&arm),
            "expected an ungated arm `{arm}`, got:\n{rendered}"
        );
    }
}
