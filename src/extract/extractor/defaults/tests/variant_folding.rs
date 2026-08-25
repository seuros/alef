use super::*;

/// The reported defect, reduced with synthetic names. A tagged enum used as a field default
/// via its struct variant — the ordinary way to spell "the default is this preset" — must
/// fold into its own field values rather than leaving the whole field `Unresolved`.
#[test]
fn a_struct_variant_default_folds_with_its_field_values() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Cfg { pub kind: Kind }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self { kind: Kind::Curated { label: "balanced".to_string(), weight: 3 } }
                    }
                }
            "#,
        "Cfg",
        &[("kind", TypeRef::Named("Kind".to_string()))],
    );

    assert_eq!(
        resolved,
        vec![(
            "kind".to_string(),
            DefaultValue::StructVariant(
                "Curated".to_string(),
                vec![
                    ("label".to_string(), DefaultValue::StringLiteral("balanced".to_string())),
                    ("weight".to_string(), DefaultValue::IntLiteral(3)),
                ],
            ),
        )],
        "a struct-variant enum default must fold into its own field values, not stay Unresolved"
    );
}

/// A tuple-variant enum default folds the same way, keyed by argument position rather than
/// field name.
#[test]
fn a_tuple_variant_default_folds_with_its_argument_values() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Cfg { pub kind: Kind }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self { kind: Kind::Scaled(5, "x".to_string()) }
                    }
                }
            "#,
        "Cfg",
        &[("kind", TypeRef::Named("Kind".to_string()))],
    );

    assert_eq!(
        resolved,
        vec![(
            "kind".to_string(),
            DefaultValue::TupleVariant(
                "Scaled".to_string(),
                vec![
                    DefaultValue::IntLiteral(5),
                    DefaultValue::StringLiteral("x".to_string())
                ],
            ),
        )],
        "a tuple-variant enum default must fold into its own argument values, not stay Unresolved"
    );
}

/// The control: a bare unit-variant path already folded to `EnumVariant` before this change
/// (see `a_genuine_enum_variant_default_still_lowers_to_an_enum_variant`); this pins that a
/// unit variant reached as the sole field of a struct literal is unaffected by adding
/// `TupleVariant`/`StructVariant`.
#[test]
fn a_unit_variant_default_still_folds_to_a_bare_enum_variant() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Cfg { pub kind: Kind }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self { kind: Kind::Auto }
                    }
                }
            "#,
        "Cfg",
        &[("kind", TypeRef::Named("Kind".to_string()))],
    );

    assert_eq!(
        resolved,
        vec![("kind".to_string(), DefaultValue::EnumVariant("Auto".to_string()))],
        "a unit-variant default already folded before this change and must keep doing so"
    );
}

/// The invariant this whole feature exists to protect: a struct variant with even one
/// unfoldable field must stay `Unresolved` as a whole, never `Empty` and never a
/// partially-known payload that silently drops the field it could not read.
#[test]
fn a_struct_variants_unfoldable_field_keeps_the_whole_default_unresolved_not_empty() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Cfg { pub kind: Kind }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self { kind: Kind::Curated { label: compute_label() } }
                    }
                }
            "#,
        "Cfg",
        &[("kind", TypeRef::Named("Kind".to_string()))],
    );

    assert!(
        matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
        "an unfoldable inner field must leave the whole variant default Unresolved; got {resolved:?}"
    );
    assert_ne!(
        resolved[0].1,
        DefaultValue::Empty,
        "collapsing an unfoldable struct-variant field to `Empty` is the conflation this fixes"
    );
}

/// The same invariant for the tuple-variant fold.
#[test]
fn a_tuple_variants_unfoldable_argument_keeps_the_whole_default_unresolved_not_empty() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Cfg { pub kind: Kind }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self { kind: Kind::Scaled(compute_scale()) }
                    }
                }
            "#,
        "Cfg",
        &[("kind", TypeRef::Named("Kind".to_string()))],
    );

    assert!(
        matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
        "an unfoldable argument must leave the whole variant default Unresolved; got {resolved:?}"
    );
}

/// A `..base` spread inside a struct-variant literal could carry fields this pass never
/// saw (`Kind::Curated { label: "x".to_string(), ..Default::default() }` might have more
/// fields than `label` alone). Folding without accounting for `rest` would be a guess, so
/// the whole expression stays `Unresolved` instead.
#[test]
fn a_struct_variant_with_a_rest_base_stays_unresolved() {
    let resolved = defaults_for_typed(
        r#"
                pub struct Cfg { pub kind: Kind }

                impl Default for Cfg {
                    fn default() -> Self {
                        Self { kind: Kind::Curated { label: "x".to_string(), ..Default::default() } }
                    }
                }
            "#,
        "Cfg",
        &[("kind", TypeRef::Named("Kind".to_string()))],
    );

    assert!(
        matches!(resolved.as_slice(), [(_, DefaultValue::Unresolved(_))]),
        "a `..base` spread can carry fields this pass never saw; folding without it would guess; \
             got {resolved:?}"
    );
}
