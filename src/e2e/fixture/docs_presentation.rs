//! The docs-shaped view of a [`Fixture`]: the clone a documentation snippet renders from.
//!
//! Split out of [`super`] because it answers a different question than the rest of the
//! fixture model. Everything else there describes what the executable e2e suite runs; this
//! describes what a reader is shown -- the `docs.input` / `docs.presentation` overrides, the
//! mock-response wiring that must not appear in prose, and the public sample address a
//! reader's copy-paste actually resolves against.

use super::Fixture;
#[cfg(test)]
use super::FixtureDocs;
use crate::core::config::e2e::SampleUrlTemplate;
use std::collections::BTreeMap;

impl Fixture {
    /// The docs-shaped clone of this fixture, with any `$mock_url` placeholder bound to the
    /// reserved documentation domain.
    ///
    /// Every caller that holds the project's snippet configuration should call
    /// [`Fixture::docs_call_fixture_with_sample_base_url`] (or
    /// [`Fixture::docs_call_fixture_with_sample_url`], when a per-fixture template is also
    /// configured) instead, so the address a reader sees is the one the project publishes its
    /// samples at. This spelling exists for the call sites that have no configuration in
    /// hand -- chiefly the per-backend renderers, which re-derive the docs clone from a
    /// fixture `crate::e2e::snippets`' own `render_snippet_body` already resolved, where the
    /// placeholder is long gone and this pass has nothing left to replace.
    pub fn docs_call_fixture(&self) -> Self {
        self.docs_call_fixture_with_sample_base_url(crate::core::config::e2e::DEFAULT_DOCS_SAMPLE_BASE_URL)
    }

    /// [`Fixture::docs_call_fixture`], binding `$mock_url` to `sample_base_url` -- the
    /// project's own public sample host -- rather than the reserved-domain placeholder.
    pub fn docs_call_fixture_with_sample_base_url(&self, sample_base_url: &str) -> Self {
        self.docs_call_fixture_with_sample_url(sample_base_url, None)
    }

    /// [`Fixture::docs_call_fixture_with_sample_base_url`], additionally trying this fixture's
    /// own per-fixture sample URL resolution first: when `template` is configured (see
    /// `[crates.e2e.snippets].sample_url_template`), each `$mock_url<path>` occurrence resolves
    /// `{path}` and this fixture's own `docs.sample_url_vars` against it, and publishes the
    /// templated address in place of `sample_base_url` for exactly the occurrences it can fully
    /// resolve. Any occurrence the template cannot resolve -- including every one when
    /// `template` is `None` -- keeps binding `sample_base_url` exactly as before, which is what
    /// keeps the reserved-domain warning firing correctly for a fixture that declares no facts
    /// a configured template needs. ~keep
    pub fn docs_call_fixture_with_sample_url(
        &self,
        sample_base_url: &str,
        template: Option<&SampleUrlTemplate>,
    ) -> Self {
        let mut fixture = self.clone();
        if let Some(input) = self.docs.as_ref().and_then(|docs| docs.input.as_ref()) {
            fixture.input = input.clone();
        }
        if let Some(presentation) = self.docs.as_ref().and_then(|docs| docs.presentation.as_ref()) {
            if let Some(call) = &presentation.call {
                fixture.call = Some(call.clone());
            }
            if let Some(input) = &presentation.input {
                fixture.input = input.clone();
            }
            for file in &presentation.files {
                if let Some(value) = fixture.input.pointer_mut(&file.field) {
                    *value = serde_json::Value::String(file.path.clone());
                }
            }
            if let Some(args) = &presentation.args {
                fixture.args = args.clone();
            }
        }
        fixture.mock_response = None;
        if let Some(input) = fixture.input.as_object_mut() {
            input.remove("mock_responses");
        }
        let empty_vars = BTreeMap::new();
        let vars = self.docs.as_ref().map_or(&empty_vars, |docs| &docs.sample_url_vars);
        replace_docs_mock_urls(&mut fixture.input, sample_base_url, template, vars);
        fixture
    }
}

fn replace_docs_mock_urls(
    value: &mut serde_json::Value,
    sample_base_url: &str,
    template: Option<&SampleUrlTemplate>,
    vars: &BTreeMap<String, String>,
) {
    match value {
        serde_json::Value::String(text) => {
            if text.contains(crate::e2e::codegen::MOCK_URL_PLACEHOLDER) {
                *text = resolve_mock_url_occurrences(text, sample_base_url, template, vars);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                replace_docs_mock_urls(value, sample_base_url, template, vars);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                replace_docs_mock_urls(value, sample_base_url, template, vars);
            }
        }
        _ => {}
    }
}

/// Resolve every `$mock_url<path>` occurrence in `text`, where `path` is the text between one
/// placeholder occurrence and the next (or the end of the string) -- matching how a fixture
/// actually writes these, one whole `"$mock_url/pdf/memo.pdf"` string per occurrence, so `path`
/// is the fixture's real mock-relative path in every case this is used for.
///
/// Tries [`crate::core::config::e2e::resolve_templated_sample_url`] first; falls back to plain
/// concatenation with `sample_base_url` when it returns `None`, byte-for-byte the substitution
/// this function did before per-fixture templates existed. ~keep
fn resolve_mock_url_occurrences(
    text: &str,
    sample_base_url: &str,
    template: Option<&SampleUrlTemplate>,
    vars: &BTreeMap<String, String>,
) -> String {
    let placeholder = crate::e2e::codegen::MOCK_URL_PLACEHOLDER;
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(placeholder) {
        result.push_str(&rest[..index]);
        rest = &rest[index + placeholder.len()..];
        let path_end = rest.find(placeholder).unwrap_or(rest.len());
        let path = &rest[..path_end];
        let resolved = crate::core::config::e2e::resolve_templated_sample_url(template, path, vars)
            .unwrap_or_else(|| format!("{sample_base_url}{path}"));
        result.push_str(&resolved);
        rest = &rest[path_end..];
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The substitution reaches every nesting level of a fixture's input, not just its
    /// top-level string fields: a `$mock_url` left unresolved inside a nested object or a
    /// list is published verbatim, and `$mock_url` is not an address.
    #[test]
    fn the_placeholder_is_replaced_at_every_depth_of_the_input() {
        let fixture = Fixture {
            input: serde_json::json!({
                "url": "$mock_url/report.pdf",
                "batch": ["$mock_url/a", "$mock_url/b"],
                "nested": {"fallback": "$mock_url/c"},
            }),
            ..Fixture::default()
        };

        let docs = fixture.docs_call_fixture_with_sample_base_url("https://samples.example.org");

        assert_eq!(
            docs.input,
            serde_json::json!({
                "url": "https://samples.example.org/report.pdf",
                "batch": ["https://samples.example.org/a", "https://samples.example.org/b"],
                "nested": {"fallback": "https://samples.example.org/c"},
            })
        );
    }

    /// The zero-argument spelling still resolves the placeholder -- to the reserved
    /// documentation domain -- so a call site with no configuration in hand never leaves
    /// `$mock_url` in a rendered body.
    #[test]
    fn the_unconfigured_spelling_binds_the_reserved_documentation_domain() {
        let fixture = Fixture {
            input: serde_json::json!({"url": "$mock_url/report.pdf"}),
            ..Fixture::default()
        };

        let docs = fixture.docs_call_fixture();

        assert_eq!(
            docs.input.get("url").and_then(|value| value.as_str()),
            Some("https://example.com/report.pdf")
        );
    }

    /// The defect this per-fixture resolution exists to fix: a content-addressed sample corpus
    /// cannot be expressed by concatenating a flat base with the fixture's mock path, because
    /// the real address depends on a fact about the object -- here a digest the fixture itself
    /// declares -- not on the path. With a template and a matching `sample_url_vars` entry
    /// configured, the fixture publishes its own resolved address instead of `sample_base_url`.
    #[test]
    fn a_fixture_with_a_matching_sample_url_var_publishes_its_own_templated_address() {
        let fixture = Fixture {
            input: serde_json::json!({"url": "$mock_url/pdf/fake_memo.pdf"}),
            docs: Some(FixtureDocs {
                sample_url_vars: BTreeMap::from([("digest".to_string(), "9f86d081884c7d659a2feaa".to_string())]),
                ..fixture_docs("contract")
            }),
            ..Fixture::default()
        };
        let template = SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/{digest}"))
            .expect("valid template resolves")
            .expect("a configured value produces a template");

        let docs = fixture.docs_call_fixture_with_sample_url("https://samples.example.org", Some(&template));

        assert_eq!(
            docs.input.get("url").and_then(|value| value.as_str()),
            Some("https://cdn.example.org/objects/9f86d081884c7d659a2feaa")
        );
    }

    /// The regression guard: a fixture with no per-fixture resolution configured keeps
    /// resolving through `sample_base_url` exactly as it always has, byte for byte -- a
    /// template being available in general must never change behavior for a fixture that
    /// declares no facts for it.
    #[test]
    fn a_fixture_with_no_sample_url_vars_falls_back_to_sample_base_url_unchanged() {
        let fixture = Fixture {
            input: serde_json::json!({"url": "$mock_url/pdf/fake_memo.pdf"}),
            docs: Some(fixture_docs("contract")),
            ..Fixture::default()
        };
        let template = SampleUrlTemplate::resolve(Some("https://cdn.example.org/objects/{digest}"))
            .expect("valid template resolves")
            .expect("a configured value produces a template");

        let docs = fixture.docs_call_fixture_with_sample_url("https://samples.example.org", Some(&template));

        assert_eq!(
            docs.input.get("url").and_then(|value| value.as_str()),
            Some("https://samples.example.org/pdf/fake_memo.pdf"),
            "a fixture that cannot satisfy the template's placeholders must keep publishing the \
             flat sample_base_url address, which is what keeps the reserved-domain warning honest"
        );
    }

    fn fixture_docs(topic: &str) -> FixtureDocs {
        FixtureDocs {
            topic: topic.to_string(),
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
        }
    }
}
