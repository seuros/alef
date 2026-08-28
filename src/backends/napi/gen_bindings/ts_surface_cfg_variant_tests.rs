//! Regression coverage for the TypeScript half of the cfg-unreachable foreign variant fix.
//!
//! `enums::gen_enum` learned to drop a FOREIGN (dependency-owned) cfg-gated enum variant that
//! this binding's own configured feature set proves unreachable, and the `From` impls followed --
//! but alef's two TypeScript surfaces did not. Both re-derived the variant list straight from
//! `EnumDef::variants` instead of asking `codegen::conversions::enum_variant_declaration`, the
//! authority the Rust emitter consults:
//!
//! - the public `index.d.ts` OVERLAY alef writes (`errors::gen_dts`, reached through
//!   `Backend::generate_type_stubs`), which is distinct from the `index.native.d.ts` napi's own
//!   build emits from the compiled Rust and which was already correct;
//! - the `#[napi(ts_type = "...")]` attribute on a generated struct field
//!   (`types::ts_type_for_string_enum_field`).
//!
//! The result was a public TypeScript API advertising a string literal the generated Rust enum
//! has no variant for: a consumer writing it type-checks clean and fails at runtime. Both
//! assertions below run through the REAL `Backend` entry points, not a direct emitter call, so a
//! fix that lands in a helper without being threaded to the surface still fails here.

use super::NapiBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

const GATING_FEATURE: &str = "experimental-transport";

fn napi_config_with_feature(configured_feature: Option<&str>) -> ResolvedCrateConfig {
    let features_line = configured_feature
        .map(|f| format!("features = [\"{f}\"]\n"))
        .unwrap_or_default();
    let toml_src = format!(
        "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.node]\n{features_line}"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A plain all-unit-variant enum, so `gen_enum` emits it as `#[napi(string_enum)]` -- the only
/// shape that reaches both TypeScript surfaces under test. Its `rust_path` starts with a segment
/// other than the crate's own `core_import` ("test_lib"), which is what
/// `is_host_owned_rust_path` reads to classify the enum, and therefore its cfg-gated variant, as
/// FOREIGN. A struct field typed as the enum is what makes `gen_struct` emit the
/// `#[napi(ts_type = "...")]` union. ~keep
fn foreign_cfg_string_enum_api() -> ApiSurface {
    ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: "TransportMode".to_string(),
            rust_path: "dep_crate::TransportMode".to_string(),
            serde_rename_all: Some("snake_case".to_string()),
            variants: vec![
                EnumVariant {
                    name: "Direct".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Buffered".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Experimental".to_string(),
                    cfg: Some(format!(r#"feature = "{GATING_FEATURE}""#)),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        types: vec![TypeDef {
            name: "TransportOptions".to_string(),
            rust_path: "test_lib::TransportOptions".to_string(),
            fields: vec![FieldDef {
                name: "mode".to_string(),
                ty: TypeRef::Named("TransportMode".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn index_dts(config: &ResolvedCrateConfig, api: &ApiSurface) -> String {
    let files = NapiBackend.generate_type_stubs(api, config).unwrap();
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("index.d.ts"))
        .expect("generate_type_stubs must emit index.d.ts")
        .content
        .clone()
}

fn ts_type_attribute(config: &ResolvedCrateConfig, api: &ApiSurface) -> String {
    let files = NapiBackend.generate_bindings(api, config).unwrap();
    let lib_rs = &files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("lib.rs"))
        .expect("generate_bindings must emit lib.rs")
        .content;
    let start = lib_rs
        .find("ts_type = \"")
        .expect("the TransportOptions.mode field must carry a ts_type string-enum union");
    let rest = &lib_rs[start + "ts_type = \"".len()..];
    let end = rest.find('"').expect("ts_type literal must close");
    rest[..end].to_string()
}

/// The defect: the overlay declared a member for the foreign cfg-gated variant the generated Rust
/// enum omits entirely, so `TransportMode.Experimental` / `'experimental'` type-checked against a
/// value no build of this binding can produce.
#[test]
fn index_dts_overlay_omits_foreign_variant_proven_unreachable() {
    let api = foreign_cfg_string_enum_api();
    // The binding does NOT enable the gating feature, so the dependency never compiles the
    // `Experimental` variant in and `gen_enum` drops it from the wrapper enum.
    let config = napi_config_with_feature(None);
    let dts = index_dts(&config, &api);

    assert!(
        !dts.contains("Experimental"),
        "the index.d.ts overlay must not declare a foreign cfg-gated enum member the generated \
         Rust enum omits -- a consumer could name it and never construct it, got:\n{dts}"
    );
}

/// Control for the test above: emptying the member list would also satisfy it, so pin that the
/// two REACHABLE variants survive in the overlay with their napi runtime string values. ~keep
#[test]
fn index_dts_overlay_keeps_reachable_variants() {
    let api = foreign_cfg_string_enum_api();
    let config = napi_config_with_feature(None);
    let dts = index_dts(&config, &api);

    for member in ["Direct = \"direct\",", "Buffered = \"buffered\","] {
        assert!(
            dts.contains(member),
            "the index.d.ts overlay must still declare the reachable member {member:?}, got:\n{dts}"
        );
    }
}

/// The second half of the same defect: the `#[napi(ts_type = "...")]` union on a field typed as
/// the enum enumerated the dropped variant's literal too.
#[test]
fn ts_type_attribute_omits_foreign_variant_proven_unreachable() {
    let api = foreign_cfg_string_enum_api();
    let config = napi_config_with_feature(None);
    let ts_type = ts_type_attribute(&config, &api);

    assert!(
        !ts_type.contains("'experimental'"),
        "the ts_type union must not offer the literal for a foreign cfg-gated variant the \
         generated Rust enum omits, got: {ts_type}"
    );
}

/// Control for the test above, for the same reason: an empty or nominal-only union must not pass.
#[test]
fn ts_type_attribute_keeps_reachable_variant_literals() {
    let api = foreign_cfg_string_enum_api();
    let config = napi_config_with_feature(None);
    let ts_type = ts_type_attribute(&config, &api);

    for literal in ["'direct'", "'buffered'"] {
        assert!(
            ts_type.contains(literal),
            "the ts_type union must still offer the reachable literal {literal}, got: {ts_type}"
        );
    }
}

/// Positive control across the whole fix: when the gating feature IS configured the variant is no
/// longer proven unreachable, `gen_enum` keeps it, and both TypeScript surfaces must keep it too.
/// Without this, "drop every cfg-gated foreign variant unconditionally" would pass every
/// assertion above while breaking a build that legitimately enables the feature. ~keep
#[test]
fn both_ts_surfaces_keep_foreign_variant_when_its_feature_is_configured() {
    let api = foreign_cfg_string_enum_api();
    let config = napi_config_with_feature(Some(GATING_FEATURE));

    let dts = index_dts(&config, &api);
    assert!(
        dts.contains("Experimental = \"experimental\","),
        "a foreign cfg-gated variant that is NOT proven unreachable must stay in the overlay, got:\n{dts}"
    );

    let ts_type = ts_type_attribute(&config, &api);
    assert!(
        ts_type.contains("'experimental'"),
        "a foreign cfg-gated variant that is NOT proven unreachable must stay in the ts_type union, \
         got: {ts_type}"
    );
}
