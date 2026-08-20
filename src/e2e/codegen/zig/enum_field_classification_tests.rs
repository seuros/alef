//! Regression coverage for the Zig e2e generator's enum-field classification.
//!
//! Before the four linked fixes this file's siblings (`result_is_json_struct_ir_tests.rs`,
//! `assertions.rs`'s `render_assertion`) close, the typed-struct assertion path — where a
//! Zig function returns a real struct rather than JSON `[]u8` — was unreachable for a JSON DTO
//! (every serde struct return was force-routed to `[]u8`, see `result_is_json_struct_ir_tests`)
//! and had no way to emit a real opaque-handle method-call accessor even when it was reached
//! (`optional_renderers::render_zig_with_optionals`). `render_assertion` decided whether a
//! result field was enum-typed purely from the hand-maintained `fields_enum`/
//! `[overrides.zig].enum_fields` config, and — because that path was unreachable — a consumer
//! whose `alef.toml` never populated it never actually hit `try testing.expectEqual("value",
//! result.kind)` against a real Zig enum in practice. Now that the surrounding path is live,
//! that dead classification becomes a live compile-error source: a Zig enum does not compare
//! against a `[]const u8` literal via `testing.expectEqual`.
//!
//! `render_assertion` now wires the same IR-derived classification the gleam e2e generator uses
//! (`FieldResolver::ir_enum_fields` + `with_ir_enum_map`, anchored at the call's declared Rust
//! return type via `resolve_declared_result_type`), and skips the `equals` assertion instead of
//! emitting code that cannot compile. These tests drive the real entry point,
//! `render_test_file`, with no `fields_enum`/`enum_fields` config at all — the classification
//! must come from the IR alone.

use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::codegen::call_ir::CallIr;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::{Assertion, Fixture};

use super::test_file::render_test_file;

/// The emitted skip line the enum branch produces, minus the field name.
const ENUM_SKIP_MARKER: &str = "comparison not yet supported on zig's typed-struct result";

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
///
/// None of the three types declares `has_serde`, so `result_is_json_struct_ir_tests`'s fix
/// does not route any of these calls through the JSON path — they stay on the typed-struct
/// accessor path this module's fix targets.
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
    render_test_file(
        "smoke",
        &[fixture],
        e2e_config,
        "process",
        "result",
        &[],
        "sample",
        "sample",
        &ResolvedCrateConfig::default(),
        type_defs,
        &[],
        ir,
        enums,
    )
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
            out.contains("testing.expectEqual(\"key_value\""),
            !case.expect_enum_branch,
            "{}: a raw-literal comparison must be emitted only for the non-enum field, got:\n{out}",
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
            "zig".to_string(),
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
