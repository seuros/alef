//! Regression: brew must resolve a namespace-prefixed fixture field to the path the
//! value actually occupies in the CLI's JSON output.
//!
//! A fixture field like `batch.completed_count` groups the assertion under a virtual
//! `batch` label; the CLI prints `completed_count` at the top level. Brew built its jq
//! path from `FieldResolver::resolve`, which only applies aliases and never strips the
//! prefix, so it emitted `.batch.completed_count` — `null` against every real payload,
//! and therefore an assertion that checked nothing while reading as coverage.
//!
//! The prefix must only be stripped when it really is a virtual label: a genuinely
//! nested field (`metrics.total_lines`, where `metrics` is a declared result field)
//! must keep its full path.

use alef::core::config::NewAlefConfig;
use alef::e2e::codegen::E2eCodegen;
use alef::e2e::codegen::brew::BrewCodegen;
use alef::e2e::fixture::{Assertion, Fixture, FixtureGroup};

/// The shape the CLI actually prints for the call under test. `completed_count` is
/// top-level; `metrics` is a real nested object.
fn cli_payload() -> serde_json::Value {
    serde_json::json!({
        "completed_count": 2,
        "failed_count": 0,
        "total_count": 2,
        "results": [{ "url": "https://example.test/a" }],
        "metrics": { "total_lines": 41 }
    })
}

fn build_config() -> NewAlefConfig {
    let toml_src = r#"
[workspace]
languages = ["python"]

[[crates]]
name = "test-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["brew"]
result_fields = ["completed_count", "failed_count", "total_count", "results", "metrics"]

[crates.e2e.call]
function = "process"
module = "testlib"
result_var = "result"
args = [
  { name = "url", field = "url", type = "mock_url" },
]
"#;
    toml::from_str(toml_src).expect("config parses")
}

fn build_fixture_group() -> FixtureGroup {
    FixtureGroup {
        category: "batch".to_string(),
        fixtures: vec![Fixture {
            id: "namespaced_fields".to_string(),
            category: Some("batch".to_string()),
            description: "Namespace-prefixed and genuinely nested field assertions".to_string(),
            input: serde_json::json!({ "url": "/page1" }),
            assertions: vec![
                Assertion {
                    assertion_type: "equals".to_string(),
                    field: Some("batch.completed_count".to_string()),
                    value: Some(serde_json::json!(2)),
                    ..Assertion::default()
                },
                Assertion {
                    assertion_type: "equals".to_string(),
                    field: Some("metrics.total_lines".to_string()),
                    value: Some(serde_json::json!(41)),
                    ..Assertion::default()
                },
            ],
            source: "test.json".to_string(),
            ..Fixture::default()
        }],
    }
}

fn generate_category_script() -> String {
    let cfg = build_config();
    let resolved = cfg.clone().resolve().expect("config resolves").remove(0);
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config present");
    let files = BrewCodegen
        .generate(&[build_fixture_group()], &e2e, &resolved, &[], &[], &[], &[])
        .expect("brew generation succeeds");
    files
        .iter()
        .find(|f| f.path.to_string_lossy().ends_with("test_batch.sh"))
        .expect("test_batch.sh is emitted")
        .content
        .clone()
}

/// Every jq path the script extracts a value with, in emission order.
fn extracted_jq_paths(script: &str) -> Vec<String> {
    script
        .lines()
        .filter_map(|line| {
            let rest = line.split_once("| jq -r '")?.1;
            let path = rest.split_once('\'')?.0;
            Some(path.to_string())
        })
        .collect()
}

/// Resolve a simple dotted jq path (`.a.b`) against a JSON document.
fn resolve_jq_path<'a>(payload: &'a serde_json::Value, jq_path: &str) -> Option<&'a serde_json::Value> {
    let pointer = jq_path.replace('.', "/");
    payload.pointer(&pointer)
}

#[test]
fn namespace_prefixed_field_resolves_against_the_real_payload_shape() {
    let script = generate_category_script();
    let paths = extracted_jq_paths(&script);
    assert_eq!(
        paths,
        vec![".completed_count".to_string(), ".metrics.total_lines".to_string()],
        "the virtual `batch.` prefix must be stripped and the real `metrics.` path kept; got script:\n{script}"
    );

    // The generated paths must resolve to the asserted values against the payload the
    // CLI really prints — the check the emitted assertion is supposed to perform.
    let payload = cli_payload();
    let expected = [serde_json::json!(2), serde_json::json!(41)];
    for (jq_path, expected_value) in paths.iter().zip(expected.iter()) {
        let found = resolve_jq_path(&payload, jq_path)
            .unwrap_or_else(|| panic!("jq path {jq_path} resolves to nothing in the CLI payload"));
        assert_eq!(found, expected_value, "jq path {jq_path} read the wrong value");
    }
}

/// The literal path the bug emitted. Asserting its absence separately keeps the
/// failure message pointed at the defect rather than at a list mismatch.
#[test]
fn namespace_prefix_is_not_emitted_as_a_jq_object_step() {
    let script = generate_category_script();
    assert!(
        !script.contains(".batch.completed_count"),
        "the virtual namespace prefix must not appear in a jq path; got script:\n{script}"
    );
}

/// Sanity: `resolve_jq_path` is capable of failing, so the payload check above is not
/// vacuous. The pre-fix path is exactly what must not resolve.
#[test]
fn the_pre_fix_jq_path_resolves_to_nothing() {
    assert!(
        resolve_jq_path(&cli_payload(), ".batch.completed_count").is_none(),
        "the buggy path must not resolve — otherwise the payload check proves nothing"
    );
}
