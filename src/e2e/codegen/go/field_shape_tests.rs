use std::collections::HashSet;

use crate::core::config::e2e::CallConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_function::{GoTestFunctionContext, render_test_function};

fn render_field_assertion(field: FieldDef, enums: &[EnumDef], configured_optional: bool) -> String {
    let mut optional = HashSet::new();
    if configured_optional {
        optional.insert(field.name.clone());
    }
    let config = E2eConfig {
        call: CallConfig {
            function: "inspect".into(),
            module: "example.com/sample".into(),
            returns_result: true,
            ..Default::default()
        },
        fields_optional: optional,
        ..Default::default()
    };
    let fixture = Fixture {
        id: "field_shape".into(),
        description: "field shape".into(),
        assertions: vec![Assertion {
            assertion_type: "equals".into(),
            field: Some(field.name.clone()),
            value: Some(serde_json::json!({"value": "sample"})),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![field],
        ..Default::default()
    }];
    let functions = vec![FunctionDef {
        name: "inspect".into(),
        return_type: TypeRef::Named("Envelope".into()),
        ..Default::default()
    }];
    let mut output = String::new();
    render_test_function(
        &mut output,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample",
            e2e_config: &config,
            adapters: &[],
            data_enum_names: &HashSet::new(),
            config: &Default::default(),
            type_defs: &type_defs,
            enums,
            errors: &[],
            functions: &functions,
        },
    );
    output
}

#[test]
fn optional_data_interface_field_is_nullable_but_not_dereferenced() {
    let choice = EnumDef {
        name: "Choice".into(),
        variants: vec![EnumVariant {
            name: "Value".into(),
            fields: vec![FieldDef {
                name: "value".into(),
                ty: TypeRef::String,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let output = render_field_assertion(
        FieldDef {
            name: "choice".into(),
            ty: TypeRef::Named("Choice".into()),
            optional: true,
            ..Default::default()
        },
        &[choice],
        true,
    );

    assert!(
        !output.contains("*result.Choice"),
        "sealed interfaces are not pointers:\n{output}"
    );
}

#[test]
fn required_unresolved_named_field_uses_raw_message_pointer_shape() {
    let types = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![FieldDef {
            name: "payload".into(),
            ty: TypeRef::Named("ForeignPayload".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let resolver = FieldResolver::new(
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    )
    .with_ir_result_fields(
        FieldResolver::ir_result_field_facts_with_enums(&types, &[], "go"),
        Some("Envelope".into()),
    );
    assert_eq!(resolver.target_field_is_pointer("payload"), Some(true));

    let output = render_field_assertion(
        FieldDef {
            name: "payload".into(),
            ty: TypeRef::Named("ForeignPayload".into()),
            ..Default::default()
        },
        &[],
        false,
    );

    assert!(
        output.contains("*result.Payload"),
        "unresolved named fields are pointers:\n{output}"
    );
}

#[test]
fn optional_vec_assertions_follow_go_slice_shape_over_global_optionality() {
    let config = E2eConfig {
        call: CallConfig {
            function: "inspect".into(),
            module: "example.com/sample".into(),
            returns_result: true,
            ..Default::default()
        },
        fields_optional: HashSet::from(["items".into()]),
        fields_array: HashSet::from(["items".into()]),
        ..Default::default()
    };
    let fixture = Fixture {
        id: "optional_vec_shape".into(),
        assertions: vec![Assertion {
            assertion_type: "min_length".into(),
            field: Some("items".into()),
            value: Some(serde_json::json!(1)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![FieldDef {
            name: "items".into(),
            ty: TypeRef::Vec(Box::new(TypeRef::String)),
            optional: true,
            ..Default::default()
        }],
        ..Default::default()
    }];
    let functions = vec![FunctionDef {
        name: "inspect".into(),
        return_type: TypeRef::Named("Envelope".into()),
        ..Default::default()
    }];
    let mut output = String::new();
    render_test_function(
        &mut output,
        &fixture,
        GoTestFunctionContext {
            import_alias: "sample",
            e2e_config: &config,
            adapters: &[],
            data_enum_names: &HashSet::new(),
            config: &Default::default(),
            type_defs: &type_defs,
            enums: &[],
            errors: &[],
            functions: &functions,
        },
    );

    assert!(output.contains("len(result.Items)"), "expected slice length:\n{output}");
    assert!(
        !output.contains("len(*result.Items)"),
        "must not dereference slice:\n{output}"
    );
}
