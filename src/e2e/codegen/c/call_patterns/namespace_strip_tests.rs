//! Regression coverage for the virtual-namespace strip in the C e2e accessor emitters.
//!
//! ~keep A fixture may group assertions under a virtual label that has no counterpart on the
//! result type: `interaction.total_count` addresses `total_count`. `FieldResolver::
//! result_relative_path` is the single definition of where such a field's value actually
//! lives, and its own doc says "do not add a fourth copy" -- but two hand-inlined copies of
//! that logic survived in this backend (`test_function`'s plain-function branch and
//! `call_patterns`'s engine-factory branch) while a third site had been migrated. Nothing
//! failed when the strip was deleted outright from all three: the 298 tests under
//! `e2e::codegen::c` never exercised a namespaced field path at all. These tests close that
//! gap, so a future re-inlining (or a regression in the shared helper) is caught here.
//!
//! ~keep Submodule of `call_patterns` (which owns the engine-factory branch, one of the two
//! deduped sites) rather than a sibling of `c.rs`: `c.rs` is over the repo's 1,000-line cap,
//! so registering a module there would grow a remediation target. See `file-modularization`
//! in CLAUDE.md and the sibling `batch_url_regression_tests`.

use std::collections::{HashMap, HashSet};

use super::super::*;
use crate::core::ir::{FieldDef, FunctionDef, PrimitiveType, TypeDef, TypeRef};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

fn set(entries: &[&str]) -> HashSet<String> {
    entries.iter().map(|s| (*s).to_string()).collect()
}

/// A result struct declaring `total_count` -- and pointedly NOT declaring `interaction`, so a
/// generator that fails to strip addresses a member the type does not have.
fn report_type_defs() -> Vec<TypeDef> {
    vec![TypeDef {
        name: "Report".into(),
        fields: vec![FieldDef {
            name: "total_count".into(),
            ty: TypeRef::Primitive(PrimitiveType::U64),
            ..FieldDef::default()
        }],
        ..TypeDef::default()
    }]
}

fn report_functions() -> Vec<FunctionDef> {
    vec![FunctionDef {
        name: "generate_report".into(),
        return_type: TypeRef::Named("Report".into()),
        ..FunctionDef::default()
    }]
}

/// The fixture asserts on the namespaced spelling; `result_fields` lists only the real field,
/// which is what opts the strip in (see `namespace_stripped_path`).
fn namespaced_fixture() -> Fixture {
    Fixture {
        id: "namespaced_total".into(),
        description: "assert a field grouped under a virtual label".into(),
        input: serde_json::json!({}),
        assertions: vec![Assertion {
            assertion_type: "equals".to_string(),
            field: Some("interaction.total_count".to_string()),
            value: Some(serde_json::json!(7)),
            ..Default::default()
        }],
        ..Fixture::default()
    }
}

fn namespaced_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &set(&["total_count"]),
        &HashSet::new(),
        &HashSet::new(),
    )
}

/// Builds the shared config for a plain (non-factory) call.
fn plain_e2e_config() -> E2eConfig {
    let mut e2e = E2eConfig::default();
    e2e.call.function = "generate_report".into();
    e2e.result_fields = set(&["total_count"]);
    e2e
}

fn sample_crate_config() -> ResolvedCrateConfig {
    ResolvedCrateConfig {
        name: "sample".into(),
        ..ResolvedCrateConfig::default()
    }
}

fn render(e2e: &E2eConfig) -> String {
    let fixture = namespaced_fixture();
    let config = sample_crate_config();
    let type_defs = report_type_defs();
    let functions = report_functions();
    let ir = CallIr {
        functions: &functions,
        type_defs: &type_defs,
    };

    render_test_file(
        "report",
        &[&fixture],
        "sample_ffi.h",
        "sample",
        "result",
        e2e,
        "c",
        &namespaced_resolver(),
        &config,
        &type_defs,
        &[],
        &[],
        ir,
    )
    .expect("test file renders")
}

/// Asserts the emitted accessor addresses the stripped path, and never navigates through the
/// virtual label. `interaction` is not a member of `Report`, so an unstripped accessor chain
/// does not compile against the generated header.
fn assert_addresses_the_stripped_field(rendered: &str) {
    assert!(
        rendered.contains("sample_report_total_count(result)"),
        "the virtual `interaction.` label must strip so the accessor reads `total_count` \
         directly off the result, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("sample_report_interaction("),
        "`interaction` is a virtual label, not a member of `Report` -- emitting an accessor \
         for it addresses a field the header never declares:\n{rendered}"
    );
}

/// The plain-function branch of `render_test_function_impl` (no client or engine factory).
#[test]
fn plain_function_branch_strips_the_virtual_namespace() {
    assert_addresses_the_stripped_field(&render(&plain_e2e_config()));
}

/// The engine-factory branch, which `call_patterns::render_engine_factory_test_function`
/// owns. It carried its own inlined copy of the strip, so it needs its own coverage: a fix
/// applied to only one of the two branches was the historic shape of this defect.
#[test]
fn engine_factory_branch_strips_the_virtual_namespace() {
    let mut e2e = plain_e2e_config();
    e2e.call.overrides.insert(
        "c".into(),
        crate::core::config::e2e::CallOverride {
            c_engine_factory: Some("CrawlConfig".into()),
            header: Some("sample_ffi.h".into()),
            ..Default::default()
        },
    );
    assert_addresses_the_stripped_field(&render(&e2e));
}
