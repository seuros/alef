//! End-to-end regression coverage, through the full docs-snippet pipeline, for the CS8602 this
//! closes in `dereference_optional_iterate_collections` (`snippet.rs`): an optional COLLECTION
//! iterated directly by a `foreach` needs the null-forgiving `!` on the loop's collection
//! expression, since `foreach` calls `GetEnumerator()` on it -- a dereference. This is
//! deliberately NOT fixed in the shared `FieldResolver::accessor` /
//! `render_csharp_with_optionals` (`src/e2e/field_access/optional_renderers.rs`): an earlier
//! attempt to fix it there by dropping the `Field` arm's `!is_leaf` guard also marked optional
//! SCALAR leaves read bare (an assertion, a `Console.WriteLine` -- not a dereference), breaking
//! `test_accessor_csharp`/`test_accessor_csharp_with_optionals`
//! (`src/e2e/field_access/tests.rs`) and disagreeing with every other backend's accessor
//! contract (e.g. Rust unwraps the optional PARENT, never the leaf).

use super::*;
use crate::core::config::ResolvedCrateConfig;
use crate::e2e::config::{CallConfig, E2eConfig};
use crate::e2e::fixture::Fixture;

#[test]
fn an_optional_leaf_field_iterated_directly_gets_the_null_forgiving_operator() {
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "config_keywords", "description": "Show detected keywords", "input": null,
        "docs": {"topic": "guides", "presentation": {"operations": [
            {"op": "iterate", "path": "keywords", "item": "keyword", "fields": []}
        ]}}
    }))
    .expect("fixture");
    let e2e = E2eConfig {
        call: CallConfig {
            function: "detect_keywords".into(),
            result_var: "result".into(),
            ..CallConfig::default()
        },
        result_fields: ["keywords".to_string()].into_iter().collect(),
        fields_optional: ["keywords".to_string()].into_iter().collect(),
        ..E2eConfig::default()
    };
    let config = ResolvedCrateConfig {
        name: "sample_core".into(),
        ..ResolvedCrateConfig::default()
    };

    let body = render_snippet_body(&fixture, &e2e, &config, &[], &[]).expect("snippet renders");

    assert!(
        body.contains("foreach (var keyword in result.Keywords!)"),
        "a nullable leaf collection iterated directly must carry the null-forgiving `!` \
         (CS8602 otherwise):\n{body}"
    );
}
