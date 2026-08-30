//! Typed Python value rendering for generated test functions.

use std::collections::{BTreeSet, HashMap};

use heck::ToSnakeCase;

use crate::e2e::escape::escape_python;
use crate::e2e::fixture::FixtureDocsFileInput;

use super::super::json::json_to_python_literal;

/// Read-only rendering environment shared, unchanged, across every level of the
/// `render_kwarg_field_value` recursion (through `render_struct_constructor` and the
/// nested-container helpers) and reused by the `json_object` arg emitters below. All four
/// fields are borrows the whole call tree reads but never writes, so the struct derives `Copy`
/// -- passing it down the recursion is a pointer-width copy, not a clone of `type_defs`/
/// `enums`/`docs_files` themselves.
///
/// `used_struct_types` is deliberately NOT a field here: it is a per-call *output* accumulator
/// that every level mutates, not read-only shared state, so it stays its own `&mut` argument at
/// each call site. Folding a mutable accumulator into an otherwise `Copy`, read-only bundle
/// would force every function to take `&mut KwargRenderContext` and forfeit the very simplicity
/// -- cheap, ordinary reborrows -- this struct exists to buy. ~keep
#[derive(Clone, Copy)]
pub(in crate::e2e::codegen::python) struct KwargRenderContext<'a> {
    pub type_defs: &'a [crate::core::ir::TypeDef],
    pub enums: &'a [crate::core::ir::EnumDef],
    pub enum_fields: &'a HashMap<String, String>,
    pub docs_files: &'a [FixtureDocsFileInput],
}

/// Output accumulator for one fixture argument's emission: the setup lines it needs
/// (`bindings`) and the expression that becomes its slot in the call's keyword-argument list
/// (`kwarg_exprs`). Bundled because every `json_object` arg emitter below appends to both
/// together, in lockstep, never to one alone. Unlike `KwargRenderContext` this is mutated on
/// every call, so it holds `&mut` fields and is passed by `&mut` reference rather than `Copy`.
pub(in crate::e2e::codegen::python) struct ArgSink<'a> {
    pub bindings: &'a mut Vec<String>,
    pub kwarg_exprs: &'a mut Vec<String>,
}

/// How a `json_object` argument's JSON value should become a Python expression -- the three
/// pieces of `alef.toml` call-config the branches of `emit_json_object_arg` dispatch on
/// together. Bundled because every branch needs some subset of exactly these three fields and
/// nothing else about the call.
pub(in crate::e2e::codegen::python) struct ConstructorSpec<'a> {
    pub options_type: Option<&'a str>,
    pub options_via: &'a str,
    pub element_type: &'a Option<String>,
}

/// Identifies the mock-server fixture a `json_object` argument's placeholder URL resolves
/// against. Only consulted once `value_contains_mock_url_placeholder` finds a placeholder to
/// substitute, so it stays its own small struct rather than folding into `ConstructorSpec`
/// (which every call needs) or `KwargRenderContext` (which describes IR, not the fixture).
pub(in crate::e2e::codegen::python) struct MockUrlInfo<'a> {
    pub fixture_id: &'a str,
    pub has_host_root_route: bool,
}

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
pub(in crate::e2e::codegen::python) fn render_kwarg_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    pointer: &str,
    context: KwargRenderContext<'_>,
    used_struct_types: &mut BTreeSet<String>,
) -> String {
    if let Some(rendered) = render_enum_field_value(field_name, value, containing_type, context) {
        return rendered;
    }
    if let Some(rendered) = render_docs_file_field_value(pointer, context.docs_files) {
        return rendered;
    }
    if let Some(rendered) =
        render_nested_container_field_value(field_name, value, containing_type, pointer, context, used_struct_types)
    {
        return rendered;
    }

    json_to_python_literal(value)
}

/// Tries each nested-container shape in turn -- single struct, array-of-structs, then
/// map-of-structs. The three share an identical signature: each resolves `field_name`'s declared
/// type against `context.type_defs` and, on a match, recurses through [`render_struct_constructor`].
/// Split out of [`render_kwarg_field_value`] to keep that function under the file's per-function
/// line limit; grouping the three container shapes here (rather than inlining each) is what keeps
/// the split effective.
fn render_nested_container_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    pointer: &str,
    context: KwargRenderContext<'_>,
    used_struct_types: &mut BTreeSet<String>,
) -> Option<String> {
    if let Some(rendered) =
        render_nested_struct_field_value(field_name, value, containing_type, pointer, context, used_struct_types)
    {
        return Some(rendered);
    }
    if let Some(rendered) =
        render_nested_array_field_value(field_name, value, containing_type, pointer, context, used_struct_types)
    {
        return Some(rendered);
    }
    render_nested_map_field_value(field_name, value, containing_type, pointer, context, used_struct_types)
}

/// Enum branch of [`render_kwarg_field_value`]: an explicitly configured `enum_fields` entry, or
/// an auto-detected enum field type, renders as `EnumType("variant")`. Mirrors the original
/// inline logic exactly -- an `enum_fields` hit with a non-string value falls through to the
/// remaining branches rather than trying auto-detection.
fn render_enum_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    context: KwargRenderContext<'_>,
) -> Option<String> {
    if let Some(enum_type) = context.enum_fields.get(field_name) {
        if let Some(s) = value.as_str() {
            return Some(format!("{enum_type}(\"{s}\")"));
        }
    } else if let Some(auto_enum_type) =
        resolve_field_enum_type(field_name, containing_type, context.type_defs, context.enums)
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
fn render_nested_struct_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    pointer: &str,
    context: KwargRenderContext<'_>,
    used_struct_types: &mut BTreeSet<String>,
) -> Option<String> {
    let nested = resolve_field_struct_type(field_name, containing_type, context.type_defs)?;
    let obj = value.as_object()?;
    Some(render_struct_constructor(nested, obj, pointer, context, used_struct_types))
}

/// Nested-array branch of [`render_kwarg_field_value`]: a field typed `Vec<Struct>` (optionally
/// `Optional`) renders as a Python list of that class's constructor calls.
fn render_nested_array_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    pointer: &str,
    context: KwargRenderContext<'_>,
    used_struct_types: &mut BTreeSet<String>,
) -> Option<String> {
    let elem = resolve_field_element_struct_type(field_name, containing_type, context.type_defs)?;
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
            render_struct_constructor(elem, obj, &item_pointer, context, used_struct_types)
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
fn render_nested_map_field_value(
    field_name: &str,
    value: &serde_json::Value,
    containing_type: Option<&str>,
    pointer: &str,
    context: KwargRenderContext<'_>,
    used_struct_types: &mut BTreeSet<String>,
) -> Option<String> {
    let elem = resolve_field_map_value_struct_type(field_name, containing_type, context.type_defs)?;
    let map_obj = value.as_object()?;
    if map_obj.values().any(|entry| !entry.is_object()) {
        return None;
    }
    let items: Vec<String> = map_obj
        .iter()
        .map(|(key, entry)| {
            let entry_obj = entry.as_object().expect("checked above: every entry is an object");
            let entry_pointer = format!("{pointer}/{}", escape_json_pointer(key));
            let ctor = render_struct_constructor(elem, entry_obj, &entry_pointer, context, used_struct_types);
            format!("\"{}\": {ctor}", escape_python(key))
        })
        .collect();
    Some(format!("{{{}}}", items.join(", ")))
}

/// Build a `TypeName(field=value, ...)` constructor call for `type_def`, recursing through
/// [`render_kwarg_field_value`] for each field so arbitrarily deep nested config types resolve
/// the same way at every depth.
fn render_struct_constructor(
    type_def: &crate::core::ir::TypeDef,
    obj: &serde_json::Map<String, serde_json::Value>,
    pointer: &str,
    context: KwargRenderContext<'_>,
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
                &field_pointer,
                context,
                used_struct_types,
            );
            format!("{snake_key}={rendered}")
        })
        .collect();
    format!("{}({})", type_def.name, kwargs.join(", "))
}

/// Returns `true` if the arg was fully emitted (caller should `continue`).
pub(super) fn emit_json_object_arg(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    spec: &ConstructorSpec<'_>,
    mock: &MockUrlInfo<'_>,
    context: KwargRenderContext<'_>,
) -> bool {
    if crate::e2e::codegen::value_contains_mock_url_placeholder(value) {
        return emit_json_object_arg_with_mock_url(sink, value, var_name, spec, mock);
    }

    match spec.options_via {
        "dict" => emit_json_object_arg_dict_mode(sink, value, var_name, spec.element_type),
        "json" => emit_json_object_arg_json_mode(sink, value, var_name),
        "from_json" => emit_json_object_arg_from_json_mode(sink, value, var_name, spec.options_type),
        _ => emit_json_object_arg_default_mode(sink, value, var_name, spec, context),
    }
}

/// `options_via = "dict"` branch: an array of objects paired with `element_type` emits plain
/// dict literals (the bindings expect `[{"type": "click", ...}, ...]`, not constructor calls);
/// anything else falls back to a single JSON literal for the whole value.
fn emit_json_object_arg_dict_mode(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    element_type: &Option<String>,
) -> bool {
    if let (Some(_elem_type), Some(arr)) = (element_type, value.as_array())
        && !arr.is_empty()
        && arr.iter().all(|v| v.is_object())
    {
        let items: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_object())
            .map(emit_python_object_item)
            .collect();
        sink.bindings.push(format!("    {var_name} = [{}]", items.join(", ")));
        sink.kwarg_exprs.push(var_name.to_string());
        return true;
    }
    let literal = json_to_python_literal(value);
    let noqa = if literal.contains("/tmp/") { "  # noqa: S108" } else { "" };
    sink.bindings.push(format!("    {var_name} = {literal}{noqa}"));
    sink.kwarg_exprs.push(var_name.to_string());
    true
}

/// `options_via = "json"` branch: the value round-trips through `json.loads(...)`.
fn emit_json_object_arg_json_mode(sink: &mut ArgSink<'_>, value: &serde_json::Value, var_name: &str) -> bool {
    let json_str = serde_json::to_string(value).unwrap_or_default();
    let escaped = escape_python(&json_str);
    sink.bindings.push(format!("    {var_name} = json.loads(\"{escaped}\")"));
    sink.kwarg_exprs.push(var_name.to_string());
    true
}

/// `options_via = "from_json"` branch: the value round-trips through the configured type's
/// `from_json(...)` classmethod. Requires `options_type`; without it there is no method to call,
/// so the caller falls back to the remaining arg-emission paths.
fn emit_json_object_arg_from_json_mode(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    options_type: Option<&str>,
) -> bool {
    let Some(opts_type) = options_type else {
        return false;
    };
    let json_str = serde_json::to_string(value).unwrap_or_default();
    let escaped = escape_python(&json_str);
    sink.bindings
        .push(format!("    {var_name} = {opts_type}.from_json(\"{escaped}\")"));
    sink.kwarg_exprs.push(var_name.to_string());
    true
}

/// Default (`options_via` unset or unrecognized) branch: either a "batch" array of typed items
/// (`element_type`), or a single "kwargs"-mode constructor call (`options_type`).
fn emit_json_object_arg_default_mode(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    spec: &ConstructorSpec<'_>,
    context: KwargRenderContext<'_>,
) -> bool {
    if emit_json_object_arg_typed_array(sink, value, var_name, spec.element_type, context) {
        return true;
    }
    emit_json_object_arg_typed_kwargs(sink, value, var_name, spec.options_type, context)
}

/// Batch-array sub-branch of the default mode: an array of objects paired with `element_type`
/// constructs a typed instance per item via [`emit_python_typed_instance`].
fn emit_json_object_arg_typed_array(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    element_type: &Option<String>,
    context: KwargRenderContext<'_>,
) -> bool {
    let Some(elem_type) = element_type else {
        return false;
    };
    if value.is_null() {
        return false;
    }
    let Some(arr) = value.as_array() else {
        return false;
    };
    if !arr.iter().all(|item| item.is_object()) {
        return false;
    }
    let items: Vec<String> = arr
        .iter()
        .filter_map(|item| item.as_object())
        .enumerate()
        .map(|(index, obj)| {
            let pointer = format!("/{index}");
            emit_python_typed_instance(obj, elem_type, &pointer, context)
        })
        .collect();
    sink.bindings.push(format!("    {var_name} = [{}]", items.join(", ")));
    sink.kwarg_exprs.push(var_name.to_string());
    true
}

/// Single-object sub-branch of the default mode: a "kwargs"-mode constructor call, recursing
/// through [`render_kwarg_field_value`] for every field.
fn emit_json_object_arg_typed_kwargs(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    options_type: Option<&str>,
    context: KwargRenderContext<'_>,
) -> bool {
    let (Some(opts_type), Some(obj)) = (options_type, value.as_object()) else {
        return false;
    };
    let mut used_struct_types = BTreeSet::new();
    let kwargs: Vec<String> = obj
        .iter()
        .map(|(k, v)| {
            let snake_key = k.to_snake_case();
            let field_pointer = format!("/{}", escape_json_pointer(k));
            let py_val =
                render_kwarg_field_value(k, v, Some(opts_type), &field_pointer, context, &mut used_struct_types);
            format!("{snake_key}={py_val}")
        })
        .collect();
    let constructor = format!("{opts_type}({})", kwargs.join(", "));
    sink.bindings.push(format!("    {var_name} = {constructor}"));
    sink.kwarg_exprs.push(var_name.to_string());
    true
}

fn emit_json_object_arg_with_mock_url(
    sink: &mut ArgSink<'_>,
    value: &serde_json::Value,
    var_name: &str,
    spec: &ConstructorSpec<'_>,
    mock: &MockUrlInfo<'_>,
) -> bool {
    let json_str = serde_json::to_string(value).unwrap_or_default();
    let escaped = escape_python(&json_str);
    let env_key = crate::e2e::codegen::mock_url_env_key(mock.fixture_id);
    let fallback = format!(
        "os.environ['MOCK_SERVER_URL'] + '/fixtures/{}'",
        mock.fixture_id
    );
    let base_expr = if mock.has_host_root_route {
        format!("os.environ.get('{env_key}') or {fallback}")
    } else {
        fallback
    };
    sink.bindings.push(format!("    {var_name}_mock_base_url = {base_expr}"));
    sink.bindings.push(format!(
        "    {var_name}_json = \"{escaped}\".replace(\"{}\", {var_name}_mock_base_url)",
        crate::e2e::codegen::MOCK_URL_PLACEHOLDER
    ));

    match (spec.options_via, spec.options_type) {
        ("from_json", Some(opts_type)) => {
            sink.bindings
                .push(format!("    {var_name} = {opts_type}.from_json({var_name}_json)"));
        }
        ("dict", _) | (_, None) | ("json", _) => {
            sink.bindings.push(format!("    {var_name} = json.loads({var_name}_json)"));
        }
        (_, Some(opts_type)) => {
            sink.bindings
                .push(format!("    {var_name} = {opts_type}(**json.loads({var_name}_json))"));
        }
    }
    sink.kwarg_exprs.push(var_name.to_string());
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
    pointer: &str,
    context: KwargRenderContext<'_>,
) -> String {
    let mut used_struct_types = BTreeSet::new();
    let kwargs: Vec<String> = obj
        .iter()
        .map(|(k, v)| {
            let snake_key = k.to_snake_case();
            let field_pointer = format!("{pointer}/{}", escape_json_pointer(k));
            let rendered =
                render_kwarg_field_value(k, v, Some(elem_type), &field_pointer, context, &mut used_struct_types);
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
