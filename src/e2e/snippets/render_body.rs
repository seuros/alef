//! Rendering one fixture's snippet body, and reporting the addresses that reached it.
//!
//! Split out of [`super`], which owns the driver loop, the coverage ledger and the write
//! guards. This module owns the narrower question of how a single fixture/language cell
//! becomes target-language source: which recipe renders it (an extension's, or the
//! backend's built-in), and which public addresses its URL arguments are bound to.

use super::*;

/// A rendered snippet body, plus whether the addresses in it came from the
/// reserved-domain placeholder rather than a configured public sample host.
///
/// The flag travels with the body instead of being logged where it is discovered so the
/// run can report once, naming every affected fixture, rather than emitting one warning per
/// fixture per language.
pub(super) struct RenderedSnippetBody {
    pub body: String,
    pub used_placeholder_sample_url: bool,
}

pub(super) fn render_snippet_body(
    extensions: &[Box<dyn crate::Extension>],
    generator: &dyn E2eCodegen,
    fixture: &Fixture,
    language: &str,
    context: &SnippetRenderContext<'_>,
    sample_base_url: DocsSampleBaseUrl<'_>,
) -> Result<RenderedSnippetBody> {
    let docs_fixture = fixture.docs_call_fixture_with_sample_base_url(sample_base_url.base());
    for extension in extensions {
        if let Some(body) = extension
            .render_e2e_snippet(
                &docs_fixture,
                context.e2e,
                context.crate_config,
                language,
                context.type_defs,
                context.enums,
            )
            .map_err(|error| anyhow::anyhow!("extension `{}` could not render snippet: {error:#}", extension.name()))?
        {
            if body.trim().is_empty() {
                bail!("extension `{}` returned an empty snippet body", extension.name());
            }
            mock_harness_guard::reject_mock_harness_scaffolding(&body, &docs_fixture, language)?;
            return Ok(rendered(body, sample_base_url));
        }
    }
    let call = context.e2e.resolve_call_for_fixture(
        docs_fixture.call.as_deref(),
        &docs_fixture.id,
        &docs_fixture.resolved_category(),
        &docs_fixture.tags,
        &docs_fixture.input,
    );
    let docs_fixture = mock_url_defaults::with_default_mock_url_literals(docs_fixture, call, sample_base_url);
    let fixture = &docs_fixture;
    if let Some(kind) = recipe_policy::extension_owned_recipe_kind(fixture, fixture.resolved_args(call)) {
        bail!("{kind} fixture requires an extension-owned documentation recipe");
    }
    let effective_function = call
        .effective_function(language)
        .or_else(|| {
            // The naive identity fallback below derives a symbol name from the raw
            // `fixture.call` config text (`register_fn`/`unregister_fn`/`clear_fn`), which
            // can diverge from what the FFI backend actually generates (see
            // `trait_bridge_function_identity`'s doc comment). A fixture author who set
            // `skip.languages` for this language has already declared that the harness
            // (and by extension this naive fallback) cannot speak for it here; only trust
            // the fallback when the fixture is not skipped for this language. Fixtures with
            // a real, extension-owned recipe never reach this branch — the extension loop
            // above already returned their body, and `recipe_policy::extension_owned_recipe_kind`
            // already bailed for fixtures that require one but lack it.
            let skipped_for_language = fixture.skip.as_ref().is_some_and(|skip| skip.should_skip(language));
            (!skipped_for_language && crate::e2e::fixture::canonical_language(language) == "c")
                .then(|| crate::e2e::codegen::recipe::trait_bridge_function_identity(context.crate_config, fixture))
                .flatten()
        })
        .unwrap_or_default();
    if effective_function.trim().is_empty() {
        bail!(
            "built-in `{language}` snippet recipe has no function identity; configure a call function or provide an extension-owned documentation recipe"
        );
    }
    let body = generator
        .render_snippet_body_with_functions(
            fixture,
            context.e2e,
            context.crate_config,
            context.type_defs,
            context.enums,
            context.functions,
            context.errors,
        )
        .map_err(|error| anyhow::anyhow!("built-in `{language}` snippet recipe is incompatible: {error:#}"))?;
    if body.trim().is_empty() {
        bail!("built-in `{language}` snippet recipe returned an empty body");
    }
    mock_harness_guard::reject_mock_harness_scaffolding(&body, fixture, language)?;
    Ok(rendered(body, sample_base_url))
}

/// Pair a rendered body with the one thing a run can honestly say about its addresses:
/// whether the published text carries the reserved-domain placeholder.
///
/// Deliberately measured on the finished body rather than on what
/// [`mock_url_defaults::with_default_mock_url_literals`] injected. A fixture that writes
/// `"$mock_url"` in its own `docs.input` never reaches that injection branch -- the
/// placeholder is already substituted by [`Fixture::docs_call_fixture_with_sample_base_url`]
/// before the module runs, and the resulting absolute URL reads to it as an address the
/// author chose. Reporting the injection would therefore stay silent about a snippet that
/// does publish an unreachable address. The published text cannot.
fn rendered(body: String, sample_base_url: DocsSampleBaseUrl<'_>) -> RenderedSnippetBody {
    let used_placeholder_sample_url = sample_base_url.is_placeholder() && body.contains(sample_base_url.base());
    RenderedSnippetBody {
        body,
        used_placeholder_sample_url,
    }
}

/// The number of fixture ids a placeholder-URL warning names before it stops enumerating; a
/// consumer with hundreds of URL fixtures should get a usable message, not a log dump.
const PLACEHOLDER_SAMPLE_URL_FIXTURES_NAMED: usize = 10;

/// Say, once per run, that snippets were published carrying an address nobody serves.
///
/// The alternative -- emitting the placeholder and saying nothing -- is what let a reader
/// copy a quick start that could never work: the address is syntactically perfect and fails
/// only when run. Not an error, because a project may legitimately have no public sample
/// host, and failing here would break every consumer that already ships these snippets.
pub(super) fn report_placeholder_sample_urls(fixtures: &[String], sample_base_url: DocsSampleBaseUrl<'_>) {
    if fixtures.is_empty() {
        return;
    }
    let named = fixtures
        .iter()
        .take(PLACEHOLDER_SAMPLE_URL_FIXTURES_NAMED)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let remaining = fixtures.len().saturating_sub(PLACEHOLDER_SAMPLE_URL_FIXTURES_NAMED);
    tracing::warn!(
        target: "alef::e2e::snippets",
        fixtures = fixtures.len(),
        base_url = sample_base_url.base(),
        config_key = crate::core::config::e2e::SAMPLE_BASE_URL_CONFIG_KEY,
        "{} documentation snippet fixture(s) publish the reserved placeholder address `{}`, \
         which serves nothing: a reader who copies them gets a request that cannot succeed. \
         Set `{}` to a host that really serves your sample inputs. Affected fixtures: {named}{}",
        fixtures.len(),
        sample_base_url.base(),
        crate::core::config::e2e::SAMPLE_BASE_URL_CONFIG_KEY,
        if remaining > 0 {
            format!(" (+{remaining} more)")
        } else {
            String::new()
        }
    );
}
