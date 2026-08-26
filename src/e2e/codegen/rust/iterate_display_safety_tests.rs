//! A Rust docs snippet's `for` loop over an `Iterate` operation's collection must borrow, never
//! move, and its per-item field formatter must never claim `Display` for a type alef cannot
//! positively vouch for.
//!
//! ~keep Reproduces a real consumer regen failure: `snippet_body.rs.jinja` emitted
//! `for table in result.results[0].tables { println!("{}", table.cells); }` for a plain
//! (non-`Option`) `Vec<Table>` field whose element type `Table` declares `cells: Vec<Vec<String>>`
//! — a move out of an index expression (E0507) AND a `{}` against a type with no `Display` impl,
//! stacked in one line. This fixes both: the loop always borrows (`.iter()` when the
//! collection itself is not optional, `.iter().flatten()` when it is, so the loop variable is a
//! reference either way), and each per-item field is checked against an ALLOWLIST of types alef
//! can positively confirm implement `Display` (`String`, `char`, numeric/`bool` primitives) rather
//! than the `field_types`-presence check `FieldResolver::is_display_unsafe` uses for `Show`/a
//! `fields`-less `Iterate`, which never records a bare `Vec<Vec<String>>` at all.

use crate::core::config::NewAlefConfig;
use crate::core::ir::{FieldDef, FunctionDef, TypeDef, TypeRef};
use crate::e2e::codegen::E2eCodegen;
use crate::e2e::codegen::rust::RustE2eCodegen;
use crate::e2e::fixture::Fixture;

/// `SampleResult { tables: Vec<Table>, maybe_tables: Option<Vec<Table>> }`,
/// `Table { name: String, cells: Vec<Vec<String>> }` — `name` is display-safe, `cells` is the
/// nested-collection shape the allowlist must refuse even though `field_types` never records it.
fn ir() -> (Vec<TypeDef>, Vec<FunctionDef>) {
    let table = TypeDef {
        name: "Table".into(),
        fields: vec![
            FieldDef {
                name: "name".into(),
                ty: TypeRef::String,
                optional: false,
                ..FieldDef::default()
            },
            FieldDef {
                name: "cells".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Vec(Box::new(TypeRef::String)))),
                optional: false,
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    };
    let sample_result = TypeDef {
        name: "SampleResult".into(),
        fields: vec![
            FieldDef {
                name: "tables".into(),
                ty: TypeRef::Vec(Box::new(TypeRef::Named("Table".into()))),
                optional: false,
                ..FieldDef::default()
            },
            FieldDef {
                name: "maybe_tables".into(),
                ty: TypeRef::Optional(Box::new(TypeRef::Vec(Box::new(TypeRef::Named("Table".into()))))),
                optional: true,
                ..FieldDef::default()
            },
        ],
        ..TypeDef::default()
    };
    (
        vec![sample_result, table],
        vec![FunctionDef {
            name: "convert".into(),
            return_type: TypeRef::Named("SampleResult".into()),
            ..FunctionDef::default()
        }],
    )
}

fn snippet_body(operations_json: serde_json::Value) -> String {
    let config_text = r#"
[workspace]
languages = ["rust"]
[[crates]]
name = "example-core"
sources = ["src/lib.rs"]
[crates.e2e]
fixtures = "fixtures"
[crates.e2e.call]
function = "convert"
module = "example_core"
result_var = "result"
args = [{ name = "html", field = "html", type = "string" }]
"#;
    let config: NewAlefConfig = toml::from_str(config_text).expect("config parses");
    let e2e = config.crates[0].e2e.clone().expect("e2e config");
    let resolved = config.resolve().expect("config resolves").remove(0);
    let fixture: Fixture = serde_json::from_value(serde_json::json!({
        "id": "sample_fixture",
        "description": "Sample fixture",
        "input": {"html": "<p>Hello</p>"},
        "assertions": [],
        "docs": {
            "topic": "smoke",
            "stem": "sample_fixture",
            "presentation": {"operations": operations_json},
        },
    }))
    .expect("fixture parses");
    let (type_defs, functions) = ir();
    RustE2eCodegen
        .render_snippet_body_with_functions(&fixture, &e2e, &resolved, &type_defs, &[], &functions, &[])
        .expect("rust snippet body renders")
}

/// The defect: a plain (non-`Option`) collection field moves out of its owner with no adapter at
/// all. Fails against the pre-fix template, which only appended `.iter().flatten()` when
/// `operation.optional` was set.
#[test]
fn a_plain_collection_iterate_borrows_rather_than_moves() {
    let body = snippet_body(serde_json::json!([{
        "op": "iterate", "path": "tables", "item": "table", "fields": ["name"],
    }]));
    assert!(
        body.contains("for table in result.tables.iter() {"),
        "a plain Vec field must be borrowed with `.iter()`, not moved:\n{body}"
    );
}

/// The control for the loop-borrowing fix: an `Option`-wrapped collection must keep its existing
/// `.iter().flatten()` adapter, not gain a second, redundant `.iter()`.
#[test]
fn an_optional_collection_iterate_keeps_iter_flatten_without_double_borrowing() {
    let body = snippet_body(serde_json::json!([{
        "op": "iterate", "path": "maybe_tables", "item": "table", "fields": ["name"],
    }]));
    assert!(
        body.contains("for table in result.maybe_tables.iter().flatten() {"),
        "an optional collection must keep exactly one `.iter().flatten()`, no extra adapter:\n{body}"
    );
}

/// The defect: `display: true` on an `Iterate` with per-item `fields` is applied uniformly, so a
/// `Vec<Vec<String>>` field (never recorded in `field_types`, since nothing there peels `Vec`) is
/// formatted with `{}` and fails to compile. Fails against the pre-fix code, which had no
/// per-field answer at all for an `Iterate`'s `fields`.
#[test]
fn a_non_display_safe_per_item_field_falls_back_to_debug_formatting() {
    let body = snippet_body(serde_json::json!([{
        "op": "iterate", "path": "tables", "item": "table", "fields": ["cells"], "display": true,
    }]));
    assert!(
        body.contains("println!(\"{:?}\", table.cells);"),
        "a `Vec<Vec<String>>` per-item field must fall back to `{{:?}}`:\n{body}"
    );
    assert!(
        !body.contains("println!(\"{}\", table.cells);"),
        "a `Vec<Vec<String>>` per-item field must never be formatted with `{{}}`:\n{body}"
    );
}

/// The control that stops an over-broad fix: a `String` per-item field must keep rendering with
/// `{}` when `display: true` is set, not regress to unconditional `{:?}`.
#[test]
fn a_display_safe_per_item_field_keeps_display_formatting() {
    let body = snippet_body(serde_json::json!([{
        "op": "iterate", "path": "tables", "item": "table", "fields": ["name"], "display": true,
    }]));
    assert!(
        body.contains("println!(\"{}\", table.name);"),
        "a `String` per-item field must keep `{{}}` when `display: true`:\n{body}"
    );
}

/// Both fixes stacked in one operation, matching the exact consumer shape this table exists to
/// fix: a plain collection with a mixed display-safe/unsafe field set.
#[test]
fn a_plain_collection_with_mixed_display_safety_fields_compiles_shaped_output() {
    let body = snippet_body(serde_json::json!([{
        "op": "iterate", "path": "tables", "item": "table", "fields": ["name", "cells"], "display": true,
    }]));
    assert!(body.contains("for table in result.tables.iter() {"), "{body}");
    assert!(body.contains("println!(\"{}\", table.name);"), "{body}");
    assert!(body.contains("println!(\"{:?}\", table.cells);"), "{body}");
}

// ~keep The "fields-less `Iterate`" and "`Show`" controls — proving
// `downgrade_display_unsafe_operations` (the whole-operation oracle) is unaffected by this
// table's new per-item-field allowlist — are exercised at the `presentation::resolve` level in
// `presentation/iterate_field_display_safety_tests.rs`, where the resulting `display` flag can be
// asserted directly rather than through the empty-loop-body rendering both shapes share.
