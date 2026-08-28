//! A WASM test file must import every enum class its own body names, spelled the way the body
//! spells it.
//!
//! Both halves matter and fail identically at run time (`ReferenceError: WasmInputKind is not
//! defined`): an import that is missing, and an import that is present under a different name
//! than the reference. The import block used to be derived from the hand-written `enum_fields`
//! config rather than from the emitter, which produced exactly those two failures — a field whose
//! enum type comes from the IR with no config entry was referenced and never imported, and the
//! fallback import path spelled the bare IR name where the body emits the `wasm_type_prefix`-ed
//! one.
//!
//! Every assertion below therefore compares the EMITTED reference against the EMITTED import,
//! never against a hardcoded name alone: an assertion that only checks "some import exists" is
//! satisfied by an import naming the wrong symbol. ~keep

use super::*;

const PKG: &str = "@sample/wasm";
const WASM_TYPE_PREFIX: &str = "Wasm";
const ENUM_NAME: &str = "InputKind";

fn json_object_arg(element_type: &str) -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: "input".to_string(),
        field: "input".to_string(),
        arg_type: "json_object".to_string(),
        optional: false,
        owned: false,
        element_type: Some(element_type.to_string()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn handle_arg() -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: "client".to_string(),
        field: "client_config".to_string(),
        arg_type: "handle".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn not_error_fixture(id: &str, input: serde_json::Value) -> Fixture {
    Fixture {
        id: id.to_string(),
        category: Some("document".to_string()),
        description: "process a document".to_string(),
        input,
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn input_kind_enum() -> EnumDef {
    EnumDef {
        name: ENUM_NAME.to_string(),
        variants: vec![
            crate::core::ir::EnumVariant {
                name: "Uri".to_string(),
                ..Default::default()
            },
            crate::core::ir::EnumVariant {
                name: "Bytes".to_string(),
                ..Default::default()
            },
        ],
        serde_rename_all: Some("snake_case".to_string()),
        ..Default::default()
    }
}

fn type_def(name: &str, fields: Vec<crate::core::ir::FieldDef>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        fields,
        ..Default::default()
    }
}

fn field(name: &str, ty: TypeRef) -> crate::core::ir::FieldDef {
    crate::core::ir::FieldDef {
        name: name.to_string(),
        ty,
        ..Default::default()
    }
}

/// The right-hand side the generated body assigns to `<something>.<field>`.
fn assigned_expression(output: &str, field: &str) -> String {
    let needle = format!(".{field} = ");
    let start = output
        .find(&needle)
        .unwrap_or_else(|| panic!("generated body must assign `{field}`:\n{output}"))
        + needle.len();
    let rest = &output[start..];
    let end = rest
        .find(';')
        .unwrap_or_else(|| panic!("assignment to `{field}` must terminate:\n{output}"));
    rest[..end].to_string()
}

/// The class the generated body names on the left of the `.` in `EnumClass.Member`.
fn referenced_class(output: &str, field: &str) -> String {
    let expression = assigned_expression(output, field);
    let (class_name, member) = expression
        .split_once('.')
        .unwrap_or_else(|| panic!("`{field}` must be assigned an `EnumClass.Member` reference, got `{expression}`"));
    assert!(
        !member.is_empty() && member.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "`{field}` must be assigned a plain enum member, got `{expression}`"
    );
    class_name.to_string()
}

/// The identifiers the generated file imports from the binding package.
fn binding_imports(output: &str) -> Vec<String> {
    let line = output
        .lines()
        .find(|line| line.starts_with("import") && line.contains(PKG))
        .unwrap_or_else(|| panic!("generated test must import its bindings:\n{output}"));
    let open = line.find("{ ").expect("binding import must be a named-import list");
    let close = line.rfind(" }").expect("binding import must be a named-import list");
    line[open + 2..close]
        .split(", ")
        .map(|entry| entry.trim().to_string())
        .collect()
}

fn assert_reference_is_imported(output: &str, field: &str, expected_class: &str) {
    let referenced = referenced_class(output, field);
    assert_eq!(
        referenced, expected_class,
        "the body must reference the enum class the wasm binding exports:\n{output}"
    );
    let imports = binding_imports(output);
    assert!(
        imports.iter().any(|identifier| identifier == &referenced),
        "the body references `{referenced}.…` but the binding import list is {imports:?}; an \
         import naming a different symbol fails exactly like a missing one:\n{output}"
    );
}

/// The defect: a wasm struct field whose enum type is known only from the IR.
///
/// No `enum_fields` entry exists for `kind`, so an import list derived from that config learns
/// nothing about `WasmInputKind` — while the body emits `WasmInputKind.Uri` from the field's
/// declared `TypeRef::Named`.
#[test]
fn wasm_imports_an_enum_referenced_from_the_ir_with_no_hand_written_entry() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "processDocument".to_string();
    e2e_config.call.args = vec![json_object_arg("DocumentInput")];
    let fixture = not_error_fixture("process_document", serde_json::json!({ "kind": "uri" }));

    let output = render_test_file(
        "wasm",
        "document",
        &[&fixture],
        "",
        PKG,
        "processDocument",
        &[],
        None,
        None,
        &e2e_config,
        &[type_def(
            "DocumentInput",
            vec![field("kind", TypeRef::Named(ENUM_NAME.to_string()))],
        )],
        &[input_kind_enum()],
        &[],
        WASM_TYPE_PREFIX,
        &Default::default(),
        &[],
    );

    assert_reference_is_imported(&output, "kind", &format!("{WASM_TYPE_PREFIX}{ENUM_NAME}"));
}

/// Control: a hand-written `enum_fields` entry still resolves, and still agrees with the import.
///
/// The field is declared `String` in the IR, so the reference can only come from the config
/// override — the path that already worked and must keep working.
#[test]
fn wasm_imports_an_enum_named_by_a_hand_written_enum_fields_entry() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "processDocument".to_string();
    e2e_config.call.args = vec![json_object_arg("DocumentInput")];
    e2e_config.call.overrides.insert(
        "wasm".to_string(),
        crate::e2e::config::CallOverride {
            enum_fields: [("kind".to_string(), ENUM_NAME.to_string())].into_iter().collect(),
            ..Default::default()
        },
    );
    let fixture = not_error_fixture("process_document", serde_json::json!({ "kind": "uri" }));

    let output = render_test_file(
        "wasm",
        "document",
        &[&fixture],
        "",
        PKG,
        "processDocument",
        &[],
        None,
        None,
        &e2e_config,
        &[type_def("DocumentInput", vec![field("kind", TypeRef::String)])],
        &[input_kind_enum()],
        &[],
        WASM_TYPE_PREFIX,
        &Default::default(),
        &[],
    );

    assert_reference_is_imported(&output, "kind", &format!("{WASM_TYPE_PREFIX}{ENUM_NAME}"));
}

/// The prefix half of the same defect, on the handle-config path.
///
/// A handle arg with no `json_object` arg alongside it leaves `needs_options_import` false, so the
/// import list is assembled by the trailing WASM fallback block. That block used to push the bare
/// `enum_fields` value while the body emitted the prefixed class — an import that exists, names a
/// symbol the package does not export, and passes any assertion that only counts imports. ~keep
#[test]
fn wasm_handle_config_imports_the_prefixed_enum_class_the_body_references() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "fetchDocument".to_string();
    e2e_config.call.args = vec![handle_arg()];
    e2e_config.call.overrides.insert(
        "wasm".to_string(),
        crate::e2e::config::CallOverride {
            handle_config_type: Some("WasmClientConfig".to_string()),
            nested_types: [("retry".to_string(), "WasmRetryConfig".to_string())]
                .into_iter()
                .collect(),
            enum_fields: [("kind".to_string(), ENUM_NAME.to_string())].into_iter().collect(),
            ..Default::default()
        },
    );

    let fixture = not_error_fixture(
        "fetch_document",
        serde_json::json!({ "client_config": { "retry": { "kind": "bytes" } } }),
    );

    let output = render_test_file(
        "wasm",
        "document",
        &[&fixture],
        "",
        PKG,
        "fetchDocument",
        &[handle_arg()],
        None,
        None,
        &e2e_config,
        &[
            type_def("ClientConfig", vec![]),
            type_def("RetryConfig", vec![field("kind", TypeRef::String)]),
        ],
        &[input_kind_enum()],
        &[],
        WASM_TYPE_PREFIX,
        &Default::default(),
        &[],
    );

    assert_reference_is_imported(&output, "kind", &format!("{WASM_TYPE_PREFIX}{ENUM_NAME}"));
}
