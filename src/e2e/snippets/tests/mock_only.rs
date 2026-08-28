//! End-to-end coverage for a mock-only sample corpus: `[crates.e2e.snippets].mock_only` and
//! the per-fixture `docs.sample_url` override that opts back out of it.
//!
//! Driven through the real snippet driver rather than
//! [`super::super::sample_url_policy`] in isolation, for the reason [`super::sample_urls`]
//! states: what a consumer cares about is what reaches the published markdown and what the run
//! reports about it, and an intermediate assertion in this area has agreed with broken output
//! before.
//!
//! The load-bearing test here is
//! [`mock_only_silences_the_fixture_that_inherited_it_and_not_the_one_that_claimed_a_url`]. Every
//! other test in this file would still pass if `mock_only` were implemented as a blanket mute
//! of the reserved-domain warning, which is exactly what it must not be. ~keep

use super::sample_urls::{only_snippet_content, url_e2e_config, url_fixture};
use super::*;

/// A host that really serves something, for the handful of fixtures in an otherwise mock-only
/// corpus whose sample inputs genuinely are published.
const HOSTED_SAMPLE_URL: &str = "https://samples.example.org";

/// [`url_fixture`] under a different id, so a two-fixture run renders two distinct snippet
/// paths (`snippet_path` stems on `docs.stem` or the fixture id).
fn url_fixture_named(id: &str) -> Fixture {
    let mut fixture = url_fixture();
    fixture.id = id.to_string();
    fixture
}

/// [`url_fixture_named`], additionally declaring the public address its own sample input is
/// served at -- the opt-in a mock-only corpus's genuinely hosted fixtures use.
fn url_fixture_hosted_at(id: &str, sample_url: &str) -> Fixture {
    let mut fixture = url_fixture_named(id);
    fixture.docs = Some(crate::e2e::fixture::FixtureDocs {
        sample_url: Some(sample_url.to_string()),
        ..fixture.docs.expect("url_fixture always carries docs")
    });
    fixture
}

fn python_report(mock_only: bool, fixtures: &[Fixture]) -> Result<SnippetGenerationReport> {
    let (e2e, crate_config) = url_e2e_config();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        mock_only,
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

/// The control, and the reason this feature can be called narrow at all: with `mock_only`
/// unset, a corpus that configures no sample host warns for every fixture exactly as it did
/// before the key existed. If this ever goes quiet, the new lever has become a blanket mute
/// and the tests below are measuring nothing. ~keep
#[test]
fn a_corpus_without_mock_only_warns_exactly_as_it_did_before() {
    let report = python_report(false, &[url_fixture()]).expect("python snippet report renders");

    assert_eq!(
        report.placeholder_sample_url_fixtures,
        vec!["extract_uri".to_string()],
        "an unconfigured corpus must still name the fixture whose snippet cannot be run as published"
    );
}

/// The request: a corpus whose sample URLs genuinely do not exist stops being asked to
/// configure a host it does not have, or to acknowledge every fixture/language pair by hand.
#[test]
fn a_mock_only_corpus_emits_no_reserved_domain_warning() {
    let report = python_report(true, &[url_fixture()]).expect("python snippet report renders");

    assert!(
        report.placeholder_sample_url_fixtures.is_empty(),
        "a corpus that declares itself mock-only has nothing left to configure, so there is \
         nothing to report: {:?}",
        report.placeholder_sample_url_fixtures
    );
}

/// `mock_only` is a statement about addresses, not about rendering: the published snippet is
/// byte-identical to the one an unconfigured corpus publishes today, because a snippet still
/// has to show some URL. Only the diagnosis changes.
#[test]
fn a_mock_only_corpus_publishes_exactly_what_an_unconfigured_corpus_publishes() {
    let mock_only = python_report(true, &[url_fixture()]).expect("python snippet report renders");
    let unconfigured = python_report(false, &[url_fixture()]).expect("python snippet report renders");

    assert_eq!(
        only_snippet_content(&mock_only),
        only_snippet_content(&unconfigured),
        "declaring a corpus mock-only must not change a single byte of generated documentation"
    );
    assert!(
        only_snippet_content(&mock_only).contains("https://example.com/pdf/report.pdf"),
        "the illustrative reserved-domain address is still what a mock-only snippet shows:\n{}",
        only_snippet_content(&mock_only)
    );
}

/// The per-fixture override, in the direction that matters most: under a mock-only default, a
/// fixture that declares its own public address publishes that address, resolved exactly as a
/// corpus-wide `sample_base_url` of the same value would have resolved it.
#[test]
fn a_per_fixture_sample_url_under_mock_only_resolves_normally() {
    let report = python_report(true, &[url_fixture_hosted_at("extract_uri", HOSTED_SAMPLE_URL)])
        .expect("python snippet report renders");
    let content = only_snippet_content(&report);

    assert!(
        content.contains("https://samples.example.org/pdf/report.pdf"),
        "the fixture's own declared host must reach the published snippet, with its \
         mock-relative path appended:\n{content}"
    );
    assert!(
        !content.contains("example.com"),
        "no trace of the reserved placeholder may survive a fixture-level declaration:\n{content}"
    );
    assert!(
        report.placeholder_sample_url_fixtures.is_empty(),
        "a fixture publishing a real address has nothing to warn about"
    );
}

/// Semantic 2, and the answer to "is this a global mute": a fixture that is mock-only by
/// inherited default and then gains a real URL is warned about the moment that URL does not
/// resolve to a routable address. The corpus-level default suppresses "this fixture has no
/// public URL"; it must never suppress "this fixture's URL is broken".
#[test]
fn a_broken_per_fixture_sample_url_is_still_warned_about_under_mock_only() {
    let report = python_report(
        true,
        &[url_fixture_hosted_at("extract_uri", "https://example.com/hosted")],
    )
    .expect("python snippet report renders");

    assert_eq!(
        report.placeholder_sample_url_fixtures,
        vec!["extract_uri".to_string()],
        "a fixture that claims a public address and publishes the reserved documentation domain \
         anyway is reporting a broken URL, not the absence of one -- mock_only must not cover it"
    );
}

/// The decisive test: one run, one `mock_only` corpus, two fixtures, two different outcomes.
/// The fixture that inherited the default is silent; the fixture that claimed a URL and did
/// not resolve it is named. A blanket mute cannot produce this result, and neither can an
/// implementation that keys suppression off anything coarser than the individual fixture. ~keep
#[test]
fn mock_only_silences_the_fixture_that_inherited_it_and_not_the_one_that_claimed_a_url() {
    let report = python_report(
        true,
        &[
            url_fixture_named("inherits_mock_only"),
            url_fixture_hosted_at("claims_a_broken_url", "https://example.com/hosted"),
        ],
    )
    .expect("python snippet report renders");

    assert_eq!(
        report.placeholder_sample_url_fixtures,
        vec!["claims_a_broken_url".to_string()],
        "exactly the fixture that claimed an address must be named: the other one has nothing \
         to configure, and this one has something broken to fix"
    );
}

/// A fixture-level override also wins in the other direction: with a corpus-wide sample host
/// configured and `mock_only` unset, a fixture's own `docs.sample_url` replaces that host for
/// itself alone, leaving every sibling fixture on the corpus base.
#[test]
fn a_per_fixture_sample_url_overrides_a_configured_corpus_base_for_that_fixture_alone() {
    let (e2e, crate_config) = url_e2e_config();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        sample_base_url: Some("https://corpus.example.org".to_string()),
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
    let fixtures = [
        url_fixture_named("uses_corpus_base"),
        url_fixture_hosted_at("uses_own_url", "https://its-own.example.org"),
    ];

    let report = generate_snippet_report_with_extensions(&fixtures, &["python".into()], &snippet_config, &context, &[])
        .expect("python snippet report renders");

    let bodies: BTreeMap<&str, &str> = report
        .snippets
        .iter()
        .map(|snippet| (snippet.fixture_id.as_str(), snippet.file.content.as_str()))
        .collect();
    assert!(
        bodies["uses_corpus_base"].contains("https://corpus.example.org/pdf/report.pdf"),
        "the undeclared fixture stays on the corpus base:\n{}",
        bodies["uses_corpus_base"]
    );
    assert!(
        bodies["uses_own_url"].contains("https://its-own.example.org/pdf/report.pdf"),
        "the declaring fixture uses its own address:\n{}",
        bodies["uses_own_url"]
    );
    assert!(
        !bodies["uses_own_url"].contains("corpus.example.org"),
        "the corpus base must not leak into a fixture that overrode it:\n{}",
        bodies["uses_own_url"]
    );
}

/// `mock_only` and `sample_base_url` state contradictory facts about the same corpus. Failing
/// is what keeps the suppression rule exhaustive: were both allowed, a fixture that failed to
/// resolve against the configured base would be classified as having no address at all and
/// then muted, and `mock_only` would have become the blanket mute it must not be.
#[test]
fn mock_only_alongside_a_configured_sample_base_url_fails_the_run() {
    let (e2e, crate_config) = url_e2e_config();
    let snippet_config = SnippetConfig {
        output: "docs/snippets".into(),
        mock_only: true,
        sample_base_url: Some(HOSTED_SAMPLE_URL.to_string()),
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
            .expect_err("two contradictory claims about one corpus cannot both be honoured");

    let message = format!("{error:#}");
    assert!(
        message.contains("mock_only") && message.contains("sample_base_url"),
        "the failure must name both halves of the contradiction: {message}"
    );
}

/// A fixture-level declaration is validated like the corpus-level one -- an address a reader
/// cannot paste must fail the run rather than reach published documentation -- and the failure
/// names the fixture and the fixture key, not an `alef.toml` key its author never wrote.
#[test]
fn an_unusable_per_fixture_sample_url_fails_the_run_naming_the_fixture() {
    let error = python_report(true, &[url_fixture_hosted_at("extract_uri", "samples.example.org")])
        .expect_err("a scheme-less fixture address cannot form a public URL");

    let message = format!("{error:#}");
    assert!(
        message.contains("extract_uri"),
        "the failure must name the offending fixture: {message}"
    );
    assert!(
        message.contains("docs.sample_url"),
        "the failure must name the key its author actually wrote: {message}"
    );
}
