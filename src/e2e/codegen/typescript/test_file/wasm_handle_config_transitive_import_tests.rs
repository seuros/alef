//! A handle-owner fixture's config value is materialized through the same recursive builder
//! (`ts_builder_expression_inner`, reached via `handle_values.rs::render_value`) that constructs
//! `json_object` config args. That builder consults BOTH the IR (`derive_nested_types_for_wasm`)
//! AND the call's raw `nested_types` override at every recursion depth, so a class reachable only
//! through an override-introduced key -- a fixture-authoring convenience naming a field the IR
//! does not literally declare, e.g. `nested_types.auth = "WasmAuthConfig"` on a `CrawlConfig` IR
//! type with no `auth` field -- is still built correctly: `WasmAuthConfig.default()` inside an
//! IIFE assigned to `.auth`.
//!
//! `render_test_file`'s import collector used to derive its transitive nested-class walk
//! (`collect_transitive_nested_types_for_wasm`) from `type_defs` alone, never consulting the
//! override. It could therefore follow the IR as far as real struct fields went, but never across
//! an override-introduced edge -- so anything reachable ONLY beyond that edge (a further-nested
//! class the override's own target type `AuthConfig` declares as a REAL IR field, e.g.
//! `AuthConfig.ssrf: SsrfPolicy`) was built into the body by the emitter's recursion but never
//! reached by this collector: `ReferenceError: WasmSsrfPolicy is not defined` at runtime, `tsc`
//! silent because `WasmSsrfPolicy` is a perfectly valid identifier the import line just never
//! named. This survives even a correctly-configured `nested_types.auth` entry, because that entry
//! names the edge into `AuthConfig`, not the edge onward into `SsrfPolicy`.
//!
//! Split out of `tests.rs` (a remediation target at the 1,000-line cap, pinned by
//! `tests/file_size_baseline.txt`, and must not grow) rather than added there. ~keep

use super::tests::{make_field, make_type};
use super::*;
use crate::core::ir::{PrimitiveType, TypeRef};

/// `CrawlConfig` (the handle-owner's config type) has real IR fields, but none named `auth` --
/// the "auth" edge exists only in the call override below. `AuthConfig` genuinely declares
/// `ssrf: SsrfPolicy` as an IR field, so once the walk reaches `AuthConfig` (via the override),
/// `SsrfPolicy` is a real, IR-reachable next hop.
fn handle_config_type_defs() -> Vec<TypeDef> {
    vec![
        make_type(
            "CrawlConfig",
            vec![make_field("max_depth", TypeRef::Primitive(PrimitiveType::U32))],
        ),
        make_type(
            "AuthConfig",
            vec![make_field("ssrf", TypeRef::Named("SsrfPolicy".to_string()))],
        ),
        make_type(
            "SsrfPolicy",
            vec![make_field("allow_private", TypeRef::Primitive(PrimitiveType::Bool))],
        ),
    ]
}

fn crawl_call_args() -> Vec<ArgMapping> {
    vec![ArgMapping {
        name: "engine".to_string(),
        field: "engine".to_string(),
        arg_type: "handle".to_string(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }]
}

fn crawl_fixture() -> Fixture {
    Fixture {
        id: "crawl_with_auth".to_string(),
        category: Some("crawl".to_string()),
        description: "crawl with an authenticated engine".to_string(),
        input: serde_json::json!({ "engine": { "auth": { "ssrf": { "allow_private": true } } } }),
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "not_error".to_string(),
            ..Default::default()
        }],
        ..Fixture::default()
    }
}

fn binding_import_line<'a>(output: &'a str, pkg_name: &str) -> &'a str {
    output
        .lines()
        .find(|line| line.starts_with("import") && line.contains(pkg_name))
        .unwrap_or_else(|| panic!("must render a binding import line, got:\n{output}"))
}

/// The regression this file exists to pin: `SsrfPolicy` is reachable only by first crossing the
/// override-introduced `auth` edge, then a real IR field on `AuthConfig`. Both the body's
/// constructed class and the collected import must agree on `WasmSsrfPolicy`.
#[test]
fn wasm_handle_config_class_reached_only_through_an_override_edge_is_imported() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "crawl".to_string();
    e2e_config.call.args = crawl_call_args();
    e2e_config.call.overrides.insert(
        "wasm".to_string(),
        crate::e2e::config::CallOverride {
            handle_config_type: Some("WasmCrawlConfig".to_string()),
            nested_types: [("auth".to_string(), "WasmAuthConfig".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        },
    );
    let fixture = crawl_fixture();
    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "wasm",
        "crawl",
        &[&fixture],
        "",
        "@test/wasm",
        "crawl",
        &e2e_config.call.args,
        None,
        None,
        &e2e_config,
        &handle_config_type_defs(),
        &[],
        &[],
        "Wasm",
        &config,
        &[],
    );

    assert!(
        output.contains("WasmAuthConfig.default()"),
        "the outer override-named class must still be constructed;\n{output}"
    );
    assert!(
        output.contains("WasmSsrfPolicy.default()"),
        "the class reached only beyond the override edge must still be constructed;\n{output}"
    );
    assert_eq!(
        binding_import_line(&output, "@test/wasm"),
        "import { crawl, createEngine, WasmCrawlConfig, WasmAuthConfig, WasmSsrfPolicy } from \"@test/wasm\";",
        "the import must carry every class the body constructs, including the one reached only \
         beyond the override edge, full output:\n{output}"
    );
}

/// Negative control proving the assertion above is not vacuous: with no `nested_types.auth`
/// entry, "auth" is not a class-typed key at all -- it renders as a plain object literal, and
/// neither `WasmAuthConfig` nor `WasmSsrfPolicy` is constructed or needs importing. If this test
/// and the one above ever produced the same import line, that would be proof the collector is not
/// actually reading the override.
#[test]
fn wasm_handle_config_without_the_override_edge_imports_neither_nested_class() {
    let mut e2e_config = E2eConfig::default();
    e2e_config.call.function = "crawl".to_string();
    e2e_config.call.args = crawl_call_args();
    e2e_config.call.overrides.insert(
        "wasm".to_string(),
        crate::e2e::config::CallOverride {
            handle_config_type: Some("WasmCrawlConfig".to_string()),
            ..Default::default()
        },
    );
    let fixture = crawl_fixture();
    let config = crate::core::config::ResolvedCrateConfig::default();

    let output = render_test_file(
        "wasm",
        "crawl",
        &[&fixture],
        "",
        "@test/wasm",
        "crawl",
        &e2e_config.call.args,
        None,
        None,
        &e2e_config,
        &handle_config_type_defs(),
        &[],
        &[],
        "Wasm",
        &config,
        &[],
    );

    assert!(
        !output.contains("WasmAuthConfig.default()") && !output.contains("WasmSsrfPolicy.default()"),
        "with no override edge, neither class should be constructed;\n{output}"
    );
    assert_eq!(
        binding_import_line(&output, "@test/wasm"),
        "import { crawl, createEngine, WasmCrawlConfig } from \"@test/wasm\";",
        "with no override edge, the import must name only the handle config type, full output:\n{output}"
    );
}
