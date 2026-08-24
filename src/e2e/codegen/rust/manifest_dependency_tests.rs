//! The generated e2e crate must declare every crate its own emitted test bodies name.
//!
//! ~keep These tests run the whole `RustE2eCodegen::generate` pass rather than
//! `render_cargo_toml` alone: the defect they pin is a disagreement between two files of the
//! same crate — the manifest and the test body — so only a check that reads both can see it.
//! Each pair is a positive and a negative: a manifest that declared the crate unconditionally
//! would satisfy the positive half while proving nothing.

use super::{RUST_JSON_CRATE_PATH, RustE2eCodegen};
use crate::core::config::{NewAlefConfig, ResolvedCrateConfig};
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::codegen::streaming_assertions::RUST_STREAM_CRATE_PATH;
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::{Fixture, FixtureGroup};

/// Parse a `[crates.e2e]` block into the pair `generate` needs.
fn config(text: &str) -> (E2eConfig, ResolvedCrateConfig) {
    let parsed: NewAlefConfig = toml::from_str(text).expect("e2e config must parse");
    let e2e = parsed.crates[0].e2e.clone().expect("e2e config present");
    let resolved = parsed.resolve().expect("config resolves").remove(0);
    (e2e, resolved)
}

/// One group holding one fixture, asserting up front that the fixture wants no mock server —
/// no `mock_response`, no `http` block, no `input.mock_responses`. Every case below depends on
/// that, because the manifest conditions under test were all reachable only through it.
fn lone_group(fixture: serde_json::Value) -> Vec<FixtureGroup> {
    let fixture: Fixture = serde_json::from_value(fixture).expect("fixture must parse");
    assert!(
        !fixture.needs_mock_server(),
        "the whole point of these fixtures is that they need no mock server"
    );
    vec![FixtureGroup {
        category: "generated".to_string(),
        fixtures: vec![fixture],
    }]
}

/// `(manifest, joined test bodies)` for the generated crate.
fn generate(config_text: &str, fixture: serde_json::Value) -> (String, String) {
    let (e2e, resolved) = config(config_text);
    let files = RustE2eCodegen
        .generate(&lone_group(fixture), &e2e, &resolved, &[], &[], &[], &[])
        .expect("rust e2e crate generates");

    let manifest = files
        .iter()
        .find(|file| file.path.file_name().is_some_and(|name| name == "Cargo.toml"))
        .map(|file| file.content.clone())
        .expect("generated crate has a Cargo.toml");
    let bodies = files
        .iter()
        .filter(|file| file.path.extension().is_some_and(|ext| ext == "rs"))
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (manifest, bodies)
}

/// A streaming call, parameterized only by the `streaming` flag that decides whether the emitted
/// body drains a stream.
fn streaming_config(streaming: bool) -> String {
    format!(
        r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"

[crates.e2e.call]
function = "read_events"
module = "sample_core"
result_var = "result"
async = true
streaming = {streaming}
"#
    )
}

fn streaming_fixture() -> serde_json::Value {
    serde_json::json!({
        "id": "read_events_ok",
        "description": "read the events",
        "input": null,
        "assertions": [{"type": "not_error"}]
    })
}

/// A call whose result carries a collection field, so a `contains` assertion over it takes the
/// containment recipe's serializing arm.
const COLLECTION_CONFIG: &str = r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
fields_array = ["items"]

[crates.e2e.call]
function = "read_items"
module = "sample_core"
result_var = "result"
"#;

fn collection_fixture(assertion: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": "read_items_ok",
        "description": "read the items",
        "input": null,
        "assertions": [assertion]
    })
}

/// `StreamingFieldResolver::collect_snippet` writes `tokio_stream::StreamExt` into every Rust
/// streaming body. The manifest used to key that dependency off `needs_mock_server`, a flag the
/// recipe never consults, so a streaming fixture that mocks nothing emitted a test file naming a
/// crate the crate did not depend on: E0433 at `cargo test`, not at generation.
#[test]
fn a_streaming_fixture_without_a_mock_server_declares_the_stream_crate() {
    let (manifest, bodies) = generate(&streaming_config(true), streaming_fixture());

    assert!(
        bodies.contains(RUST_STREAM_CRATE_PATH),
        "premise broken: the emitted body no longer names the stream crate, so this test proves \
         nothing about the manifest:\n{bodies}"
    );
    assert!(
        manifest.contains("tokio-stream = "),
        "the body names `tokio_stream::` but the manifest declares no `tokio-stream`:\n{manifest}"
    );
}

/// The negative half: the dependency must follow the body, not appear unconditionally.
#[test]
fn a_non_streaming_fixture_does_not_declare_the_stream_crate() {
    let (manifest, bodies) = generate(&streaming_config(false), streaming_fixture());

    assert!(
        !bodies.contains(RUST_STREAM_CRATE_PATH),
        "premise broken: a non-streaming body must not name the stream crate:\n{bodies}"
    );
    assert!(
        !manifest.contains("tokio-stream"),
        "no emitted body names the stream crate, so the manifest must not declare it:\n{manifest}"
    );
}

/// The same disagreement one crate over: `assertions::containment_predicate` serializes each
/// element through `serde_json::to_value` for a `contains` over a collection field, while
/// `needs_serde_json` only ever read the call's argument types.
#[test]
fn a_collection_containment_assertion_declares_the_json_crate() {
    let (manifest, bodies) = generate(
        COLLECTION_CONFIG,
        collection_fixture(serde_json::json!({"type": "contains", "field": "items", "value": "widget"})),
    );

    assert!(
        bodies.contains(RUST_JSON_CRATE_PATH),
        "premise broken: the containment recipe no longer names the json crate:\n{bodies}"
    );
    assert!(
        manifest.contains("serde_json = "),
        "the body names `serde_json::` but the manifest declares no `serde_json`:\n{manifest}"
    );
}

/// The negative half: an assertion that needs no serialization must not pull the crate in.
#[test]
fn a_collection_emptiness_assertion_does_not_declare_the_json_crate() {
    let (manifest, bodies) = generate(
        COLLECTION_CONFIG,
        collection_fixture(serde_json::json!({"type": "not_empty", "field": "items"})),
    );

    assert!(
        !bodies.contains(RUST_JSON_CRATE_PATH),
        "premise broken: an emptiness check must not name the json crate:\n{bodies}"
    );
    assert!(
        !manifest.contains("serde_json"),
        "no emitted body names the json crate, so the manifest must not declare it:\n{manifest}"
    );
}
