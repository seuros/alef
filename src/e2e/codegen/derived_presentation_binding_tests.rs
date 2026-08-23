//! The call emitter and the presentation resolver must agree about whether a docs snippet
//! consumes the call's result.
//!
//! ~keep `rust/test_file/test_function.rs` asks exactly one question before deciding whether the
//! call binds a named `result` (rather than `let _ =`) and whether a `Result`-returning call is
//! unwrapped first: [`Fixture::has_docs_presentation`]. Task #199 then taught
//! `presentation::resolve` to derive show operations from the fixture's own assertions, without
//! teaching that predicate about the derivation — so for the commonest fixture shape of all (a
//! docs-tagged fixture with assertions and no hand-authored `shows`/`presentation`) the emitter
//! answered "nothing consumes the result" while the snippet printed `result.<field>`.
//!
//! The result shipped in 0.67.2 as `E0425: cannot find value 'result' in this scope` on all 283
//! generated Rust snippets in one consumer repo — and would have been `E0609` on the `Result`
//! wrapper for any that had bound it. Both halves are pinned below, plus the negative control: a
//! fixture that derives nothing must still bind nothing and keep the whole-result display.

use crate::core::config::NewAlefConfig;
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::codegen::rust::RustE2eCodegen;
use crate::e2e::fixture::Fixture;

const CONFIG: &str = r#"
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

fn snippet_body(fixture_json: serde_json::Value) -> String {
    let config: NewAlefConfig = toml::from_str(CONFIG).expect("config parses");
    let e2e = config.crates[0].e2e.clone().expect("e2e config");
    let resolved = config.resolve().expect("config resolves").remove(0);
    let fixture: Fixture = serde_json::from_value(fixture_json).expect("fixture parses");
    RustE2eCodegen
        .render_snippet_body(&fixture, &e2e, &resolved, &[], &[])
        .expect("rust snippet body renders")
}

fn derived_fixture() -> serde_json::Value {
    serde_json::json!({
        "id": "smoke_simple_paragraph",
        "description": "Simple paragraph converts correctly",
        "input": {"html": "<p>Hello World</p>"},
        "assertions": [{"type": "equals", "field": "content", "value": "Hello World\n"}],
        "docs": {"topic": "smoke", "stem": "smoke_simple_paragraph"}
    })
}

/// The `E0425` half: the snippet references `result`, so the call must bind it. ~keep
#[test]
fn a_snippet_presenting_a_derived_field_binds_the_result_it_references() {
    let body = snippet_body(derived_fixture());

    assert!(
        body.contains("result.content"),
        "the derived presentation must still be rendered (task #199):\n{body}"
    );
    assert!(
        !body.contains("let _ ="),
        "a snippet that references `result` must not discard the call's value:\n{body}"
    );
    assert!(
        body.contains("let result = convert("),
        "the call must bind the name the presentation reads:\n{body}"
    );
}

/// The `E0609` half: `convert` returns `Result<_, _>`, which has no `content` field, so the
/// binding has to unwrap before the presentation touches it. ~keep
#[test]
fn a_result_returning_call_is_unwrapped_before_a_derived_field_access() {
    let body = snippet_body(derived_fixture());

    assert!(
        body.contains("let result = convert(html, ).expect(\"call failed\");")
            || body.contains(".expect(\"call failed\")"),
        "a `Result`-returning call feeding a field access must be unwrapped:\n{body}"
    );
}

/// The negative control. A fixture whose assertions name nothing showable derives no operations,
/// so the snippet keeps the pre-#199 whole-result display — and must not gain a binding it never
/// uses, which would be an `unused_variables` warning in every generated snippet. ~keep
#[test]
fn a_snippet_with_no_derived_presentation_keeps_the_whole_result_display() {
    let body = snippet_body(serde_json::json!({
        "id": "smoke_not_error",
        "description": "Call succeeds",
        "input": {"html": "<p>Hello World</p>"},
        "assertions": [{"type": "not_error"}],
        "docs": {"topic": "smoke", "stem": "smoke_not_error"}
    }));

    assert!(
        body.contains("println!(\"{:?}\", result)"),
        "with nothing to present, the snippet still shows the result it produced:\n{body}"
    );
    assert!(
        !body.contains("result."),
        "no field access may be invented for a fixture whose assertions name none:\n{body}"
    );
}
