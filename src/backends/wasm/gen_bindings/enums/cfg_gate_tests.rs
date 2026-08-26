use super::{gen_tagged_enum_binding_to_core, gen_tagged_enum_core_to_binding};
use crate::core::ir::{EnumDef, EnumVariant};

fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn tagged_enum(rust_path: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: "VisitorResult".to_string(),
        rust_path: rust_path.to_string(),
        variants,
        serde_tag: Some("type".to_string()),
        ..Default::default()
    }
}

/// The regression this task fixes: `gen_tagged_enum_binding_to_core` and
/// `gen_tagged_enum_core_to_binding` already read `EnumVariant::cfg` and emitted a `#[cfg(...)]`
/// guard, but never checked whether the enum's `rust_path` was owned by the host crate before
/// doing so. A host-owned cfg-gated variant must keep its arm, gated, in both directions.
#[test]
fn host_owned_cfg_variant_keeps_its_arm_and_gate_in_both_directions() {
    let en = tagged_enum(
        "mylib::VisitorResult",
        vec![
            unit_variant("Continue", None),
            unit_variant("Thumbnail", Some(r#"feature = "thumbnails""#)),
        ],
    );

    let binding_to_core = gen_tagged_enum_binding_to_core(&en, "mylib", "Wasm");
    assert!(
        binding_to_core.contains("Self::Thumbnail"),
        "the host-owned variant's arm must still be emitted, got:\n{binding_to_core}"
    );
    assert_eq!(
        binding_to_core.matches("#[cfg(feature = \"thumbnails\")]").count(),
        1,
        "the host-owned variant's arm must carry its #[cfg] guard exactly once, got:\n{binding_to_core}"
    );

    let core_to_binding = gen_tagged_enum_core_to_binding(&en, "mylib", "Wasm");
    assert!(
        core_to_binding.contains("mylib::VisitorResult::Thumbnail"),
        "the host-owned variant's arm must still be emitted, got:\n{core_to_binding}"
    );
    assert_eq!(
        core_to_binding.matches("#[cfg(feature = \"thumbnails\")]").count(),
        1,
        "the host-owned variant's arm must carry its #[cfg] guard exactly once, got:\n{core_to_binding}"
    );
}

/// A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's own
/// cfg gate. Before this fix, both functions re-emitted it verbatim regardless of ownership,
/// naming a feature this WASM crate never declares -- an `unexpected cfg condition value`
/// warning. The arm must be dropped entirely instead.
#[test]
fn foreign_owned_cfg_variant_arm_is_dropped_not_gated_in_both_directions() {
    let en = tagged_enum(
        "dep_crate::VisitorResult",
        vec![
            unit_variant("Continue", None),
            unit_variant("Testkit", Some(r#"feature = "testkit""#)),
        ],
    );

    let binding_to_core = gen_tagged_enum_binding_to_core(&en, "mylib", "Wasm");
    assert!(
        !binding_to_core.contains("#[cfg(feature = \"testkit\")]"),
        "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{binding_to_core}"
    );
    assert!(
        !binding_to_core.contains("Self::Testkit"),
        "a foreign-crate cfg-gated variant must not be referenced, got:\n{binding_to_core}"
    );

    let core_to_binding = gen_tagged_enum_core_to_binding(&en, "mylib", "Wasm");
    assert!(
        !core_to_binding.contains("#[cfg(feature = \"testkit\")]"),
        "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{core_to_binding}"
    );
    assert!(
        !core_to_binding.contains("::Testkit"),
        "a foreign-crate cfg-gated variant must not be referenced, got:\n{core_to_binding}"
    );
}

/// Negative control: an ungated enum emits no `#[cfg(...)]` at all.
#[test]
fn ungated_enum_emits_no_cfg_in_either_direction() {
    let en = tagged_enum(
        "mylib::VisitorResult",
        vec![unit_variant("Continue", None), unit_variant("Skip", None)],
    );

    let binding_to_core = gen_tagged_enum_binding_to_core(&en, "mylib", "Wasm");
    assert!(
        !binding_to_core.contains("#[cfg("),
        "ungated enum must not emit #[cfg(...)], got:\n{binding_to_core}"
    );

    let core_to_binding = gen_tagged_enum_core_to_binding(&en, "mylib", "Wasm");
    assert!(
        !core_to_binding.contains("#[cfg("),
        "ungated enum must not emit #[cfg(...)], got:\n{core_to_binding}"
    );
}

/// A cfg-gated first variant must not be chosen as the unconditional `_ =>` default in
/// `gen_tagged_enum_binding_to_core` -- the fallback must skip to the next ungated variant.
#[test]
fn default_variant_skips_a_cfg_gated_first_variant() {
    let en = tagged_enum(
        "mylib::VisitorResult",
        vec![
            unit_variant("Thumbnail", Some(r#"feature = "thumbnails""#)),
            unit_variant("Continue", None),
        ],
    );

    let binding_to_core = gen_tagged_enum_binding_to_core(&en, "mylib", "Wasm");
    assert!(
        binding_to_core.contains("_ => Self::Continue,"),
        "the unconditional default must fall back to the ungated variant, got:\n{binding_to_core}"
    );
}
