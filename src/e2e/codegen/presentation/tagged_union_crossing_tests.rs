//! Task #543: a field reached only through a `fields_method_calls`-declared tagged-union
//! crossing must survive presentation trimming, in both the derived (from `assertions`) and
//! hand-authored (`docs.presentation.operations`) shapes -- not just in the executable e2e
//! assertion renderers, which already resolved this correctly via
//! [`crate::e2e::field_access::FieldResolver::ir_enum_fields`].
//!
//! ~keep The consumer shape this module mirrors: `metadata.format` is a real field whose type is
//! a tagged union (`FormatInfo`); `.format.variant` is not a plain field access, but a real
//! accessor once `fields_method_calls` names the crossing; `variant`'s own payload type
//! (`VariantDetail`) declares a scalar field (`detail`) and an array field (`tags`) that are
//! reachable ONLY through that crossing. Before this module's fix, `presentation::build_resolver`
//! / `anchor_to_declared_result_type` never anchored `FieldResolver`'s `ir_enum_map` at all, so
//! every one of these paths was refused at the union boundary and dropped from the generated docs
//! snippet -- even though the consumer's own `fields_method_calls` declared exactly how to cross
//! it, and the executable e2e generators (which anchor the same map via `with_ir_enum_map`)
//! rendered the very same paths correctly.

use super::*;
use crate::core::config::e2e::CallConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

/// `convert` -> `Envelope { document: Document }`, `Document { label: String, format: FormatInfo
/// }`. `FormatInfo` is a tagged union with one variant, `Variant(VariantDetail)`, and
/// `VariantDetail { detail: String, tags: Vec<String> }` is reachable only by crossing the union.
fn type_defs() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Envelope".to_string(),
            fields: vec![field("document", TypeRef::Named("Document".to_string()))],
            ..TypeDef::default()
        },
        TypeDef {
            name: "Document".to_string(),
            fields: vec![
                field("label", TypeRef::String),
                field("format", TypeRef::Named("FormatInfo".to_string())),
            ],
            ..TypeDef::default()
        },
        TypeDef {
            name: "VariantDetail".to_string(),
            fields: vec![
                field("detail", TypeRef::String),
                field("tags", TypeRef::Vec(Box::new(TypeRef::String))),
            ],
            ..TypeDef::default()
        },
    ]
}

/// The crossing itself: `FormatInfo::Variant` carries exactly one field (`variant:
/// VariantDetail`), the shape [`crate::e2e::field_access::FieldResolver::ir_enum_fields`]
/// records as a `variant_payload_types` entry.
fn enums() -> Vec<EnumDef> {
    vec![EnumDef {
        name: "FormatInfo".to_string(),
        variants: vec![EnumVariant {
            name: "Variant".to_string(),
            fields: vec![field("variant", TypeRef::Named("VariantDetail".to_string()))],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }]
}

fn convert_returning(type_name: &str) -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "convert".to_string(),
        return_type: TypeRef::Named(type_name.to_string()),
        ..FunctionDef::default()
    }]
}

fn docs_fixture(presentation_operations: serde_json::Value, assertions: serde_json::Value) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"html": "<p>Hello World</p>"},
        "assertions": assertions,
        "docs": {
            "topic": "smoke",
            "stem": "sample_fixture",
            "presentation": {"operations": presentation_operations},
        }
    }))
    .expect("fixture must parse")
}

/// A docs-tagged fixture with no hand-authored `shows`/`presentation`, so every operation is
/// derived from `assertions` -- the common shape, and the one the task's own evidence names
/// ("generated DOC SNIPPETS omit those fields").
fn derived_docs_fixture(assertions: serde_json::Value) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"html": "<p>Hello World</p>"},
        "assertions": assertions,
        "docs": {"topic": "smoke", "stem": "sample_fixture"}
    }))
    .expect("fixture must parse")
}

fn config_with_crossing_declared() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "convert".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        fields_method_calls: ["document.format.variant".to_string()].into_iter().collect(),
        ..E2eConfig::default()
    }
}

fn config_without_crossing_declared() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "convert".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    }
}

/// The fix: an authored `show` through a declared crossing survives, both for the scalar field
/// beneath it (`detail`) and the array field beneath it (`tags`) -- the exact shape
/// `sheet_count`/`sheet_names` take past the consumer's own `.excel` crossing.
#[test]
fn an_authored_show_through_a_declared_method_call_crossing_survives() {
    let fixture = docs_fixture(
        serde_json::json!([
            {"op": "show", "path": "document.format.variant.detail"},
            {"op": "show", "path": "document.format.variant.tags"},
        ]),
        serde_json::json!([]),
    );

    let operations = resolve(
        &fixture,
        &config_with_crossing_declared(),
        "python",
        &type_defs(),
        &enums(),
        &convert_returning("Envelope"),
    );

    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.expression.as_str())
            .collect::<Vec<_>>(),
        vec![
            "result.document.format.variant.detail",
            "result.document.format.variant.tags"
        ],
        "both fields beneath a declared crossing must survive trimming: {operations:?}"
    );
}

/// The same paths, DERIVED from `assertions` rather than hand-authored -- the shape the task's
/// evidence actually reports (generated docs, not a hand-written `docs.presentation`).
#[test]
fn a_derived_show_through_a_declared_method_call_crossing_survives() {
    let fixture = derived_docs_fixture(serde_json::json!([
        {"type": "equals", "field": "document.format.variant.detail", "value": "Excel"},
        {"type": "not_empty", "field": "document.format.variant.tags"},
    ]));

    let operations = resolve(
        &fixture,
        &config_with_crossing_declared(),
        "python",
        &type_defs(),
        &enums(),
        &convert_returning("Envelope"),
    );

    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.expression.as_str())
            .collect::<Vec<_>>(),
        vec![
            "result.document.format.variant.detail",
            "result.document.format.variant.tags"
        ],
        "a derived show through a declared crossing must not be silently dropped: {operations:?}"
    );
}

/// The negative control: the identical shape, with the IR enum data present but NO
/// `fields_method_calls` entry vouching for the crossing, must still be dropped. Without this,
/// a version of the fix that trusted any tagged-union crossing whenever `enums` merely resolved
/// -- rather than requiring the consumer's own config to declare it -- would pass the positive
/// test above for the wrong reason.
#[test]
fn the_same_path_without_a_declared_crossing_is_still_dropped() {
    let fixture = docs_fixture(
        serde_json::json!([{"op": "show", "path": "document.format.variant.detail"}]),
        serde_json::json!([]),
    );

    let operations = resolve(
        &fixture,
        &config_without_crossing_declared(),
        "python",
        &type_defs(),
        &enums(),
        &convert_returning("Envelope"),
    );

    assert_eq!(
        operations,
        Vec::new(),
        "an undeclared crossing must still be refused even though the IR enum data resolves: {operations:?}"
    );
}

/// [`type_defs`] and [`enums`], one union level deeper: `VariantDetail` (the FIRST crossing's own
/// payload type) declares a SECOND enum-typed field, `nested: NestedInfo`, itself a tagged union
/// with a single variant `Deep(DeepDetail)`, and `DeepDetail { value: String }` is reachable only
/// by crossing that second union in turn. Mirrors the shape a follow-up consumer report described
/// as still dropped from generated doc snippets even after the first crossing was fixed.
fn type_defs_double_crossing() -> Vec<TypeDef> {
    let mut type_defs = type_defs();
    for type_def in &mut type_defs {
        if type_def.name == "VariantDetail" {
            type_def
                .fields
                .push(field("nested", TypeRef::Named("NestedInfo".to_string())));
        }
    }
    type_defs.push(TypeDef {
        name: "DeepDetail".to_string(),
        fields: vec![field("value", TypeRef::String)],
        ..TypeDef::default()
    });
    type_defs
}

fn enums_double_crossing() -> Vec<EnumDef> {
    let mut enums = enums();
    enums.push(EnumDef {
        name: "NestedInfo".to_string(),
        variants: vec![EnumVariant {
            name: "Deep".to_string(),
            fields: vec![field("deep", TypeRef::Named("DeepDetail".to_string()))],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    });
    enums
}

fn config_with_both_crossings_declared() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "convert".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        fields_method_calls: [
            "document.format.variant".to_string(),
            "document.format.variant.nested.deep".to_string(),
        ]
        .into_iter()
        .collect(),
        ..E2eConfig::default()
    }
}

/// The decisive case this module exists to add: a leaf reached through TWO chained declared
/// crossings (`variant`, then `deep`) must survive doc-snippet trimming, not just a leaf one
/// crossing deep. Both crossings are declared in `fields_method_calls`.
#[test]
fn a_derived_show_through_two_chained_method_call_crossings_survives() {
    let fixture = derived_docs_fixture(serde_json::json!([
        {"type": "equals", "field": "document.format.variant.nested.deep.value", "value": "ok"},
    ]));

    let operations = resolve(
        &fixture,
        &config_with_both_crossings_declared(),
        "python",
        &type_defs_double_crossing(),
        &enums_double_crossing(),
        &convert_returning("Envelope"),
    );

    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.expression.as_str())
            .collect::<Vec<_>>(),
        vec!["result.document.format.variant.nested.deep.value"],
        "a leaf reached through two chained declared crossings must not be silently dropped: {operations:?}"
    );
}

/// The negative control for the chained case: only the OUTER crossing (`variant`) is declared,
/// the inner one (`nested.deep`) is not. The leaf past the undeclared inner crossing must still
/// be dropped -- proving the walk commits to a real refusal for an inner crossing missing its own
/// declaration, rather than treating the outer declaration as blanket permission for everything
/// beneath it.
#[test]
fn a_derived_show_through_an_undeclared_inner_crossing_is_still_dropped() {
    let fixture = derived_docs_fixture(serde_json::json!([
        {"type": "equals", "field": "document.format.variant.nested.deep.value", "value": "ok"},
    ]));

    let operations = resolve(
        &fixture,
        &config_with_crossing_declared(),
        "python",
        &type_defs_double_crossing(),
        &enums_double_crossing(),
        &convert_returning("Envelope"),
    );

    assert_eq!(
        operations,
        Vec::new(),
        "an undeclared inner crossing must still be dropped even with the outer one declared: {operations:?}"
    );
}
