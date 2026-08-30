//! Unit tests for `typed_values.rs`, split into a sibling file (matching the
//! `test_file.rs`/`lint_clean_python_tests.rs` split) to keep `typed_values.rs` itself under
//! the file-size cap after adding `Map` coverage.

use super::*;

#[test]
fn emit_bytes_arg_file_path_uses_path_read_bytes() {
    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let value = serde_json::Value::String("pdf/memo.pdf".to_string());
    emit_bytes_arg(&mut bindings, &mut exprs, &value, "content");
    assert!(bindings[0].contains("Path("), "got: {:?}", bindings[0]);
    assert!(bindings[0].contains("read_bytes"), "got: {:?}", bindings[0]);
}

#[test]
fn emit_bytes_arg_base64_uses_b64decode() {
    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let value = serde_json::Value::String("/9j/4AAQ".to_string());
    emit_bytes_arg(&mut bindings, &mut exprs, &value, "data");
    assert!(bindings[0].contains("b64decode"), "got: {:?}", bindings[0]);
}

#[test]
fn emit_json_object_arg_enum_field_emits_constructor_call() {
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

    let enum_def = EnumDef {
        name: "OutputFormat".to_string(),
        rust_path: "demo::OutputFormat".to_string(),
        variants: vec![EnumVariant {
            name: "Markdown".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let type_def = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields: vec![FieldDef {
            name: "output_format".to_string(),
            ty: TypeRef::Named("OutputFormat".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let enums = vec![enum_def];
    let type_defs = vec![type_def];

    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let value = serde_json::json!({"output_format": "markdown"});
    let done = emit_json_object_arg(
        &mut bindings,
        &mut exprs,
        &value,
        "opts",
        Some("ExtractionConfig"),
        "kwargs",
        &HashMap::new(),
        &None,
        "fixture",
        false,
        &type_defs,
        &enums,
        &[],
    );
    assert!(done);
    // Constructor-call form works for both (str, Enum) subclasses and #[pyclass] tagged-union
    // structs. Attribute access (OutputFormat.MARKDOWN) fails for the latter because they have
    // no class-level variant constants.
    assert!(
        bindings[0].contains("OutputFormat(\"markdown\")"),
        "expected constructor-call emission, got: {:?}",
        bindings[0]
    );
    assert!(
        !bindings[0].contains("OutputFormat.MARKDOWN"),
        "must not emit attribute access, got: {:?}",
        bindings[0]
    );
}

#[test]
fn emit_json_object_arg_dict_mode_emits_literal() {
    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let value = serde_json::json!({"key": "val"});
    let type_defs: Vec<crate::core::ir::TypeDef> = Vec::new();
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();
    let done = emit_json_object_arg(
        &mut bindings,
        &mut exprs,
        &value,
        "opts",
        None,
        "dict",
        &HashMap::new(),
        &None,
        "fixture",
        false,
        &type_defs,
        &enums,
        &[],
    );
    assert!(done);
    assert!(bindings[0].contains("\"key\""), "got: {:?}", bindings[0]);
}

#[test]
fn emit_json_object_arg_reads_documented_nested_file() {
    let mut bindings = Vec::new();
    let mut expressions = Vec::new();
    let value = serde_json::json!({"bytes": "document.pdf"});
    let done = emit_json_object_arg(
        &mut bindings,
        &mut expressions,
        &value,
        "input",
        Some("DocumentInput"),
        "kwargs",
        &HashMap::new(),
        &None,
        "fixture",
        false,
        &[],
        &[],
        &[FixtureDocsFileInput {
            field: "/bytes".into(),
            path: "document.pdf".into(),
        }],
    );

    assert!(done);
    assert_eq!(
        bindings,
        [r#"    input = DocumentInput(bytes=Path("document.pdf").read_bytes())"#]
    );
}

/// Regression for the nested-config construction defect: a config field whose own type is
/// itself a generated pyclass (e.g. `nested: NestedConfig` inside
/// `ExtractionConfig`) must be constructed with that class, not emitted as a raw dict --
/// pyo3 rejects a dict where a native class instance is required.
#[test]
fn emit_json_object_arg_kwargs_mode_constructs_nested_struct_field() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let inner_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "model".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let outer_type = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields: vec![FieldDef {
            name: "nested".to_string(),
            ty: TypeRef::Named("NestedConfig".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![outer_type, inner_type];
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();

    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let value = serde_json::json!({"nested": {"model": "standard"}});
    let done = emit_json_object_arg(
        &mut bindings,
        &mut exprs,
        &value,
        "opts",
        Some("ExtractionConfig"),
        "kwargs",
        &HashMap::new(),
        &None,
        "fixture",
        false,
        &type_defs,
        &enums,
        &[],
    );

    assert!(done);
    assert_eq!(
        bindings,
        [r#"    opts = ExtractionConfig(nested=NestedConfig(model="standard"))"#],
        "nested struct field must be constructed with its own class, got: {bindings:?}"
    );
}

/// Batch-call counterpart of the nested-config regression above: a "batch" argument passes
/// an array of typed items via `element_type` (see `emit_python_typed_instance`), and each
/// item's own nested struct fields must resolve the same way a single top-level config does.
#[test]
fn emit_json_object_arg_batch_mode_constructs_nested_struct_field_in_each_item() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let inner_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "model".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let item_type = TypeDef {
        name: "BatchFileItem".to_string(),
        rust_path: "demo::BatchFileItem".to_string(),
        fields: vec![FieldDef {
            name: "nested".to_string(),
            ty: TypeRef::Named("NestedConfig".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![item_type, inner_type];
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();

    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let value = serde_json::json!([{"nested": {"model": "standard"}}]);
    let element_type = Some("BatchFileItem".to_string());
    let done = emit_json_object_arg(
        &mut bindings,
        &mut exprs,
        &value,
        "items",
        None,
        "kwargs",
        &HashMap::new(),
        &element_type,
        "fixture",
        false,
        &type_defs,
        &enums,
        &[],
    );

    assert!(done);
    assert_eq!(
        bindings,
        [r#"    items = [BatchFileItem(nested=NestedConfig(model="standard"))]"#],
        "each batch item's nested struct field must be constructed with its own class, got: {bindings:?}"
    );
}

/// Map counterpart of the nested-config regression above: a field typed `Map<String,
/// NestedConfig>` must construct every value with its own class, not fall through to a raw
/// dict-of-dicts. Before `resolve_field_map_value_struct_type`/`render_nested_map_field_value`
/// existed, `render_kwarg_field_value` never inspected `TypeRef::Map`, so this exact shape
/// fell all the way through to `json_to_python_literal` and emitted a plain dict.
#[test]
fn emit_json_object_arg_kwargs_mode_constructs_nested_struct_map_values() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let inner_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        fields: vec![FieldDef {
            name: "model".to_string(),
            ty: TypeRef::String,
            ..Default::default()
        }],
        ..Default::default()
    };
    let outer_type = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields: vec![FieldDef {
            name: "profiles".to_string(),
            ty: TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("NestedConfig".to_string())),
            ),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![outer_type, inner_type];
    let enums: Vec<crate::core::ir::EnumDef> = Vec::new();

    let mut bindings = Vec::new();
    let mut exprs = Vec::new();
    let value = serde_json::json!({"profiles": {"first": {"model": "standard"}}});
    let done = emit_json_object_arg(
        &mut bindings,
        &mut exprs,
        &value,
        "opts",
        Some("ExtractionConfig"),
        "kwargs",
        &HashMap::new(),
        &None,
        "fixture",
        false,
        &type_defs,
        &enums,
        &[],
    );

    assert!(done);
    assert_eq!(
        bindings,
        [r#"    opts = ExtractionConfig(profiles={"first": NestedConfig(model="standard")})"#],
        "map values must be constructed with their own class, got: {bindings:?}"
    );
}

/// A map field whose declared value type is not itself a known struct (e.g. `Map<String,
/// String>`) must fall through to the plain-dict fallback unchanged.
#[test]
fn resolve_field_map_value_struct_type_returns_none_for_non_struct_map_value() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let type_def = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields: vec![FieldDef {
            name: "labels".to_string(),
            ty: TypeRef::Map(Box::new(TypeRef::String), Box::new(TypeRef::String)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![type_def];

    let result = resolve_field_map_value_struct_type("labels", Some("ExtractionConfig"), &type_defs);
    assert_eq!(result, None);
}

/// `Optional<Map<String, Struct>>` must unwrap the same way `Optional<Vec<Struct>>` does for
/// [`resolve_field_element_struct_type`].
#[test]
fn resolve_field_map_value_struct_type_unwraps_optional() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let inner_type = TypeDef {
        name: "NestedConfig".to_string(),
        rust_path: "demo::NestedConfig".to_string(),
        ..Default::default()
    };
    let outer_type = TypeDef {
        name: "ExtractionConfig".to_string(),
        rust_path: "demo::ExtractionConfig".to_string(),
        fields: vec![FieldDef {
            name: "profiles".to_string(),
            ty: TypeRef::Optional(Box::new(TypeRef::Map(
                Box::new(TypeRef::String),
                Box::new(TypeRef::Named("NestedConfig".to_string())),
            ))),
            ..Default::default()
        }],
        ..Default::default()
    };
    let type_defs = vec![outer_type, inner_type];

    let result = resolve_field_map_value_struct_type("profiles", Some("ExtractionConfig"), &type_defs);
    assert_eq!(result.map(|t| t.name.as_str()), Some("NestedConfig"));
}

#[test]
fn resolve_field_enum_type_detects_enum_field() {
    use crate::core::ir::{EnumDef, EnumVariant, FieldDef, TypeDef, TypeRef};

    let enum_def = EnumDef {
        name: "TierStrategy".to_string(),
        rust_path: "module::TierStrategy".to_string(),
        variants: vec![EnumVariant {
            name: "Auto".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let type_def = TypeDef {
        name: "ConversionOptions".to_string(),
        rust_path: "module::ConversionOptions".to_string(),
        fields: vec![FieldDef {
            name: "tier_strategy".to_string(),
            ty: TypeRef::Named("TierStrategy".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let enums = vec![enum_def];
    let type_defs = vec![type_def];

    let result = resolve_field_enum_type("tier_strategy", Some("ConversionOptions"), &type_defs, &enums);
    assert_eq!(result, Some("TierStrategy".to_string()));
}

#[test]
fn resolve_field_enum_type_returns_none_for_non_enum_field() {
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    let type_def = TypeDef {
        name: "ConversionOptions".to_string(),
        rust_path: "module::ConversionOptions".to_string(),
        fields: vec![FieldDef {
            name: "timeout".to_string(),
            ty: TypeRef::Named("u64".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let enums: Vec<crate::core::ir::EnumDef> = vec![];
    let type_defs = vec![type_def];

    let result = resolve_field_enum_type("timeout", Some("ConversionOptions"), &type_defs, &enums);
    assert_eq!(result, None);
}
