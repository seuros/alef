//! End-to-end regression coverage for alef #544: a FOREIGN (dependency-owned) cfg-gated enum
//! variant run through the REAL `MagnusBackend::generate_bindings` path, not a direct
//! `conversions::gen_enum_from_*_cfg` call. Mirrors
//! `backends::wasm::gen_bindings::cfg_variant_e2e_tests`, the pattern task #538 established for
//! wasm; this is the same defect in Magnus's `magnus_conv_config` construction site, which built
//! its `ConversionConfig` without `configured_features` set.

use super::MagnusBackend;
use crate::core::backend::Backend;
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant, FunctionDef, ParamDef, TypeRef};

fn magnus_config_with_feature(configured_feature: Option<&str>) -> ResolvedCrateConfig {
    let features_line = configured_feature
        .map(|f| format!("features = [\"{f}\"]\n"))
        .unwrap_or_default();
    let toml_src = format!(
        "[workspace]\nlanguages = [\"ruby\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.ruby]\ngem_name = \"test_lib\"\n{features_line}"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

/// A different first path segment than the crate's own `core_import` ("test_lib") is what
/// `is_host_owned_rust_path` reads to classify this enum -- and every one of its cfg-gated
/// variants -- as FOREIGN. ~keep
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

/// Like `foreign_cfg_enum_api`, but also declares a function taking the enum as a PARAMETER
/// (not just a return type) -- `impl From<BindingEnum> for CoreType` is only generated for
/// types `input_type_names` finds among parameter types, so the plain `foreign_cfg_enum_api`
/// fixture (return-type-only) never exercises the binding->core direction at all. ~keep
fn foreign_cfg_enum_api_with_param_function() -> ApiSurface {
    let mut api = foreign_cfg_enum_api();
    api.functions.push(FunctionDef {
        name: "set_routing_strategy".to_string(),
        rust_path: "test_lib::set_routing_strategy".to_string(),
        params: vec![ParamDef {
            name: "strategy".to_string(),
            ty: TypeRef::Named("RoutingStrategy".to_string()),
            ..Default::default()
        }],
        return_type: TypeRef::Unit,
        ..Default::default()
    });
    api
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
        .expect("generated crate must convert the foreign enum from core to the binding type");
    let end = lib_rs[start..]
        .find("\n}")
        .map(|i| start + i + 2)
        .expect("conversion impl must close");
    &lib_rs[start..end]
}

fn binding_to_core_conversion(lib_rs: &str) -> &str {
    let start = lib_rs
        .find("impl From<RoutingStrategy> for dep_crate::RoutingStrategy {")
        .expect("generated crate must convert the binding enum back to the foreign core type");
    let end = lib_rs[start..]
        .find("\n}")
        .map(|i| start + i + 2)
        .expect("conversion impl must close");
    &lib_rs[start..end]
}

/// alef #544: `magnus_conv_config` (the only `ConversionConfig` construction site in the Magnus
/// backend that reaches `gen_enum_from_core_to_binding_cfg`) never set `configured_features`, so
/// `codegen::conversions::enums::has_unresolved_foreign_cfg_variants` always saw `None` and had to
/// assume a foreign cfg-gated variant might still exist -- emitting a trailing
/// `_ => Default::default()` catch-all that is unreachable (a `cargo clippy -D warnings` failure)
/// once the binding's own feature set actually proves the foreign variant can never appear.
#[test]
fn generate_bindings_omits_unreachable_catch_all_for_foreign_variant_proven_unreachable_end_to_end() {
    let api = foreign_cfg_enum_api();
    // The binding does NOT enable "extra-tier", so the foreign `Extra` variant is provably
    // unreachable for this build: the dependency itself never compiles that variant in.
    let config = magnus_config_with_feature(None);
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
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
/// dropped for a foreign variant -- see `codegen::conversions::enums::emit_cfg_gated_arm`). ~keep
#[test]
fn generate_bindings_keeps_catch_all_for_foreign_variant_not_proven_unreachable_end_to_end() {
    let api = foreign_cfg_enum_api();
    let config = magnus_config_with_feature(Some("extra-tier"));
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = core_to_binding_conversion(lib_rs);

    assert!(
        conversion.contains("_ => Default::default(),"),
        "a foreign cfg-gated variant that is NOT proven unreachable must keep the catch-all so the \
         match stays exhaustive, got:\n{conversion}"
    );
}

/// THE E0004 REGRESSION this task fixes, reproduced end to end: Magnus's own enum declaration
/// (`classes::gen_enum`) declares every variant unconditionally -- it never consults
/// `configured_features` at all -- so `Extra` is still part of the real generated
/// `RoutingStrategy` Rust type even though this binding's own configured feature set proves the
/// CORE dependency's own `Extra` variant unreachable. `impl From<RoutingStrategy> for
/// dep_crate::RoutingStrategy` matches over that declared type, not the core type, so dropping
/// its catch-all on the core-side proof leaves a real gap: `error[E0004]: non-exhaustive
/// patterns`. ~keep
#[test]
fn generate_bindings_keeps_binding_to_core_catch_all_for_foreign_variant_proven_unreachable_end_to_end() {
    let api = foreign_cfg_enum_api_with_param_function();
    let config = magnus_config_with_feature(None);
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = binding_to_core_conversion(lib_rs);

    assert!(
        conversion.contains("_ => Default::default(),"),
        "Magnus's enum declaration declares every variant unconditionally regardless of \
         configured_features, so the binding->core match must keep its catch-all even when the \
         core dependency's own variant is proven unreachable -- omitting it is \
         error[E0004]: non-exhaustive patterns, got:\n{conversion}"
    );
}
