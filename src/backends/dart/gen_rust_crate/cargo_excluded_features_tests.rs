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
