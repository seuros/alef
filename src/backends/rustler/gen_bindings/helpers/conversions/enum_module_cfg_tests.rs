//! Regression coverage for `gen_elixir_enum_module_with_known_types`'s cfg filtering.
//!
//! Before this fix, this Elixir-facing `.ex` module generator documented and exposed every
//! variant of an enum unconditionally, while the Rust NIF declaration (`gen_bindings/types.rs`,
//! once fixed) and the conversions already dropped a foreign cfg-gated variant this binding's
//! own configured feature set proves unreachable. An Elixir caller could therefore reference an
//! atom the NIF layer can never actually produce or accept -- a `@type`, an accessor function,
//! and a `wire_value/1` clause for a value that cannot round-trip. This module now asks the same
//! `codegen::conversions::enums::enum_variant_declaration` authority the NIF declaration does.
//!
//! Assertions check exact rendered text (full lines / exact function names), not a bare substring
//! `.contains`, since a variant name is often a substring of a longer identifier.

use super::super::gen_elixir_enum_module_with_known_types;
use crate::core::ir::{EnumDef, EnumVariant};
use ahash::AHashSet;

fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn sync_mode_enum() -> EnumDef {
    EnumDef {
        name: "SyncMode".to_string(),
        rust_path: "dep_crate::SyncMode".to_string(),
        variants: vec![
            unit_variant("Manual", None),
            unit_variant("Automatic", None),
            unit_variant("Testkit", Some(r#"feature = "testkit""#)),
        ],
        ..Default::default()
    }
}

/// The exact reported shape: a foreign enum with two always-present variants and one gated
/// behind a feature ("testkit") this binding does not configure in production. The excluded
/// variant must vanish from the `@type`, the atom accessor, and the `wire_value/1` dispatch --
/// not merely be un-asserted. The control half (same shape, "testkit" configured) proves the
/// drop is conditional on `configured_features`, not a blanket "foreign means gone" rule.
#[test]
fn foreign_cfg_excluded_variant_vanishes_from_ex_module_unless_active() {
    let excluded = gen_elixir_enum_module_with_known_types(
        &sync_mode_enum(),
        "SampleApp",
        &AHashSet::new(),
        "mylib",
        Some(&[]),
    );
    assert!(
        excluded.contains("@type t :: :manual | :automatic"),
        "the @type union must list exactly the two retained atoms, got:\n{excluded}"
    );
    assert!(
        excluded.contains("def manual, do: @manual") && excluded.contains("def automatic, do: @automatic"),
        "the two always-present variants must still get accessor functions, got:\n{excluded}"
    );
    assert!(
        !excluded.contains("testkit") && !excluded.contains("Testkit"),
        "the excluded variant must not appear anywhere in the module, got:\n{excluded}"
    );

    let active_features = vec!["testkit".to_string()];
    let active = gen_elixir_enum_module_with_known_types(
        &sync_mode_enum(),
        "SampleApp",
        &AHashSet::new(),
        "mylib",
        Some(&active_features),
    );
    assert!(
        active.contains("@type t :: :manual | :automatic | :testkit"),
        "with \"testkit\" configured, the @type union must list all three atoms, got:\n{active}"
    );
    assert!(
        active.contains("def testkit, do: @testkit"),
        "with \"testkit\" configured, the accessor function must be present, got:\n{active}"
    );
    assert!(
        active.contains("def wire_value(:testkit), do: \"Testkit\""),
        "with \"testkit\" configured, the wire_value/1 clause must be present, got:\n{active}"
    );
}

/// Same fact on the data-enum path: a foreign cfg-gated STRUCT-shaped variant must not get a
/// per-variant `@type` alias or a generated constructor function.
#[test]
fn foreign_cfg_excluded_variant_vanishes_from_data_enum_ex_module() {
    let en = EnumDef {
        name: "Payload".to_string(),
        rust_path: "dep_crate::Payload".to_string(),
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "value".to_string(),
                    ty: crate::core::ir::TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Testkit".to_string(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "value".to_string(),
                    ty: crate::core::ir::TypeRef::String,
                    ..Default::default()
                }],
                cfg: Some(r#"feature = "testkit""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let out = gen_elixir_enum_module_with_known_types(&en, "SampleApp", &AHashSet::new(), "mylib", Some(&[]));
    assert!(out.contains("Text"), "the always-present data variant must still be documented, got:\n{out}");
    assert!(
        !out.contains("Testkit"),
        "the excluded data variant must not appear anywhere in the module, got:\n{out}"
    );
}

/// Host-owned cfg-gated variants are unaffected: `enum_variant_declaration` never resolves a
/// host-owned gate to `Drop`, so the module keeps it regardless of `configured_features`,
/// matching every other backend's declaration surface.
#[test]
fn host_owned_cfg_variant_is_always_kept_in_the_ex_module() {
    let en = EnumDef {
        name: "LogLevel".to_string(),
        rust_path: "mylib::LogLevel".to_string(),
        variants: vec![
            unit_variant("Info", None),
            unit_variant("Trace", Some(r#"feature = "verbose-logging""#)),
        ],
        ..Default::default()
    };
    let out = gen_elixir_enum_module_with_known_types(&en, "SampleApp", &AHashSet::new(), "mylib", Some(&[]));
    assert!(
        out.contains("def trace, do: @trace"),
        "a host-owned cfg-gated variant must stay exposed even with no features configured, got:\n{out}"
    );
}
