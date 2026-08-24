//! End-to-end coverage for the documentation sample base URL.
//!
//! These drive the real snippet driver rather than
//! [`super::super::mock_url_defaults::with_default_mock_url_literals`] in isolation: the
//! question a consumer cares about is what reaches the published markdown, and every
//! intermediate assertion in this area has at some point agreed with a broken output. Each
//! test therefore asserts on the rendered snippet file's content.

use super::*;
use crate::core::config::NewAlefConfig;

const SAMPLE_HOST: &str = "https://samples.example.org";

/// A fixture with a `mock_url` argument, documentation metadata, and a relative input path
/// -- the shape a URL-centric consumer's fixtures actually have.
fn url_fixture() -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "extract_uri",
        "description": "Extract a document from a URI",
        "input": {"url": "/pdf/report.pdf"},
        "assertions": [{"type": "not_error"}],
        "docs": {"topic": "contract", "side_effects": "network"},
    }))
    .expect("fixture must parse")
}

fn url_e2e_config() -> (E2eConfig, ResolvedCrateConfig) {
    let cfg_str = r#"
[workspace]
languages = ["python"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "extract_uri"
module = "example_api"
args = [{ name = "url", field = "url", type = "mock_url" }]
"#;
    let cfg: NewAlefConfig = toml::from_str(cfg_str).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
    let resolved = cfg.resolve().expect("config resolves").remove(0);
    (e2e, resolved)
}

fn python_snippet_report(sample_base_url: Option<&str>) -> SnippetGenerationReport {
    let (e2e, crate_config) = url_e2e_config();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        sample_base_url: sample_base_url.map(str::to_string),
        ..SnippetConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };
    generate_snippet_report_with_extensions(&[url_fixture()], &["python".into()], &snippet_config, &context, &[])
        .expect("python snippet report renders")
}

fn only_snippet_content(report: &SnippetGenerationReport) -> &str {
    assert_eq!(
        report.snippets.len(),
        1,
        "exactly one fixture/language cell is rendered"
    );
    &report.snippets[0].file.content
}

/// The defect: a reader copies the published quick start and it fails, because the address
/// in it belongs to nobody. With a project's own sample host configured, the same fixture's
/// relative path resolves against that host all the way into the markdown.
#[test]
fn a_configured_sample_base_url_reaches_the_published_snippet() {
    let report = python_snippet_report(Some(SAMPLE_HOST));
    let content = only_snippet_content(&report);

    assert!(
        content.contains("https://samples.example.org/pdf/report.pdf"),
        "the published snippet must carry the configured sample host:\n{content}"
    );
    assert!(
        !content.contains("example.com"),
        "no trace of the reserved placeholder may survive configuration:\n{content}"
    );
    assert!(
        report.placeholder_sample_url_fixtures.is_empty(),
        "a configured run has no placeholder to report"
    );
}

/// The unconfigured default is unchanged -- every consumer already shipping these snippets
/// keeps generating exactly what it generated before -- but the run now names the fixtures
/// it published an unrunnable address for instead of implying they work.
#[test]
fn an_unconfigured_run_keeps_the_placeholder_and_reports_every_fixture_using_it() {
    let report = python_snippet_report(None);
    let content = only_snippet_content(&report);

    assert!(
        content.contains("https://example.com/pdf/report.pdf"),
        "the unconfigured fallback is unchanged:\n{content}"
    );
    assert_eq!(
        report.placeholder_sample_url_fixtures,
        vec!["extract_uri".to_string()],
        "the run must name the fixture whose snippet cannot be run as published"
    );
}

/// The other route an address takes into a snippet: a fixture that spells `$mock_url`
/// explicitly is resolved by [`Fixture::docs_call_fixture_with_sample_base_url`] before the
/// defaults pass ever runs, so that substitution has to honour the configured host too --
/// otherwise a fixture that opted *in* to the placeholder convention is the one case still
/// publishing an unreachable address.
#[test]
fn an_explicit_mock_url_placeholder_resolves_against_the_configured_sample_base_url() {
    let (e2e, crate_config) = url_e2e_config();
    let fixture = Fixture {
        preserve_input_urls: true,
        input: serde_json::json!({"url": "$mock_url/pdf/report.pdf"}),
        ..url_fixture()
    };
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        sample_base_url: Some(SAMPLE_HOST.into()),
        ..SnippetConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report =
        generate_snippet_report_with_extensions(&[fixture], &["python".into()], &snippet_config, &context, &[])
            .expect("python snippet report renders");

    let content = only_snippet_content(&report);
    assert!(
        content.contains("https://samples.example.org/pdf/report.pdf"),
        "the placeholder must resolve against the configured host:\n{content}"
    );
    assert!(
        !content.contains("$mock_url"),
        "no unresolved placeholder may reach published documentation:\n{content}"
    );
}

/// Reporting is not blanket: a fixture that never publishes the placeholder address must
/// not be named, or the report degrades into noise nobody reads.
#[test]
fn a_fixture_that_declares_its_own_absolute_url_is_not_reported_as_a_placeholder_use() {
    let (e2e, crate_config) = url_e2e_config();
    let fixture = Fixture {
        preserve_input_urls: true,
        input: serde_json::json!({"url": "https://docs.example.org/pdf/report.pdf"}),
        ..url_fixture()
    };
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        ..SnippetConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let report =
        generate_snippet_report_with_extensions(&[fixture], &["python".into()], &snippet_config, &context, &[])
            .expect("python snippet report renders");

    assert!(
        only_snippet_content(&report).contains("https://docs.example.org/pdf/report.pdf"),
        "the fixture's own address is what gets published"
    );
    assert!(
        report.placeholder_sample_url_fixtures.is_empty(),
        "no placeholder address reached the published snippet, so nothing may be reported"
    );
}

/// A `sample_base_url` that cannot form a URL fails the run. Falling back to the placeholder
/// instead would publish a broken address *and* discard the operator's stated intent.
#[test]
fn an_unusable_sample_base_url_fails_generation_naming_the_config_key() {
    let (e2e, crate_config) = url_e2e_config();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        sample_base_url: Some("samples.example.org".into()),
        ..SnippetConfig::default()
    };
    let context = SnippetRenderContext {
        e2e: &e2e,
        crate_config: &crate_config,
        type_defs: &[],
        enums: &[],
        functions: &[],
        errors: &[],
    };

    let error =
        generate_snippet_report_with_extensions(&[url_fixture()], &["python".into()], &snippet_config, &context, &[])
            .expect_err("a scheme-less base cannot be published");

    let message = format!("{error:#}");
    assert!(
        message.contains("sample_base_url"),
        "the error must name the key to fix: {message}"
    );
}

/// The tension this feature exists inside: the docs address must never reach the executable
/// suite. The e2e test body for the same fixture is rendered from the untouched fixture and
/// still binds the mock server, with the configured sample host nowhere in it.
#[test]
fn the_configured_sample_url_never_reaches_the_executable_e2e_test() {
    use crate::e2e::codegen::E2eCodegen;
    use crate::e2e::codegen::python::PythonE2eCodegen;
    use crate::e2e::fixture::FixtureGroup;

    let (e2e, crate_config) = url_e2e_config();
    let report = python_snippet_report(Some(SAMPLE_HOST));
    assert!(only_snippet_content(&report).contains(SAMPLE_HOST));

    let groups = vec![FixtureGroup {
        category: "contract".into(),
        fixtures: vec![url_fixture()],
    }];
    let files = PythonE2eCodegen
        .generate_gated(&groups, &e2e, &crate_config, &[], &[], &[], &[])
        .expect("python e2e test files render");
    let test_body = files
        .iter()
        .map(|file| file.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !test_body.contains(SAMPLE_HOST),
        "the executable suite must never be pointed at the documentation sample host:\n{test_body}"
    );
    assert!(
        test_body.contains("MOCK_SERVER"),
        "the executable suite still binds the mock server:\n{test_body}"
    );
}
