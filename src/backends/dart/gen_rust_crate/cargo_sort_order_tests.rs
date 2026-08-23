//! `cargo sort --check` conformance for the Dart bridge crate's `Cargo.toml`.
//!
//! Unlike the scaffold-emitted binding crates (python/node/php/ffi/…), the Dart bridge crate's
//! manifest is never written by `alef scaffold` -- it is regenerated on every `alef generate` /
//! `alef all` by [`super::cargo::emit_cargo_toml`], so it never went through the scaffold sweep
//! in `src/scaffold/tests/cargo_table_order.rs` / `dependency_key_order.rs`. Nothing previously
//! drove this emitter's real output through `cargo_sort_order`'s checker. ~keep

use super::cargo::emit_cargo_toml;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FunctionDef};
use crate::test_support::cargo_sort_order::{assert_canonical_table_order, assert_dependency_keys_sorted};

fn minimal_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "sample-lib".to_string(),
        ..Default::default()
    }
}

/// A plain manifest with only the always-emitted tables.
#[test]
fn plain_manifest_is_cargo_sort_clean() {
    let api = ApiSurface::default();
    let config = minimal_config();
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");
    assert_canonical_table_order("dart Cargo.toml", &file.content);
    assert!(
        assert_dependency_keys_sorted("dart Cargo.toml", &file.content) > 0,
        "expected at least one dependency key to have been compared:\n{}",
        file.content
    );
}

/// A manifest carrying every optional table this emitter can produce: `[features]` (cfg-gated
/// enum variant), `[target.'cfg(...)'.dependencies]` (target dep override), and the trailing
/// `[lints.rust]` block that must sort after every dependency table.
#[test]
fn manifest_with_features_and_target_overrides_is_cargo_sort_clean() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "ImageOutputFormat".to_string(),
            variants: vec![EnumVariant {
                name: "Heic".to_string(),
                cfg: Some("feature = \"heic\"".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        functions: vec![FunctionDef {
            name: "native_ping".to_string(),
            rust_path: "sample_lib::native_ping".to_string(),
            is_async: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut config = minimal_config();
    config.dart = Some(crate::core::config::languages::DartConfig {
        target_dep_overrides: vec![crate::core::config::languages::DartTargetDepOverride {
            cfg: "target_os = \"macos\"".to_string(),
            features: vec!["heic".to_string()],
            default_features: true,
        }],
        ..Default::default()
    });
    let file = emit_cargo_toml("packages/dart/rust", &api, &config, "sample_lib");
    assert_canonical_table_order("dart Cargo.toml (features + target overrides)", &file.content);
    assert!(
        assert_dependency_keys_sorted("dart Cargo.toml (features + target overrides)", &file.content) > 0,
        "expected at least one dependency key to have been compared:\n{}",
        file.content
    );
    toml::from_str::<toml::Value>(&file.content).expect("generated Cargo.toml must be valid TOML");
}
