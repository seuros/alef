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

/// A content-addressed template a URL-centric consumer's corpus needs -- `sample_base_url`
/// cannot express this shape at all, since the object's real address depends on a fact about
/// the object (here `{digest}`), not on the fixture's mock path.
const CONTENT_ADDRESSED_TEMPLATE: &str = "https://cdn.example.org/objects/{digest}";

/// A fixture with a `mock_url` argument, documentation metadata, and a relative input path
/// -- the shape a URL-centric consumer's fixtures actually have.
pub(super) fn url_fixture() -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "extract_uri",
        "description": "Extract a document from a URI",
        "input": {"url": "/pdf/report.pdf"},
        "assertions": [{"type": "not_error"}],
        "docs": {"topic": "contract", "side_effects": "network"},
    }))
    .expect("fixture must parse")
}

/// [`url_fixture`], additionally declaring the fact [`CONTENT_ADDRESSED_TEMPLATE`] needs --
/// the shape a fixture author writes once so their own corpus's real address reaches
/// published documentation.
fn url_fixture_with_digest(digest: &str) -> Fixture {
    let mut fixture = url_fixture();
    fixture.docs = Some(crate::e2e::fixture::FixtureDocs {
        sample_url_vars: std::collections::BTreeMap::from([("digest".to_string(), digest.to_string())]),
        ..fixture.docs.expect("url_fixture always carries docs")
    });
    fixture
}

pub(super) fn url_e2e_config() -> (E2eConfig, ResolvedCrateConfig) {
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

/// As [`python_snippet_report`], but also configuring `sample_url_template` and taking the
/// fixture set explicitly, so the per-fixture template tests can drive a mix of fixtures that
/// do and do not supply what the template needs.
fn python_snippet_report_with_template(
    sample_base_url: Option<&str>,
    sample_url_template: Option<&str>,
    fixtures: &[Fixture],
) -> Result<SnippetGenerationReport> {
    python_snippet_report_with_template_and_manifest(sample_base_url, sample_url_template, None, fixtures)
}

/// As [`python_snippet_report_with_template`], additionally configuring
/// `[crates.e2e.snippets].sample_url_manifest` against a manifest file this helper writes to a
/// temporary project root -- `generate_snippet_report_with_extensions` resolves the manifest
/// path relative to the process current directory, so the run must actually execute inside that
/// directory (see [`crate::test_support::CwdGuard`]), the same mechanism `tests::curated` uses
/// for `curated_snippets`.
fn python_snippet_report_with_template_and_manifest(
    sample_base_url: Option<&str>,
    sample_url_template: Option<&str>,
    manifest: Option<(&str, &str)>,
    fixtures: &[Fixture],
) -> Result<SnippetGenerationReport> {
    let directory = tempfile::tempdir().expect("temp dir");
    let _cwd = crate::test_support::CwdGuard::enter(directory.path());
    let sample_url_manifest = manifest.map(|(manifest_json, variable)| {
        std::fs::write(directory.path().join("manifest.json"), manifest_json).expect("write manifest");
        crate::core::config::e2e::SampleUrlManifestConfig {
            path: "manifest.json".to_string(),
            variable: variable.to_string(),
        }
    });
    let (e2e, crate_config) = url_e2e_config();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        sample_base_url: sample_base_url.map(str::to_string),
        sample_url_template: sample_url_template.map(str::to_string),
        sample_url_manifest,
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
    generate_snippet_report_with_extensions(fixtures, &["python".into()], &snippet_config, &context, &[])
}

/// [`url_fixture`], additionally declaring [`FixtureDocs::body_file`] -- the corpus-relative
/// path a `[crates.e2e.snippets].sample_url_manifest` entry is looked up by, distinct from
/// [`url_fixture_with_digest`]'s direct `docs.sample_url_vars` declaration.
fn url_fixture_with_body_file(body_file: &str) -> Fixture {
    let mut fixture = url_fixture();
    fixture.docs = Some(crate::e2e::fixture::FixtureDocs {
        body_file: Some(body_file.to_string()),
        ..fixture.docs.expect("url_fixture always carries docs")
    });
    fixture
}

pub(super) fn only_snippet_content(report: &SnippetGenerationReport) -> &str {
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

/// The measured defect this feature exists to fix: `sample_base_url` can only express a flat
/// prefix concatenated with a fixture's mock path, which cannot produce a content-addressed
/// URL. With `[crates.e2e.snippets].sample_url_template` configured and this fixture supplying
/// the digest the template needs, the published snippet carries the fixture's own resolved
/// address instead of any flat base.
#[test]
fn a_fixture_with_a_configured_template_and_matching_vars_publishes_its_own_url() {
    let report = python_snippet_report_with_template(
        None,
        Some(CONTENT_ADDRESSED_TEMPLATE),
        &[url_fixture_with_digest("9f86d081884c7d659a2feaa0c55ad015")],
    )
    .expect("python snippet report renders");
    let content = only_snippet_content(&report);

    assert!(
        content.contains("https://cdn.example.org/objects/9f86d081884c7d659a2feaa0c55ad015"),
        "the published snippet must carry the fixture's own templated address:\n{content}"
    );
    assert!(
        !content.contains("example.com"),
        "no trace of the reserved placeholder may survive a fully resolved template:\n{content}"
    );
    assert!(
        report.placeholder_sample_url_fixtures.is_empty(),
        "a fully resolved template has no placeholder to report"
    );
}

/// The regression guard: a fixture that declares none of the facts a configured template
/// needs falls back to `sample_base_url` exactly as it did before per-fixture templates
/// existed -- a template being available in general must never change behavior for a fixture
/// that does not participate in it.
#[test]
fn a_fixture_without_matching_vars_falls_back_to_sample_base_url_unchanged() {
    let report =
        python_snippet_report_with_template(Some(SAMPLE_HOST), Some(CONTENT_ADDRESSED_TEMPLATE), &[url_fixture()])
            .expect("python snippet report renders");
    let content = only_snippet_content(&report);

    assert!(
        content.contains("https://samples.example.org/pdf/report.pdf"),
        "a fixture missing the template's required facts must keep publishing the flat \
         sample_base_url address:\n{content}"
    );
    assert!(
        !content.contains("cdn.example.org"),
        "no templated address may appear for a fixture that never supplied what the template \
         needed:\n{content}"
    );
    assert!(
        report.placeholder_sample_url_fixtures.is_empty(),
        "a configured sample_base_url has no placeholder to report, template or not"
    );
}

/// The case that keeps this feature from becoming a silencer: a template is configured, but
/// this fixture supplies none of the facts it needs, and the project configures no
/// `sample_base_url` either -- the fixture must still publish the reserved placeholder and the
/// run must still warn about it, exactly as an unconfigured project always has.
#[test]
fn a_fixture_with_neither_template_vars_nor_sample_base_url_still_warns() {
    let report = python_snippet_report_with_template(None, Some(CONTENT_ADDRESSED_TEMPLATE), &[url_fixture()])
        .expect("python snippet report renders");
    let content = only_snippet_content(&report);

    assert!(
        content.contains("https://example.com/pdf/report.pdf"),
        "an unresolved fixture must keep publishing the reserved placeholder address:\n{content}"
    );
    assert_eq!(
        report.placeholder_sample_url_fixtures,
        vec!["extract_uri".to_string()],
        "a template being configured must not silence the placeholder warning for a fixture it \
         cannot actually resolve"
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

/// The report field is only half the signal: a `SnippetGenerationReport` field nothing
/// prints is exactly the silence this change is about. The run must also say it, once,
/// naming the key that fixes it.
#[test]
#[tracing_test::traced_test]
fn an_unconfigured_run_warns_once_naming_the_key_that_fixes_it() {
    python_snippet_report(None);

    assert!(
        logs_contain("sample_base_url"),
        "the warning must name the config key a project sets to fix this"
    );
    assert!(
        logs_contain("extract_uri"),
        "the warning must name the affected fixture, not just a count"
    );
}

#[test]
#[tracing_test::traced_test]
fn a_configured_run_stays_silent() {
    python_snippet_report(Some(SAMPLE_HOST));

    assert!(
        !logs_contain("sample_base_url"),
        "a project that configured a real sample host must not be nagged"
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

/// The template's counterpart: a `sample_url_template` that cannot form a URL fails the run
/// before anything renders, the same posture `sample_base_url` takes.
#[test]
fn an_unusable_sample_url_template_fails_generation_naming_the_config_key() {
    let error = python_snippet_report_with_template(None, Some("cdn.example.org/objects/{digest}"), &[url_fixture()])
        .expect_err("a scheme-less template cannot be published");

    let message = format!("{error:#}");
    assert!(
        message.contains("sample_url_template"),
        "the error must name the key to fix: {message}"
    );
}

/// The measured defect `[crates.e2e.snippets].sample_url_manifest` exists to fix: hand-copying a
/// digest into every fixture's `docs.sample_url_vars` does not scale for a content-addressed
/// corpus with hundreds of entries. With the manifest configured and this fixture's
/// `docs.body_file` covered by it, the published snippet carries the manifest-derived address.
#[test]
fn a_fixture_whose_body_file_the_manifest_covers_publishes_its_address_through_the_real_pipeline() {
    let report = python_snippet_report_with_template_and_manifest(
        None,
        Some(CONTENT_ADDRESSED_TEMPLATE),
        Some((r#"{"pdf/report.pdf": "9f86d081884c7d659a2feaa0c55ad015"}"#, "digest")),
        &[url_fixture_with_body_file("pdf/report.pdf")],
    )
    .expect("python snippet report renders");
    let content = only_snippet_content(&report);

    assert!(
        content.contains("https://cdn.example.org/objects/9f86d081884c7d659a2feaa0c55ad015"),
        "the published snippet must carry the manifest-derived address:\n{content}"
    );
    assert!(
        report.placeholder_sample_url_fixtures.is_empty(),
        "a fully resolved manifest entry has no placeholder to report"
    );
}

/// The regression guard: a fixture whose `docs.body_file` the manifest does not mention falls
/// back to `sample_base_url` exactly as an uncovered fixture always has -- a manifest being
/// configured in general must never change behavior for a fixture it says nothing about.
#[test]
fn a_fixture_whose_body_file_the_manifest_does_not_cover_falls_back_through_the_real_pipeline() {
    let report = python_snippet_report_with_template_and_manifest(
        Some(SAMPLE_HOST),
        Some(CONTENT_ADDRESSED_TEMPLATE),
        Some((r#"{"pdf/other.pdf": "9f86d081884c7d659a2feaa0c55ad015"}"#, "digest")),
        &[url_fixture_with_body_file("pdf/report.pdf")],
    )
    .expect("python snippet report renders");
    let content = only_snippet_content(&report);

    assert!(
        content.contains("https://samples.example.org/pdf/report.pdf"),
        "a fixture the manifest does not cover must keep publishing the flat sample_base_url \
         address:\n{content}"
    );
    assert!(
        !content.contains("cdn.example.org"),
        "no templated address may appear for a fixture the manifest never covered:\n{content}"
    );
}

/// The anti-silencer case for the manifest specifically: a manifest is configured, but it does
/// not cover this fixture, and the project configures no `sample_base_url` either -- the
/// fixture must still publish the reserved placeholder and the run must still warn about it.
/// Companion to [`a_fixture_with_neither_template_vars_nor_sample_base_url_still_warns`], which
/// must keep passing unmodified: this manifest mechanism must never become a second way to
/// silence that warning wholesale.
#[test]
fn a_manifest_configured_but_not_covering_this_fixture_still_warns() {
    let report = python_snippet_report_with_template_and_manifest(
        None,
        Some(CONTENT_ADDRESSED_TEMPLATE),
        Some((r#"{"pdf/other.pdf": "9f86d081884c7d659a2feaa0c55ad015"}"#, "digest")),
        &[url_fixture()],
    )
    .expect("python snippet report renders");
    let content = only_snippet_content(&report);

    assert!(
        content.contains("https://example.com/pdf/report.pdf"),
        "an unresolved fixture must keep publishing the reserved placeholder address:\n{content}"
    );
    assert_eq!(
        report.placeholder_sample_url_fixtures,
        vec!["extract_uri".to_string()],
        "a manifest being configured must not silence the placeholder warning for a fixture it \
         does not cover"
    );
}

/// Precedence, driven through the real pipeline: a fixture's own `docs.sample_url_vars` entry
/// wins over a manifest entry for the same placeholder -- the defensible default documented on
/// `SnippetConfig::sample_url_manifest` and on `merge_manifest_vars`.
#[test]
fn a_fixtures_own_sample_url_var_outranks_the_manifest_through_the_real_pipeline() {
    let mut fixture = url_fixture_with_body_file("pdf/report.pdf");
    fixture.docs = Some(crate::e2e::fixture::FixtureDocs {
        sample_url_vars: std::collections::BTreeMap::from([("digest".to_string(), "from-fixture".to_string())]),
        ..fixture.docs.expect("url_fixture_with_body_file always carries docs")
    });

    let report = python_snippet_report_with_template_and_manifest(
        None,
        Some(CONTENT_ADDRESSED_TEMPLATE),
        Some((r#"{"pdf/report.pdf": "from-manifest"}"#, "digest")),
        &[fixture],
    )
    .expect("python snippet report renders");
    let content = only_snippet_content(&report);

    assert!(
        content.contains("https://cdn.example.org/objects/from-fixture"),
        "an explicit docs.sample_url_vars entry must win over the manifest for the same key:\n{content}"
    );
    assert!(
        !content.contains("from-manifest"),
        "the manifest's value for the same key must not reach the published snippet:\n{content}"
    );
}

/// A configured manifest that cannot be read at all -- missing from disk -- fails the run before
/// anything renders, naming the configured path, the same posture `sample_base_url` and
/// `sample_url_template` take on their own invalid configuration.
#[test]
fn a_missing_sample_url_manifest_fails_generation_naming_the_path() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _cwd = crate::test_support::CwdGuard::enter(directory.path());
    let (e2e, crate_config) = url_e2e_config();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        sample_url_manifest: Some(crate::core::config::e2e::SampleUrlManifestConfig {
            path: "does-not-exist.json".to_string(),
            variable: "digest".to_string(),
        }),
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
            .expect_err("a missing manifest file cannot be published");

    let message = format!("{error:#}");
    assert!(
        message.contains("does-not-exist.json"),
        "the error must name the configured manifest path: {message}"
    );
}

/// A configured manifest that is not valid JSON fails the run the same way, naming the path
/// rather than silently behaving as if no manifest were configured -- a malformed manifest must
/// never look identical to "not configured".
#[test]
fn a_malformed_sample_url_manifest_fails_generation_naming_the_path() {
    let directory = tempfile::tempdir().expect("temp dir");
    let _cwd = crate::test_support::CwdGuard::enter(directory.path());
    std::fs::write(directory.path().join("manifest.json"), "not json").expect("write manifest");
    let (e2e, crate_config) = url_e2e_config();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        sample_url_manifest: Some(crate::core::config::e2e::SampleUrlManifestConfig {
            path: "manifest.json".to_string(),
            variable: "digest".to_string(),
        }),
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
            .expect_err("a malformed manifest cannot be published");

    let message = format!("{error:#}");
    assert!(
        message.contains("manifest.json"),
        "the error must name the configured manifest path: {message}"
    );
}

/// Build a snippet config identical to [`python_snippet_report`]'s unconfigured case but with
/// `acknowledged_warnings` set, so a test can drive the real acknowledgement wiring in
/// `generate_snippet_report_with_extensions` rather than the engine in isolation.
fn python_snippet_report_with_acknowledgements(
    acknowledged_warnings: Vec<WarningAcknowledgement>,
) -> Result<SnippetGenerationReport> {
    let (e2e, crate_config) = url_e2e_config();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        acknowledged_warnings,
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
}

fn reserved_domain_ack(identity: &str, target: &str) -> WarningAcknowledgement {
    WarningAcknowledgement {
        category: AcknowledgeableWarningCategory::DocSnippetReservedDomain,
        identity: identity.to_string(),
        target: target.to_string(),
        reason: None,
    }
}

/// Task #540's decisive test, driven through the real production path rather than the engine
/// in isolation: an acknowledgement configured in `alef.toml` for a warning that never fires
/// this run must fail the run, not pass silently. This run renders only `"python"` (see
/// `python_snippet_report_with_acknowledgements`), so an entry naming target `"go"` can never
/// be matched here -- it is unconditionally stale, whether the identity is right or not. ~keep
#[test]
fn a_stale_acknowledgement_fails_the_run_instead_of_passing_silently() {
    let error = python_snippet_report_with_acknowledgements(vec![reserved_domain_ack("extract_uri", "go")])
        .expect_err("an acknowledgement that matches nothing this run must fail generation");

    let message = format!("{error:#}");
    assert!(
        message.contains("extract_uri") && message.contains("go"),
        "the failure must name the stale entry: {message}"
    );
    assert!(
        message.to_lowercase().contains("matched nothing") || message.to_lowercase().contains("stale"),
        "the failure must say the acknowledgement is stale, not fail for an unrelated reason: {message}"
    );
}

/// The companion to the stale test above: an acknowledgement that DOES match must suppress the
/// warning and the run must succeed, with the matched count reported and accurate.
#[test]
fn a_matching_acknowledgement_suppresses_the_warning_and_reports_a_nonzero_matched_count() {
    let report = python_snippet_report_with_acknowledgements(vec![reserved_domain_ack("extract_uri", "python")])
        .expect("a matching acknowledgement must let the run succeed");

    assert!(
        report.placeholder_sample_url_fixtures.is_empty(),
        "an acknowledged occurrence must not be reported as an unresolved placeholder use"
    );
    assert_eq!(
        report.acknowledged_warning_count, 1,
        "exactly one warning occurrence was acknowledged"
    );
}

/// Requirement 4, driven through the real config path: a category that has no business being
/// acknowledged at this location is rejected even though `virtual_field_path` is a legitimate
/// variant elsewhere on [`AcknowledgeableWarningCategory`]. Proves the rejection is wired into
/// production `alef.toml` config, not only asserted against the engine directly.
#[test]
fn a_category_this_location_does_not_service_is_rejected_even_when_legitimate_elsewhere() {
    let error = python_snippet_report_with_acknowledgements(vec![WarningAcknowledgement {
        category: AcknowledgeableWarningCategory::VirtualFieldPath,
        identity: "result.0::Ok::path".to_string(),
        target: "python".to_string(),
        reason: None,
    }])
    .expect_err("a category this ledger scope does not accept must be rejected");

    let message = format!("{error:#}");
    assert!(
        message.contains("virtual_field_path"),
        "the failure must name the offending category: {message}"
    );
    assert!(
        message.contains("doc_snippet_reserved_domain"),
        "the failure must name what IS accepted here: {message}"
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
