//! Regression coverage for the C# docs-snippet renderer's streaming-adapter blindness.
//!
//! Split out of `snippet.rs`'s own `#[cfg(test)] mod tests`, which was already close to the
//! 1,000-line cap (`file-modularization`).
//!
//! Before this fix, `render_snippet_body_with_ir` always called `build_args_and_setup` with
//! `adapter_request_type: None` -- a literal, hardcoded `None` -- so a fixture calling a
//! streaming adapter whose C# facade takes a typed request record (`StreamEventsAsync(ulong
//! engine, SampleStreamRequest req)`) instead rendered the flat scalar-arg shape
//! (`Facade.StreamEventsAsync(engine, url)`) every plain call takes. `csc` rejects that:
//! `string` is not assignable to a request record (CS1503). `csharp/streaming.rs` (the
//! generated e2e test path) already resolved this correctly via `config.adapters`; these tests
//! pin that the snippet path now asks the same seam instead of carrying its own, silently
//! stale answer. The `mock_url_list` (batch) half of this also used to be a *separate*
//! post-processing step that existed only in `streaming.rs` -- folding it into
//! `build_args_and_setup` itself (see `setup.rs`) is what lets this path pick it up at all. ~keep

use super::*;
use crate::core::config::ResolvedCrateConfig;
use crate::core::config::e2e::StreamingConfig;
use crate::core::config::extras::{AdapterConfig, AdapterPattern};
use crate::e2e::config::{CallConfig, CallOverride, E2eConfig};
use crate::e2e::fixture::Fixture;

fn line_containing<'a>(body: &'a str, needle: &str) -> &'a str {
    body.lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle} in:\n{body}"))
}

fn engine_handle_arg() -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: "engine".into(),
        field: "input.engine".into(),
        arg_type: "handle".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn url_arg() -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: "url".into(),
        field: "input.url".into(),
        arg_type: "mock_url".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn urls_arg() -> crate::e2e::config::ArgMapping {
    crate::e2e::config::ArgMapping {
        name: "urls".into(),
        field: "input.urls".into(),
        arg_type: "mock_url_list".into(),
        optional: false,
        owned: false,
        element_type: None,
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn streaming_adapter(name: &str, request_type: &str) -> AdapterConfig {
    AdapterConfig {
        name: name.to_string(),
        pattern: AdapterPattern::Streaming,
        core_path: format!("sample::{name}"),
        params: Vec::new(),
        returns: None,
        error_type: None,
        owner_type: Some("SampleEngine".to_string()),
        item_type: Some("sample::SampleEvent".to_string()),
        gil_release: false,
        trait_name: None,
        trait_method: None,
        detect_async: false,
        request_type: Some(request_type.to_string()),
        skip_languages: Vec::new(),
    }
}

/// The defect this fix closes: a single-item streaming call whose facade takes a typed
/// request record. Without the fix the call rendered `SampleConverter.StreamEventsAsync(engine,
/// url)` -- `string` is not assignable to `SampleStreamRequest`, CS1503 -- instead of the real
/// `SampleConverter.StreamEventsAsync(engine, urlReq)` the C# backend actually generates.
#[test]
fn a_single_streaming_call_passes_the_typed_request_not_the_flat_url() {
    let fixture = Fixture {
        id: "stream_a_document".into(),
        description: "Stream a document".into(),
        input: serde_json::json!({"engine": {}, "url": "https://example.com/doc"}),
        preserve_input_urls: true,
        ..Fixture::default()
    };
    let mut call = CallConfig {
        function: "stream_events".into(),
        result_var: "events".into(),
        r#async: true,
        streaming: Some(StreamingConfig::Enabled(true)),
        args: vec![engine_handle_arg(), url_arg()],
        ..CallConfig::default()
    };
    call.overrides.insert("csharp".into(), CallOverride::default());
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        adapters: vec![streaming_adapter("stream_events", "sample::SampleStreamRequest")],
        ..ResolvedCrateConfig::default()
    };
    let body = render_snippet_body(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &config,
        &[],
        &[],
    )
    .expect("snippet renders");

    assert_eq!(
        line_containing(&body, "var url ="),
        "var url = \"https://example.com/doc\";"
    );
    assert_eq!(
        line_containing(&body, "urlReq ="),
        "var urlReq = new SampleStreamRequest { Url = url };"
    );
    assert_eq!(
        line_containing(&body, "StreamEventsAsync("),
        "await foreach (var chunk in SampleConverter.StreamEventsAsync(engine, urlReq))"
    );
    assert!(
        !body.contains("StreamEventsAsync(engine, url)"),
        "must not pass the raw handle and URL positionally:\n{body}"
    );
}

/// The batch counterpart: a `mock_url_list` arg wrapped in the adapter's declared batch
/// request type, exactly as `csharp/streaming.rs` already renders it for the generated e2e
/// test -- and previously ONLY there, since that wrapping used to be a post-processing step
/// bolted onto `streaming.rs` alone rather than living inside `build_args_and_setup`.
#[test]
fn a_batch_streaming_call_wraps_the_url_list_in_the_typed_batch_request() {
    let fixture = Fixture {
        id: "stream_several_documents".into(),
        description: "Stream several documents".into(),
        input: serde_json::json!({
            "engine": {},
            "urls": ["https://a.example.com/doc", "https://b.example.com/doc"]
        }),
        preserve_input_urls: true,
        ..Fixture::default()
    };
    let mut call = CallConfig {
        function: "stream_events_batch".into(),
        result_var: "events".into(),
        r#async: true,
        streaming: Some(StreamingConfig::Enabled(true)),
        args: vec![engine_handle_arg(), urls_arg()],
        ..CallConfig::default()
    };
    call.overrides.insert("csharp".into(), CallOverride::default());
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        adapters: vec![streaming_adapter(
            "stream_events_batch",
            "sample::SampleBatchStreamRequest",
        )],
        ..ResolvedCrateConfig::default()
    };
    let body = render_snippet_body(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &config,
        &[],
        &[],
    )
    .expect("snippet renders");

    assert_eq!(
        line_containing(&body, "var urls ="),
        "var urls = new System.Collections.Generic.List<string>(new string[] { \
         \"https://a.example.com/doc\", \"https://b.example.com/doc\" });"
    );
    assert_eq!(
        line_containing(&body, "urlsReq ="),
        "var urlsReq = new SampleBatchStreamRequest { Urls = urls };"
    );
    assert_eq!(
        line_containing(&body, "StreamEventsBatchAsync("),
        "await foreach (var chunk in SampleConverter.StreamEventsBatchAsync(engine, urlsReq))"
    );
}

/// Negative control, and the pin that keeps this change scoped: an ordinary `mock_url` call
/// with NO matching adapter (the common case for every non-streaming fixture) must render the
/// flat shape exactly as before, even though `config.adapters` is non-empty -- proving the
/// lookup is scoped to the call's own function name, not "any adapter configured anywhere
/// disables the flat shape". Without this control, a change that wrapped every mock_url arg
/// unconditionally would pass the two tests above just as well.
#[test]
fn a_non_streaming_call_with_no_matching_adapter_keeps_the_flat_mock_url_shape() {
    let fixture = Fixture {
        id: "scrape_a_page".into(),
        description: "Scrape a page".into(),
        input: serde_json::json!({"url": "https://example.com/doc"}),
        preserve_input_urls: true,
        ..Fixture::default()
    };
    let mut call = CallConfig {
        function: "scrape".into(),
        result_var: "result".into(),
        args: vec![url_arg()],
        ..CallConfig::default()
    };
    call.overrides.insert("csharp".into(), CallOverride::default());
    let config = ResolvedCrateConfig {
        name: "sample".into(),
        // A streaming adapter IS configured, just for a different function -- proves the
        // per-call lookup, not mere presence of `config.adapters`, gates the wrapping.
        adapters: vec![streaming_adapter("stream_events", "sample::SampleStreamRequest")],
        ..ResolvedCrateConfig::default()
    };
    let body = render_snippet_body(
        &fixture,
        &E2eConfig {
            call,
            ..E2eConfig::default()
        },
        &config,
        &[],
        &[],
    )
    .expect("snippet renders");

    assert_eq!(
        line_containing(&body, "SampleConverter.Scrape("),
        "var result = SampleConverter.Scrape(url);"
    );
    assert!(
        !body.contains("Req = new"),
        "a call with no matching adapter must not wrap its mock_url arg in a request DTO:\n{body}"
    );
}
