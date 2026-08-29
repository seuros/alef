//! End-to-end regression coverage for extendr's enum DECLARATION surfaces (fieldless enums via
//! the shared `codegen::generators::enums::gen_enum`, and the flat-data-enum struct/conversions
//! this module builds bespoke -- `bridges::gen_extendr_flat_data_enum_struct`/`_from_core`/
//! `_to_core`). Both used to ignore `EnumVariant::cfg` entirely on the declaration side while the
//! plain-enum conversion arms (`enum_conversions.rs`) already dropped a foreign cfg-gated
//! variant's arm when this binding's own configured feature set proved it unreachable -- an R
//! caller could construct the wrapper value the compiled dependency can never produce. The flat
//! struct is the second shape alef-task's Dart/Elixir fix (alef #534/#536) warned would recur: a
//! tuple-payload enum lowers to a struct-with-discriminator representation where an excluded
//! variant survives as a FIELD, invisible to any variant-list filtering.

use super::super::ExtendrBackend;
use super::make_config;
use crate::core::backend::Backend;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::new_config::NewAlefConfig;
use crate::core::ir::*;
use std::collections::BTreeSet;

fn make_config_with_feature(configured_feature: &str) -> ResolvedCrateConfig {
    let toml_src = format!(
        "[workspace]\nlanguages = [\"r\"]\n[[crates]]\nname = \"test-lib\"\nsources = [\"src/lib.rs\"]\n\
         [crates.r]\npackage_name = \"testlib\"\nfeatures = [\"{configured_feature}\"]\n"
    );
    let cfg: NewAlefConfig = toml::from_str(&toml_src).unwrap();
    cfg.resolve().unwrap().remove(0)
}

fn generate(api: &ApiSurface, config: &ResolvedCrateConfig) -> String {
    ExtendrBackend
        .generate_bindings(api, config)
        .expect("extendr generation")
        .iter()
        .map(|f| format!("// ==== {} ====\n{}", f.path.display(), f.content))
        .collect::<Vec<_>>()
        .join("\n")
}

fn returning_function(name: &str, enum_name: &str) -> FunctionDef {
    FunctionDef {
        name: name.to_string(),
        rust_path: format!("test_lib::{name}"),
        return_type: TypeRef::Named(enum_name.to_string()),
        ..Default::default()
    }
}

fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

/// Extracts the `pub enum RoutingStrategy { ... }` declaration body from the generated crate.
fn wrapper_enum_declaration(out: &str) -> &str {
    let start = out
        .find("pub enum RoutingStrategy {")
        .expect("generated crate must declare the RoutingStrategy wrapper enum");
    let end = out[start..]
        .find("\n}")
        .map(|i| start + i + 2)
        .expect("enum declaration must close");
    &out[start..end]
}

/// The exact set of variant names declared in a discriminant-free unit-enum body -- every line
/// shaped `Name,`, never a bare substring match.
fn declared_variant_names(rendered: &str) -> BTreeSet<String> {
    // ~keep This renderer emits the whole declaration on one line WITH `= N` discriminants, so
    // parse comma-separated fragments and strip the discriminant -- a line-based parse yields an
    // EMPTY set here, which compares unequal to any expectation and blames the generator for a
    // parser bug. Attributes such as `#[default]` may precede a name, so take the text after `]`.
    rendered
        .split(',')
        .filter_map(|fragment| {
            let fragment = fragment.rsplit(']').next()?;
            let (name, rest) = fragment.trim().split_once(" = ")?;
            let name = name.trim();
            let rest: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if name.is_empty() || rest.is_empty() {
                return None;
            }
            Some(name.to_string())
        })
        .collect()
}

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|s| s.to_string()).collect()
}

/// THE gap this task fixes at the plain-enum declaration surface: a FOREIGN enum with one
/// cfg-excluded variant and two retained ones renders exactly the retained set in the declared
/// Rust enum. Positive control (same fixture, feature configured) proves the drop is conditional
/// on the proof, not a blanket foreign-owned rule.
#[test]
fn declares_exact_retained_variant_set_for_foreign_variant_proven_unreachable() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "RoutingStrategy".to_string(),
            rust_path: "dep_crate::RoutingStrategy".to_string(),
            variants: vec![
                unit_variant("Primary", None),
                unit_variant("Secondary", None),
                unit_variant("Extra", Some(r#"feature = "extra-tier""#)),
            ],
            ..Default::default()
        }],
        functions: vec![returning_function("get_strategy", "RoutingStrategy")],
        ..Default::default()
    };

    let excluded = generate(&api, &make_config());
    let excluded_decl = wrapper_enum_declaration(&excluded);
    assert_eq!(
        declared_variant_names(excluded_decl),
        names(&["Primary", "Secondary"]),
        "the declared set must be exactly the two retained variants, got:\n{excluded_decl}"
    );

    let active = generate(&api, &make_config_with_feature("extra-tier"));
    let active_decl = wrapper_enum_declaration(&active);
    assert_eq!(
        declared_variant_names(active_decl),
        names(&["Primary", "Secondary", "Extra"]),
        "with \"extra-tier\" configured, the declared set must include the retained foreign \
         variant, got:\n{active_decl}"
    );
}

/// Host-owned control: a variant behind a HOST-owned cfg gate must never be dropped from the
/// declaration, regardless of `configured_features`.
#[test]
fn never_drops_host_owned_cfg_variant_from_declaration() {
    let api = ApiSurface {
        enums: vec![EnumDef {
            name: "RoutingStrategy".to_string(),
            rust_path: "test_lib::RoutingStrategy".to_string(),
            variants: vec![
                unit_variant("Primary", None),
                unit_variant("Secondary", None),
                unit_variant("Extra", Some(r#"feature = "extra-tier""#)),
            ],
            ..Default::default()
        }],
        functions: vec![returning_function("get_strategy", "RoutingStrategy")],
        ..Default::default()
    };

    let out = generate(&api, &make_config());
    let decl = wrapper_enum_declaration(&out);
    assert_eq!(
        declared_variant_names(decl),
        names(&["Primary", "Secondary", "Extra"]),
        "a host-owned cfg-gated variant must stay declared even with no features configured, \
         got:\n{decl}"
    );
}

fn tuple_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields: vec![FieldDef {
            name: "_0".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        is_tuple: true,
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn output_format_api(rust_path: &str) -> ApiSurface {
    ApiSurface {
        enums: vec![EnumDef {
            name: "OutputFormat".to_string(),
            rust_path: rust_path.to_string(),
            variants: vec![
                unit_variant("Standard", None),
                tuple_variant("Custom", None),
                tuple_variant("Testkit", Some(r#"feature = "testkit""#)),
            ],
            ..Default::default()
        }],
        functions: vec![returning_function("get_format", "OutputFormat")],
        ..Default::default()
    }
}

/// The second shape alef-task's Dart/Elixir fix uncovered, reproduced for extendr's flat
/// data-enum struct: a FOREIGN tuple-payload variant proven unreachable must not survive as a
/// FIELD on the flat struct, invisible to any variant-list filtering. Positive control (feature
/// configured) proves the field is conditional on the proof.
#[test]
fn flat_data_enum_struct_omits_field_for_foreign_variant_proven_unreachable_control_kept_when_active() {
    let api = output_format_api("dep_crate::OutputFormat");

    let excluded = generate(&api, &make_config());
    assert!(
        excluded.contains("pub struct OutputFormat {"),
        "this fixture must exercise the flat struct path, got:\n{excluded}"
    );
    assert!(
        excluded.contains("pub custom: Option<String>"),
        "the always-present variant's field must still be declared, got:\n{excluded}"
    );
    assert!(
        !excluded.contains("testkit"),
        "the excluded variant's field must not appear anywhere on the flat struct, got:\n{excluded}"
    );

    let active = generate(&api, &make_config_with_feature("testkit"));
    assert!(
        active.contains("pub testkit: Option<String>"),
        "with \"testkit\" configured, the field must be declared, got:\n{active}"
    );
}

/// The matching conversion-arm gap: a FOREIGN tuple-payload variant's `From<core>`/`From<binding>`
/// match arm must be dropped entirely (this generated crate can never declare a dependency's own
/// feature as its own Cargo feature) -- the flat struct's own catch-all covers the resulting gap.
#[test]
fn flat_data_enum_conversions_drop_arm_for_foreign_variant() {
    let api = output_format_api("dep_crate::OutputFormat");
    let out = generate(&api, &make_config());

    assert!(
        out.contains("impl From<dep_crate::OutputFormat> for OutputFormat {"),
        "the core->binding flat conversion must be emitted, got:\n{out}"
    );
    assert!(
        out.contains("dep_crate::OutputFormat::Custom(_0) => Self {"),
        "the always-present tuple variant's arm must still be emitted, got:\n{out}"
    );
    assert!(
        !out.contains("Testkit"),
        "the excluded variant must not be named anywhere in the flat conversions, got:\n{out}"
    );
}

/// Host-owned control for the flat struct: a variant behind a HOST-owned cfg gate keeps both its
/// field and its conversion arms (guarded by a matching `#[cfg(...)]` on the arms), regardless of
/// `configured_features`.
#[test]
fn flat_data_enum_never_drops_host_owned_cfg_variant() {
    let api = output_format_api("test_lib::OutputFormat");
    let out = generate(&api, &make_config());

    assert!(
        out.contains("pub testkit: Option<String>"),
        "a host-owned cfg-gated variant's field must stay declared even with no features \
         configured, got:\n{out}"
    );
    assert!(
        out.contains("#[cfg(feature = \"testkit\")]"),
        "a host-owned cfg-gated variant's conversion arm must carry its matching #[cfg(...)] \
         guard, got:\n{out}"
    );
}
