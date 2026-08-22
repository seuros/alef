//! Regression coverage pinning the Elixir e2e generator's `rename_all` handling to the
//! canonical `crate::codegen::naming::wire_variant_value`.
//!
//! `match_unit_enum_atom` used to derive the wire-format tag value for a unit enum variant
//! with a locally hand-rolled `apply_rename_all` helper that disagreed with the canonical
//! naming module on two strategies: an absent `rename_all` (or an unrecognized strategy
//! string) left the name unchanged in `wire_variant_value` but lowercased it to snake_case
//! here, and `"UPPERCASE"` uppercased the raw name in `wire_variant_value` but routed through
//! `to_shouty_snake_case` (inserting underscores) here. Since the rustler binding itself
//! (`src/backends/rustler/gen_bindings/public_api_args.rs`,
//! `src/backends/rustler/gen_bindings/types.rs`) already computes wire tags via
//! `wire_variant_value`, the mismatch meant the e2e generator could fail to recognize a
//! fixture's wire-tag string as matching any variant for the common no-`rename_all` case.
//!
//! These tests drive `match_unit_enum_atom` (the real entry point, not a reimplementation)
//! with the exact wire value `wire_variant_value` computes for each strategy, so the two
//! functions cannot silently drift apart again.

use crate::codegen::naming::wire_variant_value;
use crate::core::ir::{EnumDef, EnumVariant};

use super::args::match_unit_enum_atom;

fn unit_enum(variant_name: &str, rename_all: Option<&str>) -> EnumDef {
    EnumDef {
        name: "ElementKind".to_string(),
        variants: vec![EnumVariant {
            name: variant_name.to_string(),
            ..EnumVariant::default()
        }],
        serde_rename_all: rename_all.map(str::to_string),
        ..EnumDef::default()
    }
}

/// Strategies where the local helper used to disagree with `wire_variant_value`
/// (absent, `UPPERCASE`, `SCREAMING_SNAKE_CASE`, and an unknown strategy string),
/// plus `snake_case` and `kebab-case` as controls that already agreed.
const STRATEGIES: &[Option<&str>] = &[
    None,
    Some("UPPERCASE"),
    Some("SCREAMING_SNAKE_CASE"),
    Some("not_a_real_strategy"),
    Some("snake_case"),
    Some("kebab-case"),
];

#[test]
fn match_unit_enum_atom_agrees_with_wire_variant_value_for_every_strategy() {
    let variant_name = "ElementBased";
    for rename_all in STRATEGIES {
        let enum_def = unit_enum(variant_name, *rename_all);
        let expected_wire = wire_variant_value(variant_name, None, *rename_all);
        let value = serde_json::Value::String(expected_wire.clone());

        let matched = match_unit_enum_atom(&value, &enum_def);
        assert_eq!(
            matched,
            Some(":element_based".to_string()),
            "rename_all={rename_all:?}: expected the wire value {expected_wire:?} produced by \
             wire_variant_value to match variant `{variant_name}`, got {matched:?}"
        );
    }
}

#[test]
fn match_unit_enum_atom_rejects_a_value_that_does_not_match_wire_variant_value() {
    // A value equal to the raw snake_case rendering of the variant name must NOT match when
    // the canonical wire value for the configured strategy is something else (e.g. no
    // rename_all leaves the name unchanged as "ElementBased", not "element_based").
    let enum_def = unit_enum("ElementBased", None);
    let value = serde_json::Value::String("element_based".to_string());
    assert_eq!(match_unit_enum_atom(&value, &enum_def), None);
}
