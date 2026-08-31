//! Regression coverage for the one interaction that makes memoizing this traversal subtle: a
//! result produced while the cycle guard fired is path-dependent and must NOT be cached.
//!
//! `resolve_named` cuts recursion when a name is already on the active path, returning `false`.
//! That `false` describes the path, not the (value, name) pair, so storing it would hand a
//! sibling branch -- one with no such cycle -- an answer that branch would never have computed.
//! The graph below is the smallest shape found where that distinction is observable, and it is a
//! false NEGATIVE: a real file input disappears.
//!
//! Trace, with `v0` the root object and `v1 = v0["payload"]`:
//!
//! 1. `SampleEnvelope` flattens into `SampleHeader`; active = {Envelope, Header}.
//! 2. `SampleHeader`'s `payload` is an object, not a string, so its bytes check fails; it then
//!    flattens into `SampleBody`; active = {Envelope, Header, Body}.
//! 3. `SampleBody`'s `payload` field resolves `SampleHeader` against `v1` -- but `SampleHeader` is
//!    on the active path, so the guard CUTS and returns `false`.
//! 4. `SampleBody` and `SampleHeader` therefore both answer `false` for `v0` -- both tainted.
//! 5. `SampleEnvelope`'s second flattened field resolves `SampleBody` against `v0` again. This
//!    time `SampleHeader` is NOT on the path, so `(v1, SampleHeader)` is reached fresh and its
//!    `payload` really is `"documents/sample.bin"` -- the answer is `true`.
//!
//! Cache step 4 and step 5 never runs: the scan returns `false` and the generated suite silently
//! omits the `test_documents` working-directory setup. ~keep

use crate::core::config::e2e::{ArgMapping, CallConfig};
use crate::core::ir::{FieldDef, TypeDef, TypeRef};
use crate::e2e::fixture::Fixture;

fn envelope_arg() -> ArgMapping {
    ArgMapping {
        name: "request".into(),
        field: "input".into(),
        arg_type: "json_object".into(),
        optional: false,
        owned: true,
        element_type: Some("SampleEnvelope".into()),
        go_type: None,
        vec_inner_is_ref: false,
        trait_name: None,
    }
}

fn flattened_field(name: &str, target: &str) -> FieldDef {
    FieldDef {
        name: name.into(),
        ty: TypeRef::Named(target.into()),
        serde_flatten: true,
        ..Default::default()
    }
}

/// `Envelope -> {Header, Body}`, `Header -> Body`, `Body -> Header`: a cycle between `Header` and
/// `Body` that `Envelope` also enters from a second, cycle-free direction. ~keep
fn cyclic_flatten_types() -> Vec<TypeDef> {
    vec![
        TypeDef {
            name: "SampleEnvelope".into(),
            fields: vec![
                flattened_field("header", "SampleHeader"),
                flattened_field("body", "SampleBody"),
            ],
            ..Default::default()
        },
        TypeDef {
            name: "SampleHeader".into(),
            fields: vec![
                FieldDef {
                    name: "payload".into(),
                    ty: TypeRef::Bytes,
                    ..Default::default()
                },
                flattened_field("body", "SampleBody"),
            ],
            ..Default::default()
        },
        TypeDef {
            name: "SampleBody".into(),
            fields: vec![FieldDef {
                name: "payload".into(),
                ty: TypeRef::Named("SampleHeader".into()),
                ..Default::default()
            }],
            ..Default::default()
        },
    ]
}

fn scan(input: serde_json::Value) -> bool {
    let fixture = Fixture {
        input,
        ..Default::default()
    };
    let call = CallConfig {
        args: vec![envelope_arg()],
        ..Default::default()
    };
    super::fixture_uses_test_documents(&fixture, &call, &cyclic_flatten_types(), &[])
}

#[test]
fn cycle_cut_result_is_not_cached_for_a_cycle_free_sibling_branch() {
    // Caching the tainted `false` from the Header -> Body -> Header cut turns this into `false`. ~keep
    assert!(scan(
        serde_json::json!({"payload": {"payload": "documents/sample.bin"}})
    ));
}

#[test]
fn cyclic_flatten_graph_without_a_document_path_reports_no_file_input() {
    // Control: same cyclic graph, same shape of value, no document path anywhere. If the test
    // above passed because the traversal answers `true` too eagerly, this one goes red. ~keep
    assert!(!scan(serde_json::json!({"payload": {"payload": "inline text"}})));
}
