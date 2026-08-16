//! Shared resolution of `client_factory` construction arguments.
//!
//! Bindings whose API hangs off a client object are generated through a factory call
//! of the shape `factory(<credential>, <base URL>, <trailing args…>)`. The first two
//! slots are the generator's business — it owns the credential expression and knows
//! whether a mock server is in play — but everything after them is project-specific,
//! so it comes from configuration (`client_factory_trailing_args`) or, for a single
//! documentation snippet, from that fixture's `docs.client`.
//!
//! Every entry point here takes the fixture's documentation client as an explicit
//! `Option`, never reading it off the fixture itself. A renderer shared between the
//! docs path and the executable e2e suite therefore has to name the docs override at
//! its call site, and the e2e call site names `None` — the docs endpoint cannot reach
//! a real test by omission.

use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::FixtureDocsClient;

/// Verbatim argument expressions to emit after the credential and base-URL slots of a
/// `client_factory` call, resolved most-specific source first:
///
/// 1. `docs_client`'s own `args.<language>` — documentation snippets only,
/// 2. `[e2e.calls.<name>.overrides.<language>] client_factory_trailing_args`,
/// 3. `[e2e.call.overrides.<language>] client_factory_trailing_args`,
/// 4. `fallback`.
///
/// `fallback` is the list the caller hardcoded before this hook was wired up. Passing
/// it keeps every project that has not configured the override rendering byte-for-byte
/// what it renders today, so adopting the hook is not a breaking change.
pub fn trailing_args(
    docs_client: Option<&FixtureDocsClient>,
    e2e_config: &E2eConfig,
    call_config: &CallConfig,
    language: &str,
    fallback: &[&str],
) -> Vec<String> {
    if let Some(args) = docs_client.and_then(|client| client.args_for(language)) {
        return args.to_vec();
    }
    call_config
        .overrides
        .get(language)
        .map(|overrides| overrides.client_factory_trailing_args.clone())
        .filter(|args| !args.is_empty())
        .or_else(|| {
            e2e_config
                .call
                .overrides
                .get(language)
                .map(|overrides| overrides.client_factory_trailing_args.clone())
                .filter(|args| !args.is_empty())
        })
        .unwrap_or_else(|| fallback.iter().map(|arg| (*arg).to_string()).collect())
}

/// The base URL a documentation snippet constructs its client with, or `None` when the
/// fixture documents no particular endpoint and the generator should fall back to its
/// usual choice (the mock server, or nothing).
pub fn docs_base_url(docs_client: Option<&FixtureDocsClient>) -> Option<&str> {
    docs_client.and_then(|client| client.base_url.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::config::CallOverride;

    const FALLBACK: [&str; 3] = ["None", "None", "None"];

    fn config_with_trailing_args(args: &[&str]) -> E2eConfig {
        let mut call = CallConfig::default();
        call.overrides.insert(
            "rust".into(),
            CallOverride {
                client_factory_trailing_args: args.iter().map(|arg| (*arg).to_string()).collect(),
                ..CallOverride::default()
            },
        );
        E2eConfig {
            call,
            ..E2eConfig::default()
        }
    }

    fn docs_client() -> FixtureDocsClient {
        FixtureDocsClient {
            base_url: Some("https://llm.internal.example.com/v1".into()),
            args: [("rust".to_string(), vec!["Some(60)".to_string(), "None".to_string()])]
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn unconfigured_projects_keep_the_argument_list_the_generator_hardcoded() {
        let e2e_config = E2eConfig::default();
        assert_eq!(
            trailing_args(None, &e2e_config, &e2e_config.call, "rust", &FALLBACK),
            vec!["None", "None", "None"],
            "an absent override must not shorten the factory's argument list"
        );
    }

    #[test]
    fn file_level_override_replaces_the_fallback() {
        let e2e_config = config_with_trailing_args(&["Some(30)", "Some(5)"]);
        assert_eq!(
            trailing_args(None, &e2e_config, &e2e_config.call, "rust", &FALLBACK),
            vec!["Some(30)", "Some(5)"]
        );
    }

    #[test]
    fn override_is_scoped_to_its_own_language() {
        let e2e_config = config_with_trailing_args(&["Some(30)"]);
        assert_eq!(
            trailing_args(None, &e2e_config, &e2e_config.call, "java", &["null"]),
            vec!["null"],
            "a rust override must not leak into the java factory call"
        );
    }

    #[test]
    fn named_call_override_wins_over_the_file_level_one() {
        let e2e_config = config_with_trailing_args(&["file"]);
        let mut named = CallConfig::default();
        named.overrides.insert(
            "rust".into(),
            CallOverride {
                client_factory_trailing_args: vec!["call".into()],
                ..CallOverride::default()
            },
        );
        assert_eq!(
            trailing_args(None, &e2e_config, &named, "rust", &FALLBACK),
            vec!["call"]
        );
    }

    #[test]
    fn named_call_without_its_own_list_inherits_the_file_level_one() {
        let e2e_config = config_with_trailing_args(&["file"]);
        assert_eq!(
            trailing_args(None, &e2e_config, &CallConfig::default(), "rust", &FALLBACK),
            vec!["file"],
            "a named call that omits the key must not fall through to the fallback"
        );
    }

    #[test]
    fn fixture_docs_client_outranks_every_configured_list() {
        let client = docs_client();
        let e2e_config = config_with_trailing_args(&["file"]);
        assert_eq!(
            trailing_args(Some(&client), &e2e_config, &e2e_config.call, "rust", &FALLBACK),
            vec!["Some(60)", "None"]
        );
        assert_eq!(
            trailing_args(Some(&client), &e2e_config, &e2e_config.call, "java", &["null"]),
            vec!["null"],
            "a language the fixture says nothing about keeps the configured default"
        );
    }

    #[test]
    fn a_caller_that_declares_no_docs_client_reads_no_base_url() {
        assert_eq!(docs_base_url(None), None);
        assert_eq!(
            docs_base_url(Some(&docs_client())),
            Some("https://llm.internal.example.com/v1")
        );
        assert_eq!(
            docs_base_url(Some(&FixtureDocsClient::default())),
            None,
            "a docs client that only overrides trailing args must not invent an endpoint"
        );
    }
}
