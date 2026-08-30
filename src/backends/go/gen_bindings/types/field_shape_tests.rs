use std::collections::HashSet;

use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

use super::gen_struct_type;

fn go_compile(generated: &str, declarations: &str) -> std::process::Output {
    let go = which::which("go").expect("Go is required for generated-Go compile fixtures");
    let directory = tempfile::tempdir().expect("create Go compile fixture");
    std::fs::write(directory.path().join("go.mod"), "module example.com/shape\n\ngo 1.24\n").expect("write Go module");
    std::fs::write(
        directory.path().join("shape.go"),
        format!("package shape\n\nimport \"encoding/json\"\n\n{declarations}\n{generated}"),
    )
    .expect("write generated Go source");
    std::process::Command::new(go)
        .arg("test")
        .arg("./...")
        .current_dir(directory.path())
        .output()
        .expect("run Go compiler")
}

fn assert_go_compiles(generated: &str, declarations: &str) {
    let output = go_compile(generated, declarations);
    assert!(
        output.status.success(),
        "generated Go failed to compile:\n{}\n{generated}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn generated_go_compile_check_rejects_broken_source() {
    let output = go_compile("func broken() { missingSymbol() }", "");
    assert!(!output.status.success(), "compile control unexpectedly passed");
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
        ty: TypeRef::Optional(Box::new(TypeRef::Named("Choice".into()))),
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
    assert_go_compiles(
        &output,
        "func init() { data, _ := json.Marshal(Envelope{}); var object map[string]any; _ = json.Unmarshal(data, &object); if _, present := object[\"payload\"]; present { panic(\"nil payload was not omitted\") } }",
    );
}

#[test]
fn optional_non_emitted_named_fields_use_raw_message_in_struct_and_marshal_aux() {
    for name in ["Excluded", "Opaque", "Foreign", "VisitorOwned"] {
        let type_def = TypeDef {
            name: "Envelope".into(),
            fields: vec![
                FieldDef {
                    name: "payload".into(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named(name.into()))),
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
        let output = gen_struct_type(
            &type_def,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::from([type_def.name.as_str()]),
            &[],
        );
        assert_eq!(
            output.matches("Payload *json.RawMessage").count(),
            2,
            "{name}:\n{output}"
        );
        assert_go_compiles(&output, "");
    }
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
                ty: TypeRef::Optional(Box::new(TypeRef::Named("Choice".into()))),
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
