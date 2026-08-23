//! A field path derived from a fixture's own assertions is only shown when the assertion
//! renderer would have rendered a field access against the result for it.
//!
//! ~keep Task #199 taught `presentation` to fall back to `Assertion::field` when a docs-tagged
//! fixture hand-authors neither `docs.shows` nor `docs.presentation`, which is right: without it
//! every snippet bottomed out at a bare `print(result)` and never demonstrated consuming the
//! return value. What it did not do was ask whether the field it read is a field the backend
//! resolves against the result at all. Three whole classes of assertion name something else
//! entirely, and every one of them shipped as a non-compiling accessor in 0.67.2:
//!
//! * an error-path fixture (`{"type": "error"}` plus `{"field": "error.status_code"}`) — every
//!   backend's error block renders the must-fail check and returns, so no other assertion is
//!   ever rendered (see `error_path_assertions`), and `error` is a real field name on some
//!   unrelated IR type often enough that the flat name oracle waves it through;
//! * a call whose result is not a struct (`result_is_simple` / `result_is_bytes`) — the field is
//!   a pseudo-field naming the buffer or scalar itself, exactly as `java/assertions.rs`'s
//!   byte-buffer arm documents;
//! * a field the availability oracle already rejects (`FieldResolver::is_valid_for_result`) —
//!   the assertion renderer emits a skip marker for it and the snippet emitted an accessor.
//!
//! The companion direction matters just as much: a field that DOES resolve must still be shown,
//! or the next reader "fixes" this by reverting #199 and returns every snippet to
//! `print(result)`.

use super::*;
use crate::core::config::e2e::{CallConfig, CallOverride};
use crate::core::ir::{FieldDef, TypeDef};

fn docs_fixture(assertions: serde_json::Value) -> Fixture {
    serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"html": "<p>Hello World</p>"},
        "assertions": assertions,
        "docs": {"topic": "smoke", "stem": "sample_fixture"}
    }))
    .expect("fixture must parse")
}

fn call_config() -> E2eConfig {
    E2eConfig {
        call: CallConfig {
            function: "convert".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        ..E2eConfig::default()
    }
}

/// A type declaring `error`, standing in for the unrelated IR struct (liter-llm's
/// `ResponseObject`) whose field name is what made the flat reachability oracle accept
/// `error.status_code` on a result type that has no `error` member at all. ~keep
fn type_defs_declaring_error() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "ConversionResult".to_string(),
            fields: vec![FieldDef {
                name: "content".to_string(),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
        TypeDef {
            name: "ResponseObject".to_string(),
            fields: vec![FieldDef {
                name: "error".to_string(),
                ..FieldDef::default()
            }],
            ..TypeDef::default()
        },
    ]
}

#[test]
fn an_error_path_assertion_field_derives_no_accessor() {
    let fixture = docs_fixture(serde_json::json!([
        {"type": "error", "value": "Authentication"},
        {"type": "equals", "field": "error.status_code", "value": 401}
    ]));

    let operations = resolve(&fixture, &call_config(), "java", &type_defs_declaring_error(), &[]);

    assert_eq!(
        operations,
        Vec::new(),
        "an error fixture documents a failure mode; no backend renders a success-path accessor for it"
    );
}

#[test]
fn a_field_the_availability_oracle_rejects_derives_no_accessor() {
    let fixture = docs_fixture(serde_json::json!([{"type": "is_true", "field": "cost_tracked"}]));
    let mut config = call_config();
    config.result_fields = ["content".to_string()].into_iter().collect();

    let operations = resolve(&fixture, &config, "csharp", &[], &[]);

    assert_eq!(
        operations,
        Vec::new(),
        "`cost_tracked` is not a member of the result; the assertion renderer skips it and so must the snippet"
    );
}

#[test]
fn a_pseudo_field_on_a_byte_buffer_result_derives_no_accessor() {
    let fixture = docs_fixture(serde_json::json!([
        {"type": "not_error"},
        {"type": "not_empty", "field": "audio"}
    ]));
    let mut config = call_config();
    config.call.overrides.insert(
        "csharp".to_string(),
        CallOverride {
            result_is_bytes: true,
            ..CallOverride::default()
        },
    );

    let operations = resolve(&fixture, &config, "csharp", &[], &[]);

    assert_eq!(
        operations,
        Vec::new(),
        "a `byte[]` result has no `audio` member -- the field names the buffer itself"
    );
}

/// The same fixture against a backend whose override does NOT declare the result opaque still
/// shows the field: the shape flags are per-language for a reason, and a blanket skip would be
/// the same over-correction in the other direction. ~keep
#[test]
fn a_pseudo_field_still_resolves_for_a_backend_whose_result_is_a_struct() {
    let fixture = docs_fixture(serde_json::json!([
        {"type": "not_error"},
        {"type": "not_empty", "field": "audio"}
    ]));
    let mut config = call_config();
    config.call.overrides.insert(
        "csharp".to_string(),
        CallOverride {
            result_is_bytes: true,
            ..CallOverride::default()
        },
    );

    let operations = resolve(&fixture, &config, "python", &[], &[]);

    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].expression, "result.audio");
}

/// The task #199 behaviour this filter must not regress: a field that genuinely resolves on the
/// return type still produces `result.<field>`, IR present. ~keep
#[test]
fn a_field_that_resolves_on_the_return_type_still_derives_an_accessor() {
    let fixture = docs_fixture(serde_json::json!([
        {"type": "equals", "field": "content", "value": "Hello World\n"},
        {"type": "not_empty", "field": "content"}
    ]));

    let operations = resolve(&fixture, &call_config(), "python", &type_defs_declaring_error(), &[]);

    assert_eq!(
        operations.len(),
        1,
        "the duplicate `content` field must not be shown twice"
    );
    assert_eq!(operations[0].kind, "show");
    assert_eq!(operations[0].expression, "result.content");
}

/// A streaming fixture's assertions name stream-level predicates (`stream.has_page_event`,
/// `stream.event_count_min`), which every backend resolves against the drained chunk list, never
/// as members of the result. crawlberg's `crawl-stream-events` snippet emitted
/// `result.stream.hasPageEvent` on a Dart `List`. ~keep
#[test]
fn a_streaming_virtual_field_derives_no_accessor() {
    let fixture = docs_fixture(serde_json::json!([
        {"type": "is_true", "field": "stream.has_page_event"},
        {"type": "greater_than_or_equal", "field": "stream.event_count_min", "value": 1}
    ]));

    let operations = resolve(&fixture, &call_config(), "dart", &[], &[]);

    assert_eq!(
        operations,
        Vec::new(),
        "stream-level predicates resolve against the drained chunk list, not against the result"
    );
}

/// An assertion *grouping* prefix is not a member path. `rate_limit.min_duration_ms` reached
/// crawlberg's snippets as `result.rateLimit.minDurationMs` on a `CrawlResult` that declares no
/// `rate_limit` at all — the IR only ever declared `rate_limit_ms`, elsewhere. This is the case
/// `is_valid_for_result` alone cannot catch: it defaults an unrecognized name to valid, so
/// `result_field_oracle_knows` has to supply the second half of the answer. ~keep
#[test]
fn an_assertion_grouping_prefix_derives_no_accessor() {
    let fixture = docs_fixture(serde_json::json!([
        {"type": "greater_than", "field": "rate_limit.min_duration_ms", "value": 100}
    ]));
    let type_defs = vec![TypeDef {
        name: "CrawlResult".to_string(),
        fields: vec![
            FieldDef {
                name: "content".to_string(),
                ..FieldDef::default()
            },
            FieldDef {
                name: "rate_limit_ms".to_string(),
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    }];

    let operations = resolve(&fixture, &call_config(), "csharp", &type_defs, &[]);

    assert_eq!(
        operations,
        Vec::new(),
        "`rate_limit` is an assertion grouping the IR has never heard of; the IR declares only `rate_limit_ms`"
    );
}

/// A hand-authored `docs.shows` is an explicit authoring decision, not a derivation, and stays
/// exempt from the filter — otherwise a deliberately-documented virtual/namespaced path the
/// oracle has never heard of would silently vanish from its snippet. ~keep
#[test]
fn a_hand_authored_shows_entry_is_never_filtered() {
    let mut fixture = docs_fixture(serde_json::json!([{"type": "not_error"}]));
    fixture.docs.as_mut().expect("docs").shows = vec!["cost_tracked".to_string()];
    let mut config = call_config();
    config.result_fields = ["content".to_string()].into_iter().collect();

    let operations = resolve(&fixture, &config, "python", &[], &[]);

    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].expression, "result.cost_tracked");
}
