//! Regression coverage for `gen_enum`'s declaration-side cfg filtering.
//!
//! Before this fix, `gen_enum` declared every variant of a `NifUnitEnum`/`NifTaggedEnum`
//! unconditionally, while the `From` impls elsewhere already dropped a foreign cfg-gated
//! variant's conversion arm whenever this binding's own configured feature set proved it
//! unreachable (see `cfg_gate_tests.rs` for that half). An Elixir caller could therefore pass
//! the atom/tuple the NIF decoder never actually matches, failing with `badarg` at runtime
//! instead of the declaration simply never advertising the value. `gen_enum` now asks the same
//! `codegen::conversions::enums::enum_variant_declaration` authority every other Rust-emitting
//! backend's own wrapper declaration already consults. `gen_rustler_flat_data_enum` (a separate
//! emitter `gen_enum` dispatches to for a specific data-enum shape) had the identical gap on its
//! own struct-field declaration and is fixed and covered the same way.
//!
//! Assertions use exact rendered lines (`"    Name,\n"`) rather than a bare substring `.contains`
//! check, since a variant name can be a substring of a longer identifier.

use super::gen_enum;
use crate::core::ir::{ApiSurface, EnumDef, EnumVariant};

fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn unit_enum(rust_path: &str, variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: "SyncMode".to_string(),
        rust_path: rust_path.to_string(),
        variants,
        ..Default::default()
    }
}

fn declared_line(name: &str) -> String {
    format!("    {name},\n")
}

/// The exact reported shape: a foreign enum with two always-present variants and one gated
/// behind a feature ("testkit") this binding does not configure in production. The excluded
/// variant must be entirely absent from the `NifUnitEnum` declaration; the control half (same
/// shape, "testkit" configured) proves the drop is conditional, not a blanket foreign-owned rule.
#[test]
fn foreign_variant_proven_unreachable_dropped_from_unit_enum_control_kept_when_active() {
    fn sync_mode() -> EnumDef {
        unit_enum(
            "dep_crate::SyncMode",
            vec![
                unit_variant("Manual", None),
                unit_variant("Automatic", None),
                unit_variant("Testkit", Some(r#"feature = "testkit""#)),
            ],
        )
    }

    let excluded = gen_enum(&sync_mode(), "SampleCrate", &ApiSurface::default(), "mylib", Some(&[]));
    assert!(
        excluded.contains(&declared_line("Manual")) && excluded.contains(&declared_line("Automatic")),
        "the two always-present variants must still be declared, got:\n{excluded}"
    );
    assert!(
        !excluded.contains(&declared_line("Testkit")) && !excluded.contains("Testkit"),
        "the excluded variant must not appear anywhere in the declaration, got:\n{excluded}"
    );

    let active_features = vec!["testkit".to_string()];
    let active = gen_enum(
        &sync_mode(),
        "SampleCrate",
        &ApiSurface::default(),
        "mylib",
        Some(&active_features),
    );
    assert!(
        active.contains(&declared_line("Testkit")),
        "with \"testkit\" configured, the variant must be declared, got:\n{active}"
    );
}

/// Same fact, exercised on the `NifTaggedEnum` (struct-shaped data-variant) path instead of the
/// unit-enum path -- `gen_enum` dispatches to a different template block per shape, so both must
/// be verified. Fields are STRUCT-style (`is_tuple: false`) deliberately: a single-tuple-field
/// data enum instead routes through `is_flat_data_enum`'s flat `NifStruct` shape (a different
/// emitter, `gen_rustler_flat_data_enum`, covered separately below) rather than `NifTaggedEnum`.
#[test]
fn foreign_variant_proven_unreachable_dropped_from_tagged_enum_declaration() {
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
                is_tuple: false,
                ..Default::default()
            },
            EnumVariant {
                name: "Testkit".to_string(),
                fields: vec![crate::core::ir::FieldDef {
                    name: "value".to_string(),
                    ty: crate::core::ir::TypeRef::String,
                    ..Default::default()
                }],
                is_tuple: false,
                cfg: Some(r#"feature = "testkit""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let out = gen_enum(&en, "SampleCrate", &ApiSurface::default(), "mylib", Some(&[]));
    assert!(
        out.contains("#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rustler::NifTaggedEnum)]"),
        "this fixture must exercise the NifTaggedEnum path, not the flat NifStruct path, got:\n{out}"
    );
    assert!(out.contains("Text"), "the always-present data variant must still be declared, got:\n{out}");
    assert!(
        !out.contains("Testkit"),
        "the excluded data variant must not appear anywhere in the declaration, got:\n{out}"
    );
}

/// The real second defect the tagged-enum fixture above accidentally masked: when every data
/// variant carries a single tuple field of a Named/String type, `gen_enum` dispatches to
/// `gen_rustler_flat_data_enum`'s flat `NifStruct` shape (one discriminator field + one optional
/// field per variant) instead of `NifTaggedEnum` -- a DIFFERENT emitter that read
/// `enum_def.variants` directly with no cfg awareness at all. A foreign cfg-gated variant proven
/// unreachable leaked through as a struct FIELD (`pub testkit: Option<String>`), not merely an
/// enum case, because filtering the variant list alone does not touch this separate function.
#[test]
fn foreign_variant_proven_unreachable_dropped_from_flat_struct_declaration_control_kept_when_active() {
    fn payload_enum() -> EnumDef {
        EnumDef {
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
                    is_tuple: true,
                    ..Default::default()
                },
                EnumVariant {
                    name: "Testkit".to_string(),
                    fields: vec![crate::core::ir::FieldDef {
                        name: "value".to_string(),
                        ty: crate::core::ir::TypeRef::String,
                        ..Default::default()
                    }],
                    is_tuple: true,
                    cfg: Some(r#"feature = "testkit""#.to_string()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    let excluded = gen_enum(&payload_enum(), "SampleCrate", &ApiSurface::default(), "mylib", Some(&[]));
    assert!(
        excluded.contains("rustler::NifStruct"),
        "this fixture must exercise the flat NifStruct path, got:\n{excluded}"
    );
    assert!(
        excluded.contains("pub text: Option<String>"),
        "the always-present variant's field must still be declared, got:\n{excluded}"
    );
    assert!(
        !excluded.contains("testkit"),
        "the excluded variant's field must not appear anywhere on the flat struct, got:\n{excluded}"
    );

    let active_features = vec!["testkit".to_string()];
    let active = gen_enum(
        &payload_enum(),
        "SampleCrate",
        &ApiSurface::default(),
        "mylib",
        Some(&active_features),
    );
    assert!(
        active.contains("pub testkit: Option<String>"),
        "with \"testkit\" configured, the field must be declared, got:\n{active}"
    );
}

/// Host-owned cfg-gated variants are unaffected by this fix: `enum_variant_declaration` never
/// resolves a host-owned gate to `Drop`, so the declaration keeps it regardless of
/// `configured_features`, matching every other backend's declaration surface.
#[test]
fn host_owned_cfg_variant_is_always_kept_in_the_declaration() {
    let en = unit_enum(
        "mylib::LogLevel",
        vec![
            unit_variant("Info", None),
            unit_variant("Trace", Some(r#"feature = "verbose-logging""#)),
        ],
    );
    let out = gen_enum(&en, "SampleCrate", &ApiSurface::default(), "mylib", Some(&[]));
    assert!(
        out.contains(&declared_line("Trace")),
        "a host-owned cfg-gated variant must stay declared even with no features configured, got:\n{out}"
    );
}

/// Edge case: the variant marked `#[default]` is itself the one a foreign cfg proves
/// unreachable. The default-value `impl Default` must fall back to another declared variant
/// instead of referencing a variant name the enum no longer declares (which would not compile).
#[test]
fn default_variant_selection_skips_a_variant_dropped_from_the_declaration() {
    let en = EnumDef {
        name: "SyncMode".to_string(),
        rust_path: "dep_crate::SyncMode".to_string(),
        variants: vec![
            EnumVariant {
                name: "Testkit".to_string(),
                is_default: true,
                cfg: Some(r#"feature = "testkit""#.to_string()),
                ..Default::default()
            },
            unit_variant("Manual", None),
        ],
        ..Default::default()
    };

    let out = gen_enum(&en, "SampleCrate", &ApiSurface::default(), "mylib", Some(&[]));
    assert!(
        !out.contains("Testkit"),
        "the dropped default variant must not appear anywhere, including the Default impl, got:\n{out}"
    );
    assert!(
        out.contains("Manual"),
        "the Default impl must fall back to a variant the declaration actually keeps, got:\n{out}"
    );
}
