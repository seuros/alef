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

/// A payload-carrying `#[serde(untagged)]` union. `emit_enum` takes its `enum class` branch —
/// the only branch that declares `toWire()` — solely when every variant is fieldless, so this
/// shape is emitted as a Kotlin sealed class with neither `toWire()` nor the JVM facade's
/// `getValue()`.
fn stage_output_union() -> EnumDef {
    EnumDef {
        name: "StageOutput".to_string(),
        variants: vec![EnumVariant {
            name: "Text".to_string(),
            fields: vec![FieldDef {
                name: "_0".to_string(),
                ty: TypeRef::String,
                ..FieldDef::default()
            }],
            is_tuple: true,
            ..EnumVariant::default()
        }],
        serde_untagged: true,
        ..EnumDef::default()
    }
}

/// `process` returns `ProcessResult { kind: DataNodeKind }`, `other` returns
/// `OtherResult { kind: String }` (same leaf name, unrelated non-enum type — proves the
/// classification is anchored per-call rather than matching on the leaf name alone),
/// `process_optional` returns `OptionalResult { kind: Option<DataNodeKind> }`, and
/// `process_union` returns `UnionResult { kind: StageOutput }` — the payload-carrying shape,
/// carried in the same surface as the unit-only enum so one IR exercises both branches.
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
        TypeDef {
            name: "UnionResult".to_string(),
            fields: vec![kind_field(TypeRef::Named("StageOutput".to_string()), false)],
            ..TypeDef::default()
        },
    ];
    let enums = vec![data_node_kind_enum(), stage_output_union()];
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
        FunctionDef {
            name: "process_union".to_string(),
            return_type: TypeRef::Named("UnionResult".to_string()),
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
    let out = render_android(&fixture, &e2e_config, &type_defs, &enums, &functions);
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

/// Render `call` in kotlin_android mode (the `kotlin_android` positional flag `render` pins to
/// `false`), where enum-typed fields lower through `toWire()` rather than the JVM facade's
/// `getValue()`.
fn render_android(
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
        true,
        &ResolvedCrateConfig::default(),
        type_defs,
        enums,
        functions,
    )
    .expect("render_test_method succeeds");
    out
}

/// The compile-shape discriminator: one IR carrying BOTH a unit-only enum and a payload-carrying
/// union must lower only the first through the scalar accessor, in each Kotlin target.
///
/// `toWire()` (kotlin_android) and `getValue()` (JVM facade) are both emitted only on the
/// fieldless-variant branch of the binding backends' enum emitters, so applying either to a sealed
/// class is an unresolved reference at Kotlin compile time. Asserting the union case still renders
/// an `assertEquals(` against `result.kind` separates "the accessor was correctly withheld" from
/// "the assertion was skipped entirely", which would satisfy the absence check for the wrong
/// reason. ~keep
#[test]
fn a_payload_carrying_union_field_is_not_lowered_through_to_wire_or_get_value() {
    let (type_defs, enums, functions) = table_ir();
    let unit_config = e2e_config_for("process", |_| {});
    let union_config = e2e_config_for("process_union", |_| {});
    let unit_fixture = fixture_calling("process");
    let union_fixture = fixture_calling("process_union");

    let android_unit = render_android(&unit_fixture, &unit_config, &type_defs, &enums, &functions);
    assert!(
        android_unit.contains(".toWire()"),
        "the unit-only enum must still lower through .toWire() in kotlin_android, got:\n{android_unit}"
    );

    let android_union = render_android(&union_fixture, &union_config, &type_defs, &enums, &functions);
    assert!(
        !android_union.contains(".toWire()"),
        "a payload-carrying union is a Kotlin sealed class with no toWire(), got:\n{android_union}"
    );
    assert!(
        android_union.contains("assertEquals(") && android_union.contains("result.kind"),
        "the union assertion must still be rendered — an absent .toWire() only proves the fix \
         when the assertion itself was emitted, got:\n{android_union}"
    );

    let jvm_unit = render(&unit_fixture, &unit_config, &type_defs, &enums, &functions);
    assert!(
        jvm_unit.contains(ENUM_MARKER),
        "the unit-only enum must still lower through .getValue() on the JVM, got:\n{jvm_unit}"
    );

    let jvm_union = render(&union_fixture, &union_config, &type_defs, &enums, &functions);
    assert!(
        !jvm_union.contains(ENUM_MARKER),
        "the JVM facade declares no getValue() on the union wrapper either, got:\n{jvm_union}"
    );
    assert!(
        jvm_union.contains("assertEquals(") && jvm_union.contains("result.kind"),
        "the union assertion must still be rendered on the JVM path too, got:\n{jvm_union}"
    );
}
