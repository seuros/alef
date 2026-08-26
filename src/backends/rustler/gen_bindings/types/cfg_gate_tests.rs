use super::{gen_rustler_flat_data_enum_from_core, gen_rustler_flat_data_enum_to_core};
use crate::core::ir::{EnumDef, EnumVariant};

fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn flat_enum(rust_path: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: "VisitorResult".to_string(),
        rust_path: rust_path.to_string(),
        variants,
        ..Default::default()
    }
}

/// The regression this task fixes: `gen_rustler_flat_data_enum_from_core` and
/// `gen_rustler_flat_data_enum_to_core` never read `EnumVariant::cfg`, so a cfg-gated variant on
/// a flat-struct data enum was referenced unconditionally in both `From`-impl directions --
/// E0599 in a build excluding its feature. A host-owned variant must keep its arm, gated with
/// `#[cfg(...)]`, in both directions.
#[test]
fn host_owned_cfg_variant_keeps_its_arm_and_gate_in_both_directions() {
    let en = flat_enum(
        "mylib::VisitorResult",
        vec![
            unit_variant("Continue", None),
            unit_variant("Thumbnail", Some(r#"feature = "thumbnails""#)),
        ],
    );

    let from_core = gen_rustler_flat_data_enum_from_core(&en, "mylib");
    assert!(
        from_core.contains("mylib::VisitorResult::Thumbnail"),
        "the host-owned variant's arm must still be emitted, got:\n{from_core}"
    );
    assert_eq!(
        from_core.matches("#[cfg(feature = \"thumbnails\")]").count(),
        1,
        "the host-owned variant's arm must carry its #[cfg] guard exactly once, got:\n{from_core}"
    );

    let to_core = gen_rustler_flat_data_enum_to_core(&en, "mylib");
    assert!(
        to_core.contains("mylib::VisitorResult::Thumbnail"),
        "the host-owned variant's arm must still be emitted, got:\n{to_core}"
    );
    assert_eq!(
        to_core.matches("#[cfg(feature = \"thumbnails\")]").count(),
        1,
        "the host-owned variant's arm must carry its #[cfg] guard exactly once, got:\n{to_core}"
    );
}

/// A variant merged in from a foreign `[[crates.source_crates]]` crate carries that crate's own
/// cfg gate. Forwarding it as `#[cfg(...)]` names a feature this Rustler crate never declares --
/// an `unexpected cfg condition value` warning -- so the arm must be dropped entirely instead,
/// mirroring `codegen::conversions::enums::emit_cfg_gated_arm`.
#[test]
fn foreign_owned_cfg_variant_arm_is_dropped_not_gated_in_both_directions() {
    let en = flat_enum(
        "dep_crate::VisitorResult",
        vec![
            unit_variant("Continue", None),
            unit_variant("Testkit", Some(r#"feature = "testkit""#)),
        ],
    );

    let from_core = gen_rustler_flat_data_enum_from_core(&en, "mylib");
    assert!(
        !from_core.contains("#[cfg(feature = \"testkit\")]"),
        "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{from_core}"
    );
    assert!(
        !from_core.contains("::Testkit"),
        "a foreign-crate cfg-gated variant must not be referenced, got:\n{from_core}"
    );
    assert!(
        from_core.contains("_ => Self::default(),"),
        "dropping the arm must still leave the match exhaustive via the catch-all, got:\n{from_core}"
    );

    let to_core = gen_rustler_flat_data_enum_to_core(&en, "mylib");
    assert!(
        !to_core.contains("#[cfg(feature = \"testkit\")]"),
        "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{to_core}"
    );
    assert!(
        !to_core.contains("::Testkit"),
        "a foreign-crate cfg-gated variant must not be referenced, got:\n{to_core}"
    );
}

/// Negative control: an ungated enum emits no `#[cfg(...)]` at all.
#[test]
fn ungated_enum_emits_no_cfg_in_either_direction() {
    let en = flat_enum(
        "mylib::VisitorResult",
        vec![unit_variant("Continue", None), unit_variant("Skip", None)],
    );

    let from_core = gen_rustler_flat_data_enum_from_core(&en, "mylib");
    assert!(
        !from_core.contains("#[cfg("),
        "ungated enum must not emit #[cfg(...)], got:\n{from_core}"
    );

    let to_core = gen_rustler_flat_data_enum_to_core(&en, "mylib");
    assert!(!to_core.contains("#[cfg("), "ungated enum must not emit #[cfg(...)], got:\n{to_core}");
}
