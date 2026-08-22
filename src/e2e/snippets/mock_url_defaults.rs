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
//! A declared value gets the same treatment when it is mock-server *shorthand* rather
//! than a meaningful address: a bare path like `"/seed1"` in a `batch_urls` list has no
//! scheme or host of its own -- every backend's shared arg-binding code (see
//! `e2e/codegen/*/args.rs`, `*/setup.rs`) resolves it against *some* server base, the
//! live mock server for the executable suite and, once this module rewrites it, the
//! same [`DEFAULT_DOCS_MOCK_URL`] literal for docs. A value that already carries a
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

use crate::core::config::e2e::CallConfig;
use crate::e2e::fixture::Fixture;

/// The address alef injects for a `mock_url` / `mock_url_list` argument that has no
/// declared value, so a documentation snippet reads like real client code rather than
/// harness wiring.
const DEFAULT_DOCS_MOCK_URL: &str = "https://example.com";

/// Where a `mock_url` / `mock_url_list` argument's declared value stands relative to
/// this module's zero-edit default.
enum DeclaredValue {
    /// `input` has no value for this field (nor, for a list field, any of
    /// [`crate::e2e::codegen::resolve_urls_field`]'s aliases): the zero-edit default
    /// literal applies outright.
    Undeclared,
    /// `input` declares a value with no `scheme://`, meaning every backend already
    /// resolves it against a server base rather than treating it as self-contained.
    /// Carries the docs-ready rewrite (each bare element prefixed with
    /// [`DEFAULT_DOCS_MOCK_URL`]) so the caller only has to write it back.
    HarnessRelative(serde_json::Value),
    /// `input` declares a value that already names a scheme: a deliberate literal the
    /// fixture is testing, left untouched.
    AlreadyMeaningful,
}

/// Whether `value` already names an explicit scheme (`"http://..."`, `"gopher://..."`,
/// ...) rather than a bare path meant to be resolved against a server base.
fn has_url_scheme(value: &str) -> bool {
    value.contains("://")
}

fn scalar_declared_value(input: &serde_json::Value, field: &str) -> DeclaredValue {
    let Some(value) = crate::e2e::codegen::resolve_field(input, field).as_str() else {
        return DeclaredValue::Undeclared;
    };
    if has_url_scheme(value) {
        DeclaredValue::AlreadyMeaningful
    } else {
        DeclaredValue::HarnessRelative(serde_json::Value::String(format!("{DEFAULT_DOCS_MOCK_URL}{value}")))
    }
}

fn list_declared_value(input: &serde_json::Value, field: &str) -> DeclaredValue {
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
                serde_json::Value::String(format!("{DEFAULT_DOCS_MOCK_URL}{url}"))
            }
        })
        .collect();
    DeclaredValue::HarnessRelative(serde_json::Value::Array(rewritten))
}

/// Inject [`DEFAULT_DOCS_MOCK_URL`] for every `mock_url` / `mock_url_list` argument
/// `call` declares that this fixture's `input` leaves undeclared or declares only as
/// mock-server shorthand (a bare path with no scheme), then mark the fixture to bind
/// the literal instead of the mock server. A fixture that declares a value with an
/// explicit scheme (e.g. an SSRF fixture's literal loopback address, or an explicit
/// `"$mock_url"` placeholder) is left untouched; if that fixture has not also set
/// `preserve_input_urls`, the literal is silently discarded in favor of the mock server
/// address in both the executable suite and this snippet, so that case is logged loudly
/// rather than passed over in silence.
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
        let declared = if is_list {
            list_declared_value(&fixture.input, &field)
        } else {
            scalar_declared_value(&fixture.input, &field)
        };
        let value = match declared {
            DeclaredValue::Undeclared => {
                if is_list {
                    serde_json::json!([DEFAULT_DOCS_MOCK_URL])
                } else {
                    serde_json::Value::String(DEFAULT_DOCS_MOCK_URL.to_string())
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
    fn leaves_an_already_declared_absolute_url_untouched() {
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
        assert!(
            !result.preserve_input_urls,
            "a scheme-carrying literal is left for the fixture author to opt in explicitly"
        );
    }

    /// A batch-style fixture that declares a bare path (no scheme) rather than leaving the
    /// field unset -- e.g. `"batch_urls": ["/seed1", "/seed2"]` -- is the shape most of
    /// task #140's residual rejections had: the field passed the module's old
    /// `already_declared` check and so was never given a docs-safe literal.
    #[test]
    fn rewrites_a_declared_relative_scalar_url_to_the_default_base() {
        let fixture = Fixture {
            input: serde_json::json!({"url": "/seed1"}),
            ..Fixture::default()
        };
        let call = call_with_args(vec![mock_url_arg("url")]);

        let result = with_default_mock_url_literals(fixture, &call);

        assert_eq!(
            result.input.get("url").and_then(|v| v.as_str()),
            Some("https://example.com/seed1")
        );
        assert!(result.preserve_input_urls);
    }

    #[test]
    fn rewrites_every_relative_element_of_a_declared_url_list() {
        let fixture = Fixture {
            input: serde_json::json!({"urls": ["/seed1", "/seed2"]}),
            ..Fixture::default()
        };
        let call = call_with_args(vec![mock_url_list_arg("urls")]);

        let result = with_default_mock_url_literals(fixture, &call);

        assert_eq!(
            result.input.get("urls").and_then(|v| v.as_array()),
            Some(&vec![
                serde_json::Value::String("https://example.com/seed1".to_string()),
                serde_json::Value::String("https://example.com/seed2".to_string()),
            ])
        );
        assert!(result.preserve_input_urls);
    }

    /// A mixed list keeps any already-scheme-carrying element verbatim and only rewrites
    /// the bare ones -- a list is not forced into the all-or-nothing "already meaningful"
    /// bucket just because one of its entries happens to have a scheme.
    #[test]
    fn a_mixed_url_list_only_rewrites_its_relative_elements() {
        let fixture = Fixture {
            input: serde_json::json!({"urls": ["/seed1", "http://127.0.0.1:9/seed2"]}),
            ..Fixture::default()
        };
        let call = call_with_args(vec![mock_url_list_arg("urls")]);

        let result = with_default_mock_url_literals(fixture, &call);

        assert_eq!(
            result.input.get("urls").and_then(|v| v.as_array()),
            Some(&vec![
                serde_json::Value::String("https://example.com/seed1".to_string()),
                serde_json::Value::String("http://127.0.0.1:9/seed2".to_string()),
            ])
        );
        assert!(result.preserve_input_urls);
    }

    #[test]
    fn leaves_an_already_declared_absolute_url_list_untouched() {
        let fixture = Fixture {
            input: serde_json::json!({"urls": ["http://127.0.0.1:9/a", "gopher://127.0.0.1:9/b"]}),
            ..Fixture::default()
        };
        let call = call_with_args(vec![mock_url_list_arg("urls")]);

        let result = with_default_mock_url_literals(fixture, &call);

        assert_eq!(
            result.input.get("urls").and_then(|v| v.as_array()),
            Some(&vec![
                serde_json::Value::String("http://127.0.0.1:9/a".to_string()),
                serde_json::Value::String("gopher://127.0.0.1:9/b".to_string()),
            ])
        );
        assert!(!result.preserve_input_urls);
    }

    /// `"batch_urls": []` (e.g. `batch_scrape_empty_urls_error.json`, testing the
    /// empty-input error path) must be marked preserved even though it declares no
    /// scheme at all: `all()` over an empty iterator is vacuously true, so without this
    /// case an empty list would be classified `AlreadyMeaningful` and left with
    /// `preserve_input_urls` unset, and every backend's non-preserved `mock_url_list`
    /// branch still emits an unconditional `MOCK_SERVER_URL`-bearing base line even when
    /// the path list it feeds is empty.
    #[test]
    fn a_declared_empty_url_list_is_marked_preserved() {
        let fixture = Fixture {
            input: serde_json::json!({"urls": []}),
            ..Fixture::default()
        };
        let call = call_with_args(vec![mock_url_list_arg("urls")]);

        let result = with_default_mock_url_literals(fixture, &call);

        assert_eq!(result.input.get("urls").and_then(|v| v.as_array()), Some(&vec![]));
        assert!(result.preserve_input_urls);
    }

    /// The counterpart bug task #140 also asked to check: a fixture that declares a
    /// meaningful literal but forgets `preserve_input_urls` has that literal silently
    /// discarded at codegen time (see `preserved_url_literal`/`preserved_url_list` in
    /// `e2e::codegen`) -- both the executable suite and the docs snippet bind the mock
    /// server instead. This module cannot fix that discard (the bind happens downstream,
    /// per-backend), but it is the one seam that already inspects every mock_url/
    /// mock_url_list argument's declared value, so it is where the loud signal belongs.
    #[test]
    #[tracing_test::traced_test]
    fn warns_when_a_meaningful_url_is_declared_without_preserve_input_urls() {
        let fixture = Fixture {
            id: "validation_probe".into(),
            input: serde_json::json!({"url": "http://127.0.0.1:9/"}),
            ..Fixture::default()
        };
        let call = call_with_args(vec![mock_url_arg("url")]);

        with_default_mock_url_literals(fixture, &call);

        assert!(
            logs_contain("without setting preserve_input_urls"),
            "expected a warning naming the missing preserve_input_urls opt-in"
        );
    }

    #[test]
    #[tracing_test::traced_test]
    fn does_not_warn_once_preserve_input_urls_is_set() {
        let fixture = Fixture {
            id: "validation_probe".into(),
            preserve_input_urls: true,
            input: serde_json::json!({"url": "http://127.0.0.1:9/"}),
            ..Fixture::default()
        };
        let call = call_with_args(vec![mock_url_arg("url")]);

        with_default_mock_url_literals(fixture, &call);

        assert!(
            !logs_contain("without setting preserve_input_urls"),
            "a fixture that already opted in must not be flagged"
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

    /// A single seam pinned end to end: a batch-style fixture declaring bare `batch_urls`
    /// paths (the shape task #140 traced most residual rejections to) must clear BOTH real
    /// gates the production pipeline runs it through -- this module's rewrite AND
    /// [`crate::e2e::snippets::mock_harness_guard::reject_mock_harness_scaffolding`] -- by
    /// actually rendering a Python documentation snippet body, not just inspecting the
    /// intermediate `Fixture` this module returns. Driving both real functions is what would
    /// have caught the sibling defect this task also found: the Python `mock_url_list`
    /// arg-binding code (`e2e::codegen::python::test_function::args`) used to push its
    /// `{var}_base = os.environ['MOCK_SERVER_URL'] + ...` line unconditionally, before
    /// checking whether the fixture was preserved, so even a fully rewritten and preserved
    /// list still leaked into the snippet body and failed the guard.
    #[test]
    fn a_declared_relative_batch_url_list_survives_the_full_snippet_pipeline() {
        use crate::core::config::NewAlefConfig;
        use crate::e2e::codegen::E2eCodegen;
        use crate::e2e::codegen::python::PythonE2eCodegen;

        let cfg_str = r#"
[workspace]
languages = ["python"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "batch_scrape"
module = "example_api"
args = [{ name = "urls", field = "batch_urls", type = "mock_url_list" }]
"#;
        let cfg: NewAlefConfig = toml::from_str(cfg_str).expect("config parses");
        let e2e = cfg.crates[0].e2e.clone().expect("e2e config");
        let resolved = cfg.resolve().expect("config resolves").remove(0);

        let fixture = Fixture {
            id: "batch_crawl_basic".into(),
            input: serde_json::json!({"batch_urls": ["/seed1", "/seed2"]}),
            ..Fixture::default()
        };

        // Mirrors `snippets::render_snippet_body`'s own sequencing exactly: transform for
        // docs first, THEN apply this module's defaults to the docs-transformed clone.
        let docs_fixture = fixture.docs_call_fixture();
        let call = e2e.resolve_call_for_fixture(
            docs_fixture.call.as_deref(),
            &docs_fixture.id,
            &docs_fixture.resolved_category(),
            &docs_fixture.tags,
            &docs_fixture.input,
        );
        let docs_fixture = with_default_mock_url_literals(docs_fixture, call);

        assert_eq!(
            docs_fixture.input.get("batch_urls").and_then(|v| v.as_array()),
            Some(&vec![
                serde_json::Value::String("https://example.com/seed1".to_string()),
                serde_json::Value::String("https://example.com/seed2".to_string()),
            ]),
            "the module must rewrite the declared relative paths before codegen ever sees them"
        );
        assert!(docs_fixture.preserve_input_urls);

        let body = PythonE2eCodegen
            .render_snippet_body(&docs_fixture, &e2e, &resolved, &[], &[])
            .expect("python snippet body renders");

        assert!(
            !body.contains("MOCK_SERVER"),
            "rendered snippet body must carry no mock-harness wiring:\n{body}"
        );
        crate::e2e::snippets::mock_harness_guard::reject_mock_harness_scaffolding(&body, &docs_fixture, "python")
            .expect("the harness-leak guard must accept a fully preserved, rewritten snippet body");
    }
}
