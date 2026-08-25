// Separate file rather than adding to `regressions.rs`, which is already well over the
// 1,000-line file-modularization cap. ~keep
use super::super::types::gen_enum_from_i32_rs_helper;
use crate::core::ir::*;

/// A variant behind `#[cfg(feature = "...")]` does not exist in a build without that feature.
/// The `from_i32` reconstruction helper used to emit an ungated arm naming it, which is a hard
/// compile error in the consumer's crate — observed as a generated
/// `2 => Some(dep::TierStrategy::Tier1)` against a dependency whose `Tier1` is testkit-only.
/// The discriminant stays reserved so numbering is stable across feature subsets. ~keep
fn tiered_enum() -> EnumDef {
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

#[test]
fn from_i32_helper_gates_a_cfg_variant_arm() {
    let rendered = gen_enum_from_i32_rs_helper(&tiered_enum(), "dep_crate");

    assert!(
        rendered.contains("#[cfg(any(test, feature = \"testkit\"))]"),
        "the cfg-gated variant's arm must carry its #[cfg], got:\n{rendered}"
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
}

#[test]
fn from_i32_helper_leaves_ungated_variants_alone() {
    let rendered = gen_enum_from_i32_rs_helper(&tiered_enum(), "dep_crate");

    for (index, variant) in [(0, "Auto"), (1, "Tier2")] {
        let arm = format!("{index} => Some(dep_crate::TierStrategy::{variant}),");
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
    assert!(
        rendered.contains("2 => Some(dep_crate::TierStrategy::Tier1),"),
        "the gated variant keeps discriminant 2 so numbering stays stable, got:\n{rendered}"
    );
}
