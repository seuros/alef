//! Unit tests for the rust e2e codegen module entry point.
//!
//! ~keep Split out of `mod.rs` rather than added to it: that file is over the
//! `file-modularization` cap, and the ratchet in `tests/file_size_ratchet.rs` lets it shrink
//! but never grow.

use super::*;

#[test]
fn resolve_crate_name_uses_config_name() {
    use crate::core::config::NewAlefConfig;
    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["rust"]

[[crates]]
name = "my-lib"
sources = ["src/lib.rs"]

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
[crates.e2e.call]
function = "process"
module = "my_lib"
result_var = "result"
"#,
    )
    .unwrap();
    let e2e = cfg.crates[0].e2e.clone().unwrap();
    let resolved = cfg.resolve().unwrap().remove(0);
    let name = resolve_crate_name(&e2e, &resolved);
    assert_eq!(name, "my-lib");
}

#[test]
fn snippet_body_matches_rust_client_json_and_async_rendering() {
    use crate::core::config::NewAlefConfig;
    use crate::e2e::codegen::E2eCodegen;

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "chat"
module = "example_core"
async = true
args = [{ name = "request", field = "input", type = "json_object", owned = true }]
[crates.e2e.call.overrides.rust]
client_factory = "create_client"
options_type = "ChatRequest"
"#,
    )
    .expect("snippet config must parse");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "client_chat",
        "description": "send a request",
        "input": {"model": "example-model", "url": "$mock_url/guide", "messages": []},
        "mock_response": {"status": 200},
        "docs": {
            "topic": "guides",
            "presentation": {
                "input": {"model": "example-model", "url": "https://api.example.com/guide", "messages": []},
                "operations": [{"op": "show", "path": "message"}]
            }
        },
        "assertions": [{"type": "not_error"}]
    }))
    .expect("fixture must parse");

    let rendered = RustE2eCodegen
        .render_snippet_body(&fixture, &e2e, &resolved, &[], &[])
        .expect("snippet renders");

    assert!(rendered.contains("let request: ChatRequest"), "{rendered}");
    assert!(rendered.contains("example_core::create_client"), "{rendered}");
    assert!(rendered.contains("client.chat(request).await"), "{rendered}");
    assert!(rendered.contains(".await.expect(\"call failed\")"), "{rendered}");
    assert!(rendered.contains("println!(\"{:?}\", result.message);"), "{rendered}");
    assert!(rendered.contains("#[tokio::main]"), "{rendered}");
    assert!(!rendered.contains("#[tokio::test]"), "{rendered}");
    assert!(!rendered.contains("fn test_"), "{rendered}");
    assert!(rendered.contains("https://api.example.com/guide"), "{rendered}");
    assert!(!rendered.contains("MOCK_SERVER"), "{rendered}");
    assert!(!rendered.contains("E2E_ALLOW_PRIVATE_NETWORK"), "{rendered}");
    assert!(!rendered.contains("$mock_url"), "{rendered}");
}

#[test]
fn successful_snippet_binds_and_displays_the_call_result() {
    use crate::e2e::codegen::E2eCodegen;

    let fixture = make_fixture("list_widgets", serde_json::Value::Null);
    let mut e2e = crate::e2e::config::E2eConfig::default();
    e2e.call.function = "list_widgets".into();
    e2e.call.result_var = "widgets".into();

    let rendered = RustE2eCodegen
        .render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[])
        .expect("Rust snippet renders");

    assert!(rendered.contains("let widgets = list_widgets()"), "{rendered}");
    assert!(rendered.contains("println!(\"{:?}\", widgets)"), "{rendered}");
    assert!(!rendered.contains("let _ = list_widgets()"), "{rendered}");
    assert!(
        !rendered.contains("match widgets {"),
        "a fixture with no error assertion must not get the error branch:\n{rendered}"
    );
}

#[test]
fn error_fixture_snippet_matches_the_result_instead_of_panicking() {
    use crate::e2e::codegen::E2eCodegen;

    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "rate_limit_429",
        "description": "Surface a rate-limit failure",
        "input": null,
        "assertions": [{"type": "error"}]
    }))
    .expect("fixture must parse");
    let mut e2e = crate::e2e::config::E2eConfig::default();
    e2e.call.function = "chat".into();
    e2e.call.result_var = "result".into();

    let rendered = RustE2eCodegen
        .render_snippet_body(&fixture, &e2e, &ResolvedCrateConfig::default(), &[], &[])
        .expect("Rust snippet renders");

    assert!(rendered.contains("let result = chat()"), "{rendered}");
    assert!(rendered.contains("match result {"), "{rendered}");
    assert!(
        rendered.contains("Ok(value) => println!(\"{:?}\", value),"),
        "{rendered}"
    );
    assert!(rendered.contains("Err(error) => println!(\"{error}\"),"), "{rendered}");
    assert!(
        !rendered.contains(".expect(\"call failed\")"),
        "the error branch must not panic on the failure it documents:\n{rendered}"
    );
    assert!(
        !rendered.contains("println!(\"{:?}\", result);"),
        "the moved result must not be printed after the match:\n{rendered}"
    );
}

/// Pins that a `client_factory` fixture's Rust documentation snippet reads its credential
/// via `std::env::var(...)` — the substitution `render_snippet_body` applies over the
/// harness's hardcoded `"test-key".to_string()` literal (mod.rs ~line 220-229) — and never
/// carries the e2e mock-server env vars, fixture route, or literal credential.
#[test]
fn client_factory_snippet_never_points_the_reader_at_the_mock_server() {
    use crate::core::config::NewAlefConfig;
    use crate::e2e::codegen::E2eCodegen;

    let cfg: NewAlefConfig = toml::from_str(
        r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "sample-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "chat"
result_var = "result"
[crates.e2e.call.overrides.rust]
client_factory = "create_client"
"#,
    )
    .expect("snippet config must parse");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "rate_limit_429",
        "description": "Rate limited",
        "input": null,
        "mock_response": {"status": 429}
    }))
    .expect("fixture must parse");

    let rendered = RustE2eCodegen
        .render_snippet_body(&fixture, &e2e, &resolved, &[], &[])
        .expect("Rust snippet renders");

    assert!(
        !rendered.contains("MOCK_SERVER"),
        "mock-server env var leaked:\n{rendered}"
    );
    assert!(
        !rendered.contains("/fixtures/rate_limit_429"),
        "mock-server fixture route leaked:\n{rendered}"
    );
    assert!(
        !rendered.contains("\"test-key\""),
        "literal credential leaked:\n{rendered}"
    );
    assert!(
        rendered.contains("std::env::var(\"API_KEY\").expect(\"API_KEY must be set\")"),
        "credential is not read from the environment:\n{rendered}"
    );
    assert!(
        rendered.contains(
            "sample_core::create_client(std::env::var(\"API_KEY\").expect(\"API_KEY must be set\"), \
             None, None, None, None).unwrap();"
        ),
        "client is not constructed the way a reader would:\n{rendered}"
    );
}

#[test]
fn raw_literal_handles_backticks_and_blank_line_after_fence() {
    let input = "<pre><code>```rust\nlet value = r#\"sample\"#;\n```\n\nnext</code></pre>";
    let literal = crate::e2e::escape::rust_raw_string(input);
    let expression = syn::parse_str::<syn::Expr>(&literal).expect("generated raw literal parses");
    assert!(matches!(expression, syn::Expr::Lit(_)), "{literal}");
    assert!(literal.starts_with("r##\""), "{literal}");
}

#[test]
fn snippet_extraction_preserves_multiline_raw_literal_contents() {
    let rendered = concat!(
        "use sample::process;\n",
        "\n",
        "fn test_multiline() {\n",
        "    let source = r#\"# A comment\n",
        "def greet(name):\n",
        "    return name\n",
        "\n",
        "import os\n",
        "\"#;\n",
        "    let _ = process(source);\n",
        "}\n",
        "}\n",
    );

    let (_, body, _) = extract_rust_snippet(rendered).expect("snippet extracts");
    let body = body.join("\n");

    assert!(body.contains("def greet(name):"), "{body}");
    assert!(body.contains("import os"), "{body}");
    assert!(body.contains("\ndef greet(name):"), "{body}");
    assert!(!body.lines().any(|line| line == "}"), "{body}");
    syn::parse_file(&format!("fn main() {{\n{body}\n}}")).expect("generated snippet body parses");
}

#[test]
fn snippet_extraction_survives_a_bare_closing_brace_inside_a_raw_string() {
    let rendered = concat!(
        "use sample::process;\n",
        "\n",
        "fn test_brace_in_literal() {\n",
        "    let source = r#\"fn example() {\n",
        "}\n",
        "let after = 1;\n",
        "\"#;\n",
        "    let _ = process(source);\n",
        "}\n",
    );

    let (_, body, _) = extract_rust_snippet(rendered).expect("snippet extracts");
    let body = body.join("\n");

    assert!(body.contains("fn example() {"), "{body}");
    assert!(body.contains("let after = 1;"), "{body}");
    assert!(body.contains("let _ = process(source);"), "{body}");
}
