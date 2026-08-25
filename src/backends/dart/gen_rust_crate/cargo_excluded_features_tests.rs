//! Regression for alef-task #331: `excluded_default_features` only trimmed the wrapper's own
//! `[features] default = [...]` array (built from cfg-gated enum variants -- see
//! `cargo::feature_cfg_tests::cargo_toml_excludes_named_features_from_default_but_keeps_forwarding_entries`),
//! never the core dependency's own explicit `features = [...]` line (built from
//! `[crates.dart].features`). Forwarding an excluded name into that line unions it straight back
//! into the core crate via Cargo's feature unification, defeating a `target_dep_overrides` entry
//! that turned it off for a specific cfg target -- the same defect
//! `RubyConfig::excluded_default_features` fixed for the Magnus crate, generalized here through
//! the shared `scaffold::core_dep_features_excluding` helper. ~keep
//!
//! Kept as a sibling file (matching `cargo_sort_order_tests.rs`) rather than a submodule of
//! `cargo.rs` so this coverage does not push that emitter file over the repo's 1,000-line cap.

use super::cargo::emit_cargo_toml;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::languages::DartConfig;
use crate::core::ir::ApiSurface;

/// Asserts both directions: the excluded name is dropped from the core dep line, and a name
/// nobody excluded still is forwarded.
#[test]
fn cargo_toml_excludes_named_feature_from_core_dep_line_but_keeps_others() {
    let api = ApiSurface::default();
    let config = ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        dart: Some(DartConfig {
            features: Some(vec!["native-http".to_string(), "wasm-http".to_string()]),
            excluded_default_features: vec!["native-http".to_string()],
            ..Default::default()
        }),
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");

    let core_dep_line = file
        .content
        .lines()
        .find(|l| l.trim_start().starts_with("sample_lib ="))
        .expect("core dependency line must be emitted");
    assert!(
        !core_dep_line.contains("native-http"),
        "excluded_default_features must drop the name from the core dependency's own explicit \
         features = [...] line, not just the wrapper's default array:\n{core_dep_line}"
    );
    assert!(
        core_dep_line.contains("wasm-http"),
        "a feature nobody excluded must still be forwarded to the core dependency line:\n{core_dep_line}"
    );
    toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
}

/// Regression for alef-task #371: an `excluded_default_features` name that gates no item in the
/// extracted API surface (e.g. a Cargo-only feature that only affects a dependency's `build.rs`
/// linking, such as `libheif-sys` via `heic` -- the doc comment's own example) is never
/// discovered by `shared_cfg::collect_cfg_features`, which walks `#[cfg(feature = "X")]`
/// attributes on IR nodes. The `[features]` table was built exclusively from that discovery set,
/// so a config-only name never got its promised opt-in forwarding entry (`<name> =
/// ["<core>/<name>"]`) at all -- breaking `cargo build -p <crate>-dart --features <name>` on
/// desktop, exactly the escape hatch `excluded_default_features` documents as always available.
/// `cargo_toml_excludes_named_features_from_default_but_keeps_forwarding_entries` in `cargo.rs`
/// does not catch this: it excludes `heic` from an enum variant `#[cfg(feature = "heic")]`, so
/// `heic` IS discoverable there and only exercises the already-working half.
#[test]
fn cargo_toml_forwards_excluded_feature_not_referenced_by_any_cfg_attribute() {
    let api = ApiSurface::default();
    let config = ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        dart: Some(DartConfig {
            excluded_default_features: vec!["heic".to_string()],
            ..Default::default()
        }),
        ..Default::default()
    };
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");

    assert!(
        file.content.contains("[features]"),
        "a config-only excluded_default_features name must still produce a [features] table:\n{}",
        file.content
    );
    assert!(
        file.content.contains(r#"heic = ["sample_lib/heic"]"#),
        "a config-only excluded_default_features name (not referenced by any #[cfg(feature = ...)] \
         in the API surface) must still get a forwarding entry so `cargo build --features heic` \
         keeps working:\n{}",
        file.content
    );
    let default_line = file
        .content
        .lines()
        .find(|l| l.starts_with("default = ["))
        .expect("default = [...] line must be emitted");
    assert!(
        !default_line.contains("\"heic\""),
        "default = [...] must NOT contain excluded `heic`; got: {default_line}"
    );
    toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
}
