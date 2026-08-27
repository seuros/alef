use super::{
    ConversionConfig, gen_enum_from_binding_to_core, gen_enum_from_binding_to_core_cfg, gen_enum_from_core_to_binding,
    gen_enum_from_core_to_binding_cfg,
};
use crate::core::ir::{EnumDef, EnumVariant};

fn simple_enum() -> EnumDef {
    EnumDef {
        name: "Backend".to_string(),
        rust_path: "my_crate::Backend".to_string(),
        variants: vec![
            EnumVariant {
                name: "Cpu".to_string(),
                ..Default::default()
            },
            EnumVariant {
                name: "Gpu".to_string(),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// The regression this task fixes: `gen_enum_from_core_to_binding_cfg` /
/// `gen_enum_from_binding_to_core_cfg` hard-coded `cfg => Option::<&str>::None` for every arm --
/// the shared `conversions/enum_from_core_to_binding` / `enum_from_binding_to_core` templates
/// already supported an `#[cfg(...)]` per arm, but nothing ever populated it. This function is
/// shared by napi, magnus, rustler, and wasm, so this one fix covers all four. A host-owned
/// cfg-gated variant (`rust_path` rooted in the same crate as `core_import`) keeps its arm in
/// both directions and the arm now carries its `#[cfg(...)]` guard.
///
/// A second, later regression lived right next to this one: both directions added a trailing
/// `_ => Default::default()` catch-all whenever ANY variant carried a cfg, host-owned or not. A
/// host-owned variant's arm carries the identical `#[cfg(...)]` as the variant itself, so the
/// two always compile in or out together and the match stays exhaustive either way -- the
/// catch-all is unreachable and trips `-D warnings`' `unreachable_patterns` the moment the
/// gating feature is active (the default once cfg features are forwarded, alef #464). See
/// `enum_conversion_needs_catch_all`.
#[test]
fn host_cfg_variant_keeps_its_arm_and_gains_a_cfg_guard_in_both_directions() {
    let mut enum_def = simple_enum();
    enum_def.variants[1].cfg = Some(r#"feature = "gpu-accel""#.to_string());

    let core_to_binding = gen_enum_from_core_to_binding(&enum_def, "my_crate");
    assert!(
        core_to_binding.contains("my_crate::Backend::Gpu => Self::Gpu"),
        "host-owned variant's From<CoreType> arm must still be emitted, got:\n{core_to_binding}"
    );
    assert_eq!(
        core_to_binding.matches("#[cfg(feature = \"gpu-accel\")]").count(),
        1,
        "the From<CoreType> arm must carry the #[cfg] guard exactly once, got:\n{core_to_binding}"
    );
    assert!(
        !core_to_binding.contains("_ => Default::default()"),
        "a host-owned cfg-gated variant must not trigger a catch-all (unreachable pattern under \
         -D warnings), got:\n{core_to_binding}"
    );

    let binding_to_core = gen_enum_from_binding_to_core(&enum_def, "my_crate");
    assert!(
        binding_to_core.contains("Backend::Gpu => Self::Gpu"),
        "host-owned variant's From<BindingEnum> arm must still be emitted, got:\n{binding_to_core}"
    );
    assert_eq!(
        binding_to_core.matches("#[cfg(feature = \"gpu-accel\")]").count(),
        1,
        "the From<BindingEnum> arm must carry the #[cfg] guard exactly once, got:\n{binding_to_core}"
    );
    assert!(
        !binding_to_core.contains("_ => Default::default()"),
        "a host-owned cfg-gated variant must not trigger a catch-all (unreachable pattern under \
         -D warnings), got:\n{binding_to_core}"
    );
}

/// Same regression, the foreign-crate half: a variant merged in from a foreign
/// `[[crates.source_crates]]` crate (`EnumDef::rust_path` rooted in a crate other than
/// `core_import`) carries that crate's own cfg. Forwarding it verbatim as `#[cfg(...)]` names a
/// feature this binding crate never declares -- an `unexpected cfg condition value` error -- so
/// the arm must be dropped entirely instead, in both directions, mirroring
/// `backends::ffi::gen_bindings::types::gen_enum_from_i32_rs_helper` and
/// `backends::swift::gen_rust_crate::enums::emit_enum_wrapper`.
#[test]
fn foreign_cfg_variant_arm_is_dropped_not_gated_in_both_directions() {
    let mut enum_def = simple_enum();
    enum_def.rust_path = "dep_crate::Backend".to_string();
    enum_def.variants[1].cfg = Some(r#"feature = "testkit""#.to_string());

    let core_to_binding = gen_enum_from_core_to_binding(&enum_def, "my_crate");
    assert!(
        !core_to_binding.contains("#[cfg(feature = \"testkit\")]"),
        "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{core_to_binding}"
    );
    assert!(
        !core_to_binding.contains("dep_crate::Backend::Gpu =>"),
        "a foreign-crate cfg-gated variant must not be referenced in the From<CoreType> match, got:\n{core_to_binding}"
    );
    assert!(
        core_to_binding.contains("_ => Default::default()"),
        "dropping the arm must still leave the match exhaustive via the catch-all, got:\n{core_to_binding}"
    );

    let binding_to_core = gen_enum_from_binding_to_core(&enum_def, "my_crate");
    assert!(
        !binding_to_core.contains("#[cfg(feature = \"testkit\")]"),
        "no invalid #[cfg] naming an undeclared feature may be emitted, got:\n{binding_to_core}"
    );
    assert!(
        !binding_to_core.contains("Backend::Gpu => Self::Gpu"),
        "a foreign-crate cfg-gated variant must not be referenced in the From<BindingEnum> match, got:\n{binding_to_core}"
    );
    assert!(
        binding_to_core.contains("_ => Default::default()"),
        "dropping the arm must still leave the From<BindingEnum> match exhaustive via the \
         catch-all, got:\n{binding_to_core}"
    );
}

/// Negative control: an ungated enum (every variant's `cfg` is `None`, the `simple_enum()`
/// fixture as-is) must emit no `#[cfg(...)]` in either direction.
#[test]
fn ungated_enum_emits_no_cfg_in_either_direction() {
    let enum_def = simple_enum();
    let core_to_binding = gen_enum_from_core_to_binding(&enum_def, "my_crate");
    assert!(
        !core_to_binding.contains("#[cfg("),
        "ungated enum must not emit #[cfg(...)] in From<CoreType> impl, got:\n{core_to_binding}"
    );
    let binding_to_core = gen_enum_from_binding_to_core(&enum_def, "my_crate");
    assert!(
        !binding_to_core.contains("#[cfg("),
        "ungated enum must not emit #[cfg(...)] in From<BindingEnum> impl, got:\n{binding_to_core}"
    );
}

/// The fix this task adds: when the binding's own configured feature set PROVES a foreign-crate
/// cfg-gated variant's feature is off, the conversion needs no catch-all at all -- the variant
/// can never exist for this binding, so the remaining explicit arms are already exhaustive
/// against the real (feature-reduced) type this binding actually links. Load-bearing gating: if
/// `Gpu` were reachable under every feature enabled (the naive "turn everything on" fixture), this
/// case could never reproduce alef #534 -- the pre-fix blanket `has_cfg_variants` check and the
/// post-fix proof-aware check would agree by accident, passing against the still-broken code. ~keep
#[test]
fn foreign_cfg_variant_proven_disabled_needs_no_catch_all() {
    let mut enum_def = simple_enum();
    enum_def.rust_path = "dep_crate::Backend".to_string();
    enum_def.variants[1].cfg = Some(r#"feature = "gpu-accel""#.to_string());

    let configured = vec!["other-feature".to_string()];
    let config = ConversionConfig {
        configured_features: Some(configured.as_slice()),
        ..Default::default()
    };

    let core_to_binding = gen_enum_from_core_to_binding_cfg(&enum_def, "my_crate", &config);
    assert!(
        !core_to_binding.contains("_ => Default::default()"),
        "a foreign variant proven unreachable by the binding's own configured features must not \
         trigger a catch-all (unreachable pattern under -D warnings), got:\n{core_to_binding}"
    );
    assert!(
        !core_to_binding.contains("Backend::Gpu"),
        "the proven-unreachable variant must not be referenced at all, got:\n{core_to_binding}"
    );
    assert!(
        core_to_binding.contains("Backend::Cpu => Self::Cpu"),
        "the still-reachable variant must still convert, got:\n{core_to_binding}"
    );

    let binding_to_core = gen_enum_from_binding_to_core_cfg(&enum_def, "my_crate", &config);
    assert!(
        !binding_to_core.contains("_ => Default::default()"),
        "same proof, opposite direction, got:\n{binding_to_core}"
    );
}

/// Positive control for the test above: when the configured feature set does NOT rule the gate
/// out (here, the feature is explicitly requested), the conversion falls back to the existing
/// conservative behavior. Cargo feature unification could still turn a dependency's own feature
/// on some way alef's static configuration read cannot observe, so the catch-all must stay. Same
/// fixture as `foreign_cfg_variant_proven_disabled_needs_no_catch_all` except for the configured
/// feature list, proving the new logic only removes the catch-all when it can actually prove the
/// gate off, never merely because a foreign variant is present. ~keep
#[test]
fn foreign_cfg_variant_not_ruled_out_keeps_conservative_catch_all() {
    let mut enum_def = simple_enum();
    enum_def.rust_path = "dep_crate::Backend".to_string();
    enum_def.variants[1].cfg = Some(r#"feature = "gpu-accel""#.to_string());

    let configured = vec!["gpu-accel".to_string()];
    let config = ConversionConfig {
        configured_features: Some(configured.as_slice()),
        ..Default::default()
    };

    let core_to_binding = gen_enum_from_core_to_binding_cfg(&enum_def, "my_crate", &config);
    assert!(
        core_to_binding.contains("_ => Default::default()"),
        "a variant the configured features do not rule out must keep the conservative catch-all, \
         got:\n{core_to_binding}"
    );
}
