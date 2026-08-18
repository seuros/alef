use crate::core::config::GleamElementConstructor;
use crate::core::ir::{EnumDef, TypeDef};
use crate::e2e::codegen::call_ir::TargetParams;
use crate::e2e::escape::escape_gleam;
use heck::ToSnakeCase;

use super::constructors::render_gleam_element_constructor;
use super::values::json_to_gleam;

/// Build setup lines and the argument list for the function call.
///
/// Returns `Err(reason)` when the test must be skipped entirely, and the caller emits that
/// reason as a `// skipped` comment with a `Nil` body rather than broken code. Two things
/// produce it: a `json_object` arg with no element-constructor recipe, no
/// `json_object_wrapper` and no `from_json` route (the generated call would pass a raw JSON
/// string where the Gleam binding expects a typed record), and a *declared-type* mismatch --
/// see [`unrepresentable_named_param`].
///
/// Gleam is statically typed, so each arg type must produce a correctly-typed expression:
/// - `file_path` -> quoted string literal
/// - `bytes` -> setup: `let assert Ok(data__) = e2e_gleam.read_file_bytes(...)` and arg: `data__`
/// - `string` + optional -> `option.Some("value")` or `option.None`
/// - `string` non-optional -> `"value"`
/// - `json_object` with recipe -> list/record constructor from `element_constructors`
/// - `json_object` with wrapper -> JSON-string literal wrapped by `json_object_wrapper`
/// - `json_object` with `options_via = "from_json"` -> `<snake_type>_from_json("{json}")` NIF call
/// - `json_object` without recipe, wrapper, or from_json -> caller is signalled to skip
#[allow(clippy::too_many_arguments)]
pub(super) fn build_args_and_setup(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    fixture_id: &str,
    test_documents_path: &str,
    element_constructors: &[GleamElementConstructor],
    json_object_wrapper: Option<&str>,
    module_path: &str,
    extra_args: &[String],
    options_type: Option<&str>,
    options_via: &str,
    preserve_input_urls: bool,
    target_params: TargetParams<'_>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Result<(Vec<String>, String), String> {
    if args.is_empty() && extra_args.is_empty() {
        return Ok((Vec::new(), String::new()));
    }

    // Pre-check: if any json_object arg has no recipe, wrapper, or from_json override,
    // the call cannot be expressed in Gleam. Signal the caller to skip.
    for (index, arg) in args.iter().enumerate() {
        let arg_field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let arg_value = input.get(arg_field);
        if let Some(declared) = unrepresentable_named_param(arg, index, arg_value, target_params, type_defs, enums) {
            return Err(format!(
                "arg `{}` fills a parameter the core IR declares as `{declared}`, but its `arg_type` \
                 `{}` lowers to a bare literal; Gleam is statically typed and has no way to build a \
                 `{declared}` from one here",
                arg.name, arg.arg_type,
            ));
        }
        if arg.arg_type == "json_object" {
            let element_type = arg.element_type.as_deref().unwrap_or("");
            let has_recipe =
                !element_type.is_empty() && element_constructors.iter().any(|r| r.element_type == element_type);
            let has_wrapper = json_object_wrapper.is_some();
            let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
            let val = input.get(field).unwrap_or(&serde_json::Value::Null);
            let has_from_json = options_via == "from_json"
                && crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, val).is_some();
            // An optional json_object with no value can safely emit option.None / [].
            let val = input.get(field);
            let is_null_optional = arg.optional && matches!(val, None | Some(serde_json::Value::Null));
            if !has_recipe && !has_wrapper && !has_from_json && !is_null_optional {
                return Err(format!(
                    "json_object arg `{}` has no element-constructor recipe, no `json_object_wrapper` \
                     and no `from_json` route, so the call would pass a raw JSON string where the \
                     Gleam binding expects a typed record",
                    arg.name,
                ));
            }
        }
    }

    let mut setup_lines: Vec<String> = Vec::new();
    let mut parts: Vec<String> = Vec::new();
    let mut bytes_var_counter = 0usize;

    for arg in args {
        let field = arg.field.strip_prefix("input.").unwrap_or(&arg.field);
        let val = input.get(field);

        match arg.arg_type.as_str() {
            "handle" => {
                // Engine construction: create_engine(option.None).
                // Config construction from JSON is complex in Gleam (no JSON string constructor),
                // so we always pass option.None — default engine config covers most test cases.
                let name = &arg.name;
                let constructor = format!("create_{}", name.to_snake_case());
                setup_lines.push(format!(
                    "let assert Ok({name}) = {module_path}.{constructor}(option.None)"
                ));
                parts.push(name.clone());
                continue;
            }
            "mock_url" => {
                if let Some(url) = crate::e2e::codegen::preserved_url_literal(
                    preserve_input_urls,
                    val.unwrap_or(&serde_json::Value::Null),
                ) {
                    parts.push(format!("\"{}\"", escape_gleam(url)));
                    continue;
                }
                // Resolve the mock server base URL at runtime via envoy, then append the fixture path.
                let name = &arg.name;
                setup_lines.push(format!(
                    "let {name} = case envoy.get(\"MOCK_SERVER_URL\") {{ Ok(base) -> base <> \"/fixtures/{fixture_id}\" Error(_) -> \"http://localhost:8080/fixtures/{fixture_id}\" }}"
                ));
                parts.push(name.clone());
                continue;
            }
            "mock_url_list" => {
                let value = crate::e2e::codegen::resolve_urls_field(input, &arg.field);
                if let Some(urls) = crate::e2e::codegen::preserved_url_list(preserve_input_urls, value) {
                    let values = urls
                        .iter()
                        .map(|url| format!("\"{}\"", escape_gleam(url)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    parts.push(format!("[{values}]"));
                    continue;
                }
                let name = &arg.name;
                let values = value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .map(|path| format!("base <> \"{}\"", escape_gleam(path)))
                    .collect::<Vec<_>>()
                    .join(", ");
                setup_lines.push(format!(
                    "let {name} = case envoy.get(\"MOCK_SERVER_URL\") {{ Ok(base) -> [{values}] Error(_) -> [] }}"
                ));
                parts.push(name.clone());
                continue;
            }
            "file_path" => {
                // Always a required string path.
                // Gleam e2e runs from e2e/gleam/ so the path resolves relative
                // to the configured test-documents directory.
                let path = val.and_then(|v| v.as_str()).unwrap_or("");
                let full_path = format!("{test_documents_path}/{path}");
                parts.push(format!("\"{}\"", escape_gleam(&full_path)));
            }
            "bytes" => {
                // Read the file at runtime via Erlang file:read_file/1.
                // The fixture `data` field holds the path relative to the
                // configured test-documents directory.
                let path = val.and_then(|v| v.as_str()).unwrap_or("");
                let var_name = if bytes_var_counter == 0 {
                    "data_bytes__".to_string()
                } else {
                    format!("data_bytes_{bytes_var_counter}__")
                };
                bytes_var_counter += 1;
                // Use relative path from e2e/gleam/ project root.
                let full_path = format!("{test_documents_path}/{path}");
                setup_lines.push(format!(
                    "let assert Ok({var_name}) = e2e_gleam.read_file_bytes(\"{}\")",
                    escape_gleam(&full_path)
                ));
                parts.push(var_name);
            }
            "string" if arg.optional => {
                // Optional string: emit option.Some("value") or option.None.
                match val {
                    None | Some(serde_json::Value::Null) => {
                        parts.push("option.None".to_string());
                    }
                    Some(serde_json::Value::String(s)) if s.is_empty() => {
                        parts.push("option.None".to_string());
                    }
                    Some(serde_json::Value::String(s)) => {
                        parts.push(format!("option.Some(\"{}\")", escape_gleam(s)));
                    }
                    Some(v) => {
                        parts.push(format!("option.Some({})", json_to_gleam(v)));
                    }
                }
            }
            "string" => {
                // Non-optional string.
                match val {
                    None | Some(serde_json::Value::Null) => {
                        parts.push("\"\"".to_string());
                    }
                    Some(serde_json::Value::String(s)) => {
                        parts.push(format!("\"{}\"", escape_gleam(s)));
                    }
                    Some(v) => {
                        parts.push(json_to_gleam(v));
                    }
                }
            }
            "json_object" => {
                // from_json path: use `<snake_type>_from_json(json)` NIF.
                if options_via == "from_json" {
                    let empty_obj = serde_json::Value::Object(Default::default());
                    let config_val = val.unwrap_or(&empty_obj);
                    if let Some(opts_type) =
                        crate::e2e::codegen::recipe::json_object_constructor_type(arg, options_type, config_val)
                    {
                        if !config_val.is_null() {
                            let snake_opts = opts_type.to_snake_case();
                            let json_str = serde_json::to_string(config_val).unwrap_or_default();
                            let escaped = escape_gleam(&json_str);
                            let var_name = format!("{}_json__", arg.name);
                            setup_lines.push(format!(
                                "let assert Ok({var_name}) = {module_path}.{snake_opts}_from_json(\"{escaped}\")"
                            ));
                            parts.push(var_name);
                        }
                        continue;
                    }
                }

                // Look up a per-`element_type` constructor recipe declared in
                // `[crates.gleam.element_constructors]`. When present, build a
                // record literal from the recipe; otherwise fall back to a
                // generic JSON-string emission via `json_to_gleam`.
                let element_type = arg.element_type.as_deref().unwrap_or("");
                let recipe = if element_type.is_empty() {
                    None
                } else {
                    element_constructors.iter().find(|r| r.element_type == element_type)
                };

                if let Some(recipe) = recipe {
                    // List-of-records emission: each JSON-array item becomes
                    // one constructor call; non-array values produce an empty
                    // list (preserving the iter15 behaviour).
                    let items_expr = match val {
                        Some(serde_json::Value::Array(arr)) => {
                            let items: Vec<String> = arr
                                .iter()
                                .map(|item| render_gleam_element_constructor(item, recipe, test_documents_path))
                                .collect();
                            format!("[{}]", items.join(", "))
                        }
                        _ => "[]".to_string(),
                    };
                    if arg.optional && (val.is_none() || val == Some(&serde_json::Value::Null)) {
                        parts.push("[]".to_string());
                    } else {
                        parts.push(items_expr);
                    }
                } else if arg.optional && (val.is_none() || val == Some(&serde_json::Value::Null)) {
                    parts.push("option.None".to_string());
                } else {
                    let empty_obj = serde_json::Value::Object(Default::default());
                    let config_val = val.unwrap_or(&empty_obj);
                    let json_literal = json_to_gleam(config_val);
                    // When the project has configured a wrapper (e.g.
                    // `sample_core.config_from_json_string({json})`), substitute
                    // the placeholder; otherwise emit the bare JSON-string
                    // literal.
                    let emitted = match json_object_wrapper {
                        Some(template) => template.replace("{json}", &json_literal),
                        None => json_literal,
                    };
                    parts.push(emitted);
                }
            }
            "int" | "integer" => match val {
                None | Some(serde_json::Value::Null) if arg.optional => {}
                None | Some(serde_json::Value::Null) => parts.push("0".to_string()),
                Some(v) => parts.push(json_to_gleam(v)),
            },
            "bool" | "boolean" => match val {
                Some(serde_json::Value::Bool(true)) => parts.push("True".to_string()),
                Some(serde_json::Value::Bool(false)) | None | Some(serde_json::Value::Null) => {
                    if !arg.optional {
                        parts.push("False".to_string());
                    }
                }
                Some(v) => parts.push(json_to_gleam(v)),
            },
            _ => {
                // Fallback for unknown types.
                match val {
                    None | Some(serde_json::Value::Null) if arg.optional => {}
                    None | Some(serde_json::Value::Null) => parts.push("Nil".to_string()),
                    Some(v) => parts.push(json_to_gleam(v)),
                }
            }
        }
    }

    // Append verbatim extra_args (e.g. "option.None" for optional query params
    // like `list_files(client, query)` where gleam needs `option.None`).
    for extra in extra_args {
        parts.push(extra.clone());
    }

    Ok((setup_lines, parts.join(", ")))
}

/// The IR type name of a declared parameter this arg cannot be lowered into, or `None` when
/// the arg is renderable.
///
/// Gleam is the one backend with *nothing* to consult before this seam existed: its arg builder
/// took no `type_defs`, no `enums` and no `functions`, so a `"string"` arg (the `arg_type`
/// default) filling a record- or enum-typed parameter emitted a quoted literal that the Gleam
/// compiler rejects outright.
///
/// The answer here is Gleam's own, not a shared verdict: refuse, reusing the skip channel this
/// module already had for unrepresentable `json_object` args. Emitting a constructor instead
/// would mean reproducing `backends::gleam::…::variant_constructor_name`, whose spelling depends
/// on a cross-enum collision set this module does not have -- a guess that compiles only by
/// luck. A skip is always valid Gleam and is visible in the generated file. ~keep
///
/// Only IR-*known* names refuse. A named type absent from both registries may be a newtype the
/// binding flattens to a plain string, and refusing on it would skip tests that compile today.
/// Arg kinds that build their own typed expression are exempt for the same reason. ~keep
fn unrepresentable_named_param(
    arg: &crate::e2e::config::ArgMapping,
    index: usize,
    value: Option<&serde_json::Value>,
    target_params: TargetParams<'_>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> Option<String> {
    if matches!(
        arg.arg_type.as_str(),
        "json_object" | "handle" | "bytes" | "file_path" | "mock_url" | "mock_url_list" | "test_backend"
    ) {
        return None;
    }
    // An optional parameter the fixture leaves unset renders as `option.None` (or is omitted),
    // which is well-typed against `Option<Record>` no matter what the record is -- the value is
    // never lowered, so there is nothing to refuse. ~keep
    if arg.optional && matches!(value, None | Some(serde_json::Value::Null)) {
        return None;
    }
    let declared = target_params.declared_type_name(&arg.name, index)?;
    let known_to_ir =
        type_defs.iter().any(|ty| ty.name == declared) || enums.iter().any(|enum_def| enum_def.name == declared);
    known_to_ir.then(|| declared.to_string())
}
