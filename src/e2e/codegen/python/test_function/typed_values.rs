//! Typed Python value rendering for generated test functions.

use std::collections::{BTreeSet, HashMap};

use heck::ToSnakeCase;

use crate::e2e::escape::escape_python;
use crate::e2e::fixture::FixtureDocsFileInput;

use super::super::json::json_to_python_literal;

/// Resolve the enum type name for a field if it's an enum type in the TypeDef,
/// and return None if it's not an enum or the type cannot be resolved.
pub(in crate::e2e::codegen::python) fn resolve_field_enum_type(
    field_name: &str,
    options_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
) -> Option<String> {
    use crate::core::ir::TypeRef;

    let opts_type = options_type?;
    let type_def = type_defs.iter().find(|t| t.name == opts_type)?;
    let field = type_def.fields.iter().find(|f| f.name == field_name)?;

    // Unwrap Optional and Vec wrappers to get the inner type
    let inner_name = match &field.ty {
        TypeRef::Named(n) => Some(n.as_str()),
        TypeRef::Optional(inner) | TypeRef::Vec(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some(n.as_str()),
            _ => None,
        },
        _ => None,
    }?;

    // Check if this is an enum type
    if enums.iter().any(|e| e.name == inner_name) {
        Some(inner_name.to_string())
    } else {
        None
    }
}

/// Resolve the nested-struct type for a field if that field's type (after unwrapping
/// `Optional`) names another type known to `type_defs` -- i.e. a type this backend also
/// generates a pyclass constructor for. A field in that shape must be constructed with its own
/// class rather than passed through as a plain dict: pyo3 does not accept a dict where a native
/// class instance is required.
pub(in crate::e2e::codegen::python) fn resolve_field_struct_type<'a>(
    field_name: &str,
    options_type: Option<&str>,
    type_defs: &'a [crate::core::ir::TypeDef],
) -> Option<&'a crate::core::ir::TypeDef> {
    use crate::core::ir::TypeRef;

    let opts_type = options_type?;
    let type_def = type_defs.iter().find(|t| t.name == opts_type)?;
    let field = type_def.fields.iter().find(|f| f.name == field_name)?;

    let inner_name = match &field.ty {
        TypeRef::Named(n) => Some(n.as_str()),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Named(n) => Some(n.as_str()),
            _ => None,
        },
        _ => None,
    }?;

    type_defs.iter().find(|t| t.name == inner_name)
}

/// Resolve the nested-struct element type for a field typed `Vec<Struct>` (optionally wrapped in
/// `Optional`) -- the shape a "batch" item's own nested list field takes.
pub(in crate::e2e::codegen::python) fn resolve_field_element_struct_type<'a>(
    field_name: &str,
    options_type: Option<&str>,
    type_defs: &'a [crate::core::ir::TypeDef],
) -> Option<&'a crate::core::ir::TypeDef> {
    use crate::core::ir::TypeRef;

    let opts_type = options_type?;
    let type_def = type_defs.iter().find(|t| t.name == opts_type)?;
    let field = type_def.fields.iter().find(|f| f.name == field_name)?;

    let vec_inner = match &field.ty {
        TypeRef::Vec(inner) => Some(inner.as_ref()),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Vec(vec_inner) => Some(vec_inner.as_ref()),
            _ => None,
        },
        _ => None,
    }?;

    match vec_inner {
        TypeRef::Named(name) => type_defs.iter().find(|t| &t.name == name),
        _ => None,
    }
}

/// Resolve the nested-struct value type for a field typed `Map<K, Struct>` (optionally wrapped
/// in `Optional`) -- a map field whose values are themselves a generated pyclass (e.g.
/// `Map<String, NestedConfig>`) must construct each value with that class rather than emit the
/// map as a raw dict of dicts.
pub(in crate::e2e::codegen::python) fn resolve_field_map_value_struct_type<'a>(
    field_name: &str,
    options_type: Option<&str>,
    type_defs: &'a [crate::core::ir::TypeDef],
) -> Option<&'a crate::core::ir::TypeDef> {
    use crate::core::ir::TypeRef;

    let opts_type = options_type?;
    let type_def = type_defs.iter().find(|t| t.name == opts_type)?;
    let field = type_def.fields.iter().find(|f| f.name == field_name)?;

    let map_value = match &field.ty {
        TypeRef::Map(_, value) => Some(value.as_ref()),
        TypeRef::Optional(inner) => match inner.as_ref() {
            TypeRef::Map(_, value) => Some(value.as_ref()),
            _ => None,
        },
        _ => None,
    }?;

    match map_value {
        TypeRef::Named(name) => type_defs.iter().find(|t| &t.name == name),
        _ => None,
    }
}

/// Render one field's JSON value as a Python expression for a `kwargs`-mode constructor call,
/// recursing into nested config/struct fields so a field whose type is itself a generated
/// pyclass (e.g. `nested: NestedConfig` inside `ExtractionConfig`) is constructed with
/// that class instead of a raw dict literal. `used_struct_types` records every nested
/// constructor name this rendering references, so a caller collecting imports can run the
/// identical traversal instead of a second copy that could disagree with what actually gets
/// emitted (the same technique `handle_values::collect_used_nested_types` uses). ~keep
#[allow(clippy::too_many_arguments)]
pub(in crate::e2e::codegen::python) fn render_kwarg_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    enum_fields: &HashMap<String, String>,
    docs_files: &[FixtureDocsFileInput],
    pointer: &str,
    used_struct_types: &mut BTreeSet<String>,
) -> String {
    if let Some(rendered) = render_enum_field_value(field_name, value, containing_type, type_defs, enums, enum_fields)
    {
        return rendered;
    }
    if let Some(rendered) = render_docs_file_field_value(pointer, docs_files) {
        return rendered;
    }
    if let Some(rendered) = render_nested_container_field_value(
        field_name,
        value,
        containing_type,
        type_defs,
        enums,
        enum_fields,
        docs_files,
        pointer,
        used_struct_types,
    ) {
        return rendered;
    }

    json_to_python_literal(value)
}

/// Tries each nested-container shape in turn -- single struct, array-of-structs, then
/// map-of-structs. The three share an identical signature: each resolves `field_name`'s declared
/// type against `type_defs` and, on a match, recurses through [`render_struct_constructor`].
/// Split out of [`render_kwarg_field_value`] to keep that function under the file's per-function
/// line limit; grouping the three container shapes here (rather than inlining each) is what keeps
/// the split effective, since all three take the same nine arguments.
#[allow(clippy::too_many_arguments)]
fn render_nested_container_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    enum_fields: &HashMap<String, String>,
    docs_files: &[FixtureDocsFileInput],
    pointer: &str,
    used_struct_types: &mut BTreeSet<String>,
) -> Option<String> {
    if let Some(rendered) = render_nested_struct_field_value(
        field_name,
        value,
        containing_type,
        type_defs,
        enums,
        enum_fields,
        docs_files,
        pointer,
        used_struct_types,
    ) {
        return Some(rendered);
    }
    if let Some(rendered) = render_nested_array_field_value(
        field_name,
        value,
        containing_type,
        type_defs,
        enums,
        enum_fields,
        docs_files,
        pointer,
        used_struct_types,
    ) {
        return Some(rendered);
    }
    render_nested_map_field_value(
        field_name,
        value,
        containing_type,
        type_defs,
        enums,
        enum_fields,
        docs_files,
        pointer,
        used_struct_types,
    )
}

/// Enum branch of [`render_kwarg_field_value`]: an explicitly configured `enum_fields` entry, or
/// an auto-detected enum field type, renders as `EnumType("variant")`. Mirrors the original
/// inline logic exactly -- an `enum_fields` hit with a non-string value falls through to the
/// remaining branches rather than trying auto-detection.
fn render_enum_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    enum_fields: &HashMap<String, String>,
) -> Option<String> {
    if let Some(enum_type) = enum_fields.get(field_name) {
        if let Some(s) = value.as_str() {
            return Some(format!("{enum_type}(\"{s}\")"));
        }
    } else if let Some(auto_enum_type) = resolve_field_enum_type(field_name, containing_type, type_defs, enums)
        && let Some(s) = value.as_str()
    {
        return Some(format!("{auto_enum_type}(\"{s}\")"));
    }
    None
}

/// Docs-file branch of [`render_kwarg_field_value`]: a field whose JSON pointer matches a
/// configured fixture docs-file input renders as a file-read expression instead of its JSON value.
fn render_docs_file_field_value(pointer: &str, docs_files: &[FixtureDocsFileInput]) -> Option<String> {
    docs_files
        .iter()
        .find(|file| file.field == pointer)
        .map(|file| docs_file_expression(&file.path))
}

/// Nested-struct branch of [`render_kwarg_field_value`]: a field typed as another generated
/// pyclass (optionally `Optional`) renders as that class's constructor call.
#[allow(clippy::too_many_arguments)]
fn render_nested_struct_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    enum_fields: &HashMap<String, String>,
    docs_files: &[FixtureDocsFileInput],
    pointer: &str,
    used_struct_types: &mut BTreeSet<String>,
) -> Option<String> {
    let nested = resolve_field_struct_type(field_name, containing_type, type_defs)?;
    let obj = value.as_object()?;
    Some(render_struct_constructor(
        nested,
        obj,
        type_defs,
        enums,
        enum_fields,
        docs_files,
        pointer,
        used_struct_types,
    ))
}

/// Nested-array branch of [`render_kwarg_field_value`]: a field typed `Vec<Struct>` (optionally
/// `Optional`) renders as a Python list of that class's constructor calls.
#[allow(clippy::too_many_arguments)]
fn render_nested_array_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    enum_fields: &HashMap<String, String>,
    docs_files: &[FixtureDocsFileInput],
    pointer: &str,
    used_struct_types: &mut BTreeSet<String>,
) -> Option<String> {
    let elem = resolve_field_element_struct_type(field_name, containing_type, type_defs)?;
    let arr = value.as_array()?;
    if !arr.iter().all(|item| item.is_object()) {
        return None;
    }
    let items: Vec<String> = arr
        .iter()
        .filter_map(|item| item.as_object())
        .enumerate()
        .map(|(index, obj)| {
            let item_pointer = format!("{pointer}/{index}");
            render_struct_constructor(
                elem,
                obj,
                type_defs,
                enums,
                enum_fields,
                docs_files,
                &item_pointer,
                used_struct_types,
            )
        })
        .collect();
    Some(format!("[{}]", items.join(", ")))
}

/// Nested-map branch of [`render_kwarg_field_value`]: a field typed `Map<K, Struct>` (optionally
/// `Optional`) renders as a Python dict literal whose values are constructed with their own
/// class. The map's keys pass through as plain Python string literals -- alef's `Map<K, V>`
/// fields are always string-keyed JSON objects on the wire, so only the value side needs a
/// constructor. Falls through (returns `None`) when any entry's value is not itself an object,
/// so a malformed fixture still reaches the `json_to_python_literal` fallback instead of panicking.
#[allow(clippy::too_many_arguments)]
fn render_nested_map_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    enum_fields: &HashMap<String, String>,
    docs_files: &[FixtureDocsFileInput],
    pointer: &str,
    used_struct_types: &mut BTreeSet<String>,
) -> Option<String> {
    let elem = resolve_field_map_value_struct_type(field_name, containing_type, type_defs)?;
    let map_obj = value.as_object()?;
    if map_obj.values().any(|entry| !entry.is_object()) {
        return None;
    }
    let items: Vec<String> = map_obj
        .iter()
        .map(|(key, entry)| {
            let entry_obj = entry.as_object().expect("checked above: every entry is an object");
            let entry_pointer = format!("{pointer}/{}", escape_json_pointer(key));
            let ctor = render_struct_constructor(
                elem,
                entry_obj,
                type_defs,
                enums,
                enum_fields,
                docs_files,
                &entry_pointer,
                used_struct_types,
            );
            format!("\"{}\": {ctor}", escape_python(key))
        })
        .collect();
    Some(format!("{{{}}}", items.join(", ")))
}

/// Build a `TypeName(field=value, ...)` constructor call for `type_def`, recursing through
/// [`render_kwarg_field_value`] for each field so arbitrarily deep nested config types resolve
/// the same way at every depth.
#[allow(clippy::too_many_arguments)]
fn render_struct_constructor(
    type_def: &crate::core::ir::TypeDef,
    obj: &serde_json::Map<String, serde_json::Value>,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    enum_fields: &HashMap<String, String>,
    docs_files: &[FixtureDocsFileInput],
    pointer: &str,
    used_struct_types: &mut BTreeSet<String>,
) -> String {
    used_struct_types.insert(type_def.name.clone());
    let kwargs: Vec<String> = obj
        .iter()
        .map(|(field_name, field_value)| {
            let snake_key = field_name.to_snake_case();
            let field_pointer = format!("{pointer}/{}", escape_json_pointer(field_name));
            let rendered = render_kwarg_field_value(
                field_name,
                field_value,
                Some(type_def.name.as_str()),
                type_defs,
                enums,
                enum_fields,
                docs_files,
                &field_pointer,
                used_struct_types,
            );
            format!("{snake_key}={rendered}")
        })
        .collect();
    format!("{}({})", type_def.name, kwargs.join(", "))
}

/// Returns `true` if the arg was fully emitted (caller should `continue`).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_json_object_arg(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    value: &serde_json::Value,
    var_name: &str,
    options_type: Option<&str>,
    options_via: &str,
    enum_fields: &HashMap<String, String>,
    element_type: &Option<String>,
    fixture_id: &str,
    has_host_root_route: bool,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    docs_files: &[FixtureDocsFileInput],
) -> bool {
    if crate::e2e::codegen::value_contains_mock_url_placeholder(value) {
        return emit_json_object_arg_with_mock_url(
            arg_bindings,
            kwarg_exprs,
            value,
            var_name,
            options_type,
            options_via,
            fixture_id,
            has_host_root_route,
        );
    }

    match options_via {
        "dict" => {
            // When we have an array of objects and an element_type, emit dict literals (not constructor calls).
            // The bindings expect [{"type": "click", "selector": "#id"}, ...], not [PageAction(...), ...]
            if let (Some(_elem_type), Some(arr)) = (element_type, value.as_array())
                && !arr.is_empty()
                && arr.iter().all(|v| v.is_object())
            {
                let items: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_object())
                    .map(|obj| {
                        let dict_items: Vec<String> = obj
                            .iter()
                            .map(|(k, v)| {
                                format!(
                                    "{}: {}",
                                    json_to_python_literal(&serde_json::Value::String(k.clone())),
                                    json_to_python_literal(v)
                                )
                            })
                            .collect();
                        format!("{{{}}}", dict_items.join(", "))
                    })
                    .collect();
                arg_bindings.push(format!("    {var_name} = [{}]", items.join(", ")));
                kwarg_exprs.push(var_name.to_string());
                return true;
            }
            // Fall through to default dict behavior
            let literal = json_to_python_literal(value);
            let noqa = if literal.contains("/tmp/") {
                "  # noqa: S108"
            } else {
                ""
            };
            arg_bindings.push(format!("    {var_name} = {literal}{noqa}"));
            kwarg_exprs.push(var_name.to_string());
            true
        }
        "json" => {
            let json_str = serde_json::to_string(value).unwrap_or_default();
            let escaped = escape_python(&json_str);
            arg_bindings.push(format!("    {var_name} = json.loads(\"{escaped}\")"));
            kwarg_exprs.push(var_name.to_string());
            true
        }
        "from_json" => {
            if let Some(opts_type) = options_type {
                let json_str = serde_json::to_string(value).unwrap_or_default();
                let escaped = escape_python(&json_str);
                arg_bindings.push(format!("    {var_name} = {opts_type}.from_json(\"{escaped}\")"));
                kwarg_exprs.push(var_name.to_string());
                true
            } else {
                false
            }
        }
        _ => {
            // When we have an array with element_type, construct typed instances for Python.
            if let Some(elem_type) = element_type
                && !value.is_null()
                && let Some(arr) = value.as_array()
                && arr.iter().all(|item| item.is_object())
            {
                let items: Vec<String> = arr
                    .iter()
                    .filter_map(|item| item.as_object())
                    .enumerate()
                    .map(|(index, obj)| {
                        let pointer = format!("/{index}");
                        emit_python_typed_instance(obj, elem_type, type_defs, enums, enum_fields, docs_files, &pointer)
                    })
                    .collect();
                arg_bindings.push(format!("    {var_name} = [{}]", items.join(", ")));
                kwarg_exprs.push(var_name.to_string());
                return true;
            }
            // "kwargs" mode
            if let (Some(opts_type), Some(obj)) = (options_type, value.as_object()) {
                let mut used_struct_types = BTreeSet::new();
                let kwargs: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        let snake_key = k.to_snake_case();
                        let field_pointer = format!("/{}", escape_json_pointer(k));
                        let py_val = render_kwarg_field_value(
                            k,
                            v,
                            Some(opts_type),
                            type_defs,
                            enums,
                            enum_fields,
                            docs_files,
                            &field_pointer,
                            &mut used_struct_types,
                        );
                        format!("{snake_key}={py_val}")
                    })
                    .collect();
                let constructor = format!("{opts_type}({})", kwargs.join(", "));
                arg_bindings.push(format!("    {var_name} = {constructor}"));
                kwarg_exprs.push(var_name.to_string());
                true
            } else {
                false
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_json_object_arg_with_mock_url(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    value: &serde_json::Value,
    var_name: &str,
    options_type: Option<&str>,
    options_via: &str,
    fixture_id: &str,
    has_host_root_route: bool,
) -> bool {
    let json_str = serde_json::to_string(value).unwrap_or_default();
    let escaped = escape_python(&json_str);
    let env_key = crate::e2e::codegen::mock_url_env_key(fixture_id);
    let fallback = format!("os.environ['MOCK_SERVER_URL'] + '/fixtures/{fixture_id}'");
    let base_expr = if has_host_root_route {
        format!("os.environ.get('{env_key}') or {fallback}")
    } else {
        fallback
    };
    arg_bindings.push(format!("    {var_name}_mock_base_url = {base_expr}"));
    arg_bindings.push(format!(
        "    {var_name}_json = \"{escaped}\".replace(\"{}\", {var_name}_mock_base_url)",
        crate::e2e::codegen::MOCK_URL_PLACEHOLDER
    ));

    match (options_via, options_type) {
        ("from_json", Some(opts_type)) => {
            arg_bindings.push(format!("    {var_name} = {opts_type}.from_json({var_name}_json)"));
        }
        ("dict", _) | (_, None) | ("json", _) => {
            arg_bindings.push(format!("    {var_name} = json.loads({var_name}_json)"));
        }
        (_, Some(opts_type)) => {
            arg_bindings.push(format!("    {var_name} = {opts_type}(**json.loads({var_name}_json))"));
        }
    }
    kwarg_exprs.push(var_name.to_string());
    true
}

pub(super) fn emit_bytes_arg(
    arg_bindings: &mut Vec<String>,
    kwarg_exprs: &mut Vec<String>,
    value: &serde_json::Value,
    var_name: &str,
) {
    if let Some(raw) = value.as_str() {
        match super::super::helpers::classify_bytes_value(raw) {
            super::super::helpers::BytesKind::FilePath => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = Path(\"{escaped}\").read_bytes()"));
            }
            super::super::helpers::BytesKind::InlineText => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = b\"{escaped}\""));
            }
            super::super::helpers::BytesKind::Base64 => {
                let escaped = escape_python(raw);
                arg_bindings.push(format!("    {var_name} = base64.b64decode(\"{escaped}\")"));
            }
        }
    } else {
        arg_bindings.push(format!("    {var_name} = None"));
    }
    kwarg_exprs.push(var_name.to_string());
}

/// Emit a Python dict literal for a typed object-array element.
#[allow(dead_code)]
fn emit_python_object_item(obj: &serde_json::Map<String, serde_json::Value>) -> String {
    let items: Vec<String> = obj
        .iter()
        .map(|(k, v)| {
            format!(
                "{}: {}",
                json_to_python_literal(&serde_json::Value::String(k.clone())),
                json_to_python_literal(v)
            )
        })
        .collect();
    format!("{{{}}}", items.join(", "))
}

/// Emit a Python constructor call for a typed instance (e.g., BatchFileItem(...)), recursing
/// into any of its own fields that are themselves generated pyclasses (e.g. a batch item whose
/// `nested` field is a `NestedConfig`) via [`render_kwarg_field_value`].
fn emit_python_typed_instance(
    obj: &serde_json::Map<String, serde_json::Value>,
    elem_type: &str,
    type_defs: &[crate::core::ir::TypeDef],
    enums: &[crate::core::ir::EnumDef],
    enum_fields: &HashMap<String, String>,
    docs_files: &[FixtureDocsFileInput],
    pointer: &str,
) -> String {
    let mut used_struct_types = BTreeSet::new();
    let kwargs: Vec<String> = obj
        .iter()
        .map(|(k, v)| {
            let snake_key = k.to_snake_case();
            let field_pointer = format!("{pointer}/{}", escape_json_pointer(k));
            let rendered = render_kwarg_field_value(
                k,
                v,
                Some(elem_type),
                type_defs,
                enums,
                enum_fields,
                docs_files,
                &field_pointer,
                &mut used_struct_types,
            );
            format!("{snake_key}={rendered}")
        })
        .collect();
    format!("{}({})", elem_type, kwargs.join(", "))
}

fn docs_file_expression(path: &str) -> String {
    crate::e2e::template_env::render(
        "python/docs_file_expression.py.jinja",
        minijinja::context! { path => escape_python(path) },
    )
    .trim_end()
    .to_string()
}

fn escape_json_pointer(field: &str) -> String {
    field.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
#[path = "typed_values_tests.rs"]
mod tests;
