//! The mock-harness leak guard, split out of `super` to keep `mod.rs` under the repo's
//! 1,000-line cap while this module's `mock_url_defaults` sibling adds the zero-edit
//! default that keeps most fixtures out of this guard's way in the first place.

use crate::e2e::fixture::Fixture;
use anyhow::Result;

/// Substrings that only ever appear in e2e mock-server wiring.
///
/// Each is a name the harness itself owns: the environment variables the mock server
/// exports (`MOCK_SERVER_URL`, `MOCK_SERVERS`, the per-fixture `MOCK_SERVER_<ID>`) and
/// the JVM system properties the Java/Kotlin suites read them through.
const MOCK_HARNESS_MARKERS: &[&str] = &[
    "MOCK_SERVER_URL",
    "MOCK_SERVERS",
    "MOCK_SERVER_",
    "mockServerUrl",
    "mockServer.",
];

/// Reject a snippet body that carries e2e mock-server scaffolding.
///
/// Snippet bodies are published verbatim into the docs site, so a body that still points
/// at the mock server documents the test harness rather than the library. Every language
/// — built-in or extension-supplied — funnels through `render_snippet_body`, so placing
/// the check here means a new backend inherits the guarantee instead of having to
/// re-derive it. The `Err` carries a typed [`super::MockHarnessLeak`] so the caller can
/// route it to a hard, attributed failure rather than to a coverage gap that a
/// `coverage_exceptions` entry would silently absorb.
pub(super) fn reject_mock_harness_scaffolding(body: &str, fixture: &Fixture, language: &str) -> Result<()> {
    let fixture_route = format!("/fixtures/{}", fixture.id);
    let marker = MOCK_HARNESS_MARKERS
        .iter()
        .copied()
        .chain(std::iter::once(fixture_route.as_str()))
        .find(|marker| body.contains(marker));
    if let Some(marker) = marker {
        return Err(anyhow::Error::new(super::MockHarnessLeak {
            marker: marker.to_string(),
            fixture_id: fixture.id.clone(),
            language: language.to_string(),
        }));
    }
    Ok(())
}
