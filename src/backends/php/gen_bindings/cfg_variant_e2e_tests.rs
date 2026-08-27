//! End-to-end regression coverage for alef #544: a FOREIGN (dependency-owned) cfg-gated tagged
//! data enum variant run through the REAL `PhpBackend::generate_bindings` path, not a direct
//! `types::enums::gen_flat_data_enum_from_impls` call. Mirrors
//! `backends::wasm::gen_bindings::cfg_variant_e2e_tests`, the pattern task #538 established for
//! wasm.
//!
//! Unlike magnus/pyo3/napi/wasm, PHP's tagged-data-enum `From` impl never routes through
//! `codegen::conversions::gen_enum_from_*_cfg`/`ConversionConfig` at all -- `is_tagged_data_enum`
//! enums go through the bespoke `gen_flat_data_enum_from_impls` generator instead, which computed
//! its own `has_cfg_variants` locally and ignored `configured_features` entirely (the same
//! bypass task #544 found and fixed in Rustler's flat-data-enum generator).

use super::PhpBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeRef};

fn php_config_with_feature(configured_feature: Option<&str>) -> ResolvedCrateConfig {
    let features_line = configured_feature
        .map(|f| format!("features = [\"{f}\"]\n"))
        .unwrap_or_default();
    let toml_src = format!(
        "[workspace]\nlanguages = [\"php\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.php]\n{features_line}"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A different first path segment than the crate's own `core_import` ("test_lib") is what
/// `is_host_owned_rust_path` reads to classify this enum -- and every one of its cfg-gated
/// variants -- as FOREIGN. `serde_tag` plus a data-carrying variant is what
/// `is_tagged_data_enum` requires to route this enum through `gen_flat_data_enum_from_impls`
/// rather than the plain string-enum representation. ~keep
fn foreign_cfg_tagged_data_enum_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: "RoutingStrategy".to_string(),
            rust_path: "dep_crate::RoutingStrategy".to_string(),
            serde_tag: Some("type".to_string()),
            variants: vec![
                EnumVariant {
                    name: "Primary".to_string(),
                    fields: vec![FieldDef {
                        name: "target".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
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
        .expect("generate_bindings must emit lib.rs")
        .content
}

fn core_to_binding_conversion(lib_rs: &str) -> &str {
    let start = lib_rs
        .find("impl From<dep_crate::RoutingStrategy> for RoutingStrategy {")
        .expect("generated crate must convert the foreign tagged data enum from core to the binding type");
    let end = lib_rs[start..]
        .find("\n}")
        .map(|i| start + i + 2)
        .expect("conversion impl must close");
    &lib_rs[start..end]
}

/// alef #544: `gen_flat_data_enum_from_impls` computed `has_cfg_variants` as "any variant has a
/// cfg" (host-owned or foreign) and ignored `configured_features` entirely, so it always assumed
/// a foreign cfg-gated variant might still exist -- emitting a trailing `_ => Default::default()`
/// catch-all that is unreachable (a `cargo clippy -D warnings` failure) once the binding's own
/// feature set actually proves the foreign variant can never appear.
#[test]
fn generate_bindings_omits_unreachable_catch_all_for_foreign_variant_proven_unreachable_end_to_end() {
    let api = foreign_cfg_tagged_data_enum_api();
    // The binding does NOT enable "extra-tier", so the foreign `Extra` variant is provably
    // unreachable for this build: the dependency itself never compiles that variant in.
    let config = php_config_with_feature(None);
    let files = PhpBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = core_to_binding_conversion(lib_rs);

    assert!(
        !conversion.contains("_ => Default::default(),"),
        "a foreign cfg-gated variant proven unreachable by this binding's own configured feature \
         set must not leave behind an unreachable catch-all (a cargo clippy -D warnings failure), \
         got:\n{conversion}"
    );
}

/// Positive control for the test above: when the gating feature IS configured (so the foreign
/// variant is NOT proven unreachable), the catch-all must still be emitted -- otherwise the fix
/// would have overcorrected into "never emit a catch-all," which trades one build failure
/// (unreachable pattern) for another (non-exhaustive match, since the arm itself is still always
/// dropped for a foreign variant -- see `gen_flat_data_enum_from_impls`'s own doc comment). ~keep
#[test]
fn generate_bindings_keeps_catch_all_for_foreign_variant_not_proven_unreachable_end_to_end() {
    let api = foreign_cfg_tagged_data_enum_api();
    let config = php_config_with_feature(Some("extra-tier"));
    let files = PhpBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = core_to_binding_conversion(lib_rs);

    assert!(
        conversion.contains("_ => Default::default(),"),
        "a foreign cfg-gated variant that is NOT proven unreachable must keep the catch-all so the \
         match stays exhaustive, got:\n{conversion}"
    );
}
