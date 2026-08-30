//! Runtime-index lowering tests for the `$mock_url` path, split into a sibling of
//! `typed_values_tests.rs` (matching the `test_file.rs`/`dead_helper_tests.rs` split) because
//! that file is already at 901 lines and this group would push it past the file-size cap.
//!
//! Every test here drives the real `emit_json_object_arg` entry point against real IR
//! `TypeDef`s, rather than calling `runtime_dict_index_expression` with a hand-written pointer.
//! That distinction is the point of the file: a hand-written pointer test cannot tell whether
//! the recursion actually *tags* array positions, so it stays green even if the tagging is
//! removed and every list index silently reverts to a quoted map key. ~keep

use super::*;

fn nested_url_type() -> crate::core::ir::TypeDef {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "url".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn extraction_config_with(fields: Vec<crate::core::ir::FieldDef>) -> crate::core::ir::TypeDef {
    crate::core::ir::TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields,
        ..Default::default()
    }
}

fn vec_of_nested_field() -> crate::core::ir::FieldDef {
    use crate::core::ir::{FieldDef, TypeRef};

    FieldDef {
        name: "items".to_string(),
        ty: TypeRef::Vec(Box::new(TypeRef::Named("NestedConfig".to_string()))),
        ..Default::default()
    }
}

fn map_of_nested_field() -> crate::core::ir::FieldDef {
    use crate::core::ir::{FieldDef, TypeRef};

    FieldDef {
        name: "profiles".to_string(),
        ty: TypeRef::Map(
            Box::new(TypeRef::String),
            Box::new(TypeRef::Named("NestedConfig".to_string())),
        ),
        ..Default::default()
    }
}

/// Emit `value` as a `$mock_url` `json_object` argument named `var_name` and return the
/// emitted setup lines.
fn emit_mock_url_bindings(
    value: &serde_json::Value,
    var_name: &str,
    type_defs: &[crate::core::ir::TypeDef],
    options_type: Option<&str>,
    element_type: &Option<String>,
) -> Vec<String> {
    let mut bindings = Vec::new();
    let mut expressions = Vec::new();
    let mut sink = ArgSink {
        bindings: &mut bindings,
        kwarg_exprs: &mut expressions,
    };
    let spec = ConstructorSpec {
        options_type,
        options_via: "kwargs",
        element_type,
    };
    let mock = MockUrlInfo {
        fixture_id: "fixture",
        has_host_root_route: false,
    };
    let context = KwargRenderContext {
        type_defs,
        enums: &[],
        enum_fields: &HashMap::new(),
        docs_files: &[],
        leaf_source: LeafSource::Literal,
    };

    assert!(
        emit_json_object_arg(&mut sink, value, var_name, &spec, &mock, context),
        "the $mock_url branch must claim the argument"
    );
    assert_eq!(expressions, [var_name.to_string()]);
    bindings
}

/// Control for the numeric-map-key fix: a genuine `Vec<Struct>` field must still lower each
/// element to an *integer* subscript on the runtime dict. `json.loads` turns a JSON array into a
/// Python list, so a quoted `["0"]` here raises `TypeError: list indices must be integers`.
///
/// This is the test the pointer-level unit tests could not be: they hand-build a `~2`-tagged
/// pointer and so pass whether or not the recursion tags anything. Reverting
/// `array_item_pointer(pointer, index, context.leaf_source)` in `render_value_for_type_ref`'s
/// `TypeRef::Vec` arm back to `format!("{pointer}/{index}")` leaves every existing test green
/// and fails only this one.
#[test]
fn mock_url_vec_struct_fields_lower_to_integer_subscripts() {
    let type_defs = vec![extraction_config_with(vec![vec_of_nested_field()]), nested_url_type()];
    let value = serde_json::json!({"items": [{"url": "$mock_url/a"}, {"url": "$mock_url/b"}]});

    let bindings = emit_mock_url_bindings(&value, "opts", &type_defs, Some("ExtractionConfig"), &None);

    let expected = r#"    opts = ExtractionConfig(items=[NestedConfig(url=opts_data["items"][0]["url"]), NestedConfig(url=opts_data["items"][1]["url"])])"#;
    assert_eq!(
        bindings.last().map(String::as_str),
        Some(expected),
        "vec elements must index the runtime list by position, got: {bindings:?}"
    );
}

/// The discriminating test for the numeric-map-key defect: one config carrying *both* a
/// `Map<String, NestedConfig>` keyed `"0"` and a `Vec<NestedConfig>`, so the two accessors are
/// decided in the same rendering pass from the same-looking pointer text `0`.
///
/// Any classification that reads the segment's text rather than the owner's IR type must render
/// the two identically -- `[0]` for both (the pre-fix behaviour, which raises `KeyError: 0` on
/// the map) or `["0"]` for both (a naive quote-everything fix, which raises `TypeError` on the
/// list). Only deciding from `TypeRef::Map` versus `TypeRef::Vec` produces the split below.
#[test]
fn mock_url_numeric_map_keys_and_list_indices_lower_differently() {
    let type_defs = vec![
        extraction_config_with(vec![vec_of_nested_field(), map_of_nested_field()]),
        nested_url_type(),
    ];
    let value = serde_json::json!({
        "items": [{"url": "$mock_url/a"}],
        "profiles": {"0": {"url": "$mock_url/b"}},
    });

    let bindings = emit_mock_url_bindings(&value, "opts", &type_defs, Some("ExtractionConfig"), &None);

    let expected = r#"    opts = ExtractionConfig(items=[NestedConfig(url=opts_data["items"][0]["url"])], profiles={"0": NestedConfig(url=opts_data["profiles"]["0"]["url"])})"#;
    assert_eq!(
        bindings.last().map(String::as_str),
        Some(expected),
        "the numeric map key must stay a quoted lookup while the list index stays an integer, \
         got: {bindings:?}"
    );
}

/// The `$mock_url` array short-circuit regression, pinned on a *two*-element array so the
/// element index has to advance: before the fix `emit_json_object_arg_with_mock_url` never read
/// `spec.element_type` at all and fell into its `(_, None)` arm, emitting
/// `items = json.loads(items_json)` -- a list of raw dicts where the bindings expect
/// `BatchItem` instances holding `NestedConfig` instances.
///
/// The whole binding list is pinned, so a regression that re-adds the `json.loads` short-circuit,
/// drops the `items_data` parse it depends on, or emits a constant element index fails here.
#[test]
fn mock_url_typed_array_constructs_each_element_at_its_own_index() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let item_type = TypeDef {
        name: "BatchItem".to_string(),
        rust_path: "demo::BatchItem".to_string(),
        fields: vec![FieldDef {
            name: "nested".to_string(),
            ty: TypeRef::Named("NestedConfig".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![item_type, nested_url_type()];
    let value = serde_json::json!([
        {"nested": {"url": "$mock_url/a"}},
        {"nested": {"url": "$mock_url/b"}},
    ]);
    let element_type = Some("BatchItem".to_string());

    let bindings = emit_mock_url_bindings(&value, "items", &type_defs, None, &element_type);

    assert_eq!(
        bindings.len(),
        4,
        "expected the base-url, substituted-json, runtime-parse and constructor lines, got: {bindings:?}"
    );
    let expected_parse = "    items_data = json.loads(items_json)";
    let expected_construct = r#"    items = [BatchItem(nested=NestedConfig(url=items_data[0]["nested"]["url"])), BatchItem(nested=NestedConfig(url=items_data[1]["nested"]["url"]))]"#;
    let lowering: Vec<&str> = bindings[2..].iter().map(String::as_str).collect();
    assert_eq!(
        lowering,
        [expected_parse, expected_construct],
        "each mock-url array element must be constructed with element_type at its own index, \
         got: {bindings:?}"
    );
}
