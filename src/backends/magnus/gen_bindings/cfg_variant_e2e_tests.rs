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

/// Companion to this file's other alef #544 test: Magnus's own enum declaration
/// (`classes::gen_enum`) now consults `enum_variant_declaration` too, so a FOREIGN variant this
/// binding's own configured feature set proves unreachable is dropped from the declared
/// `RoutingStrategy` Rust type itself -- not just from the conversion arms. `impl From<
/// RoutingStrategy> for dep_crate::RoutingStrategy` matches over that declaration, so once it no
/// longer declares `Extra`, the match is exhaustive without a catch-all; keeping one anyway would
/// be an unreachable pattern under `cargo clippy -D warnings`. Before `magnus_conv_config` set
/// `declaration_drops_unreachable_foreign_variants: true` to match, this assertion inverted --
/// the catch-all was still required because the declaration hadn't caught up yet (see this test's
/// prior form, which asserted the catch-all stayed). ~keep
#[test]
fn generate_bindings_omits_binding_to_core_catch_all_for_foreign_variant_proven_unreachable_end_to_end() {
    let api = foreign_cfg_enum_api_with_param_function();
    let config = magnus_config_with_feature(None);
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = binding_to_core_conversion(lib_rs);

    assert!(
        !conversion.contains("_ => Default::default(),"),
        "Magnus's enum declaration now drops a foreign variant proven unreachable, so the \
         binding->core match is exhaustive without a catch-all -- keeping one is an unreachable \
         pattern (a cargo clippy -D warnings failure), got:\n{conversion}"
    );
    assert!(
        !conversion.contains("Extra"),
        "the dropped foreign variant must not be named anywhere in the binding->core conversion, \
         got:\n{conversion}"
    );
}

/// Positive control for the test above: when the gating feature IS configured (so the foreign
/// variant is NOT proven unreachable), Magnus's declaration still keeps `Extra` unconditionally
/// (`enum_variant_declaration` only drops a PROVEN-unreachable foreign variant), so the
/// binding->core match must keep its catch-all -- otherwise the fix would have overcorrected into
/// "never emit a catch-all," trading one build failure (unreachable pattern) for another
/// (non-exhaustive match).
#[test]
fn generate_bindings_keeps_binding_to_core_catch_all_for_foreign_variant_not_proven_unreachable_end_to_end() {
    let api = foreign_cfg_enum_api_with_param_function();
    let config = magnus_config_with_feature(Some("extra-tier"));
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let conversion = binding_to_core_conversion(lib_rs);

    assert!(
        conversion.contains("_ => Default::default(),"),
        "a foreign cfg-gated variant that is NOT proven unreachable is still declared \
         unconditionally, so the binding->core match must keep its catch-all, got:\n{conversion}"
    );
}

fn returning_function(name: &str, enum_name: &str) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        return_type: TypeRef::Named(enum_name.to_string()),
        ..Default::default()
    }
}

/// Three-variant fixture (two always-present, one foreign cfg-gated) for the DECLARATION-set
/// tests below -- distinct from `foreign_cfg_enum_api`'s two-variant shape so "the excluded
/// variant is gone" and "the other variants are still all there" are both actually exercised.
fn foreign_cfg_enum_api_three_variants() -> ApiSurface {
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
                    name: "Secondary".to_string(),
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

/// Extracts the `pub enum RoutingStrategy { ... }` declaration body from generated `lib.rs`.
fn wrapper_enum_declaration(lib_rs: &str) -> &str {
    let start = lib_rs
        .find("pub enum RoutingStrategy {")
        .expect("generated crate must declare the RoutingStrategy wrapper enum");
    let end = lib_rs[start..]
        .find("\n}")
        .map(|i| start + i + 2)
        .expect("enum declaration must close");
    &lib_rs[start..end]
}

/// The exact set of variant names declared in a unit-enum body -- every line shaped `Name,` (no
/// discriminant, unlike PyO3's), never a bare substring match.
fn declared_variant_names(rendered: &str) -> std::collections::BTreeSet<String> {
    // ~keep Parse by line, not by `,`: this renderer emits one variant per line with no
    // discriminant, so a comma-split would fold `pub enum X {` into the first variant's fragment
    // and silently drop that variant. The sibling parsers in `codegen::generators::enums` and the
    // extendr tests split on `,` because THEIR renderers emit the whole declaration on one line
    // with `= N` discriminants. Same question, three output shapes; do not unify them.
    rendered
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let name = trimmed.strip_suffix(',')?;
            let is_variant_ident =
                name.starts_with(|c: char| c.is_ascii_uppercase()) && name.chars().all(|c| c.is_alphanumeric());
            is_variant_ident.then(|| name.to_string())
        })
        .collect()
}

fn names(values: &[&str]) -> std::collections::BTreeSet<String> {
    values.iter().map(|s| s.to_string()).collect()
}

/// THE gap this task fixes at the declaration surface: a FOREIGN enum with one cfg-excluded
/// variant and two retained ones renders exactly the retained set in Magnus's own enum
/// declaration. Positive control (same fixture, feature configured) proves the drop is
/// conditional on the proof, not a blanket foreign-owned rule.
#[test]
fn generate_bindings_declares_exact_retained_variant_set_for_foreign_variant_proven_unreachable() {
    let api = foreign_cfg_enum_api_three_variants();

    let excluded_config = magnus_config_with_feature(None);
    let excluded_files = MagnusBackend.generate_bindings(&api, &excluded_config).unwrap();
    let excluded_decl = wrapper_enum_declaration(lib_rs_content(&excluded_files));
    assert_eq!(
        declared_variant_names(excluded_decl),
        names(&["Primary", "Secondary"]),
        "the declared set must be exactly the two retained variants, got:\n{excluded_decl}"
    );

    let active_config = magnus_config_with_feature(Some("extra-tier"));
    let active_files = MagnusBackend.generate_bindings(&api, &active_config).unwrap();
    let active_decl = wrapper_enum_declaration(lib_rs_content(&active_files));
    assert_eq!(
        declared_variant_names(active_decl),
        names(&["Primary", "Secondary", "Extra"]),
        "with \"extra-tier\" configured, the declared set must include the retained foreign \
         variant, got:\n{active_decl}"
    );
}

/// Host-owned control: a variant behind a HOST-owned cfg gate must never be dropped from the
/// declaration, regardless of `configured_features` -- `enum_variant_declaration` never resolves
/// a host-owned gate to `Drop`.
#[test]
fn generate_bindings_never_drops_host_owned_cfg_variant_from_declaration() {
    let mut api = foreign_cfg_enum_api_three_variants();
    api.enums[0].rust_path = "test_lib::RoutingStrategy".to_string();
    let config = magnus_config_with_feature(None);
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let decl = wrapper_enum_declaration(lib_rs_content(&files));

    assert_eq!(
        declared_variant_names(decl),
        names(&["Primary", "Secondary", "Extra"]),
        "a host-owned cfg-gated variant must stay declared even with no features configured, \
         got:\n{decl}"
    );
}

/// Magnus-specific surface pyo3 doesn't have: a DATA-carrying enum's declaration also filters
/// through `enum_variant_declaration` (unlike PyO3, which skips data enums entirely --
/// `enum_has_data_variants` -- and represents them as a struct wrapper instead). A foreign
/// cfg-gated struct variant proven unreachable must not appear in the declared Rust enum body.
#[test]
fn generate_bindings_declares_exact_retained_variant_set_for_data_enum_foreign_variant() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: "PageAction".to_string(),
            rust_path: "dep_crate::PageAction".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Scrape".to_string(),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Click".to_string(),
                    fields: vec![crate::core::ir::FieldDef {
                        name: "selector".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                EnumVariant {
                    name: "Testkit".to_string(),
                    fields: vec![crate::core::ir::FieldDef {
                        name: "note".to_string(),
                        ty: TypeRef::String,
                        ..Default::default()
                    }],
                    cfg: Some(r#"feature = "testkit""#.to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = magnus_config_with_feature(None);
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let decl_start = lib_rs
        .find("pub enum PageAction {")
        .expect("generated crate must declare the PageAction data enum");
    let decl_end = lib_rs[decl_start..]
        .find("\n}")
        .map(|i| decl_start + i + 2)
        .expect("enum declaration must close");
    let decl = &lib_rs[decl_start..decl_end];

    assert!(
        decl.contains("Scrape,") && decl.contains("Click { selector: String },"),
        "the two always-present variants must still be declared, got:\n{decl}"
    );
    assert!(
        !decl.contains("Testkit"),
        "the excluded data variant must not appear anywhere in the declaration, got:\n{decl}"
    );
}

/// Edge case: the variant marked `#[default]` is itself the FOREIGN one a configured feature set
/// proves unreachable. `impl Default for RoutingStrategy` must fall back to another declared
/// variant instead of referencing a variant name the declaration no longer emits, which would not
/// compile.
#[test]
fn generate_bindings_default_variant_selection_skips_a_variant_dropped_from_the_declaration() {
    let api = ApiSurface {
        crate_name: "test-lib".to_string(),
        version: "0.1.0".to_string(),
        enums: vec![EnumDef {
            name: "RoutingStrategy".to_string(),
            rust_path: "dep_crate::RoutingStrategy".to_string(),
            variants: vec![
                EnumVariant {
                    name: "Extra".to_string(),
                    is_default: true,
                    cfg: Some(r#"feature = "extra-tier""#.to_string()),
                    ..Default::default()
                },
                EnumVariant {
                    name: "Primary".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        functions: vec![returning_function("get_strategy", "RoutingStrategy")],
        ..Default::default()
    };
    let config = magnus_config_with_feature(None);
    let files = MagnusBackend.generate_bindings(&api, &config).unwrap();
    let lib_rs = lib_rs_content(&files);
    let decl = wrapper_enum_declaration(lib_rs);

    assert!(
        !decl.contains("Extra"),
        "the dropped default variant must not appear in the declared set, got:\n{decl}"
    );
    assert!(
        lib_rs.contains("fn default() -> Self { Self::Primary }"),
        "impl Default must fall back to a variant the declaration actually keeps, got:\n{lib_rs}"
    );
}
