//! Regression coverage for the Dart e2e generator's enum-field classification.
//!
//! `render_test_case` used to decide whether a result field is enum-typed purely from the
//! hand-maintained `fields_enum` / `[e2e.call.overrides.dart] enum_fields` config
//! (`assertions.rs`'s `is_enum_field` in the `equals`/`field_equals` and `not_equals` arms). A
//! consumer whose `alef.toml` never declared that entry got NO `.wireValue` accessor for an
//! honest-to-goodness `DataNodeKind` enum field.
//!
//! Unlike the statically-typed backends, this does not fail to compile: Dart accessors are
//! always property access (`result.kind`), so `expect(result.kind.toString(),
//! equals('key_value'))` compiles fine. It just asserts the WRONG string — `.toString()` on a
//! Dart enum returns its declaration name (`"DataNodeKind.keyValue"`), never the serde wire
//! value (`"KeyValue"`), so the generated test either fails at runtime or — worse — silently
//! passes only when the fixture's expected value happens to already be the accidental
//! `toString()` spelling. `.wireValue` (the extension `gen_bindings::wire_value` emits on the
//! generated enum) is what surfaces the real wire value, so its absence from the emitted
//! assertion is the observable defect these tests check for.
//!
//! `test_case.rs` now wires the same IR-derived classification the rust/csharp/swift/gleam e2e
//! generators use (`FieldResolver::ir_enum_fields` + `with_ir_enum_map`, anchored at the call's
//! declared Rust return type via `resolve_declared_result_type`). These tests drive the real
//! entry point, `render_test_case`, with no `fields_enum`/`enum_fields` config at all — the
//! classification must come from the IR alone. ~keep

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::config::{CallConfig, CallOverride, E2eConfig};
use crate::e2e::field_access::DartFirstClassMap;
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_case::{DartTestCaseContext, render_test_case};

/// The `.wireValue` accessor the enum branch emits — the observable signal that a field was
/// classified as enum-typed.
const ENUM_WRAPPER_MARKER: &str = ".wireValue";

/// A `DataNodeKind`-shaped enum: two unit variants, no serde rename overrides.
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

/// A payload-carrying `#[serde(untagged)]` union: flutter_rust_bridge renders this as a freezed
/// sealed class, NOT the plain Dart `enum` that `gen_bindings::wire_value::flat_wire_enums`
/// attaches the `.wireValue` extension to (its filter requires every variant to be fieldless).
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

fn e2e_config_for(call: &str, extra: impl FnOnce(&mut CallConfig)) -> E2eConfig {
    let mut call_config = CallConfig {
        function: call.to_string(),
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
    let config = ResolvedCrateConfig {
        name: "sample".to_string(),
        ..ResolvedCrateConfig::default()
    };
    let dart_first_class_map = DartFirstClassMap::default();
    let mut out = String::new();
    render_test_case(
        &mut out,
        fixture,
        DartTestCaseContext {
            e2e_config,
            lang: "dart",
            bridge_class: "Sample",
            dart_first_class_map: &dart_first_class_map,
            adapters: &[],
            config: &config,
            type_defs,
            enums,
            functions,
            errors: &[],
            native_typed_dtos: false,
            is_snippet: false,
        },
    );
    out
}

struct Case {
    name: &'static str,
    call: &'static str,
    expect_enum_wrapper: bool,
}

const CASES: &[Case] = &[
    Case {
        name: "an enum-typed field with no fields_enum config gets .wireValue via the IR",
        call: "process",
        expect_enum_wrapper: true,
    },
    Case {
        name: "a same-named non-enum field on an unrelated type is not misclassified as enum",
        call: "other",
        expect_enum_wrapper: false,
    },
    Case {
        name: "an Option<Enum> field is classified as enum via the IR",
        call: "process_optional",
        expect_enum_wrapper: true,
    },
    Case {
        name: "a payload-carrying union field does not get .wireValue (freezed sealed class has none)",
        call: "process_union",
        expect_enum_wrapper: false,
    },
];

#[test]
fn enum_field_classification_table() {
    let (type_defs, enums, functions) = table_ir();
    for case in CASES {
        let e2e_config = e2e_config_for(case.call, |_| {});
        let fixture = fixture_calling(case.call);
        let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
        let has_enum_wrapper = out.contains(ENUM_WRAPPER_MARKER);
        assert_eq!(
            has_enum_wrapper, case.expect_enum_wrapper,
            "{}: expected .wireValue wrapper = {}, got:\n{out}",
            case.name, case.expect_enum_wrapper
        );
    }
}

/// An explicit per-call `enum_fields` entry keeps working unchanged (config wins) — the IR only
/// rescues fields the config never mentioned. `other.kind` is `String` in the IR, so only the
/// config entry can make this classify as enum.
#[test]
fn an_explicit_enum_fields_config_entry_still_classifies_as_enum() {
    let (type_defs, enums, functions) = table_ir();
    let e2e_config = e2e_config_for("other", |call| {
        call.overrides.insert(
            "dart".to_string(),
            CallOverride {
                enum_fields: [("kind".to_string(), "DataNodeKind".to_string())].into_iter().collect(),
                ..CallOverride::default()
            },
        );
    });
    let fixture = fixture_calling("other");
    let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
    assert!(
        out.contains(ENUM_WRAPPER_MARKER),
        "explicit enum_fields config must still classify the field as enum, got:\n{out}"
    );
}

/// The compile-shape discriminator: one IR carrying BOTH a unit-only enum and a payload-carrying
/// union must lower only the first through `.wireValue`.
///
/// `flat_wire_enums` emits the `{{Enum}}WireValue` extension only for an enum whose every variant
/// is fieldless, so for `StageOutput` there is no such extension in the generated Dart at all and
/// `result.kind.wireValue` is an undefined getter — a `dart analyze` error, not a wrong-string
/// runtime failure. Asserting the union case still renders an `expect(...)` line separates "the
/// accessor was correctly withheld" from "the assertion was skipped entirely", which would make
/// the absence check pass for the wrong reason. ~keep
#[test]
fn a_payload_carrying_union_field_is_not_lowered_through_wire_value() {
    let (type_defs, enums, functions) = table_ir();

    let unit_out = render(
        &fixture_calling("process"),
        &e2e_config_for("process", |_| {}),
        &type_defs,
        &enums,
        &functions,
    );
    assert!(
        unit_out.contains(ENUM_WRAPPER_MARKER),
        "the unit-only enum must still lower through .wireValue, got:\n{unit_out}"
    );

    let union_out = render(
        &fixture_calling("process_union"),
        &e2e_config_for("process_union", |_| {}),
        &type_defs,
        &enums,
        &functions,
    );
    assert!(
        !union_out.contains(ENUM_WRAPPER_MARKER),
        "a payload-carrying union has no .wireValue extension in the generated Dart, so the \
         assertion must not reach for one, got:\n{union_out}"
    );
    assert!(
        union_out.contains("expect("),
        "the union assertion must still be rendered — an absent .wireValue only proves the fix \
         when the assertion itself was emitted, got:\n{union_out}"
    );
}
