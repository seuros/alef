use std::collections::{HashMap, HashSet};

use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::assertions::render_assertion;

fn envelope_types(ty: TypeRef, optional: bool) -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "Envelope".into(),
            fields: vec![FieldDef {
                name: "details".into(),
                ty,
                optional,
                ..Default::default()
            }],
            ..Default::default()
        },
        TypeDef {
            name: "Details".into(),
            ..Default::default()
        },
    ]
}

fn envelope_resolver(types: &[TypeDef], optional: bool, array: bool) -> FieldResolver {
    let optional_fields = optional
        .then(|| HashSet::from(["details".to_string()]))
        .unwrap_or_default();
    let array_fields = array
        .then(|| HashSet::from(["details".to_string()]))
        .unwrap_or_default();
    FieldResolver::new(
        &HashMap::new(),
        &optional_fields,
        &HashSet::new(),
        &array_fields,
        &HashSet::new(),
    )
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts(types, "java"),
        Some("Envelope".into()),
    )
}

fn render(ty: TypeRef, optional: bool, array: bool, assertion_type: &str) -> String {
    let types = envelope_types(ty, optional);
    let resolver = envelope_resolver(&types, optional, array);
    let mut output = String::new();
    render_assertion(
        &mut output,
        &Assertion {
            assertion_type: assertion_type.into(),
            field: Some("details".into()),
            ..Default::default()
        },
        "result",
        "Envelope",
        &resolver,
        false,
        false,
        false,
        false,
        None,
        &HashSet::new(),
        &HashMap::new(),
        false,
        &HashSet::new(),
        true,
    );
    output
}

#[test]
fn bare_record_uses_nullability_checks() {
    assert_eq!(
        render(TypeRef::Named("Details".into()), false, false, "not_empty"),
        "        assertNotNull(result.details(), \"expected non-empty value\");\n"
    );
    assert_eq!(
        render(TypeRef::Named("Details".into()), false, false, "is_empty"),
        "        assertNull(result.details(), \"expected empty value\");\n"
    );
}

#[test]
fn record_collection_uses_collection_methods() {
    assert_eq!(
        render(
            TypeRef::Vec(Box::new(TypeRef::Named("Details".into()))),
            false,
            true,
            "not_empty"
        ),
        "        assertFalse(result.details().isEmpty(), \"expected non-empty value\");\n"
    );
}

#[test]
fn optional_record_uses_optional_presence() {
    assert_eq!(
        render(TypeRef::Named("Details".into()), true, false, "is_empty"),
        "        assertTrue(java.util.Optional.ofNullable(result.details()).isEmpty(), \"expected empty value\");\n"
    );
}
