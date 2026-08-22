//! Regression coverage for the Ruby e2e generator's enum-field classification.
//!
//! `render_assertion` decided whether a result field is enum-typed purely from the
//! hand-maintained `fields_enum` config (the global `[e2e] fields_enum` set plus a per-call
//! `[e2e.calls.<x>.overrides.ruby] enum_fields` override). A consumer whose `alef.toml` never
//! declared either got `field_is_enum = false` for an honest-to-goodness `DataNodeKind` enum
//! field, silently comparing the Magnus `Symbol` Ruby binds it as against the fixture's wire
//! `String` — `:key_value == "key_value"` is `false`, so the test would assert the wrong thing
//! rather than refuse to build.
//!
//! Unlike the sibling languages, Ruby's `render_assertion` ALREADY applies `.to_s` coercion
//! unconditionally for every string-valued `equals` comparison (`equals_emits_the_newline_
//! terminated_expected_verbatim` in `assertions.rs` pins this), so misclassification does not
//! change the emitted text for the common case — the safety net was already there. `field_is_enum`
//! itself, however, IS the value the rest of the codegen (and any future assertion type) trusts
//! as "this field is an enum", and it must be correct regardless of whether today's `equals`
//! happens to paper over a wrong answer. These tests therefore drive the real production wiring
//! `render_spec_file` performs — `FieldResolver::ir_enum_fields` + `with_ir_enum_map`, anchored
//! at the call's declared Rust return type via `resolve_declared_result_type` — and assert on
//! `FieldResolver::is_enum` directly, with no `fields_enum`/`enum_fields` config at all: the
//! classification must come from the IR alone. A `render_spec_file` smoke test at the bottom
//! confirms the wiring produces a real, non-panicking assertion end to end.

use crate::core::ir::{EnumDef, EnumVariant, FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::codegen::call_ir::{CallIr, resolve_declared_result_type};
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

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

fn call_config_for(call: &str) -> CallConfig {
    CallConfig {
        function: call.to_string(),
        result_var: "result".to_string(),
        ..CallConfig::default()
    }
}

/// Build a `FieldResolver` exactly the way `spec_file.rs::render_spec_file` builds
/// `fixture_call_resolver` for a fixture's resolved call — same functions, same call order,
/// same precedence (`with_enum_fields` before `with_ir_enum_map`, so an explicit config entry
/// still wins). Anything that changes here must change in lockstep with that real call site. ~keep
fn field_resolver_for(
    call_config: &CallConfig,
    e2e_config: &E2eConfig,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    functions: &[FunctionDef],
) -> FieldResolver {
    let call_root_type = resolve_declared_result_type(call_config, "ruby", CallIr { functions, type_defs });
    FieldResolver::new(
        e2e_config.effective_fields(call_config),
        e2e_config.effective_fields_optional(call_config),
        e2e_config.effective_result_fields(call_config),
        e2e_config.effective_fields_array(call_config),
        &std::collections::HashSet::new(),
    )
    .with_enum_fields(e2e_config.effective_fields_enum(call_config).clone())
    .with_ir_enum_map(FieldResolver::ir_enum_fields(type_defs, enums), call_root_type)
}

struct Case {
    name: &'static str,
    call: &'static str,
    expect_enum: bool,
}

const CASES: &[Case] = &[
    Case {
        name: "an enum-typed field with no fields_enum config is classified as enum via the IR",
        call: "process",
        expect_enum: true,
    },
    Case {
        name: "a same-named non-enum field on an unrelated type is not misclassified as enum",
        call: "other",
        expect_enum: false,
    },
    Case {
        name: "an Option<Enum> field is classified as enum via the IR",
        call: "process_optional",
        expect_enum: true,
    },
];

#[test]
fn enum_field_classification_table() {
    let (type_defs, enums, functions) = table_ir();
    for case in CASES {
        let call_config = call_config_for(case.call);
        let e2e_config = E2eConfig::default();
        let resolver = field_resolver_for(&call_config, &e2e_config, &type_defs, &enums, &functions);
        assert_eq!(
            resolver.is_enum("kind"),
            case.expect_enum,
            "{}: expected is_enum(\"kind\") = {}",
            case.name,
            case.expect_enum
        );
    }
}

/// An explicit `fields_enum` config entry keeps working unchanged (config wins) — the IR only
/// rescues fields the config never mentioned. `other.kind` is `String` in the IR, so only the
/// config entry can make this classify as enum.
#[test]
fn an_explicit_fields_enum_config_entry_still_classifies_as_enum() {
    let (type_defs, enums, functions) = table_ir();
    let call_config = call_config_for("other");
    let mut e2e_config = E2eConfig::default();
    e2e_config.fields_enum.insert("kind".to_string());
    let resolver = field_resolver_for(&call_config, &e2e_config, &type_defs, &enums, &functions);
    assert!(
        resolver.is_enum("kind"),
        "explicit fields_enum config must still classify the field as enum"
    );
}

/// End-to-end smoke test: the real entry point, `render_spec_file`, must still produce a real
/// (non-panicking, non-skipped) `equals` assertion for the enum-typed field with no config at
/// all, proving the classification wiring is actually reachable from `generate()` and not just
/// from this test's direct `FieldResolver` construction above.
#[test]
fn render_spec_file_emits_a_real_assertion_for_the_unconfigured_enum_field() {
    let (type_defs, enums, functions) = table_ir();
    let call_config = call_config_for("process");
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert("process".to_string(), call_config);
    let fixture = Fixture {
        id: "kind_smoke".to_string(),
        description: "Kind field smoke".to_string(),
        call: Some("process".to_string()),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("kind".to_string()),
            value: Some(serde_json::Value::String("key_value".to_string())),
            ..Assertion::default()
        }],
        ..Fixture::default()
    };
    let out = super::spec_file::render_spec_file(
        "kind_smoke",
        &[&fixture],
        "Sample",
        None,
        "sample",
        None,
        &std::collections::HashMap::new(),
        false,
        &e2e_config,
        false,
        false,
        &[],
        &crate::core::config::ResolvedCrateConfig::default(),
        &type_defs,
        &[],
        &enums,
        &functions,
    );
    assert!(
        out.contains("expect(result.kind.to_s).to eq('key_value')"),
        "got:\n{out}"
    );
    assert!(!out.contains("skipped"), "got:\n{out}");
}

/// The smoke test above uses a fixture value of `"key_value"`, which happens to already be
/// snake_case — it never exercised the shape a real, unconfigured `#[serde(rename_all)]`-less
/// enum actually produces on the wire. `DataNodeKind`'s real contract (see its rustdoc in
/// the consumer crate) is "unit variants serialize as a bare string (`"KeyValue"`)" —
/// verbatim PascalCase, not snake_case. `render_spec_file` must embed that fixture literal
/// exactly as written, with no case-folding of its own: the assertion codegen layer only knows
/// how to coerce the *actual* value via `.to_s`, never to normalize the *expected* value's
/// casing. (Whether the Magnus binding's runtime `Symbol` actually equals this verbatim string
/// is proven at the backend level by
/// `backends::magnus::gen_bindings::classes::tests::gen_enum_unit_variant_wire_value_is_verbatim_without_rename_all`
/// — this test only proves the e2e-codegen layer is an honest pass-through for that value.)
#[test]
fn render_spec_file_embeds_the_verbatim_pascalcase_fixture_value_for_a_no_rename_all_enum() {
    let (type_defs, enums, functions) = table_ir();
    let call_config = call_config_for("process");
    let mut e2e_config = E2eConfig::default();
    e2e_config.calls.insert("process".to_string(), call_config);
    let fixture = Fixture {
        id: "kind_wire_shape".to_string(),
        description: "Kind field matches the real no-rename_all wire contract".to_string(),
        call: Some("process".to_string()),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("kind".to_string()),
            value: Some(serde_json::Value::String("KeyValue".to_string())),
            ..Assertion::default()
        }],
        ..Fixture::default()
    };
    let out = super::spec_file::render_spec_file(
        "kind_wire_shape",
        &[&fixture],
        "Sample",
        None,
        "sample",
        None,
        &std::collections::HashMap::new(),
        false,
        &e2e_config,
        false,
        false,
        &[],
        &crate::core::config::ResolvedCrateConfig::default(),
        &type_defs,
        &[],
        &enums,
        &functions,
    );
    assert!(
        out.contains("expect(result.kind.to_s).to eq('KeyValue')"),
        "the expected literal must stay verbatim PascalCase, not be snake_cased by codegen:\n{out}"
    );
    assert!(!out.contains("skipped"), "got:\n{out}");
}
