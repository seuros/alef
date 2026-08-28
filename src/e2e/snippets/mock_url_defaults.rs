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
//! default rather than the exception. Which address that is belongs to the project: it
//! comes from `[crates.e2e.snippets].sample_base_url` (see
//! [`crate::core::config::e2e::DocsSampleBaseUrl`]), so a project whose sample inputs are
//! really published somewhere gets snippets a reader can run, rather than a sample domain
//! of alef's choosing.
//!
//! A declared value gets the same treatment when it is mock-server *shorthand* rather
//! than a meaningful address: a bare path like `"/seed1"` in a `batch_urls` list has no
//! scheme or host of its own -- every backend's shared arg-binding code (see
//! `e2e/codegen/*/args.rs`, `*/setup.rs`) resolves it against *some* server base, the
//! live mock server for the executable suite and, once this module rewrites it, the
//! configured sample base URL for docs. A value that already carries a
//! scheme (`"http://127.0.0.1:9/"`, `"gopher://invalid.example.com/"`) is left alone:
//! the fixture author chose that specific address on purpose -- most often the literal
//! under test in an SSRF or validation fixture -- and rewriting it would silently
//! change what the fixture demonstrates.
//!
//! Must run only on the already docs-transformed fixture [`Fixture::docs_call_fixture`]
//! returns, never on the fixture the executable suite renders from: setting
//! `preserve_input_urls` here is what stops the snippet body from falling back to the
//! mock server, and doing the same on the executable fixture would point that suite at
//! an address with no server behind it.

use crate::core::config::e2e::sample_url::has_url_scheme;
use crate::core::config::e2e::{
    CallConfig, DocsSampleBaseUrl, SampleUrlManifest, SampleUrlTemplate, merge_manifest_vars,
    resolve_templated_sample_url,
};
use crate::e2e::fixture::Fixture;
use std::collections::BTreeMap;

/// Resolve one bare, mock-relative `path` (or `""` for a fixture that declares none at all) to
/// the address a documentation snippet publishes for it.
///
/// Tries per-fixture template resolution first (see
/// `crate::core::config::e2e::resolve_templated_sample_url`); falls back to
/// `sample_base_url.join(path)` -- unchanged from before per-fixture templates existed -- the
/// moment the template is unconfigured or this fixture's `vars` do not cover what it
/// references. The single seam this module resolves every mock URL literal through, so its two
/// call sites below (`scalar_declared_value` / `list_declared_value` and the undeclared-field
/// default) cannot independently drift on what "resolved" means. ~keep
fn resolve_relative_url(
    path: &str,
    sample_base_url: DocsSampleBaseUrl<'_>,
    template: Option<&SampleUrlTemplate>,
    vars: &BTreeMap<String, String>,
) -> String {
    resolve_templated_sample_url(template, path, vars).unwrap_or_else(|| sample_base_url.join(path))
}

/// Where a `mock_url` / `mock_url_list` argument's declared value stands relative to
/// this module's zero-edit default.
enum DeclaredValue {
    /// `input` has no value for this field (nor, for a list field, any of
    /// [`crate::e2e::codegen::resolve_urls_field`]'s aliases): the zero-edit default
    /// literal applies outright.
    Undeclared,
    /// `input` declares a value with no `scheme://`, meaning every backend already
    /// resolves it against a server base rather than treating it as self-contained.
    /// Carries the docs-ready rewrite (each bare element resolved against the configured
    /// sample base URL) so the caller only has to write it back.
    HarnessRelative(serde_json::Value),
    /// `input` declares a value that already names a scheme: a deliberate literal the
    /// fixture is testing, left untouched.
    AlreadyMeaningful,
}

fn scalar_declared_value(
    input: &serde_json::Value,
    field: &str,
    sample_base_url: DocsSampleBaseUrl<'_>,
    template: Option<&SampleUrlTemplate>,
    vars: &BTreeMap<String, String>,
) -> DeclaredValue {
    let Some(value) = crate::e2e::codegen::resolve_field(input, field).as_str() else {
        return DeclaredValue::Undeclared;
    };
    if has_url_scheme(value) {
        DeclaredValue::AlreadyMeaningful
    } else {
        DeclaredValue::HarnessRelative(serde_json::Value::String(resolve_relative_url(
            value,
            sample_base_url,
            template,
            vars,
        )))
    }
}

fn list_declared_value(
    input: &serde_json::Value,
    field: &str,
    sample_base_url: DocsSampleBaseUrl<'_>,
    template: Option<&SampleUrlTemplate>,
    vars: &BTreeMap<String, String>,
) -> DeclaredValue {
    let Some(elements) = crate::e2e::codegen::resolve_urls_field(input, field).as_array() else {
        return DeclaredValue::Undeclared;
    };
    // A non-string element is a shape this module does not understand; leave it for the
    // call's own type validation rather than guessing at a rewrite.
    let Some(urls) = elements
        .iter()
        .map(serde_json::Value::as_str)
        .collect::<Option<Vec<_>>>()
    else {
        return DeclaredValue::AlreadyMeaningful;
    };
    // An empty list (e.g. `"batch_urls": []` testing the empty-input error path) has no
    // scheme to check and nothing for `all()` to be vacuously true or false about; treat
    // it as a trivial rewrite so the fixture is marked preserved with its own value
    // unchanged, rather than falling into `AlreadyMeaningful` and leaving
    // `preserve_input_urls` unset -- every backend's non-preserved branch still emits an
    // unconditional `MOCK_SERVER_URL`-bearing base line even when the path list it feeds
    // is empty.
    if !urls.is_empty() && urls.iter().all(|url| has_url_scheme(url)) {
        return DeclaredValue::AlreadyMeaningful;
    }
    let rewritten = urls
        .into_iter()
        .map(|url| {
            if has_url_scheme(url) {
                serde_json::Value::String(url.to_string())
            } else {
                serde_json::Value::String(resolve_relative_url(url, sample_base_url, template, vars))
            }
        })
        .collect();
    DeclaredValue::HarnessRelative(serde_json::Value::Array(rewritten))
}

/// Inject `sample_base_url` for every `mock_url` / `mock_url_list` argument
/// `call` declares that this fixture's `input` leaves undeclared or declares only as
/// mock-server shorthand (a bare path with no scheme), then mark the fixture to bind
/// the literal instead of the mock server. A fixture that declares a value with an
/// explicit scheme (e.g. an SSRF fixture's literal loopback address, or an explicit
/// `"$mock_url"` placeholder) is left untouched; if that fixture has not also set
/// `preserve_input_urls`, the literal is silently discarded in favor of the mock server
/// address in both the executable suite and this snippet, so that case is logged loudly
/// rather than passed over in silence.
pub(super) fn with_default_mock_url_literals(
    mut fixture: Fixture,
    call: &CallConfig,
    sample_base_url: DocsSampleBaseUrl<'_>,
    template: Option<&SampleUrlTemplate>,
    manifest: Option<&SampleUrlManifest>,
) -> Fixture {
    let candidates: Vec<(String, bool)> = fixture
        .resolved_args(call)
        .iter()
        .filter_map(|arg| match arg.arg_type.as_str() {
            "mock_url" => Some((arg.field.clone(), false)),
            "mock_url_list" => Some((arg.field.clone(), true)),
            _ => None,
        })
        .collect();

    let empty_vars = BTreeMap::new();
    let fixture_vars = fixture.docs.as_ref().map_or(&empty_vars, |docs| &docs.sample_url_vars);
    let body_file = fixture.docs.as_ref().and_then(|docs| docs.body_file.as_deref());
    let vars = merge_manifest_vars(manifest, body_file, fixture_vars);
    let mut injected_any = false;
    for (field, is_list) in candidates {
        let declared = if is_list {
            list_declared_value(&fixture.input, &field, sample_base_url, template, &vars)
        } else {
            scalar_declared_value(&fixture.input, &field, sample_base_url, template, &vars)
        };
        let value = match declared {
            DeclaredValue::Undeclared => {
                if is_list {
                    serde_json::json!([resolve_relative_url("", sample_base_url, template, &vars)])
                } else {
                    serde_json::Value::String(resolve_relative_url("", sample_base_url, template, &vars))
                }
            }
            DeclaredValue::HarnessRelative(rewritten) => rewritten,
            DeclaredValue::AlreadyMeaningful => {
                if !fixture.preserve_input_urls {
                    tracing::warn!(
                        target: "alef::e2e::snippets::mock_url_defaults",
                        fixture = %fixture.id,
                        field = %field,
                        "fixture declares an absolute URL for a mock_url/mock_url_list argument \
                         without setting preserve_input_urls; the literal is discarded and the \
                         mock-server address is bound instead, in both the executable e2e suite \
                         and this fixture's documentation snippet -- set preserve_input_urls = \
                         true if the URL is meaningful to the test, or remove the declared value \
                         to let the zero-edit default stand in"
                    );
                }
                continue;
            }
        };
        if set_input_field(&mut fixture.input, &field, value) {
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
#[path = "mock_url_defaults_tests.rs"]
mod tests;
