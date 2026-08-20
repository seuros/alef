//! Regression coverage for the Elixir e2e generator's enum-field classification.
//!
//! `render_assertion` used to decide whether a result field is enum-typed purely from the
//! hand-maintained `fields_enum` config (the file-level `e2e.fields_enum` set plus a per-call
//! `enum_fields` override). A consumer whose `alef.toml` never declared either got a bare
//! `assert result.kind == "key_value"` for an honest-to-goodness `DataNodeKind` enum field.
//! The NIF binding serializes that field as an atom (`:key_value`), and Elixir does not fail to
//! compile on `:key_value == "key_value"` -- it silently evaluates to `false`, so the test
//! asserts the wrong thing rather than refusing to build.
//!
//! `render_test_case` now wires the same IR-derived classification the rust/csharp/gleam/swift/
//! dart e2e generators use (`FieldResolver::ir_enum_fields` + `with_ir_enum_map`, anchored at
//! the call's declared Rust return type via `resolve_declared_result_type`). These tests drive
//! the real entry point, `render_test_case`, with no `fields_enum`/`enum_fields` config at all --
//! the classification must come from the IR alone.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_case::render_test_case;

/// The `to_string(...)` atom-to-wire coercion Elixir wraps enum-typed fields in before
/// comparing against the fixture's wire-format expected value -- the generated-code
/// fingerprint of the enum branch.
const ENUM_MARKER: &str = "to_string(result.kind)";
/// The raw comparison emitted for a non-enum field -- the atom-vs-string mismatch that
/// silently evaluates to `false` at runtime rather than failing to compile.
const RAW_MARKER: &str = "assert result.kind == \"key_value\"";

fn data_node_kind_enum() -> EnumDef {
    EnumDef {
        name: "DataNodeKind".to_string(),
        variants: vec![
            EnumVariant {
                name: "KeyValue".to_string(),
                ..EnumVariant::default()
            },
            EnumVariant {
                name: "Sequence".to_string(),
                ..EnumVariant::default()
            },
        ],
        ..EnumDef::default()
    }
}

fn kind_field(ty: TypeRef, optional: bool) -> FieldDef {
    FieldDef {
        name: "kind".to_string(),
        ty,
        optional,
        ..FieldDef::default()
    }
}

/// `process` returns `ProcessResult { kind: DataNodeKind }`, `other` returns
/// `OtherResult { kind: String }` (same leaf name, unrelated non-enum type — proves the
/// classification is anchored per-call rather than matching on the leaf name alone), and
/// `process_optional` returns `OptionalResult { kind: Option<DataNodeKind> }`.
fn table_ir() -> (Vec<TypeDef>, Vec<EnumDef>, Vec<FunctionDef>) {
    let type_defs = vec![
        TypeDef {
            name: "ProcessResult".to_string(),
            fields: vec![kind_field(TypeRef::Named("DataNodeKind".to_string()), false)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "OtherResult".to_string(),
            fields: vec![kind_field(TypeRef::String, false)],
            ..TypeDef::default()
        },
        TypeDef {
            name: "OptionalResult".to_string(),
            fields: vec![kind_field(
                TypeRef::Optional(Box::new(TypeRef::Named("DataNodeKind".to_string()))),
                true,
            )],
            ..TypeDef::default()
        },
    ];
    let enums = vec![data_node_kind_enum()];
    let functions = vec![
        FunctionDef {
            name: "process".to_string(),
            return_type: TypeRef::Named("ProcessResult".to_string()),
            ..FunctionDef::default()
        },
        FunctionDef {
            name: "other".to_string(),
            return_type: TypeRef::Named("OtherResult".to_string()),
            ..FunctionDef::default()
        },
        FunctionDef {
            name: "process_optional".to_string(),
            return_type: TypeRef::Named("OptionalResult".to_string()),
            ..FunctionDef::default()
        },
    ];
    (type_defs, enums, functions)
}

fn fixture_calling(call: &str) -> Fixture {
    Fixture {
        id: "kind_smoke".to_string(),
        description: "Kind field smoke".to_string(),
        call: Some(call.to_string()),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("kind".to_string()),
            value: Some(serde_json::Value::String("key_value".to_string())),
            ..Assertion::default()
        }],
        ..Fixture::default()
    }
}

fn e2e_config_for(call: &str) -> E2eConfig {
    let call_config = CallConfig {
        function: call.to_string(),
        module: "MyLib".to_string(),
        result_var: "result".to_string(),
        returns_result: true,
        ..CallConfig::default()
    };
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert(call.to_string(), call_config);
    e2e_config
}

fn render(
    fixture: &Fixture,
    e2e_config: &E2eConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[FunctionDef],
) -> String {
    let mut out = String::new();
    render_test_case(
        &mut out,
        fixture,
        e2e_config,
        "",
        "",
        "",
        &[],
        None,
        None,
        &std::collections::HashMap::new(),
        None,
        &std::collections::HashSet::new(),
        &[],
        enums,
        &ResolvedCrateConfig::default(),
        type_defs,
        &[],
        functions,
    );
    out
}

struct Case {
    name: &'static str,
    call: &'static str,
    expect_enum_branch: bool,
}

const CASES: &[Case] = &[
    Case {
        name: "an enum-typed field with no fields_enum config is classified as enum via the IR",
        call: "process",
        expect_enum_branch: true,
    },
    Case {
        name: "a same-named non-enum field on an unrelated type is not misclassified as enum",
        call: "other",
        expect_enum_branch: false,
    },
    Case {
        name: "an Option<Enum> field is classified as enum via the IR",
        call: "process_optional",
        expect_enum_branch: true,
    },
];

#[test]
fn enum_field_classification_table() {
    let (type_defs, enums, functions) = table_ir();
    for case in CASES {
        let e2e_config = e2e_config_for(case.call);
        let fixture = fixture_calling(case.call);
        let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
        let took_enum_branch = out.contains(ENUM_MARKER);
        assert_eq!(
            took_enum_branch, case.expect_enum_branch,
            "{}: expected enum branch = {}, got:\n{out}",
            case.name, case.expect_enum_branch
        );
        // The dynamic-language failure mode: without the enum branch, Elixir does not fail to
        // compile -- it silently compares the atom the NIF returns against the fixture's wire
        // string, which is `false` for `:key_value == "key_value"`. Assert the raw comparison
        // is emitted ONLY on the non-enum path.
        assert_eq!(
            out.contains(RAW_MARKER),
            !case.expect_enum_branch,
            "{}: a raw comparison must be emitted only for the non-enum field, got:\n{out}",
            case.name
        );
    }
}

/// An explicit `fields_enum` config entry keeps working unchanged (config wins) — the IR only
/// rescues fields the config never mentioned. `other.kind` is `String` in the IR, so only the
/// config entry can make this classify as enum.
#[test]
fn an_explicit_fields_enum_config_entry_still_classifies_as_enum() {
    let (type_defs, enums, functions) = table_ir();
    let mut e2e_config = e2e_config_for("other");
    e2e_config.fields_enum.insert("kind".to_string());
    let fixture = fixture_calling("other");
    let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
    assert!(
        out.contains(ENUM_MARKER),
        "explicit fields_enum config must still classify the field as enum, got:\n{out}"
    );
}
