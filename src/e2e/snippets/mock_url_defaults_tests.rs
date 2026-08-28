//! Test module for `super::with_default_mock_url_literals` and friends, split out of
//! `mock_url_defaults.rs`, which was approaching the repo's 800-line split threshold once this
//! task's manifest coverage was added.

use super::*;
use crate::core::config::e2e::ArgMapping;
use crate::e2e::fixture::FixtureDocs;

fn mock_url_arg(field: &str) -> ArgMapping {
    ArgMapping {
        name: "url".into(),
        field: field.into(),
        arg_type: "mock_url".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn mock_url_list_arg(field: &str) -> ArgMapping {
    ArgMapping {
        arg_type: "mock_url_list".into(),
        ..mock_url_arg(field)
    }
}

/// The placeholder base every test that is not specifically about configuration uses,
/// so those tests keep asserting the shape of the rewrite rather than the address.
fn placeholder_base() -> DocsSampleBaseUrl<'static> {
    DocsSampleBaseUrl::resolve(None).expect("an unconfigured base always resolves")
}

fn apply_defaults(fixture: Fixture, call: &CallConfig) -> Fixture {
    with_default_mock_url_literals(fixture, call, placeholder_base(), None, None)
}

fn call_with_args(args: Vec<ArgMapping>) -> CallConfig {
    CallConfig {
        args,
        ..CallConfig::default()
    }
}

#[test]
fn injects_the_default_url_when_the_fixture_declares_none() {
    let fixture = Fixture {
        input: serde_json::json!({}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_arg("url")]);

    let result = apply_defaults(fixture, &call);

    assert_eq!(
        result.input.get("url").and_then(|v| v.as_str()),
        Some(placeholder_base().base())
    );
    assert!(
        result.preserve_input_urls,
        "injecting a literal must opt the fixture in"
    );
}

#[test]
fn leaves_an_already_declared_absolute_url_untouched() {
    let fixture = Fixture {
        input: serde_json::json!({"url": "http://127.0.0.1:9/"}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_arg("url")]);

    let result = apply_defaults(fixture, &call);

    assert_eq!(
        result.input.get("url").and_then(|v| v.as_str()),
        Some("http://127.0.0.1:9/")
    );
    assert!(
        !result.preserve_input_urls,
        "a scheme-carrying literal is left for the fixture author to opt in explicitly"
    );
}

/// A batch-style fixture that declares a bare path (no scheme) rather than leaving the
/// field unset -- e.g. `"batch_urls": ["/seed1", "/seed2"]` -- is the shape most of
/// task #140's residual rejections had: the field passed the module's old
/// `already_declared` check and so was never given a docs-safe literal.
#[test]
fn rewrites_a_declared_relative_scalar_url_to_the_default_base() {
    let fixture = Fixture {
        input: serde_json::json!({"url": "/seed1"}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_arg("url")]);

    let result = apply_defaults(fixture, &call);

    assert_eq!(
        result.input.get("url").and_then(|v| v.as_str()),
        Some("https://example.com/seed1")
    );
    assert!(result.preserve_input_urls);
}

#[test]
fn rewrites_every_relative_element_of_a_declared_url_list() {
    let fixture = Fixture {
        input: serde_json::json!({"urls": ["/seed1", "/seed2"]}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_list_arg("urls")]);

    let result = apply_defaults(fixture, &call);

    assert_eq!(
        result.input.get("urls").and_then(|v| v.as_array()),
        Some(&vec![
            serde_json::Value::String("https://example.com/seed1".to_string()),
            serde_json::Value::String("https://example.com/seed2".to_string()),
        ])
    );
    assert!(result.preserve_input_urls);
}

/// A mixed list keeps any already-scheme-carrying element verbatim and only rewrites
/// the bare ones -- a list is not forced into the all-or-nothing "already meaningful"
/// bucket just because one of its entries happens to have a scheme.
#[test]
fn a_mixed_url_list_only_rewrites_its_relative_elements() {
    let fixture = Fixture {
        input: serde_json::json!({"urls": ["/seed1", "http://127.0.0.1:9/seed2"]}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_list_arg("urls")]);

    let result = apply_defaults(fixture, &call);

    assert_eq!(
        result.input.get("urls").and_then(|v| v.as_array()),
        Some(&vec![
            serde_json::Value::String("https://example.com/seed1".to_string()),
            serde_json::Value::String("http://127.0.0.1:9/seed2".to_string()),
        ])
    );
    assert!(result.preserve_input_urls);
}

#[test]
fn leaves_an_already_declared_absolute_url_list_untouched() {
    let fixture = Fixture {
        input: serde_json::json!({"urls": ["http://127.0.0.1:9/a", "gopher://127.0.0.1:9/b"]}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_list_arg("urls")]);

    let result = apply_defaults(fixture, &call);

    assert_eq!(
        result.input.get("urls").and_then(|v| v.as_array()),
        Some(&vec![
            serde_json::Value::String("http://127.0.0.1:9/a".to_string()),
            serde_json::Value::String("gopher://127.0.0.1:9/b".to_string()),
        ])
    );
    assert!(!result.preserve_input_urls);
}

/// `"batch_urls": []` (e.g. `batch_scrape_empty_urls_error.json`, testing the
/// empty-input error path) must be marked preserved even though it declares no
/// scheme at all: `all()` over an empty iterator is vacuously true, so without this
/// case an empty list would be classified `AlreadyMeaningful` and left with
/// `preserve_input_urls` unset, and every backend's non-preserved `mock_url_list`
/// branch still emits an unconditional `MOCK_SERVER_URL`-bearing base line even when
/// the path list it feeds is empty.
#[test]
fn a_declared_empty_url_list_is_marked_preserved() {
    let fixture = Fixture {
        input: serde_json::json!({"urls": []}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_list_arg("urls")]);

    let result = apply_defaults(fixture, &call);

    assert_eq!(result.input.get("urls").and_then(|v| v.as_array()), Some(&vec![]));
    assert!(result.preserve_input_urls);
}

/// The counterpart bug task #140 also asked to check: a fixture that declares a
/// meaningful literal but forgets `preserve_input_urls` has that literal silently
/// discarded at codegen time (see `preserved_url_literal`/`preserved_url_list` in
/// `e2e::codegen`) -- both the executable suite and the docs snippet bind the mock
/// server instead. This module cannot fix that discard (the bind happens downstream,
/// per-backend), but it is the one seam that already inspects every mock_url/
/// mock_url_list argument's declared value, so it is where the loud signal belongs.
#[test]
#[tracing_test::traced_test]
fn warns_when_a_meaningful_url_is_declared_without_preserve_input_urls() {
    let fixture = Fixture {
        id: "validation_probe".into(),
        input: serde_json::json!({"url": "http://127.0.0.1:9/"}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_arg("url")]);

    apply_defaults(fixture, &call);

    assert!(
        logs_contain("without setting preserve_input_urls"),
        "expected a warning naming the missing preserve_input_urls opt-in"
    );
}

#[test]
#[tracing_test::traced_test]
fn does_not_warn_once_preserve_input_urls_is_set() {
    let fixture = Fixture {
        id: "validation_probe".into(),
        preserve_input_urls: true,
        input: serde_json::json!({"url": "http://127.0.0.1:9/"}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_arg("url")]);

    apply_defaults(fixture, &call);

    assert!(
        !logs_contain("without setting preserve_input_urls"),
        "a fixture that already opted in must not be flagged"
    );
}

#[test]
fn injects_a_single_element_default_list_for_mock_url_list() {
    let fixture = Fixture {
        input: serde_json::json!({}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_list_arg("urls")]);

    let result = apply_defaults(fixture, &call);

    assert_eq!(
        result.input.get("urls").and_then(|v| v.as_array()),
        Some(&vec![serde_json::Value::String(placeholder_base().base().to_string())])
    );
    assert!(result.preserve_input_urls);
}

/// The defect this module's configurability exists for: with a project's own sample
/// host configured, an undeclared `mock_url` argument must bind that host, not the
/// reserved documentation domain a reader cannot fetch anything from.
#[test]
fn a_configured_sample_base_url_replaces_the_placeholder_for_an_undeclared_argument() {
    let fixture = Fixture {
        input: serde_json::json!({}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_arg("url")]);
    let base = DocsSampleBaseUrl::resolve(Some("https://samples.example.org")).expect("valid base");

    let result = with_default_mock_url_literals(fixture, &call, base, None, None);

    assert_eq!(
        result.input.get("url").and_then(|v| v.as_str()),
        Some("https://samples.example.org")
    );
    assert!(result.preserve_input_urls);
}

#[test]
fn a_configured_sample_base_url_is_the_base_declared_relative_paths_resolve_against() {
    let fixture = Fixture {
        input: serde_json::json!({"urls": ["/pdf/report.pdf", "/pdf/memo.pdf"]}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_list_arg("urls")]);
    let base = DocsSampleBaseUrl::resolve(Some("https://samples.example.org/")).expect("valid base");

    let result = with_default_mock_url_literals(fixture, &call, base, None, None);

    assert_eq!(
        result.input.get("urls").and_then(|v| v.as_array()),
        Some(&vec![
            serde_json::Value::String("https://samples.example.org/pdf/report.pdf".to_string()),
            serde_json::Value::String("https://samples.example.org/pdf/memo.pdf".to_string()),
        ])
    );
    assert!(result.preserve_input_urls);
}

/// A fixture that declares its own absolute sample URL keeps it: the configured base is
/// a default for fixtures that declare nothing, never an override of an address a
/// fixture author chose.
#[test]
fn a_declared_absolute_url_outranks_the_configured_sample_base_url() {
    let fixture = Fixture {
        preserve_input_urls: true,
        input: serde_json::json!({"url": "https://docs.example.net/report.pdf"}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_arg("url")]);
    let base = DocsSampleBaseUrl::resolve(Some("https://samples.example.org")).expect("valid base");

    let result = with_default_mock_url_literals(fixture, &call, base, None, None);

    assert_eq!(
        result.input.get("url").and_then(|v| v.as_str()),
        Some("https://docs.example.net/report.pdf"),
        "an absolute declared URL is the fixture author's choice and stays verbatim"
    );
}

/// The defect this module's per-fixture template resolution exists to fix: a
/// content-addressed corpus cannot be expressed by joining a flat base with the fixture's
/// declared relative path, because the real address depends on a fact about the object --
/// here a digest the fixture itself supplies -- not on the path.
#[test]
fn a_configured_template_resolves_a_relative_scalar_url_from_fixture_vars() {
    let fixture = Fixture {
        input: serde_json::json!({"url": "/pdf/report.pdf"}),
        docs: Some(FixtureDocs {
            sample_url_vars: BTreeMap::from([("digest".to_string(), "abc123".to_string())]),
            ..empty_docs()
        }),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_arg("url")]);
    let base = DocsSampleBaseUrl::resolve(Some("https://samples.example.org")).expect("valid base");
    let template = SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/{digest}"))
        .expect("valid template resolves")
        .expect("a configured value produces a template");

    let result = with_default_mock_url_literals(fixture, &call, base, Some(&template), None);

    assert_eq!(
        result.input.get("url").and_then(|v| v.as_str()),
        Some("https://cdn.example.org/objects/abc123"),
        "a fixture supplying what the template needs must publish the templated address"
    );
    assert!(result.preserve_input_urls);
}

/// The case that keeps per-fixture templating from becoming a silencer: a template is
/// configured, but this fixture never declared the fact it needs, so resolution must fall
/// back to `sample_base_url` -- keeping the reserved-domain placeholder warning honest for
/// exactly this fixture -- rather than publishing a broken partial URL.
#[test]
fn a_configured_template_without_matching_fixture_vars_falls_back_to_sample_base_url() {
    let fixture = Fixture {
        input: serde_json::json!({"url": "/pdf/report.pdf"}),
        docs: Some(empty_docs()),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_arg("url")]);
    let base = DocsSampleBaseUrl::resolve(Some("https://samples.example.org")).expect("valid base");
    let template = SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/{digest}"))
        .expect("valid template resolves")
        .expect("a configured value produces a template");

    let result = with_default_mock_url_literals(fixture, &call, base, Some(&template), None);

    assert_eq!(
        result.input.get("url").and_then(|v| v.as_str()),
        Some("https://samples.example.org/pdf/report.pdf"),
        "a fixture missing the template's required facts must keep publishing the flat \
         sample_base_url address"
    );
    assert!(result.preserve_input_urls);
}

/// The manifest's reason to exist, driven through this module's own entry point: a fixture
/// whose `docs.body_file` the manifest covers publishes the manifest-derived address with no
/// `docs.sample_url_vars` declared at all.
#[test]
fn a_fixture_whose_body_file_is_covered_by_the_manifest_publishes_its_templated_address() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        directory.path().join("manifest.json"),
        r#"{"pdf/report.pdf": "abc123"}"#,
    )
    .expect("write manifest");
    let manifest_config = crate::core::config::e2e::SampleUrlManifestConfig {
        path: "manifest.json".to_string(),
        variable: "digest".to_string(),
    };
    let manifest = SampleUrlManifest::resolve(Some(&manifest_config), directory.path())
        .expect("valid manifest resolves")
        .expect("a configured value produces a manifest");
    let fixture = Fixture {
        input: serde_json::json!({"url": "/pdf/report.pdf"}),
        docs: Some(FixtureDocs {
            body_file: Some("pdf/report.pdf".to_string()),
            ..empty_docs()
        }),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_arg("url")]);
    let base = DocsSampleBaseUrl::resolve(Some("https://samples.example.org")).expect("valid base");
    let template = SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/{digest}"))
        .expect("valid template resolves")
        .expect("a configured value produces a template");

    let result = with_default_mock_url_literals(fixture, &call, base, Some(&template), Some(&manifest));

    assert_eq!(
        result.input.get("url").and_then(|v| v.as_str()),
        Some("https://cdn.example.org/objects/abc123"),
        "a fixture whose body_file the manifest covers must publish the manifest-derived address"
    );
    assert!(result.preserve_input_urls);
}

/// The regression guard for the manifest: a fixture whose `docs.body_file` the manifest does
/// NOT cover keeps resolving through `sample_base_url` exactly as an uncovered fixture always
/// has -- a manifest being configured in general must never change behavior for a fixture the
/// manifest says nothing about.
#[test]
fn a_fixture_whose_body_file_the_manifest_does_not_cover_falls_back_to_sample_base_url() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("manifest.json"), r#"{"pdf/other.pdf": "abc123"}"#).expect("write manifest");
    let manifest_config = crate::core::config::e2e::SampleUrlManifestConfig {
        path: "manifest.json".to_string(),
        variable: "digest".to_string(),
    };
    let manifest = SampleUrlManifest::resolve(Some(&manifest_config), directory.path())
        .expect("valid manifest resolves")
        .expect("a configured value produces a manifest");
    let fixture = Fixture {
        input: serde_json::json!({"url": "/pdf/report.pdf"}),
        docs: Some(FixtureDocs {
            body_file: Some("pdf/report.pdf".to_string()),
            ..empty_docs()
        }),
        ..Fixture::default()
    };
    let call = call_with_args(vec![mock_url_arg("url")]);
    let base = DocsSampleBaseUrl::resolve(Some("https://samples.example.org")).expect("valid base");
    let template = SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/{digest}"))
        .expect("valid template resolves")
        .expect("a configured value produces a template");

    let result = with_default_mock_url_literals(fixture, &call, base, Some(&template), Some(&manifest));

    assert_eq!(
        result.input.get("url").and_then(|v| v.as_str()),
        Some("https://samples.example.org/pdf/report.pdf"),
        "a fixture whose body_file the manifest does not mention must keep publishing the \
         flat sample_base_url address"
    );
    assert!(result.preserve_input_urls);
}

fn empty_docs() -> FixtureDocs {
    FixtureDocs {
        topic: "contract".to_string(),
        stem: None,
        paths: Default::default(),
        title: None,
        description: None,
        input: None,
        shows: Vec::new(),
        error: None,
        presentation: None,
        client: None,
        side_effects: Default::default(),
        coverage_exceptions: Default::default(),
        sample_url_vars: Default::default(),
        body_file: None,
    }
}

#[test]
fn a_fixture_with_no_mock_url_arg_is_left_unmarked() {
    let fixture = Fixture {
        input: serde_json::json!({"text": "sample"}),
        ..Fixture::default()
    };
    let call = call_with_args(vec![ArgMapping {
        name: "text".into(),
        field: "text".into(),
        arg_type: "string".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }]);

    let result = apply_defaults(fixture, &call);

    assert!(!result.preserve_input_urls);
    assert!(result.input.get("url").is_none());
}

/// A single seam pinned end to end: a batch-style fixture declaring bare `batch_urls`
/// paths (the shape task #140 traced most residual rejections to) must clear BOTH real
/// gates the production pipeline runs it through -- this module's rewrite AND
/// [`crate::e2e::snippets::mock_harness_guard::reject_mock_harness_scaffolding`] -- by
/// actually rendering a Python documentation snippet body, not just inspecting the
/// intermediate `Fixture` this module returns. Driving both real functions is what would
/// have caught the sibling defect this task also found: the Python `mock_url_list`
/// arg-binding code (`e2e::codegen::python::test_function::args`) used to push its
/// `{var}_base = os.environ['MOCK_SERVER_URL'] + ...` line unconditionally, before
/// checking whether the fixture was preserved, so even a fully rewritten and preserved
/// list still leaked into the snippet body and failed the guard.
#[test]
fn a_declared_relative_batch_url_list_survives_the_full_snippet_pipeline() {
    use crate::core::config::NewAlefConfig;
    use crate::e2e::codegen::E2eCodegen;
    use crate::e2e::codegen::python::PythonE2eCodegen;

    let cfg_str = r#"
[workspace]
languages = ["python"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "batch_scrape"
module = "example_api"
args = [{ name = "urls", field = "batch_urls", type = "mock_url_list" }]
"#;
    let cfg: NewAlefConfig = toml::from_str(cfg_str).expect("config parses");
    let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
    let resolved = cfg.resolve().expect("config resolves").remove(0);

    let fixture = Fixture {
        id: "batch_crawl_basic".into(),
        input: serde_json::json!({"batch_urls": ["/seed1", "/seed2"]}),
        ..Fixture::default()
    };

    // Mirrors `snippets::render_snippet_body`'s own sequencing exactly: transform for
    // docs first, THEN apply this module's defaults to the docs-transformed clone.
    let docs_fixture = fixture.docs_call_fixture();
    let call = e2e.resolve_call_for_fixture(
        docs_fixture.call.as_deref(),
        &docs_fixture.id,
        &docs_fixture.resolved_category(),
        &docs_fixture.tags,
        &docs_fixture.input,
    );
    let docs_fixture = with_default_mock_url_literals(docs_fixture, call, placeholder_base(), None, None);

    assert_eq!(
        docs_fixture.input.get("batch_urls").and_then(|v| v.as_array()),
        Some(&vec![
            serde_json::Value::String("https://example.com/seed1".to_string()),
            serde_json::Value::String("https://example.com/seed2".to_string()),
        ]),
        "the module must rewrite the declared relative paths before codegen ever sees them"
    );
    assert!(docs_fixture.preserve_input_urls);

    let body = PythonE2eCodegen
        .render_snippet_body(&docs_fixture, &e2e, &resolved, &[], &[])
        .expect("python snippet body renders");

    assert!(
        !body.contains("MOCK_SERVER"),
        "rendered snippet body must carry no mock-harness wiring:\n{body}"
    );
    crate::e2e::snippets::mock_harness_guard::reject_mock_harness_scaffolding(&body, &docs_fixture, "python")
        .expect("the harness-leak guard must accept a fully preserved, rewritten snippet body");
}
