//! Per-fixture sample URL resolution: `[crates.e2e.snippets].sample_url_template`.
//!
//! [`DocsSampleBaseUrl`](super::DocsSampleBaseUrl) can express exactly one shape of public
//! address: a flat prefix concatenated with a fixture's mock-relative path. That is
//! structurally insufficient for a content-addressed sample corpus, where an object's real
//! address is a function of facts about that specific object (a digest, a bucket key, ...)
//! and not of its mock path alone -- no single prefix can produce `bucket/objects/<sha256>`
//! for every fixture from one shared base.
//!
//! A [`SampleUrlTemplate`] closes that gap without touching [`DocsSampleBaseUrl`] at all. A
//! project configures a template such as `"https://cdn.example.org/objects/{digest}"`, and a
//! fixture supplies the facts its own occurrences of `{digest}` need through
//! `docs.sample_url_vars` (see `crate::e2e::fixture::FixtureDocs`) -- kept on the fixture
//! itself rather than a separate corpus-manifest file, so the fact that decides a fixture's
//! own address lives beside the fixture it describes, with nothing else to keep in sync.
//! `{path}` is always available and resolves to the fixture's mock-relative path (the same
//! string `DocsSampleBaseUrl::join` would have appended to a flat base), so a template needs
//! no per-fixture vars at all when a bucket key alone is enough to express the address.
//!
//! Resolution never overrides a fixture: [`SampleUrlTemplate::render`] returns `None` the
//! moment a placeholder has no matching fact, and every caller in `crate::e2e::snippets` and
//! `crate::e2e::fixture::docs_presentation` falls back to `sample_base_url` -- the placeholder
//! domain when that too is unconfigured -- exactly as it did before this type existed. That
//! fallback is what keeps `report_placeholder_sample_urls` warning correctly for a fixture a
//! template was configured for but cannot actually resolve: an unresolved occurrence still
//! publishes the flat base, and the existing "does the body still carry the placeholder"
//! check still catches it. Configuring a template is therefore strictly additive over
//! `sample_base_url`, never a silencer of its own right.

use super::sample_url::has_url_scheme;
use std::collections::BTreeMap;

/// The `alef.toml` key a project sets to enable per-fixture template resolution, named here so
/// every diagnostic that mentions it spells it the same way.
pub const SAMPLE_URL_TEMPLATE_CONFIG_KEY: &str = "[crates.e2e.snippets].sample_url_template";

/// The fixture-side key a fixture author sets to supply the facts its own mock-relative paths
/// need beyond `{path}`, named here for the same reason.
pub const SAMPLE_URL_VARS_FIXTURE_KEY: &str = "docs.sample_url_vars";

/// The variable name every [`SampleUrlTemplate`] binds automatically to the occurrence's
/// mock-relative path, with no fixture-side declaration required.
const PATH_VARIABLE: &str = "path";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidSampleUrlTemplate {
    #[error("`{SAMPLE_URL_TEMPLATE_CONFIG_KEY}` is empty; remove the key to disable per-fixture template resolution")]
    Empty,
    #[error("`{SAMPLE_URL_TEMPLATE_CONFIG_KEY}` must contain no whitespace, got `{value}`")]
    Whitespace { value: String },
    #[error(
        "`{SAMPLE_URL_TEMPLATE_CONFIG_KEY}` must be absolute and name a scheme (e.g. \
         `https://cdn.example.org/objects/{{digest}}`), got `{value}`"
    )]
    Relative { value: String },
    #[error("`{SAMPLE_URL_TEMPLATE_CONFIG_KEY}` has an unmatched `{{` with no closing `}}`, got `{value}`")]
    UnbalancedBraces { value: String },
    #[error("`{SAMPLE_URL_TEMPLATE_CONFIG_KEY}` has an empty `{{}}` placeholder, got `{value}`")]
    EmptyPlaceholder { value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TemplateSegment {
    Literal(String),
    Placeholder(String),
}

/// A validated `sample_url_template`, parsed once at configuration-resolution time so
/// [`Self::render`] can never fail on malformed syntax -- only on a fact the caller's fixture
/// does not supply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleUrlTemplate {
    raw: String,
    segments: Vec<TemplateSegment>,
}

impl SampleUrlTemplate {
    /// Resolve `configured` (the raw `sample_url_template` value, if any) into a validated
    /// template. `None` means the project configured no template at all, distinct from every
    /// other outcome, which is either a usable template or a configuration error -- a
    /// malformed template must fail the run rather than silently falling back to
    /// `sample_base_url`, the same posture [`super::DocsSampleBaseUrl::resolve`] takes.
    pub fn resolve(configured: Option<&str>) -> Result<Option<Self>, InvalidSampleUrlTemplate> {
        let Some(value) = configured else {
            return Ok(None);
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(InvalidSampleUrlTemplate::Empty);
        }
        if trimmed.chars().any(char::is_whitespace) {
            return Err(InvalidSampleUrlTemplate::Whitespace {
                value: trimmed.to_string(),
            });
        }
        if !has_url_scheme(trimmed) {
            return Err(InvalidSampleUrlTemplate::Relative {
                value: trimmed.to_string(),
            });
        }
        let segments = parse_segments(trimmed)?;
        Ok(Some(Self {
            raw: trimmed.to_string(),
            segments,
        }))
    }

    /// Render this template for one fixture's mock-relative `path` (e.g. `"/pdf/memo.pdf"`),
    /// resolving every `{path}` placeholder to it and every other placeholder against `vars`
    /// -- a fixture's own `docs.sample_url_vars`.
    ///
    /// Returns `None` the moment a placeholder names a variable `vars` does not carry, rather
    /// than rendering a partial URL or falling back internally: the caller decides what an
    /// unresolved occurrence means (fall back to `sample_base_url`, and let the existing
    /// placeholder-domain check keep warning about it).
    pub fn render(&self, path: &str, vars: &BTreeMap<String, String>) -> Option<String> {
        let mut result = String::with_capacity(self.raw.len());
        for segment in &self.segments {
            match segment {
                TemplateSegment::Literal(text) => result.push_str(text),
                TemplateSegment::Placeholder(name) if name == PATH_VARIABLE => result.push_str(path),
                TemplateSegment::Placeholder(name) => result.push_str(vars.get(name)?),
            }
        }
        Some(result)
    }

    /// The validated template text, verbatim -- used by diagnostics that name the configured
    /// value rather than repeat it as a raw config string.
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

fn parse_segments(template: &str) -> Result<Vec<TemplateSegment>, InvalidSampleUrlTemplate> {
    let mut segments = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        if open > 0 {
            segments.push(TemplateSegment::Literal(rest[..open].to_string()));
        }
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('}') else {
            return Err(InvalidSampleUrlTemplate::UnbalancedBraces {
                value: template.to_string(),
            });
        };
        let name = &after_open[..close];
        if name.is_empty() {
            return Err(InvalidSampleUrlTemplate::EmptyPlaceholder {
                value: template.to_string(),
            });
        }
        segments.push(TemplateSegment::Placeholder(name.to_string()));
        rest = &after_open[close + 1..];
    }
    if rest.contains('}') {
        return Err(InvalidSampleUrlTemplate::UnbalancedBraces {
            value: template.to_string(),
        });
    }
    if !rest.is_empty() {
        segments.push(TemplateSegment::Literal(rest.to_string()));
    }
    Ok(segments)
}

/// Try per-fixture template resolution for one mock-relative `path`.
///
/// `None` means the caller must fall back to its own flat `sample_base_url` -- either because
/// `template` is `None` (the project configured no per-fixture resolution at all) or because
/// this fixture's `vars` do not cover a fact the template references. The single seam both
/// `crate::e2e::snippets::mock_url_defaults` and `crate::e2e::fixture::docs_presentation` call
/// through, so the two call sites cannot independently drift on what "resolved" means. ~keep
pub fn resolve_templated_sample_url(
    template: Option<&SampleUrlTemplate>,
    path: &str,
    vars: &BTreeMap<String, String>,
) -> Option<String> {
    template.and_then(|template| template.render(path, vars))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn an_unconfigured_template_resolves_to_none() {
        assert_eq!(
            SampleUrlTemplate::resolve(None).expect("no configuration always resolves"),
            None
        );
    }

    #[test]
    fn a_configured_template_renders_the_path_placeholder() {
        let template = SampleUrlTemplate::resolve(Some("https://cdn.example.org{path}"))
            .expect("valid template resolves")
            .expect("a configured value produces a template");

        assert_eq!(
            template.render("/pdf/memo.pdf", &BTreeMap::new()),
            Some("https://cdn.example.org/pdf/memo.pdf".to_string())
        );
    }

    /// The defect this type exists to fix: a content-addressed address is a function of a
    /// fact about the object, not of its mock path, so the template must be able to ignore
    /// `{path}` entirely and resolve purely from fixture-declared facts.
    #[test]
    fn a_content_addressed_template_resolves_from_fixture_vars_alone() {
        let template = SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/{digest}"))
            .expect("valid template resolves")
            .expect("a configured value produces a template");

        assert_eq!(
            template.render(
                "/pdf/memo.pdf",
                &vars(&[("digest", "9f86d081884c7d659a2feaa0c55ad015")])
            ),
            Some("https://cdn.example.org/objects/9f86d081884c7d659a2feaa0c55ad015".to_string())
        );
    }

    #[test]
    fn a_template_mixing_path_and_fixture_vars_resolves_both() {
        let template = SampleUrlTemplate::resolve(Some("https://cdn.example.org{path}?digest={digest}"))
            .expect("valid template resolves")
            .expect("a configured value produces a template");

        assert_eq!(
            template.render("/pdf/memo.pdf", &vars(&[("digest", "abc123")])),
            Some("https://cdn.example.org/pdf/memo.pdf?digest=abc123".to_string())
        );
    }

    /// The case that keeps this feature from becoming a silencer: a template is configured,
    /// but this fixture never declared the fact the template needs, so resolution must fail
    /// rather than publish a broken partial URL.
    #[test]
    fn rendering_fails_when_a_referenced_variable_is_not_supplied() {
        let template = SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/{digest}"))
            .expect("valid template resolves")
            .expect("a configured value produces a template");

        assert_eq!(template.render("/pdf/memo.pdf", &BTreeMap::new()), None);
    }

    #[test]
    fn resolve_templated_sample_url_falls_back_to_none_with_no_template_configured() {
        assert_eq!(
            resolve_templated_sample_url(None, "/pdf/memo.pdf", &BTreeMap::new()),
            None
        );
    }

    #[test]
    fn resolve_templated_sample_url_delegates_to_the_template_when_configured() {
        let template = SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/{digest}"))
            .expect("valid template resolves")
            .expect("a configured value produces a template");

        assert_eq!(
            resolve_templated_sample_url(Some(&template), "/pdf/memo.pdf", &vars(&[("digest", "abc123")])),
            Some("https://cdn.example.org/objects/abc123".to_string())
        );
    }

    #[test]
    fn an_empty_configured_template_is_rejected() {
        assert_eq!(
            SampleUrlTemplate::resolve(Some("   ")).expect_err("an empty template cannot form a URL"),
            InvalidSampleUrlTemplate::Empty
        );
    }

    #[test]
    fn a_template_with_whitespace_is_rejected() {
        let error = SampleUrlTemplate::resolve(Some("https://cdn.example.org/my objects/{digest}"))
            .expect_err("whitespace is invalid");
        assert!(matches!(error, InvalidSampleUrlTemplate::Whitespace { .. }));
    }

    #[test]
    fn a_scheme_less_template_is_rejected() {
        let error = SampleUrlTemplate::resolve(Some("cdn.example.org/objects/{digest}"))
            .expect_err("a relative template is invalid");
        assert!(matches!(error, InvalidSampleUrlTemplate::Relative { .. }));
    }

    #[test]
    fn an_unclosed_placeholder_is_rejected() {
        let error =
            SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/{digest")).expect_err("unbalanced braces");
        assert!(matches!(error, InvalidSampleUrlTemplate::UnbalancedBraces { .. }));
    }

    #[test]
    fn a_stray_closing_brace_is_rejected() {
        let error =
            SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/digest}")).expect_err("unbalanced braces");
        assert!(matches!(error, InvalidSampleUrlTemplate::UnbalancedBraces { .. }));
    }

    #[test]
    fn an_empty_placeholder_is_rejected() {
        let error =
            SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/{}")).expect_err("empty placeholder");
        assert!(matches!(error, InvalidSampleUrlTemplate::EmptyPlaceholder { .. }));
    }
}
