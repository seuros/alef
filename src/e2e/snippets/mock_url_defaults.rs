//! Zero-edit default addresses for a documentation snippet's `mock_url` arguments.
//!
//! `mock_url` / `mock_url_list` call arguments normally ignore `input.url` entirely and
//! bind the per-fixture mock server address (see `Fixture::preserve_input_urls`'s doc
//! comment) -- correct for the executable e2e suite, which needs a live server to talk
//! to. A documentation snippet has no such server behind it, so the same binding leaks
//! `MOCK_SERVER_URL` / `MOCK_SERVER_<ID>` wiring into published prose, which
//! `reject_mock_harness_scaffolding` (in `super`) refuses outright.
//!
//! A URL-centric consumer's fixtures declare almost nothing about the url itself --
//! that is the whole point of the default binding -- so requiring every such fixture to
//! add an explicit `"url": "$mock_url"` just to keep its snippet publishable does not
//! scale. This module injects an illustrative literal for any `mock_url` /
//! `mock_url_list` argument the fixture leaves undeclared, so the zero-edit case is the
//! default rather than the exception.
//!
//! Must run only on the already docs-transformed fixture [`Fixture::docs_call_fixture`]
//! returns, never on the fixture the executable suite renders from: setting
//! `preserve_input_urls` here is what stops the snippet body from falling back to the
//! mock server, and doing the same on the executable fixture would point that suite at
//! an address with no server behind it.

use crate::core::config::e2e::CallConfig;
use crate::e2e::fixture::Fixture;

/// The address alef injects for a `mock_url` / `mock_url_list` argument that has no
/// declared value, so a documentation snippet reads like real client code rather than
/// harness wiring.
const DEFAULT_DOCS_MOCK_URL: &str = "https://example.com";

/// Inject [`DEFAULT_DOCS_MOCK_URL`] for every `mock_url` / `mock_url_list` argument
/// `call` declares that this fixture's `input` leaves unset, then mark the fixture to
/// bind the literal instead of the mock server. A fixture that already declares a value
/// (e.g. an explicit `"$mock_url"` placeholder) is left untouched.
pub(super) fn with_default_mock_url_literals(mut fixture: Fixture, call: &CallConfig) -> Fixture {
    let candidates: Vec<(String, bool)> = fixture
        .resolved_args(call)
        .iter()
        .filter_map(|arg| match arg.arg_type.as_str() {
            "mock_url" => Some((arg.field.clone(), false)),
            "mock_url_list" => Some((arg.field.clone(), true)),
            _ => None,
        })
        .collect();

    let mut injected_any = false;
    for (field, is_list) in candidates {
        let already_declared = if is_list {
            crate::e2e::codegen::resolve_urls_field(&fixture.input, &field)
                .as_array()
                .is_some()
        } else {
            crate::e2e::codegen::resolve_field(&fixture.input, &field)
                .as_str()
                .is_some()
        };
        if already_declared {
            continue;
        }
        let default = if is_list {
            serde_json::json!([DEFAULT_DOCS_MOCK_URL])
        } else {
            serde_json::Value::String(DEFAULT_DOCS_MOCK_URL.to_string())
        };
        if set_input_field(&mut fixture.input, &field, default) {
            injected_any = true;
        }
    }

    if injected_any {
        fixture.preserve_input_urls = true;
    }
    fixture
}

/// Write `value` at `field_path` inside a fixture's `input` object, creating
/// intermediate objects as needed. Mirrors the path convention
/// `crate::e2e::codegen::resolve_field` reads: a leading `"input."` prefix is
/// stripped, and the remainder is `.`-segmented.
///
/// Returns `false` without writing when `field_path` is empty or exactly `"input"` (no
/// single field to target) or when an intermediate segment already holds a non-object
/// value -- callers only reach this to add a value that is currently absent, never to
/// clobber unrelated data.
fn set_input_field(input: &mut serde_json::Value, field_path: &str, value: serde_json::Value) -> bool {
    let path = field_path.strip_prefix("input.").unwrap_or(field_path);
    if path.is_empty() || path == "input" {
        return false;
    }
    let mut segments = path.split('.').peekable();
    let mut current = input;
    while let Some(segment) = segments.next() {
        if !current.is_object() {
            *current = serde_json::Value::Object(serde_json::Map::new());
        }
        let Some(map) = current.as_object_mut() else {
            return false;
        };
        if segments.peek().is_none() {
            map.insert(segment.to_string(), value);
            return true;
        }
        current = map
            .entry(segment.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::e2e::ArgMapping;

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

        let result = with_default_mock_url_literals(fixture, &call);

        assert_eq!(
            result.input.get("url").and_then(|v| v.as_str()),
            Some(DEFAULT_DOCS_MOCK_URL)
        );
        assert!(
            result.preserve_input_urls,
            "injecting a literal must opt the fixture in"
        );
    }

    #[test]
    fn leaves_an_already_declared_url_untouched() {
        let fixture = Fixture {
            input: serde_json::json!({"url": "http://127.0.0.1:9/"}),
            ..Fixture::default()
        };
        let call = call_with_args(vec![mock_url_arg("url")]);

        let result = with_default_mock_url_literals(fixture, &call);

        assert_eq!(
            result.input.get("url").and_then(|v| v.as_str()),
            Some("http://127.0.0.1:9/")
        );
    }

    #[test]
    fn injects_a_single_element_default_list_for_mock_url_list() {
        let fixture = Fixture {
            input: serde_json::json!({}),
            ..Fixture::default()
        };
        let call = call_with_args(vec![mock_url_list_arg("urls")]);

        let result = with_default_mock_url_literals(fixture, &call);

        assert_eq!(
            result.input.get("urls").and_then(|v| v.as_array()),
            Some(&vec![serde_json::Value::String(DEFAULT_DOCS_MOCK_URL.to_string())])
        );
        assert!(result.preserve_input_urls);
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

        let result = with_default_mock_url_literals(fixture, &call);

        assert!(!result.preserve_input_urls);
        assert!(result.input.get("url").is_none());
    }
}
