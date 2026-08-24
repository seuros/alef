//! Regression: the zig JSON-mode assertion renderer must lower a namespace-prefixed
//! fixture field to the path the value actually occupies in the serialized result.
//!
//! A fixture field like `batch.completed_count` groups the assertion under a virtual
//! `batch` label; the payload carries `completed_count` at the top level. The renderer
//! built its `std.json.Value` lookup chain from `FieldResolver::resolve`, which only
//! applies aliases and never strips the prefix, so it emitted
//! `result.object.get("batch").?.object.get("completed_count").?` — a `.?` on a key
//! that is absent from every real payload, which aborts the generated zig test.
//!
//! Stripping is conditional: a genuinely nested path (`metrics.total_lines`, where
//! `metrics` is a declared result field) must keep its full chain.
//!
//! Lives in its own file rather than in `zig/assertions.rs`, which is already over the
//! repo's 1,000-line cap. ~keep

use super::assertions::render_json_assertion;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::collections::{HashMap, HashSet};

/// The shape the call under test actually serializes. `completed_count` is top-level;
/// `metrics` is a real nested object.
fn result_payload() -> serde_json::Value {
    serde_json::json!({
        "completed_count": 2,
        "failed_count": 0,
        "total_count": 2,
        "metrics": { "total_lines": 41 }
    })
}

fn resolver() -> FieldResolver {
    let result_fields: HashSet<String> = ["completed_count", "failed_count", "total_count", "metrics"]
        .into_iter()
        .map(String::from)
        .collect();
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &result_fields,
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn render_equals(field: &str, value: serde_json::Value) -> String {
    let assertion = Assertion {
        assertion_type: "equals".to_string(),
        field: Some(field.to_string()),
        value: Some(value),
        ..Assertion::default()
    };
    let mut out = String::new();
    render_json_assertion(&mut out, &assertion, "result", &resolver(), false);
    out
}

/// Recover the JSON keys an emitted `result.object.get("a").?.object.get("b").?` chain
/// navigates, in order.
fn navigated_keys(rendered: &str) -> Vec<String> {
    rendered
        .split(".object.get(\"")
        .skip(1)
        .filter_map(|rest| rest.split_once("\")").map(|(key, _)| key.to_string()))
        .collect()
}

/// Walk the recovered key chain against a payload, as the generated zig would.
fn resolve_keys<'a>(payload: &'a serde_json::Value, keys: &[String]) -> Option<&'a serde_json::Value> {
    let mut current = payload;
    for key in keys {
        current = current.get(key)?;
    }
    Some(current)
}

#[test]
fn namespace_prefixed_field_navigates_the_real_payload_shape() {
    let rendered = render_equals("batch.completed_count", serde_json::json!(2));
    let keys = navigated_keys(&rendered);
    assert_eq!(
        keys,
        vec!["completed_count".to_string()],
        "the virtual `batch` label must not become a JSON key step; rendered:\n{rendered}"
    );

    let payload = result_payload();
    let found = resolve_keys(&payload, &keys)
        .unwrap_or_else(|| panic!("emitted key chain {keys:?} resolves to nothing in the payload"));
    assert_eq!(found, &serde_json::json!(2), "the emitted chain read the wrong value");
}

#[test]
fn genuinely_nested_field_keeps_its_full_key_chain() {
    let rendered = render_equals("metrics.total_lines", serde_json::json!(41));
    let keys = navigated_keys(&rendered);
    assert_eq!(
        keys,
        vec!["metrics".to_string(), "total_lines".to_string()],
        "a declared result field must not be stripped as a namespace label; rendered:\n{rendered}"
    );

    let payload = result_payload();
    let found = resolve_keys(&payload, &keys)
        .unwrap_or_else(|| panic!("emitted key chain {keys:?} resolves to nothing in the payload"));
    assert_eq!(found, &serde_json::json!(41), "the emitted chain read the wrong value");
}

/// Control: the resolver used above CAN fail, and the pre-fix chain is exactly what
/// fails against it. Without this the payload checks would prove nothing. ~keep
#[test]
fn the_pre_fix_key_chain_resolves_to_nothing() {
    let pre_fix = ["batch".to_string(), "completed_count".to_string()];
    assert!(
        resolve_keys(&result_payload(), &pre_fix).is_none(),
        "the buggy key chain must not resolve — otherwise the payload checks are vacuous"
    );
}
