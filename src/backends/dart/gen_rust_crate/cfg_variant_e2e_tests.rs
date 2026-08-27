//! End-to-end regression coverage for alef #547: a FOREIGN (dependency-owned) cfg-gated enum
//! variant run through the REAL `gen_rust_crate::emit` path, not a direct
//! `enum_conversions::emit_from_impl_for_enum` call. Mirrors
//! `backends::wasm::gen_bindings::cfg_variant_e2e_tests`, the pattern task #538 established for
//! wasm and task #544 extended to rustler/magnus/pyo3/php.
//!
//! `enum_conversions.rs` computed its own `has_cfg_variants` locally, ignoring both host/foreign
//! ownership AND the crate's configured feature set entirely -- so a foreign cfg-gated variant
//! this binding's own configured feature set proves unreachable still produced a trailing
//! `_ => unreachable!(...)` catch-all arm. The Dart bridge crate blanket-allows
//! `unreachable_patterns` at the crate root, so this never failed a Dart build under `-D
//! warnings`, but the generated code is still wrong in the same way every other backend's was
//! before task #544/#547 -- and the crate-level allow is not a promise this task is free to widen.

use super::emit;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant};

fn dart_config_with_feature(configured_feature: Option<&str>) -> ResolvedCrateConfig {
    let features_line = configured_feature
        .map(|f| format!("features = [\"{f}\"]\n"))
        .unwrap_or_default();
    let toml_src = format!(
        "[workspace]\nlanguages = [\"dart\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.dart]\n{features_line}"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A different first path segment than the crate's own name ("test_lib") is what
/// `is_host_owned_rust_path` reads to classify this enum -- and every one of its cfg-gated
/// variants -- as FOREIGN. `emit_from_impl_for_enum` (the `impl From<CoreType> for MirrorType`
/// direction) is emitted unconditionally for every enum in the API surface, unlike the
/// mirror-to-core direction which only fires for enums reachable as function parameters. ~keep
fn foreign_cfg_enum_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: "RoutingStrategy".to_string(),
            rust_path: "dep_crate::RoutingStrategy".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Primary".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Extra".to_string(),
                    cfg: Some(r#"feature = "extra-tier""#.to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn lib_rs_content(files: &[crate::core::backend::GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("emit must produce the bridge crate's lib.rs")
        .content
}

fn core_to_binding_conversion(lib_rs: &str) -> &str {
    let start = lib_rs
        .find("impl From<dep_crate::RoutingStrategy> for RoutingStrategy {")
        .expect("generated crate must convert the foreign enum from core to the mirror type");
    let end = lib_rs[start..]
        .find("\n}")
        .map(|i| start + i + 2)
        .expect("conversion impl must close");
    &lib_rs[start..end]
}

/// alef #547: `emit_from_impl_for_enum` computed `has_cfg_variants` as "any variant has a cfg"
/// (host-owned or foreign, ignoring the binding's own feature set entirely), so it always assumed
/// a foreign cfg-gated variant might still exist -- emitting a trailing
/// `_ => unreachable!(...)` catch-all that is unreachable code once the binding's own feature set
/// actually proves the foreign variant can never appear.
#[test]
fn emit_omits_unreachable_catch_all_for_foreign_variant_proven_unreachable_end_to_end() {
    let api = foreign_cfg_enum_api();
    // The binding does NOT enable "extra-tier", so the foreign `Extra` variant is provably
    // unreachable for this build: the dependency itself never compiles that variant in.
    let config = dart_config_with_feature(None);
    let files = emit(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = core_to_binding_conversion(lib_rs);

    assert!(
        !conversion.contains("_ => unreachable!"),
        "a foreign cfg-gated variant proven unreachable by this binding's own configured feature \
         set must not leave behind a dead catch-all, got:\n{conversion}"
    );
}

/// Positive control for the test above: when the gating feature IS configured (so the foreign
/// variant is NOT proven unreachable), the catch-all must still be emitted -- otherwise the fix
/// would have overcorrected into "never emit a catch-all," which trades a dead-arm defect for a
/// non-exhaustive match (the arm itself is still always dropped for a foreign variant). ~keep
#[test]
fn emit_keeps_catch_all_for_foreign_variant_not_proven_unreachable_end_to_end() {
    let api = foreign_cfg_enum_api();
    let config = dart_config_with_feature(Some("extra-tier"));
    let files = emit(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = core_to_binding_conversion(lib_rs);

    assert!(
        conversion.contains("_ => unreachable!"),
        "a foreign cfg-gated variant that is NOT proven unreachable must keep the catch-all so the \
         match stays exhaustive, got:\n{conversion}"
    );
}
