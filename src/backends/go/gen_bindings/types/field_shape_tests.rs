use std::collections::HashSet;

use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

use super::gen_struct_type;

fn assert_go_compiles(generated: &str, declarations: &str) {
    let Ok(go) = which::which("go") else {
        return;
    };
    let directory = tempfile::tempdir().expect("create Go compile fixture");
    std::fs::write(directory.path().join("go.mod"), "module example.com/shape\n\ngo 1.24\n").expect("write Go module");
    std::fs::write(
        directory.path().join("shape.go"),
        format!("package shape\n\nimport \"encoding/json\"\n\n{declarations}\n{generated}"),
    )
    .expect("write generated Go source");
    let output = std::process::Command::new(go)
        .arg("test")
        .arg("./...")
        .current_dir(directory.path())
        .output()
        .expect("run Go compiler");
    assert!(
        output.status.success(),
        "generated Go failed to compile:\n{}\n{generated}",
        String::from_utf8_lossy(&output.stderr)
    );
}

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
    let type_def = TypeDef {
        name: "Envelope".into(),
        fields: vec![
            FieldDef {
                name: "payload".into(),
                ty: TypeRef::Named("ForeignPayload".into()),
                ..Default::default()
            },
            FieldDef {
                name: "bytes".into(),
                ty: TypeRef::Bytes,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
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
    assert_eq!(output.matches("Payload *json.RawMessage").count(), 2, "{output}");
    assert_go_compiles(&output, "");
}

#[test]
fn marshal_auxiliary_data_interface_uses_authoritative_type() {
    let (type_def, choice) = data_interface_with_bytes();
    let output = gen_struct_type(
        &type_def,
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::from([choice.name.as_str()]),
        &HashSet::from([type_def.name.as_str()]),
        &[],
    );

    assert_eq!(output.matches("Choice Choice").count(), 2, "{output}");
    assert_go_compiles(
        &output,
        "type Choice interface{}\nfunc UnmarshalChoice(json.RawMessage) (Choice, error) { return nil, nil }",
    );
}

fn data_interface_with_bytes() -> (TypeDef, EnumDef) {
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
    let type_def = TypeDef {
        name: "Envelope".into(),
        fields: vec![
            FieldDef {
                name: "choice".into(),
                ty: TypeRef::Named("Choice".into()),
                optional: true,
                ..Default::default()
            },
            FieldDef {
                name: "bytes".into(),
                ty: TypeRef::Bytes,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    (type_def, choice)
}
