use super::{Severity, validate_field_classifications};
use crate::core::config::e2e::{CallConfig, E2eConfig};
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use std::collections::HashSet;

fn field(name: &str, ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

/// One IR type carrying the shapes the assertions below rule on: a plain `Vec<String>`, a
/// genuine `Option<String>`, and a plain `String`.
fn article_ir() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "ArticleMetadata".to_string(),
        fields: vec![
            field("tags", TypeRef::Vec(Box::new(TypeRef::String)), false),
            field("subtitle", TypeRef::String, true),
            field("title", TypeRef::String, false),
            field(
                "open_graph",
                TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
                false,
            ),
        ],
        ..TypeDef::default()
    }]
}

fn config_with(optional: &[&str], array: &[&str]) -> E2eConfig {
    E2eConfig {
        fields_optional: optional.iter().map(|f| (*f).to_string()).collect(),
        fields_array: array.iter().map(|f| (*f).to_string()).collect(),
        ..E2eConfig::default()
    }
}

fn errors_only(diagnostics: &[super::ValidationError]) -> Vec<&super::ValidationError> {
    diagnostics
        .iter()
        .filter(|diag| diag.severity == Severity::Error)
        .collect()
}

/// The defect this check exists for: `metadata.article.tags` declared optional against a
/// plain `Vec<String>` used to surface only as "type annotations needed" pointed at
/// generated code, with nothing naming the config line.
#[test]
fn optional_entry_naming_a_non_optional_ir_field_is_an_error_that_names_key_and_type() {
    let diagnostics = validate_field_classifications(&config_with(&["metadata.article.tags"], &[]), &article_ir(), &[]);

    let errors = errors_only(&diagnostics);
    assert_eq!(errors.len(), 1, "expected exactly one hard error, got: {diagnostics:?}");
    let message = &errors[0].message;
    assert!(message.contains("[e2e].fields_optional"), "names the table: {message}");
    assert!(
        message.contains("`metadata.article.tags`"),
        "names the entry: {message}"
    );
    assert!(message.contains("Vec<String>"), "names the real IR type: {message}");
    assert_eq!(errors[0].file, "alef.toml");
}

/// The check must not fire on a correct declaration — the field really is `Option<String>`.
#[test]
fn optional_entry_naming_a_genuinely_optional_ir_field_produces_no_error() {
    let diagnostics = validate_field_classifications(&config_with(&["metadata.subtitle"], &[]), &article_ir(), &[]);

    assert!(
        errors_only(&diagnostics).is_empty(),
        "a correct fields_optional entry must not be rejected: {diagnostics:?}"
    );
}

/// A subscripted entry classifies the ELEMENT the subscript reaches, not the container. Every
/// key lookup on a `HashMap<String, String>` used to be rejected as contradicting the IR
/// because the subscript was stripped and the map itself was ruled on as a bare field — which
/// is how a correct `metadata.document.open_graph[title]` failed the whole run.
#[test]
fn optional_entry_subscripting_a_map_field_produces_no_error() {
    let diagnostics = validate_field_classifications(
        &config_with(&["metadata.document.open_graph[title]"], &[]),
        &article_ir(),
        &[],
    );

    assert!(
        errors_only(&diagnostics).is_empty(),
        "a map key lookup is legitimately optional: {diagnostics:?}"
    );
}

/// Indexing a list element as optional is the same shape and must also pass.
#[test]
fn optional_entry_subscripting_a_vec_field_produces_no_error() {
    let diagnostics = validate_field_classifications(&config_with(&["tags[0]"], &[]), &article_ir(), &[]);

    assert!(
        errors_only(&diagnostics).is_empty(),
        "a list index is legitimately optional: {diagnostics:?}"
    );
}

/// Subscript-awareness must not become a blanket amnesty: a subscript against a scalar is
/// still a wrong path, and the diagnostic has to say the subscript is the problem.
#[test]
fn optional_entry_subscripting_a_scalar_field_is_still_an_error() {
    let diagnostics = validate_field_classifications(&config_with(&["title[0]"], &[]), &article_ir(), &[]);

    let errors = errors_only(&diagnostics);
    assert_eq!(errors.len(), 1, "expected exactly one hard error, got: {diagnostics:?}");
    assert!(
        errors[0].message.contains("subscripts it"),
        "got: {}",
        errors[0].message
    );
    assert!(errors[0].message.contains("String"), "got: {}", errors[0].message);
}

/// `fields_array` on a subscripted entry claims the element is itself indexable, so a
/// `HashMap<String, String>` value — a plain `String` — must still be rejected.
#[test]
fn array_entry_subscripting_a_map_of_scalars_is_an_error() {
    let diagnostics = validate_field_classifications(&config_with(&[], &["open_graph[title]"]), &article_ir(), &[]);

    let errors = errors_only(&diagnostics);
    assert_eq!(errors.len(), 1, "expected exactly one hard error, got: {diagnostics:?}");
    assert!(errors[0].message.contains("fields_array"), "got: {}", errors[0].message);
}

/// `fields_array` gets the same treatment: declaring a plain `String` indexable emits `[0]`
/// against a scalar.
#[test]
fn array_entry_naming_a_non_collection_ir_field_is_an_error() {
    let diagnostics = validate_field_classifications(&config_with(&[], &["title"]), &article_ir(), &[]);

    let errors = errors_only(&diagnostics);
    assert_eq!(errors.len(), 1, "expected exactly one hard error, got: {diagnostics:?}");
    assert!(errors[0].message.contains("fields_array"), "got: {}", errors[0].message);
    assert!(errors[0].message.contains("String"), "got: {}", errors[0].message);
}

#[test]
fn array_entry_naming_a_vec_ir_field_produces_no_error() {
    let diagnostics = validate_field_classifications(&config_with(&[], &["tags"]), &article_ir(), &[]);

    assert!(
        errors_only(&diagnostics).is_empty(),
        "a correct fields_array entry must not be rejected: {diagnostics:?}"
    );
}

/// The `IrAbsent` arm. Every IR-less caller (unit tests, snippet entry points that generate
/// from empty IR slices) must keep working — nothing was consulted, so nothing is claimed.
#[test]
fn an_absent_ir_produces_no_diagnostic_at_all() {
    let diagnostics = validate_field_classifications(&config_with(&["tags"], &["title"]), &[], &[]);

    assert!(
        diagnostics.is_empty(),
        "an absent IR licenses no claim in either direction: {diagnostics:?}"
    );
}

/// The `Unresolvable` arm. A name the IR has never heard of is unverified, not wrong —
/// virtual namespace prefixes and synthetic/streaming paths legitimately look like this — so
/// it warns and generation still proceeds.
#[test]
fn an_ir_unknown_leaf_warns_rather_than_failing_generation() {
    let diagnostics =
        validate_field_classifications(&config_with(&["interaction.chunk_count"], &[]), &article_ir(), &[]);

    assert!(
        errors_only(&diagnostics).is_empty(),
        "an unverifiable entry must not fail generation: {diagnostics:?}"
    );
    assert_eq!(diagnostics.len(), 1, "expected one warning, got: {diagnostics:?}");
    assert_eq!(diagnostics[0].severity, Severity::Warning);
    assert!(
        diagnostics[0].message.contains("unverified") && diagnostics[0].message.contains("`chunk_count`"),
        "the warning must name the leaf it could not resolve: {}",
        diagnostics[0].message
    );
}

/// Index and wildcard suffixes are part of the entry spelling, not part of the field name.
#[test]
fn indexed_and_wildcard_entry_spellings_resolve_to_the_same_leaf_field() {
    let indexed = validate_field_classifications(&config_with(&["article[0].tags"], &[]), &article_ir(), &[]);
    let wildcard = validate_field_classifications(&config_with(&["article[].tags"], &[]), &article_ir(), &[]);

    assert_eq!(errors_only(&indexed).len(), 1, "got: {indexed:?}");
    assert_eq!(errors_only(&wildcard).len(), 1, "got: {wildcard:?}");
}

/// Per-call override tables are checked too, and the diagnostic names the call's own table
/// rather than the global one — the operator has to be pointed at the line they must edit.
#[test]
fn per_call_override_tables_are_checked_and_named_in_the_diagnostic() {
    let mut config = E2eConfig::default();
    config.calls.insert(
        "summarize".to_string(),
        CallConfig {
            fields_optional: HashSet::from(["tags".to_string()]),
            ..CallConfig::default()
        },
    );

    let diagnostics = validate_field_classifications(&config, &article_ir(), &[]);

    let errors = errors_only(&diagnostics);
    assert_eq!(errors.len(), 1, "got: {diagnostics:?}");
    assert!(
        errors[0].message.contains("[e2e.calls.summarize].fields_optional"),
        "got: {}",
        errors[0].message
    );
}

/// The documented under-report trade: a field name that agrees with the entry on ANY IR type
/// clears it, because a bare leaf name cannot be pinned to one result type from here. This
/// pins the trade so a later "tighten it up" change has to confront it deliberately.
#[test]
fn a_same_named_field_that_agrees_on_another_type_clears_the_entry() {
    let mut ir = article_ir();
    ir.push(TypeDef {
        name: "DraftMetadata".to_string(),
        fields: vec![field("tags", TypeRef::Vec(Box::new(TypeRef::String)), true)],
        ..TypeDef::default()
    });

    let diagnostics = validate_field_classifications(&config_with(&["tags"], &[]), &ir, &[]);

    assert!(
        errors_only(&diagnostics).is_empty(),
        "agrees-on-any-type must win, or a shared field name produces a false failure: {diagnostics:?}"
    );
}

/// Task #543: the IR shape a tagged-union crossing takes. `ArticleMetadata.format` is a real
/// field whose type (`FormatInfo`) is a tagged union no `TypeDef` walks into; the union has
/// one variant, `Variant(VariantDetail)`, and `VariantDetail` declares the leaf `detail` --
/// mirroring the consumer's own shape (`metadata.format.excel.sheet_count`) with neutral
/// names. `variant` (the crossing field's own synthesized name) lives ONLY inside
/// `EnumVariant::fields`, never inside any `TypeDef.fields`, which is exactly why
/// `ir_field_shape`'s `type_defs`-only walk could never resolve it.
fn crossing_enums() -> Vec<crate::core::ir::EnumDef> {
    use crate::core::ir::{EnumDef, EnumVariant};
    vec![EnumDef {
        name: "FormatInfo".to_string(),
        variants: vec![EnumVariant {
            name: "Variant".to_string(),
            fields: vec![field("variant", TypeRef::Named("VariantDetail".to_string()), false)],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }]
}

fn crossing_type_defs() -> Vec<TypeDef> {
    let mut ir = article_ir();
    ir[0]
        .fields
        .push(field("format", TypeRef::Named("FormatInfo".to_string()), false));
    ir.push(TypeDef {
        name: "VariantDetail".to_string(),
        fields: vec![field("detail", TypeRef::String, false)],
        ..TypeDef::default()
    });
    ir
}

fn config_with_method_calls(optional: &[&str], method_calls: &[&str]) -> E2eConfig {
    E2eConfig {
        fields_optional: optional.iter().map(|f| (*f).to_string()).collect(),
        fields_method_calls: method_calls.iter().map(|f| (*f).to_string()).collect(),
        ..E2eConfig::default()
    }
}

/// The fix: a `fields_optional` entry whose leaf is a real, IR-confirmed tagged-union
/// crossing field -- AND the consumer's own `fields_method_calls` declares the entry verbatim
/// -- produces no diagnostic at all. Before this, `check_classification_table` never saw
/// `enums`, so `variant`'s only declaration (inside `EnumVariant::fields`) was invisible and
/// the entry was reported "unverified" even though the consumer's config proved the path.
#[test]
fn a_declared_method_call_crossing_produces_no_diagnostic() {
    let diagnostics = validate_field_classifications(
        &config_with_method_calls(&["format.variant"], &["format.variant"]),
        &crossing_type_defs(),
        &crossing_enums(),
    );

    assert!(
        diagnostics.is_empty(),
        "a fields_method_calls-declared, IR-confirmed crossing must not warn or error: {diagnostics:?}"
    );
}

/// Negative control 1: the identical leaf (`variant`) is a real crossing field per the IR, but
/// NOTHING in `fields_method_calls` declares `format.variant` as a crossing. The config alone
/// proves nothing -- the warning must still fire, or a fix that trusted IR shape without the
/// consumer's own declaration would silence every accidental tagged-union-shaped leaf.
#[test]
fn an_ir_confirmed_crossing_without_a_declared_method_call_still_warns() {
    let diagnostics = validate_field_classifications(
        &config_with_method_calls(&["format.variant"], &[]),
        &crossing_type_defs(),
        &crossing_enums(),
    );

    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .collect::<Vec<_>>();
    assert_eq!(warnings.len(), 1, "got: {diagnostics:?}");
    assert!(
        warnings[0].message.contains("unverified") && warnings[0].message.contains("`variant`"),
        "got: {}",
        warnings[0].message
    );
}

/// Negative control 2: `fields_method_calls` declares the entry verbatim, but NO `enums` were
/// supplied at all, so the IR has nothing to confirm the leaf is a real crossing field with.
/// The config declaration alone must not be enough to silence the warning -- this is the
/// sabotage case a fix that trusted `fields_method_calls` unconditionally would pass for the
/// wrong reason.
#[test]
fn a_declared_method_call_with_no_ir_enum_data_still_warns() {
    let diagnostics = validate_field_classifications(
        &config_with_method_calls(&["format.variant"], &["format.variant"]),
        &crossing_type_defs(),
        &[],
    );

    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .collect::<Vec<_>>();
    assert_eq!(warnings.len(), 1, "got: {diagnostics:?}");
    assert!(
        warnings[0].message.contains("unverified"),
        "got: {}",
        warnings[0].message
    );
}
