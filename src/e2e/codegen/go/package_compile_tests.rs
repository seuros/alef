use std::path::Path;

use crate::core::config::e2e::CallConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Assertion, Fixture, FixtureGroup};

use super::GoCodegen;

fn sealed_choice_ir() -> (Vec<TypeDef>, Vec<EnumDef>, Vec<FunctionDef>) {
    let types = vec![TypeDef {
        name: "Envelope".into(),
        fields: vec![FieldDef {
            name: "choice".into(),
            ty: TypeRef::Named("Choice".into()),
            ..Default::default()
        }],
        ..Default::default()
    }];
    let enums = vec![EnumDef {
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
    }];
    let functions = vec![FunctionDef {
        name: "inspect".into(),
        return_type: TypeRef::Named("Envelope".into()),
        ..Default::default()
    }];
    (types, enums, functions)
}

fn fixture_with_assertion(assertion_type: &str, expected: &str) -> FixtureGroup {
    FixtureGroup {
        category: "shape".into(),
        fixtures: vec![Fixture {
            id: format!("choice_{assertion_type}"),
            description: "sealed choice assertion".into(),
            assertions: vec![Assertion {
                assertion_type: assertion_type.into(),
                field: Some("choice".into()),
                value: Some(serde_json::json!(expected)),
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
}

fn generate_package(assertion_type: &str, expected: &str) -> Vec<crate::core::backend::GeneratedFile> {
    let (types, enums, functions) = sealed_choice_ir();
    let config = E2eConfig {
        call: CallConfig {
            function: "inspect".into(),
            module: "example.com/sample".into(),
            returns_result: true,
            ..Default::default()
        },
        ..Default::default()
    };
    GoCodegen
        .generate(
            &[fixture_with_assertion(assertion_type, expected)],
            &config,
            &Default::default(),
            &types,
            &enums,
            &functions,
            &[],
        )
        .expect("generate complete Go e2e package")
}

fn write_generated_files(root: &Path, files: &[crate::core::backend::GeneratedFile]) {
    for file in files {
        let path = root.join(&file.path);
        std::fs::create_dir_all(path.parent().expect("generated file parent")).unwrap();
        std::fs::write(path, &file.content).unwrap();
    }
}

fn write_sample_package(root: &Path) {
    let package = root.join("packages/go");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("go.mod"), "module example.com/sample\n\ngo 1.26\n").unwrap();
    std::fs::write(
        package.join("sample.go"),
        "package sample\ntype Choice interface { isChoice() }\ntype ChoiceValue string\nfunc (ChoiceValue) isChoice() {}\ntype Envelope struct { Choice Choice }\nfunc Inspect() (*Envelope, error) { return &Envelope{Choice: ChoiceValue(\"value\")}, nil }\n",
    )
    .unwrap();
}

fn assert_generated_package_passes(assertion_type: &str, expected: &str) {
    let go = which::which("go").expect("Go is required for generated package compile fixtures");
    let root = tempfile::tempdir().expect("create generated Go package root");
    let files = generate_package(assertion_type, expected);
    write_generated_files(root.path(), &files);
    write_sample_package(root.path());
    let output = std::process::Command::new(go)
        .args(["test", "-mod=mod", "./..."])
        .current_dir(root.path().join("e2e/go"))
        .output()
        .expect("run complete generated Go package");
    assert!(
        output.status.success(),
        "{assertion_type} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn generated_data_interface_string_families_compile_and_run_with_real_helper() {
    for (assertion_type, expected) in [
        ("equals", "value"),
        ("contains", "value"),
        ("starts_with", "\"value"),
        ("ends_with", "value\""),
        ("matches_regex", "value"),
    ] {
        assert_generated_package_passes(assertion_type, expected);
    }
}

#[test]
fn generated_equals_data_interface_emits_json_helper() {
    let files = generate_package("equals", "value");
    assert!(
        files.iter().any(|file| file.path.ends_with("helpers_test.go")),
        "equals emits jsonString and must emit its package helper"
    );
}

#[test]
fn generated_json_helper_fails_test_on_marshal_error() {
    let helper = super::render_helpers_test_go();
    assert!(
        helper.contains("func jsonString(t *testing.T, value any) string"),
        "{helper}"
    );
    let go = which::which("go").expect("Go is required for generated helper runtime fixtures");
    let root = tempfile::tempdir().expect("create generated helper package");
    std::fs::write(root.path().join("go.mod"), "module example.com/helper\n\ngo 1.26\n").unwrap();
    std::fs::write(root.path().join("helpers_test.go"), helper).unwrap();
    std::fs::write(
        root.path().join("failure_test.go"),
        "package e2e_test\nimport \"testing\"\nfunc TestMarshalFailure(t *testing.T) { jsonString(t, make(chan int)) }\n",
    )
    .unwrap();
    let output = std::process::Command::new(go)
        .args(["test", "./..."])
        .current_dir(root.path())
        .output()
        .expect("run generated helper failure fixture");
    assert!(
        !output.status.success(),
        "marshal failure must fail the generated Go test"
    );
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(diagnostics.contains("marshal assertion value as JSON"), "{diagnostics}");
}
