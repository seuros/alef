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

/// Name of the single `Test*` function the runtime fixture emits.
const RUNTIME_INVARIANT_TEST: &str = "TestGeneratedStructRuntimeInvariant";

/// Exactly how many Go tests a runtime fixture must execute.
const RUNTIME_INVARIANT_TEST_COUNT: usize = 1;

/// Compile the generated struct and evaluate `assertions` inside a real `Test*` function.
///
/// `go_compile` only type-checks, and its name is the whole hazard: `go test ./...` on a
/// package with no `_test.go` files prints `[no test files]` and exits 0 *without running the
/// package's `init` functions*. A runtime invariant routed through `assert_go_compiles` is
/// therefore never evaluated — verified by breaking the omitempty invariant on purpose and
/// watching that path stay green. Runtime invariants belong here, where an emitted test file
/// makes them execute and the executed-test inventory is asserted rather than assumed. ~keep
fn assert_go_runtime_invariant(generated: &str, assertions: &str) {
    let go = which::which("go").expect("Go is required for generated-Go runtime fixtures");
    let directory = tempfile::tempdir().expect("create Go runtime fixture");
    std::fs::write(
        directory.path().join("go.mod"),
        "module example.com/shape\n\ngo 1.24\n",
    )
    .expect("write Go module");
    std::fs::write(
        directory.path().join("shape.go"),
        format!("package shape\n\nimport \"encoding/json\"\n\n{generated}"),
    )
    .expect("write generated Go source");
    std::fs::write(
        directory.path().join("shape_test.go"),
        format!(
            "package shape\n\nimport (\n\t\"encoding/json\"\n\t\"testing\"\n)\n\nfunc {RUNTIME_INVARIANT_TEST}(t *testing.T) {{\n{assertions}}}\n"
        ),
    )
    .expect("write generated Go runtime test");
    let output = std::process::Command::new(go)
        .args(["test", "-v", "./..."])
        .current_dir(directory.path())
        .output()
        .expect("run Go runtime fixture");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The exact shape of the defect this helper replaces: no test file means the invariant
    // never runs, and `go test` reports that as a green `[no test files]`. ~keep
    assert!(
        !stdout.contains("[no test files]"),
        "the runtime fixture emitted no test file, so its invariant never ran:\n{stdout}"
    );
    assert!(
        output.status.success(),
        "generated Go runtime invariant failed:\n{stdout}\n{stderr}\n{generated}"
    );
    let executed: Vec<&str> = stdout
        .lines()
        .filter(|line| {
            line.starts_with("--- PASS:") || line.starts_with("--- FAIL:") || line.starts_with("--- SKIP:")
        })
        .collect();
    assert_eq!(
        executed.len(),
        RUNTIME_INVARIANT_TEST_COUNT,
        "expected exactly {RUNTIME_INVARIANT_TEST_COUNT} Go test to run, got {executed:?}:\n{stdout}"
    );
    assert!(
        executed[0].starts_with(&format!("--- PASS: {RUNTIME_INVARIANT_TEST} ")),
        "the runtime invariant test did not run and pass: {executed:?}\n{stdout}"
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
    assert_go_runtime_invariant(
        &output,
        concat!(
            "\tdata, err := json.Marshal(Envelope{})\n",
            "\tif err != nil {\n\t\tt.Fatalf(\"marshal zero envelope: %v\", err)\n\t}\n",
            "\tvar object map[string]any\n",
            "\tif err := json.Unmarshal(data, &object); err != nil {\n",
            "\t\tt.Fatalf(\"unmarshal marshalled envelope: %v\", err)\n\t}\n",
            "\tif _, present := object[\"payload\"]; present {\n",
            "\t\tt.Fatalf(\"nil payload was not omitted: %s\", data)\n\t}\n",
        ),
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
