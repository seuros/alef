//! Regression coverage for the C engine-factory URL construction gap: batch/list fixtures
//! (URLs living in a `batch_urls`/`urls` list, no scalar `url` key) fell through to the raw
//! `getenv("MOCK_SERVER_URL")` scaffolding branch even when `preserve_input_urls` was set,
//! because the emitter only ever consulted `input.url`. That scaffolding is exactly what
//! `reject_mock_harness_scaffolding` (`src/e2e/snippets/mock_harness_guard.rs`) exists to
//! catch in a published documentation snippet.
//!
//! ~keep New submodule of `call_patterns` rather than growing `call_patterns.rs` itself,
//! `test_function.rs`, or `snippet_regressions.rs` (the latter two already over the repo's
//! 1,000-line cap; see `file-modularization` in CLAUDE.md).

use std::collections::{HashMap, HashSet};

use crate::e2e::codegen::c::assertions::{EffectiveConfigSource, FieldConfigSources};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Fixture;

fn permissive_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn global_sources() -> FieldConfigSources {
    FieldConfigSources {
        result_fields: EffectiveConfigSource::Global,
        fields: EffectiveConfigSource::Global,
    }
}

/// A batch-shaped, doc-snippet fixture: no `url` key, its addresses live under `batch_urls`,
/// and `preserve_input_urls` is set (the flag that means "this is the subject of the test /
/// documentation, do not substitute the mock server address" -- see `preserved_url_literal`'s
/// doc comment in `src/e2e/codegen/mod.rs`).
fn batch_fixture() -> Fixture {
    Fixture {
        id: "batch_crawl_basic".into(),
        description: "Crawl a batch of seed URLs".into(),
        input: serde_json::json!({"batch_urls": ["/seed1", "/seed2"]}),
        preserve_input_urls: true,
        ..Fixture::default()
    }
}

/// Drives the real production function directly (not a hand-rolled mirror of intended
/// behaviour) against a batch-shaped fixture, and asserts the rendered C body never falls
/// back to the raw mock-harness scaffolding the guard exists to reject.
///
/// Before the fix: `resolve_field(&fixture.input, "input.url")` resolves to `Null` for this
/// fixture (there is no `url` key), so `preserved_url_literal` returns `None` regardless of
/// `preserve_input_urls`, and the emitter falls unconditionally into the
/// `getenv("MOCK_SERVER_URL")` branch -- this test FAILS against that code.
///
/// After the fix: the batch/list field is consulted via the shared
/// `resolve_urls_field`/`preserved_url_list` seam (the same helpers every other backend's
/// `mock_url_list` handling already uses), the first list entry becomes the engine-factory
/// pattern's single positional `url`, and the raw scaffolding branch is never reached.
#[test]
fn batch_url_list_fixture_does_not_fall_back_to_raw_mock_scaffolding() {
    let fixture = batch_fixture();
    let mut out = String::new();

    super::render_engine_factory_test_function(
        &mut out,
        &fixture,
        "sample",
        "batch_scrape",
        "result",
        &permissive_resolver(),
        &HashMap::new(),
        &HashSet::new(),
        "BatchCrawlResults",
        "CrawlConfig",
        false,
        Some("char*"),
        &[],
        &global_sources(),
    )
    .expect("engine-factory batch fixture renders");

    assert!(
        !out.contains("getenv(\"MOCK_SERVER_URL\")"),
        "batch fixture must not fall back to raw mock-harness scaffolding \
         (this is exactly what `reject_mock_harness_scaffolding` rejects in a doc snippet):\n{out}"
    );
    assert!(
        out.contains("snprintf(url, sizeof(url), \"%s\", \"/seed1\");"),
        "expected the first batch_urls entry to become the preserved literal url:\n{out}"
    );
}

/// Control: the same batch-shaped fixture WITHOUT `preserve_input_urls` set must keep using
/// the mock-server scaffolding -- proves the fix is conditioned on the existing preservation
/// flag, not a blanket "always prefer batch_urls" change that would silently unplug the C e2e
/// suite's normal (non-doc) test generation for every batch fixture.
#[test]
fn batch_url_list_fixture_without_preserve_flag_still_uses_mock_server() {
    let mut fixture = batch_fixture();
    fixture.preserve_input_urls = false;
    let mut out = String::new();

    super::render_engine_factory_test_function(
        &mut out,
        &fixture,
        "sample",
        "batch_scrape",
        "result",
        &permissive_resolver(),
        &HashMap::new(),
        &HashSet::new(),
        "BatchCrawlResults",
        "CrawlConfig",
        false,
        Some("char*"),
        &[],
        &global_sources(),
    )
    .expect("engine-factory batch fixture renders");

    assert!(
        out.contains("getenv(\"MOCK_SERVER_URL\")"),
        "without preserve_input_urls the mock-server scaffolding must still be emitted:\n{out}"
    );
    assert!(
        !out.contains("/seed1"),
        "the batch literal must not leak when preservation is off:\n{out}"
    );
}

/// REPRODUCTION for the empty-array case: a fixture whose `batch_urls` list is
/// deliberately empty (e.g. `batch_scrape_empty_urls_error.json`, testing the
/// empty-input error path) still falls back to the raw `getenv("MOCK_SERVER_URL")`
/// scaffolding today, because `preserved_url_list(true, [])` is `Some(vec![])` and
/// `.into_iter().next()` on an empty vec is `None` -- so `preserved_url` ends up
/// `None` exactly like the undeclared case, even though `preserve_input_urls` is set.
#[test]
fn empty_batch_url_list_fixture_does_not_fall_back_to_raw_mock_scaffolding() {
    // Mirrors the real `batch_scrape_empty_urls_error.json` shape: `batch_urls: []` is
    // deliberate (testing the empty-input error path), `preserve_input_urls` is set by
    // `mock_url_defaults::with_default_mock_url_literals` for exactly this case (see its
    // `a_declared_empty_url_list_is_marked_preserved` test), and the fixture carries its
    // own "error" assertion -- the thing rule 3 requires this test to prove still holds.
    let fixture = Fixture {
        id: "batch_scrape_empty_urls_error".into(),
        description: "Batch scrape rejects an empty URL list".into(),
        input: serde_json::json!({"batch_urls": []}),
        preserve_input_urls: true,
        assertions: vec![crate::e2e::fixture::Assertion {
            assertion_type: "error".into(),
            field: None,
            value: Some(serde_json::Value::String("empty urls".into())),
            values: None,
            ..crate::e2e::fixture::Assertion::default()
        }],
        ..Fixture::default()
    };
    let mut out = String::new();

    super::render_engine_factory_test_function(
        &mut out,
        &fixture,
        "sample",
        "batch_scrape",
        "result",
        &permissive_resolver(),
        &HashMap::new(),
        &HashSet::new(),
        "BatchCrawlResults",
        "CrawlConfig",
        true,
        Some("char*"),
        &[],
        &global_sources(),
    )
    .expect("engine-factory empty-batch fixture renders");

    assert!(
        !out.contains("MOCK_SERVER_URL"),
        "an empty, deliberately-declared batch_urls list has nothing to leak and must not \
         fall back to mock-harness scaffolding:\n{out}"
    );
    assert!(
        !out.contains("getenv"),
        "no mock-harness getenv scaffolding of any kind belongs in this render:\n{out}"
    );
    assert!(
        out.contains("snprintf(url, sizeof(url), \"%s\", \"\");"),
        "the deliberately empty list must render as a literal empty url, not be dropped:\n{out}"
    );
    // Rule 3: the fixture's own empty-array error assertion must survive untouched --
    // this fix only changes URL construction, not the fixture's assertions themselves.
    assert_eq!(
        crate::e2e::codegen::declared_error_value(&fixture),
        Some("empty urls"),
        "the fixture's own error assertion must still be intact after rendering"
    );
}
