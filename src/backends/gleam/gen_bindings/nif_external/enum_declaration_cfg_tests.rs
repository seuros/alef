//! Regression coverage for `emit_enum`'s declaration-side cfg filtering.
//!
//! Before this fix `emit_enum` had no cfg handling anywhere -- not even conversion-side, since
//! Gleam has no conversion surface of its own (it only declares `@external` shims over the
//! Rustler NIF the Elixir/Rustler backend emits for the same consumer project). It read
//! `en.variants` directly and declared every variant unconditionally, so a Gleam caller could
//! pattern-match on a constructor no NIF call this build produces could ever actually return.
//! `emit_enum` now asks the same `codegen::conversions::enums::enum_variant_declaration`
//! authority every other Rust-emitting backend's own wrapper declaration already consults.
//!
//! Assertions parse the exact set of declared constructor names out of the rendered `.gleam`
//! source, never a bare substring `.contains` check, since a variant name can be a substring of
//! a longer identifier (and, for the fields test, of a field name/type too).

use super::emit_enum;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeRef};
use std::collections::HashSet;

fn unit_variant(name: &str, cfg: Option<&str>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        cfg: cfg.map(str::to_string),
        ..Default::default()
    }
}

fn foreign_enum(variants: Vec<EnumVariant>) -> EnumDef {
    EnumDef {
        name: "SyncMode".to_string(),
        rust_path: "dep_crate::SyncMode".to_string(),
        variants,
        ..Default::default()
    }
}

/// Parses the top-level constructor names out of a rendered `pub type Name { ... }` block.
/// A constructor line is indented by exactly two spaces and starts with an identifier
/// (`  Ctor` or `  Ctor(`); a field line is indented by four spaces, and the closing `  )` of a
/// data variant starts with `)`, not an identifier -- both are excluded by construction.
fn declared_constructors(rendered: &str) -> Vec<String> {
    rendered
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("  ")?;
            if rest.starts_with(' ') || rest.is_empty() {
                return None;
            }
            let first = rest.chars().next()?;
            if !(first.is_ascii_alphabetic() || first == '_') {
                return None;
            }
            Some(rest.trim_end_matches('(').to_string())
        })
        .collect()
}

/// The exact reported shape: a foreign enum with two always-present variants and one gated
/// behind a feature ("testkit") this binding does not configure in production. The excluded
/// variant must be entirely absent from the declared constructor set; the control half (same
/// shape, "testkit" configured) proves the drop is conditional, not a blanket foreign-owned rule.
#[test]
fn foreign_variant_proven_unreachable_dropped_from_unit_enum_control_kept_when_active() {
    let en = foreign_enum(vec![
        unit_variant("Manual", None),
        unit_variant("Automatic", None),
        unit_variant("Testkit", Some(r#"feature = "testkit""#)),
    ]);

    let mut excluded = String::new();
    let mut imports = Default::default();
    emit_enum(
        &en,
        &HashSet::new(),
        "sample_crate",
        Some(&[]),
        &mut excluded,
        &mut imports,
    );
    assert_eq!(
        declared_constructors(&excluded),
        vec!["Manual".to_string(), "Automatic".to_string()],
        "the excluded variant must not appear anywhere in the declaration, got:\n{excluded}"
    );

    let active_features = vec!["testkit".to_string()];
    let mut active = String::new();
    let mut imports = Default::default();
    emit_enum(
        &en,
        &HashSet::new(),
        "sample_crate",
        Some(&active_features),
        &mut active,
        &mut imports,
    );
    assert_eq!(
        declared_constructors(&active),
        vec!["Manual".to_string(), "Automatic".to_string(), "Testkit".to_string()],
        "with \"testkit\" configured, the variant must be declared, got:\n{active}"
    );
}

/// A host-owned cfg-gated variant must stay declared unconditionally -- `enum_variant_declaration`
/// never resolves a host-owned gate to `Drop`, deferring exhaustiveness to the compiler instead
/// of alef's own static feature analysis. Same fixture as above, but `rust_path` now names the
/// binding's own crate and `configured_features` is empty (the condition that dropped the
/// foreign variant above), proving the two cases are told apart, not merely that "testkit" was
/// dropped by coincidence.
#[test]
fn host_owned_cfg_gated_variant_is_never_dropped() {
    let en = EnumDef {
        name: "SyncMode".to_string(),
        rust_path: "sample_crate::SyncMode".to_string(),
        variants: vec![
            unit_variant("Manual", None),
            unit_variant("Testkit", Some(r#"feature = "testkit""#)),
        ],
        ..Default::default()
    };

    let mut out = String::new();
    let mut imports = Default::default();
    emit_enum(&en, &HashSet::new(), "sample_crate", Some(&[]), &mut out, &mut imports);
    assert_eq!(
        declared_constructors(&out),
        vec!["Manual".to_string(), "Testkit".to_string()],
        "a host-owned cfg-gated variant must stay declared regardless of configured_features, got:\n{out}"
    );
}

/// Same fact as the unit-enum case, exercised on a data (fields) variant instead -- `emit_enum`
/// dispatches to a different template block per variant shape, so both must be verified.
/// Asserts the field-carrying variant's own field line survives, and that the excluded variant's
/// field name/type never appears anywhere in the output.
#[test]
fn foreign_variant_proven_unreachable_dropped_from_data_enum_declaration() {
    let en = EnumDef {
        name: "Payload".to_string(),
        rust_path: "dep_crate::Payload".to_string(),
        variants: vec![
            EnumVariant {
                name: "Text".to_string(),
                fields: vec![FieldDef {
                    name: "value".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                ..Default::default()
            },
            EnumVariant {
                name: "Testkit".to_string(),
                fields: vec![FieldDef {
                    name: "payload".to_string(),
                    ty: TypeRef::String,
                    ..Default::default()
                }],
                cfg: Some(r#"feature = "testkit""#.to_string()),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let mut out = String::new();
    let mut imports = Default::default();
    emit_enum(&en, &HashSet::new(), "sample_crate", Some(&[]), &mut out, &mut imports);
    assert_eq!(
        declared_constructors(&out),
        vec!["Text".to_string()],
        "the excluded data variant must not appear in the declared constructor set, got:\n{out}"
    );
    assert!(
        out.contains("    value: String"),
        "the always-present variant's own field must still be declared, got:\n{out}"
    );
    assert!(
        !out.contains("payload"),
        "the excluded variant's field name must not leak into the output, got:\n{out}"
    );
}
