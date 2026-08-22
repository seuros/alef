//! Fixture-validation checks for `preserve_input_urls` consistency.
//!
//! A `mock_url` / `mock_url_list` argument binds the per-fixture mock server address and
//! ignores `input` entirely unless the fixture sets `preserve_input_urls`
//! (`crate::e2e::codegen::preserved_url_literal` and `preserved_url_list` are the seam:
//! both return `None` outright when the flag is unset, and every backend falls through to
//! the mock address on `None`). The flag and the fixture's declared input therefore have
//! to agree, and neither serde nor the JSON schema can tell when they do not -- the
//! mismatch is only visible once the call's argument list is resolved, which is here.

use crate::core::config::e2e::{ArgMapping, CallConfig};
use crate::e2e::fixture::Fixture;
use crate::e2e::validate::{Severity, ValidationError};

/// Report both directions in which a fixture's `preserve_input_urls` flag disagrees with
/// the `mock_url` / `mock_url_list` arguments the resolved call declares.
pub(super) fn check_preserve_input_urls(
    fixture: &Fixture,
    call_config: &CallConfig,
    errors: &mut Vec<ValidationError>,
) {
    let url_args: Vec<&ArgMapping> = fixture
        .resolved_args(call_config)
        .iter()
        .filter(|arg| matches!(arg.arg_type.as_str(), "mock_url" | "mock_url_list"))
        .collect();

    // ~keep An inert flag is the exact failure this option exists to prevent: the author
    // believes the fixture's own address reaches the call, while every backend keeps
    // substituting the mock server and the test quietly proves something else. Neither
    // serde nor the JSON schema rejects an unknown fixture key, so a typo elsewhere in the
    // fixture surfaces here or nowhere.
    if fixture.preserve_input_urls {
        if url_args.is_empty() {
            errors.push(ValidationError {
                file: fixture.source.clone(),
                message: format!(
                    "fixture '{}' sets preserve_input_urls but call '{}' has no mock_url or \
                     mock_url_list argument, so the flag has no effect",
                    fixture.id,
                    fixture.call.as_deref().unwrap_or("<default>")
                ),
                severity: Severity::Error,
            });
        }
        return;
    }

    for arg in url_args {
        let Some(literal) = discarded_literal(&fixture.input, arg) else {
            continue;
        };
        errors.push(ValidationError {
            file: fixture.source.clone(),
            message: format!(
                "fixture '{}' declares the absolute URL '{}' for {} argument '{}' but does not set \
                 preserve_input_urls, so the literal is discarded and the mock server address is \
                 bound instead -- set preserve_input_urls = true if the URL is meaningful to the \
                 test, or declare a mock-server-relative path instead",
                fixture.id, literal, arg.arg_type, arg.name
            ),
            severity: Severity::Error,
        });
    }
}

/// The first declared URL literal this argument would lose to the mock server address, if
/// any.
///
/// ~keep Only a value carrying a `scheme://` counts. A bare path (`"/seed1"`) has no
/// address of its own and is *meant* to be resolved against whichever server base the
/// backend binds, so substituting the mock server is the correct reading of it, not a
/// discard. The same scheme test draws the same line in
/// `crate::e2e::snippets::mock_url_defaults`, which rewrites bare paths for docs but
/// leaves scheme-carrying literals to their author.
fn discarded_literal<'a>(input: &'a serde_json::Value, arg: &ArgMapping) -> Option<&'a str> {
    if arg.arg_type == "mock_url_list" {
        return crate::e2e::codegen::resolve_urls_field(input, &arg.field)
            .as_array()?
            .iter()
            .filter_map(serde_json::Value::as_str)
            .find(|element| has_url_scheme(element));
    }
    crate::e2e::codegen::resolve_field(input, &arg.field)
        .as_str()
        .filter(|value| has_url_scheme(value))
}

/// Whether `value` already names an explicit scheme rather than a bare path meant to be
/// resolved against a server base.
fn has_url_scheme(value: &str) -> bool {
    value.contains("://")
}

#[cfg(test)]
mod tests {
    use crate::core::config::e2e::CallConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::Fixture;
    use crate::e2e::validate::{Severity, validate_fixtures_semantic};

    fn make_e2e_config() -> E2eConfig {
        E2eConfig::default()
    }

    /// A fixture declaring `input` for a single `mock_url` / `mock_url_list` argument.
    fn url_fixture(arg_type: &str, field: &str, input: serde_json::Value) -> Fixture {
        Fixture {
            id: "literal_url".to_string(),
            source: "literal_url.json".to_string(),
            input,
            args: vec![
                serde_json::from_value(serde_json::json!({
                    "name": "url",
                    "field": format!("input.{field}"),
                    "type": arg_type,
                }))
                .expect("argument mapping should deserialize"),
            ],
            ..Fixture::default()
        }
    }

    fn discard_errors(fixture: Fixture) -> Vec<String> {
        validate_fixtures_semantic(&[fixture], &make_e2e_config(), &["rust".to_string()])
            .into_iter()
            .filter(|error| error.severity == Severity::Error && error.message.contains("preserve_input_urls"))
            .map(|error| error.message)
            .collect()
    }

    #[test]
    fn scheme_carrying_scalar_url_without_preserve_is_an_error() {
        let fixture = url_fixture(
            "mock_url",
            "url",
            serde_json::json!({"url": "https://target.example/a"}),
        );

        let messages = discard_errors(fixture);

        assert_eq!(
            messages.len(),
            1,
            "a declared absolute URL that codegen discards must be reported exactly once: {messages:?}"
        );
        assert!(
            messages[0].contains("https://target.example/a"),
            "the diagnostic must name the discarded literal: {messages:?}"
        );
    }

    #[test]
    fn scheme_carrying_url_list_without_preserve_is_an_error() {
        let fixture = url_fixture(
            "mock_url_list",
            "urls",
            serde_json::json!({"urls": ["https://target.example/a"]}),
        );

        let messages = discard_errors(fixture);

        assert_eq!(
            messages.len(),
            1,
            "a declared absolute URL list that codegen discards must be reported: {messages:?}"
        );
    }

    #[test]
    fn a_mock_server_relative_path_is_not_flagged() {
        let fixture = url_fixture("mock_url", "url", serde_json::json!({"url": "/seed1"}));

        assert!(
            discard_errors(fixture).is_empty(),
            "a bare path has no address of its own and is meant to resolve against the mock server"
        );
    }

    #[test]
    fn a_mock_url_placeholder_is_not_flagged() {
        let fixture = url_fixture("mock_url", "url", serde_json::json!({"url": "$mock_url"}));

        assert!(
            discard_errors(fixture).is_empty(),
            "the $mock_url placeholder asks for the mock server address by name"
        );
    }

    #[test]
    fn setting_preserve_input_urls_silences_the_check() {
        let mut fixture = url_fixture(
            "mock_url",
            "url",
            serde_json::json!({"url": "https://target.example/a"}),
        );
        fixture.preserve_input_urls = true;

        assert!(
            discard_errors(fixture).is_empty(),
            "opting in is exactly the fix the diagnostic asks for"
        );
    }

    #[test]
    fn a_call_with_no_url_argument_is_not_flagged() {
        let fixture = url_fixture(
            "string",
            "text",
            serde_json::json!({"text": "https://target.example/a"}),
        );

        assert!(
            discard_errors(fixture).is_empty(),
            "a non-URL argument is passed through verbatim; nothing is discarded"
        );
    }

    #[test]
    fn preserve_input_urls_requires_mock_url_argument() {
        let mut fixture = url_fixture("string", "text", serde_json::json!({"text": "sample"}));
        fixture.preserve_input_urls = true;

        let errors = validate_fixtures_semantic(&[fixture], &make_e2e_config(), &["rust".to_string()]);

        assert!(errors.iter().any(|error| error.message.contains("flag has no effect")));
    }

    #[test]
    fn preserve_input_urls_accepts_scalar_and_list_url_arguments() {
        for arg_type in ["mock_url", "mock_url_list"] {
            let mut fixture = url_fixture(arg_type, "urls", serde_json::json!({}));
            fixture.preserve_input_urls = true;

            let errors = validate_fixtures_semantic(&[fixture], &make_e2e_config(), &["rust".to_string()]);

            assert!(!errors.iter().any(|error| error.message.contains("flag has no effect")));
        }
    }

    #[test]
    fn the_default_call_is_used_when_the_fixture_names_none() {
        let mut fixture = url_fixture(
            "mock_url",
            "url",
            serde_json::json!({"url": "https://target.example/a"}),
        );
        fixture.call = None;
        let config = E2eConfig {
            call: CallConfig::default(),
            ..E2eConfig::default()
        };

        let errors: Vec<_> = validate_fixtures_semantic(&[fixture], &config, &["rust".to_string()])
            .into_iter()
            .filter(|error| error.message.contains("preserve_input_urls"))
            .collect();

        assert_eq!(errors.len(), 1, "fixture-level args apply to the default call too");
    }
}
