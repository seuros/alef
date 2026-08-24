//! The docs-shaped view of a [`Fixture`]: the clone a documentation snippet renders from.
//!
//! Split out of [`super`] because it answers a different question than the rest of the
//! fixture model. Everything else there describes what the executable e2e suite runs; this
//! describes what a reader is shown -- the `docs.input` / `docs.presentation` overrides, the
//! mock-response wiring that must not appear in prose, and the public sample address a
//! reader's copy-paste actually resolves against.

use super::Fixture;

impl Fixture {
    /// The docs-shaped clone of this fixture, with any `$mock_url` placeholder bound to the
    /// reserved documentation domain.
    ///
    /// Every caller that holds the project's snippet configuration should call
    /// [`Fixture::docs_call_fixture_with_sample_base_url`] instead, so the address a reader
    /// sees is the one the project publishes its samples at. This spelling exists for the
    /// call sites that have no configuration in hand -- chiefly the per-backend renderers,
    /// which re-derive the docs clone from a fixture `crate::e2e::snippets`' own
    /// `render_snippet_body` already resolved, where the placeholder is long gone and this
    /// pass has nothing left to replace.
    pub fn docs_call_fixture(&self) -> Self {
        self.docs_call_fixture_with_sample_base_url(crate::core::config::e2e::DEFAULT_DOCS_SAMPLE_BASE_URL)
    }

    /// [`Fixture::docs_call_fixture`], binding `$mock_url` to `sample_base_url` -- the
    /// project's own public sample host -- rather than the reserved-domain placeholder.
    pub fn docs_call_fixture_with_sample_base_url(&self, sample_base_url: &str) -> Self {
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
        replace_docs_mock_urls(&mut fixture.input, sample_base_url);
        fixture
    }
}

fn replace_docs_mock_urls(value: &mut serde_json::Value, sample_base_url: &str) {
    match value {
        serde_json::Value::String(text) => {
            *text = text.replace(crate::e2e::codegen::MOCK_URL_PLACEHOLDER, sample_base_url);
        }
        serde_json::Value::Array(values) => {
            for value in values {
                replace_docs_mock_urls(value, sample_base_url);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                replace_docs_mock_urls(value, sample_base_url);
            }
        }
        _ => {}
    }
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
}
