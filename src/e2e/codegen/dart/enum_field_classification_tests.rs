//! Regression coverage for the Dart e2e generator's enum-field classification.
//!
//! `render_test_case` used to decide whether a result field is enum-typed purely from the
//! hand-maintained `fields_enum` / `[e2e.call.overrides.dart] enum_fields` config
//! (`assertions.rs`'s `is_enum_field` in the `equals`/`field_equals` and `not_equals` arms). A
//! consumer whose `alef.toml` never declared that entry got NO `_alefE2eText` wrapper for an
//! honest-to-goodness `DataNodeKind` enum field.
//!
//! Unlike the statically-typed backends, this does not fail to compile: Dart accessors are
//! always property access (`result.kind`), so `expect(result.kind.toString(),
//! equals('key_value'))` compiles fine. It just asserts the WRONG string — `.toString()` on a
//! Dart enum returns its declaration name (`"DataNodeKind.keyValue"`), never the serde wire
//! value (`"key_value"`), so the generated test either fails at runtime or — worse — silently
//! passes only when the fixture's expected value happens to already be the accidental
//! `toString()` spelling. `_alefE2eText` is what performs the enum-to-wire-value conversion, so
//! its absence from the emitted assertion is the observable defect these tests check for.
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

/// The `_alefE2eText(...)` wrapper the enum branch emits — the observable signal that a field
/// was classified as enum-typed.
const ENUM_WRAPPER_MARKER: &str = "_alefE2eText(";

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
        name: "an enum-typed field with no fields_enum config gets _alefE2eText via the IR",
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
            "{}: expected _alefE2eText wrapper = {}, got:\n{out}",
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
