use std::collections::HashSet;

use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

use super::gen_struct_type;

fn envelope_with(field: FieldDef) -> TypeDef {
    TypeDef {
        name: "Envelope".into(),
        fields: vec![field],
        ..Default::default()
    }
}

#[test]
fn optional_data_enum_field_uses_non_pointer_interface() {
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
    let type_def = envelope_with(FieldDef {
        name: "choice".into(),
        ty: TypeRef::Named("Choice".into()),
        optional: true,
        ..Default::default()
    });
    let output = gen_struct_type(
        &type_def,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from([choice.name.as_str()]),
        &HashSet::from([type_def.name.as_str()]),
        &[],
    );

    assert!(output.contains("Choice Choice `json:\"choice,omitempty\"`"), "{output}");
    assert!(
        !output.contains("Choice *Choice"),
        "sealed interfaces are not pointers:\n{output}"
    );
}

#[test]
fn required_unresolved_named_field_uses_raw_message_pointer() {
    let type_def = envelope_with(FieldDef {
        name: "payload".into(),
        ty: TypeRef::Named("ForeignPayload".into()),
        ..Default::default()
    });
    let output = gen_struct_type(
        &type_def,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from([type_def.name.as_str()]),
        &[],
    );

    assert!(
        output.contains("Payload *json.RawMessage `json:\"payload,omitempty\"`"),
        "{output}"
    );
}
