//! A generated Rust snippet must declare the crates its own recipe wrote into it.
//!
//! ~keep The check project the Rust validator builds gets its `[dependencies]` from the
//! snippet's `requires` list, and nothing else. A fixture never declares these: the streaming
//! recipe is what puts `tokio_stream::StreamExt` in the body, and `#[tokio::main]` comes from the
//! snippet template. When the recipe adds a path and the requirement list does not follow, the
//! snippet ships and fails at `E0433: cannot find module or crate` — which is not an environment
//! gap the reader can fix, but a dependency alef owes and did not pay.

use super::*;
use crate::core::config::NewAlefConfig;
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::codegen::rust::RustE2eCodegen;

const STREAMING_CONFIG: &str = r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "convert"
module = "example_core"
result_var = "result"
async = true
returns_result = true
args = [{ name = "html", field = "html", type = "string" }]
[crates.e2e.call.streaming]
enabled = true
item_type = "String"
"#;

const PLAIN_CONFIG: &str = r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "convert"
module = "example_core"
result_var = "result"
returns_result = true
args = [{ name = "html", field = "html", type = "string" }]
"#;

fn rust_snippet_body(config_text: &str) -> (Fixture, String) {
    let config: NewAlefConfig = toml::from_str(config_text).expect("config parses");
    let e2e = config.crates[0].e2e.clone().expect("e2e config");
    let resolved = config.resolve().expect("config resolves").remove(0);
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"html": "<p>Hello</p>"},
        "assertions": [{"type": "not_error"}],
        "docs": {"topic": "smoke", "stem": "sample_fixture"}
    }))
    .expect("fixture parses");
    let body = RustE2eCodegen
        .render_snippet_body_with_functions(&fixture, &e2e, &resolved, &[], &[], &[], &[])
        .expect("rust snippet body renders");
    (fixture, body)
}

/// Asserted against the body the streaming recipe really emits, not a hand-written sample, so the
/// marker and the emitted path cannot drift apart unnoticed. ~keep
#[test]
fn a_streaming_snippet_requires_the_stream_crate_its_body_names() {
    let (fixture, body) = rust_snippet_body(STREAMING_CONFIG);

    assert!(
        body.contains("tokio_stream::"),
        "the streaming recipe is expected to drain the stream through `tokio_stream`:\n{body}"
    );
    assert!(
        snippet_requirements(&fixture, "rust", &body).contains(&TOKIO_STREAM_REQUIREMENT.to_string()),
        "a body naming `tokio_stream` must carry the requirement that puts it in `[dependencies]`:\n{body}"
    );
}

/// The negative control: a snippet that never names the crate must not drag it into the check
/// project, or the requirement says nothing about the body it is attached to. ~keep
#[test]
fn a_non_streaming_snippet_requires_no_stream_crate() {
    let (fixture, body) = rust_snippet_body(PLAIN_CONFIG);

    assert!(!body.contains("tokio_stream::"), "{body}");
    assert!(
        !snippet_requirements(&fixture, "rust", &body).contains(&TOKIO_STREAM_REQUIREMENT.to_string()),
        "{body}"
    );
}

/// Every body-derived requirement is a claim about text the recipes emit; a marker that matches
/// nothing would attach a dependency to nothing, and a marker no body contains is dead. ~keep
#[test]
fn a_body_naming_none_of_the_recipe_crates_gains_no_crate_requirement() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {},
        "docs": {"topic": "smoke", "stem": "sample_fixture"}
    }))
    .expect("fixture parses");

    let requirements = snippet_requirements(&fixture, "rust", "fn main() {\n    let value = 1u8;\n}\n");

    assert_eq!(requirements, Vec::<String>::new());
}
