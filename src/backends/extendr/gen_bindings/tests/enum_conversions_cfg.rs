//! `enum_conversions.rs` used to name every enum variant unconditionally in the generated
//! `From` impls, with no awareness of `EnumVariant::cfg` at all (the module contained zero
//! occurrences of the string `cfg` before this fix). A host-owned variant behind
//! `#[cfg(feature = "x")]` produced an unconditional match arm naming it, which is `E0599` in
//! any build where `x` is off. A variant merged in from a foreign `[[crates.source_crates]]`
//! crate produced the same unconditional arm referencing a feature this generated crate never
//! declares, which is `unexpected cfg condition value`.
//!
//! The fix (`emit_cfg_gated_arm` in `enum_conversions.rs`) asks
//! `codegen::cfg::is_host_owned_rust_path` the same question every other cfg-aware backend
//! (php, ffi, rustler, magnus, napi, wasm) already asks: a host-owned gated variant keeps its
//! arm under a matching `#[cfg(...)]` guard; a foreign-owned one has its arm dropped entirely.

use super::super::ExtendrBackend;
use super::make_config;
use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::*;

fn gated_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

/// Like `make_config`, but with `configured_feature` set under `[crates.r]` -- used to prove a
/// foreign cfg-gated variant reachable (or not) via
/// `codegen::conversions::enums::enum_conversion_needs_catch_all_for_features` (alef #547). ~keep
fn make_config_with_feature(configured_feature: &str) -> ResolvedCrateConfig {
    let toml_src = format!(
        "[workspace]\nlanguages = [\"r\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.r]\npackage_name = \"testlib\"\nfeatures = [\"{configured_feature}\"]\n"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn returning_function(name: &str, enum_name: &str) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        return_type: TypeRef::Named(enum_name.to_string()),
        ..Default::default()
    }
}

fn generate_r(api: &ApiSurface) -> String {
    generate_r_with_config(api, &make_config())
}

fn generate_r_with_config(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    ExtendrBackend
        .generate_bindings(api, config)
        .expect("extendr generation")
        .iter()
        .map(|f| format!("// ==== {} ====\n{}", f.path.display(), f.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Host-owned enum (`rust_path` starts with the configured core crate name, `test_lib`): the
/// cfg-gated variant's arm must survive in BOTH conversion directions, each wrapped in the exact
/// `#[cfg(...)]` guard from the source, and the ungated variant's arm must carry no guard at all.
///
/// Fails on pre-fix code: `enum_conversions.rs`'s original `gen_from_binding_to_core` /
/// `gen_from_core_to_binding` built every arm via an unconditional `.map(...)` (never a
/// `.filter_map(...)`) and never read `variant.cfg`, so no `#[cfg(feature = "beta")]` was ever
/// emitted anywhere in the output -- the assertions below on that exact guard text fail.
#[test]
fn host_owned_cfg_gated_variant_keeps_arm_under_matching_cfg_guard() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "Status".to_string(),
            rust_path: "test_lib::Status".to_string(),
            variants: vec![
                gated_variant("Active", None),
                gated_variant("Beta", Some(r#"feature = "beta""#)),
            ],
            ..Default::default()
        }],
        functions: vec![returning_function("get_status", "Status")],
        ..Default::default()
    };

    let out = generate_r(&api);

    // Precondition: the ungated variant's arm must exist with no guard, otherwise the "cfg
    // guard precedes only the gated arm" assertions below could pass vacuously.
    assert!(
        out.contains("Status::Active => Self::Active,"),
        "ungated variant's binding->core arm missing, fixture no longer exercises conversion:\n{out}"
    );
    assert!(
        out.contains("test_lib::Status::Active => Self::Active,"),
        "ungated variant's core->binding arm missing, fixture no longer exercises conversion:\n{out}"
    );

    assert!(
        out.contains("#[cfg(feature = \"beta\")]\n            Status::Beta => Self::Beta,"),
        "host-owned cfg-gated variant must keep its binding->core arm under a matching #[cfg(...)] guard:\n{out}"
    );
    assert!(
        out.contains("#[cfg(feature = \"beta\")]\n            test_lib::Status::Beta => Self::Beta,"),
        "host-owned cfg-gated variant must keep its core->binding arm under a matching #[cfg(...)] guard:\n{out}"
    );
    // A later regression right next to this one: `catch_all` added `_ => Self::default(),` in
    // both directions whenever ANY variant carried a cfg, host-owned or not. A host-owned
    // variant's arm carries the identical `#[cfg(...)]` as the variant itself, so the two always
    // compile in or out together and the match stays exhaustive either way -- the catch-all is
    // unreachable and trips `-D warnings`' `unreachable_patterns` the moment the gating feature
    // is active (the default once cfg features are forwarded, alef #464).
    assert!(
        !out.contains("_ => Self::default(),"),
        "a host-owned cfg-gated variant alone must not trigger a catch-all (unreachable pattern \
         under -D warnings):\n{out}"
    );
}

/// Foreign-owned enum (`rust_path` does not start with the configured core crate name): the
/// cfg-gated variant's arm must be dropped entirely in BOTH directions -- the generated crate
/// never declares a Cargo feature for a foreign crate's gate, so forwarding it is
/// `unexpected cfg condition value`, and because `cfg(test)`-shaped gates are satisfied under
/// `cargo clippy --all-targets`, an ungated arm naming a variant that may not exist would still
/// compile and fail `E0599`. The ungated sibling variant's arm must still be present.
///
/// Fails on pre-fix code the same way as the host-owned test: the original code never dropped
/// any arm and never guarded any arm, so `External::Bar` (and `foreign_crate::External::Bar`)
/// appear unconditionally in pre-fix output -- the `!out.contains(...)` assertions below fail.
///
/// `make_config()` configures no features at all, so the gating feature "extra" is provably NOT
/// configured for this binding -- the foreign `Bar` variant can never exist in this build. Per
/// alef #547, the catch-all must therefore be OMITTED (it would otherwise be an unreachable
/// pattern under `-D warnings`); see the sibling positive-control test below for the case where
/// the feature IS configured. ~keep
#[test]
fn foreign_owned_cfg_gated_variant_proven_unreachable_drops_arm_and_catch_all() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "External".to_string(),
            rust_path: "foreign_crate::External".to_string(),
            variants: vec![
                gated_variant("Foo", None),
                gated_variant("Bar", Some(r#"feature = "extra""#)),
            ],
            ..Default::default()
        }],
        functions: vec![returning_function("get_external", "External")],
        ..Default::default()
    };

    let out = generate_r(&api);

    assert!(
        out.contains("External::Foo => Self::Foo,"),
        "ungated variant's binding->core arm missing, fixture no longer exercises conversion:\n{out}"
    );
    assert!(
        out.contains("foreign_crate::External::Foo => Self::Foo,"),
        "ungated variant's core->binding arm missing, fixture no longer exercises conversion:\n{out}"
    );

    assert!(
        !out.contains("External::Bar"),
        "a foreign crate's cfg-gated variant must not be named anywhere in the conversion output:\n{out}"
    );
    assert!(
        !out.contains(r#"#[cfg(feature = "extra")]"#),
        "a foreign crate's cfg gate must never be forwarded into this generated crate:\n{out}"
    );
    assert!(
        !out.contains("_ => Self::default(),"),
        "a foreign cfg-gated variant proven unreachable by this binding's own configured feature \
         set must not leave behind an unreachable catch-all (a cargo clippy -D warnings failure), \
         got:\n{out}"
    );
}

/// Positive control for the test above: when the gating feature IS configured (so the foreign
/// variant is NOT proven unreachable), the catch-all must still be emitted -- otherwise the fix
/// would have overcorrected into "never emit a catch-all," which trades one build failure
/// (unreachable pattern) for another (non-exhaustive match, since the arm itself is still always
/// dropped for a foreign variant). ~keep
#[test]
fn foreign_owned_cfg_gated_variant_not_proven_unreachable_keeps_catch_all() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "External".to_string(),
            rust_path: "foreign_crate::External".to_string(),
            variants: vec![
                gated_variant("Foo", None),
                gated_variant("Bar", Some(r#"feature = "extra""#)),
            ],
            ..Default::default()
        }],
        functions: vec![returning_function("get_external", "External")],
        ..Default::default()
    };

    let out = generate_r_with_config(&api, &make_config_with_feature("extra"));

    assert!(
        !out.contains("External::Bar"),
        "a foreign crate's cfg-gated variant must not be named anywhere in the conversion output:\n{out}"
    );
    assert!(
        out.contains("_ => Self::default(),"),
        "a foreign cfg-gated variant that is NOT proven unreachable must keep the catch-all so the \
         match stays exhaustive, got:\n{out}"
    );
}

/// Negative control: an enum with no cfg-gated variant at all must emit no `#[cfg(...)]` guard
/// and no catch-all fallback arm in its conversion impls. Without this, a "fix" that always adds
/// a catch-all (or that drops every gated arm indiscriminately regardless of ownership) would
/// still make the two tests above pass while quietly changing behavior for the common case.
#[test]
fn ungated_enum_emits_no_cfg_guard_and_no_catch_all() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "Plain".to_string(),
            rust_path: "test_lib::Plain".to_string(),
            variants: vec![gated_variant("On", None), gated_variant("Off", None)],
            ..Default::default()
        }],
        functions: vec![returning_function("get_plain", "Plain")],
        ..Default::default()
    };

    let out = generate_r(&api);

    assert!(
        out.contains("Plain::On => Self::On,") && out.contains("Plain::Off => Self::Off,"),
        "both ungated variants' binding->core arms must be present:\n{out}"
    );
    assert!(
        !out.contains("#[cfg("),
        "an ungated enum must not emit any #[cfg(...)] guard:\n{out}"
    );
    assert!(
        !out.contains("_ => Self::default()"),
        "an ungated enum with no data/excluded variants must not emit a catch-all fallback arm:\n{out}"
    );
}
