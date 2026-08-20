//! Regression coverage for the Gleam e2e generator's enum-field classification.
//!
//! `render_test_case` used to decide whether a result field is enum-typed purely from the
//! hand-maintained `fields_enum` / `[e2e.call.overrides.gleam] enum_fields` config. A consumer
//! whose `alef.toml` never declared that entry got `r.kind |> should.equal("key_value")` for an
//! honest-to-goodness `DataNodeKind` enum field. The Gleam binding emits that enum as a custom
//! type (`pub type DataNodeKind { KeyValue ... }`, see `backends/gleam/gen_bindings/nif_external.rs`
//! `emit_enum`), and `should.equal` is homogeneous — a `String` can never be compared against a
//! `DataNodeKind`, so the generated module fails to compile.
//!
//! `test_case.rs` now wires the same IR-derived classification the rust and csharp e2e generators
//! use (`FieldResolver::ir_enum_fields` + `with_ir_enum_map`, anchored at the call's declared Rust
//! return type via `resolve_declared_result_type`). These tests drive the real entry point,
//! `render_test_case`, with no `fields_enum`/`enum_fields` config at all — the classification must
//! come from the IR alone. ~keep

use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::codegen::call_ir::CallIr;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

/// The emitted skip line the enum branch produces, minus the field name.
const ENUM_SKIP_MARKER: &str = "comparison not yet supported in Gleam e2e";

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
    let ir = CallIr { functions, type_defs };
    let mut out = String::new();
    super::test_case::render_test_case(
        &mut out,
        fixture,
        e2e_config,
        "sample",
        "process",
        "result",
        &[],
        &[],
        None,
        ir,
        enums,
        &[],
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
        let e2e_config = e2e_config_for(case.call, |_| {});
        let fixture = fixture_calling(case.call);
        let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
        let took_enum_branch = out.contains(ENUM_SKIP_MARKER);
        assert_eq!(
            took_enum_branch, case.expect_enum_branch,
            "{}: expected enum branch = {}, got:\n{out}",
            case.name, case.expect_enum_branch
        );
        assert_eq!(
            out.contains("should.equal(\"key_value\")"),
            !case.expect_enum_branch,
            "{}: a string comparison must be emitted only for the non-enum field, got:\n{out}",
            case.name
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
            "gleam".to_string(),
            crate::e2e::config::CallOverride {
                enum_fields: [("kind".to_string(), "DataNodeKind".to_string())].into_iter().collect(),
                ..crate::e2e::config::CallOverride::default()
            },
        );
    });
    let fixture = fixture_calling("other");
    let out = render(&fixture, &e2e_config, &type_defs, &enums, &functions);
    assert!(
        out.contains(ENUM_SKIP_MARKER),
        "explicit enum_fields config must still classify the field as enum, got:\n{out}"
    );
}
