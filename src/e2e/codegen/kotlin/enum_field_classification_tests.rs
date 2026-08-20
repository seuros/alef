//! Regression coverage for the Kotlin e2e generator's enum-field classification.
//!
//! `render_assertion` used to decide whether a result field is enum-typed from the
//! hand-maintained `fields_enum` config (plus a per-call `type_enum_fields` auto-detect that
//! itself needs a `result_type` override to anchor to a type). A consumer whose `alef.toml`
//! never declared either got `assertEquals("key_value", result.kind)` for an honest-to-goodness
//! `DataNodeKind` enum field. The Kotlin (JVM) binding wraps that enum in a Java enum type
//! exposing `.getValue()`, so `result.kind` is a `DataNodeKind`, not a `String` —
//! `assertEquals(String, DataNodeKind)` does not compile.
//!
//! `test_method.rs` now wires the same IR-derived classification the rust/csharp/gleam/swift/dart
//! e2e generators use (`FieldResolver::ir_enum_fields` + `with_ir_enum_map`, anchored at the
//! call's declared Rust return type via `resolve_declared_result_type`). These tests drive the
//! real entry point, `render_test_method`, with no `fields_enum`/`enum_fields` config at all —
//! the classification must come from the IR alone.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_method::render_test_method;

/// The `.getValue()` accessor the JVM Kotlin binding wraps enum-typed fields in — the
/// generated-code fingerprint of the enum branch.
const ENUM_MARKER: &str = ".getValue()";

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

fn e2e_config_for(call: &str, extra: impl FnOnce(&mut CallConfig)) -> E2eConfig {
    let mut call_config = CallConfig {
        function: call.to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    };
    extra(&mut call_config);
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
    render_test_method(
        &mut out,
        fixture,
        "Facade",
        "",
        "",
        &[],
        None,
        false,
        e2e_config,
        &std::collections::HashMap::new(),
        false,
        &ResolvedCrateConfig::default(),
        type_defs,
        enums,
        functions,
    )
    .expect("render_test_method succeeds");
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
        let e2e_config = e2e_config_for(case.call, |_| {});
        let fixture = fixture_calling(case.call);
        let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
        let took_enum_branch = out.contains(ENUM_MARKER);
        assert_eq!(
            took_enum_branch, case.expect_enum_branch,
            "{}: expected enum branch = {}, got:\n{out}",
            case.name, case.expect_enum_branch
        );
        assert_eq!(
            out.contains("assertEquals(\"key_value\", result.kind())"),
            !case.expect_enum_branch,
            "{}: a raw string comparison (no .getValue()) must be emitted only for the \
             non-enum field, got:\n{out}",
            case.name
        );
    }
}

/// An explicit per-call `fields_enum` config entry keeps working unchanged (config wins) — the
/// IR only rescues fields the config never mentioned. `other.kind` is `String` in the IR, so
/// only the config entry can make this classify as enum.
#[test]
fn an_explicit_fields_enum_config_entry_still_classifies_as_enum() {
    let (type_defs, enums, functions) = table_ir();
    let e2e_config = e2e_config_for("other", |_| {});
    let mut e2e_config = e2e_config;
    e2e_config.fields_enum.insert("kind".to_string());
    let fixture = fixture_calling("other");
    let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
    assert!(
        out.contains(ENUM_MARKER),
        "explicit fields_enum config must still classify the field as enum, got:\n{out}"
    );
}

/// Regression: kotlin_android's enum-field equals assertion must stringify through
/// `.toWire()`, not `.name.lowercase()`. `.name.lowercase()` assumes every wire value is
/// the Kotlin constant name lowercased with underscores (`IN_PROGRESS` -> `"in_progress"`),
/// which only holds for enums whose Rust source has `#[serde(rename_all = "snake_case")]`.
/// `DataNodeKind` (modeled by `data_node_kind_enum()`, no `rename_all`) serializes verbatim
/// (`KeyValue`, not `key_value`); its Kotlin constant `KEY_VALUE` lowercases to `"keyvalue"`,
/// never matching a fixture written against the real wire value. `.toWire()` is generated
/// per-variant from the same mapping the `@JsonProperty` annotation commits to, so it is
/// correct unconditionally.
#[test]
fn kotlin_android_enum_equals_assertion_uses_to_wire_not_name_lowercase() {
    let (type_defs, enums, functions) = table_ir();
    let e2e_config = e2e_config_for("process", |_| {});
    let fixture = Fixture {
        id: "kind_smoke".to_string(),
        description: "Kind field smoke".to_string(),
        call: Some("process".to_string()),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("kind".to_string()),
            value: Some(serde_json::Value::String("KeyValue".to_string())),
            ..Assertion::default()
        }],
        ..Fixture::default()
    };
    let mut out = String::new();
    render_test_method(
        &mut out,
        &fixture,
        "Facade",
        "",
        "",
        &[],
        None,
        false,
        &e2e_config,
        &std::collections::HashMap::new(),
        true,
        &ResolvedCrateConfig::default(),
        &type_defs,
        &enums,
        &functions,
    )
    .expect("render_test_method succeeds");
    assert!(
        out.contains(".toWire()"),
        "expected .toWire() for a kotlin_android enum field, got:\n{out}"
    );
    assert!(
        out.contains("\"KeyValue\""),
        "expected the fixture's wire literal verbatim (no case transform), got:\n{out}"
    );
    assert!(
        !out.contains(".lowercase()"),
        "kotlin_android enum equals assertions must not guess a case transform, got:\n{out}"
    );
}
