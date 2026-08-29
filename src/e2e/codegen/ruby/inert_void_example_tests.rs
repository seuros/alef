//! The void-call fallback, tested against an IR-backed resolver.
//!
//! ~keep This lives here rather than in `examples.rs`'s own test module because the skip it
//! depends on became IR-derived in 0.77.0. `examples.rs`'s bare `resolver_knowing` helper wires
//! no IR, and without IR the hash-serialized-enum classifier deliberately never refuses -- so a
//! fixture written against that helper can only skip via the availability ORACLE, which
//! classifies as an authoring gap and makes the example an inert refusal. That is a different
//! branch from the one this test exists to pin. Relocating it also brought `examples.rs` back
//! under its recorded ratchet ceiling instead of over it.

use super::examples::render_example;
use crate::core::config::ResolvedCrateConfig;
use crate::core::ir::{EnumDef, EnumVariant, FieldDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::codegen::inert_example::take_inert_examples;
use crate::e2e::codegen::ruby::enum_variant_access::hash_serialized_enum_names;
use crate::e2e::config::E2eConfig;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use std::collections::{HashMap, HashSet};

fn field(name: &str, ty: TypeRef) -> FieldDef {
    FieldDef {
        name: name.to_string(),
        ty,
        ..FieldDef::default()
    }
}

/// A result type carrying one field whose type is a data-bearing enum. Magnus lowers such an ~keep
/// enum to a plain Ruby `Hash`, so no per-variant accessor exists -- a `GeneratorGap`, never an
/// authoring gap, which is what keeps the example publishable rather than refused.
fn ir() -> (Vec<TypeDef>, Vec<EnumDef>) {
    let type_defs = vec![TypeDef {
        name: "ProcessingResult".to_string(),
        fields: vec![field("encoding", TypeRef::Named("PayloadEncoding".to_string()))],
        ..TypeDef::default()
    }];
    let enums = vec![EnumDef {
        name: "PayloadEncoding".to_string(),
        variants: vec![EnumVariant {
            name: "Spreadsheet".to_string(),
            is_tuple: true,
            fields: vec![field("_0", TypeRef::Primitive(PrimitiveType::U32))],
            ..EnumVariant::default()
        }],
        ..EnumDef::default()
    }];
    (type_defs, enums)
}

fn resolver() -> FieldResolver {
    let (type_defs, enums) = ir();
    let map = FieldResolver::ir_enum_fields(&type_defs, &enums);
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
    .with_ir_enum_map(map, Some("ProcessingResult".to_string()))
    .with_ruby_hash_serialized_enum_names(hash_serialized_enum_names(&enums))
}

fn fixture_with(id: &str, assertions: Vec<Assertion>) -> Fixture {
    Fixture {
        id: id.to_string(),
        description: "test".to_string(),
        assertions,
        ..Default::default()
    }
}

fn render_void(fixture: &Fixture, field_resolver: &FieldResolver) -> String {
    let config = ResolvedCrateConfig::default();
    let type_defs: Vec<TypeDef> = Vec::new();
    render_example(
        fixture,
        "process",
        "SampleCrate",
        "SampleCrate",
        "result",
        &[],
        field_resolver,
        None,
        &HashMap::new(),
        &HashSet::new(),
        false,
        true,
        &E2eConfig::default(),
        None,
        &[],
        None,
        &config,
        &type_defs,
        &[],
    )
}

/// A `returns_void` call binds no `result`, so `test_function.jinja`'s `expect(result).not_to ~keep
/// be_nil` fallback has no subject. It gets the other honest, failable expectation instead --
/// "the call does not raise" -- rather than an example with no `expect` at all. The skip marker
/// naming what could not run must survive beside it, and the example must NOT be recorded as an
/// inert refusal: it still asserts something real.
#[test]
fn a_void_call_whose_assertions_all_skip_still_gets_a_failable_expectation() {
    let _ = take_inert_examples();
    let fixture = fixture_with(
        "void_all_skipped",
        vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("encoding.sheet_count".to_string()),
            value: Some(serde_json::json!(1)),
            ..Default::default()
        }],
    );

    let out = render_void(&fixture, &resolver());

    assert!(
        out.contains("expect { SampleCrate.process() }.not_to raise_error"),
        "a void call with nothing else to assert must still assert it did not raise, got:\n{out}"
    );
    assert!(
        out.contains("skipped:"),
        "the marker naming what could not run must survive beside the fallback, got:\n{out}"
    );
    assert!(
        take_inert_examples().is_empty(),
        "an example that still asserts something is not a refusal, got:\n{out}"
    );
}
