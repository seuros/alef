use super::*;

/// Build a TypeScript expression to construct an options object.
///
/// Node: configured options types can be TypeScript interfaces — return a plain object literal
/// with a type assertion (`{ key: val } as TypeName`). No Update class or fromUpdate().
///
/// WASM: alef-backend-wasm does not emit `*Update` builder classes, so we
/// instantiate the main type directly. Every wasm-bindgen-emitted struct
/// exposes an all-optional positional constructor (`new T()`) plus per-field
/// setters, so we build the value with `new T()` followed by setter
/// assignments wrapped in an IIFE so the expression can be inlined as a
/// function argument. Nested object values follow the same pattern.
#[allow(clippy::too_many_arguments)]
pub(in crate::e2e::codegen::typescript::test_file) fn ts_builder_expression(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    nested_types: &std::collections::HashMap<String, String>,
    lang: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    wasm_type_prefix: &str,
    docs_files: &[crate::e2e::fixture::FixtureDocsFileInput],
) -> String {
    ts_builder_expression_inner(
        obj,
        type_name,
        nested_types,
        lang,
        enum_fields,
        bigint_fields,
        type_defs,
        enums,
        wasm_type_prefix,
        docs_files,
        "",
        0,
    )
}

/// True when `type_name` (possibly with a `Wasm` binding-prefix) names an
/// IR enum that uses serde's internally-tagged representation
/// (`#[serde(tag = "...")]`) and has at least one variant carrying data.
///
/// WASM bindings expose such enums via field setters of type
/// `JsValue`/`Option<JsValue>`, which `serde_wasm_bindgen::from_value` then
/// deserializes from a plain JS object. Wrapping the value with the
/// per-variant `default()` factory + setters produces an opaque
/// wasm-bindgen wrapper class whose own-property table is empty — serde
/// then fails to read the discriminator. The e2e builder must emit a plain
/// JS object literal for these instead.
fn is_tagged_data_enum(type_name: &str, enums: &[EnumDef], wasm_type_prefix: &str) -> bool {
    let stripped = type_name.strip_prefix(wasm_type_prefix).unwrap_or(type_name);
    enums
        .iter()
        .any(|e| e.name == stripped && e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty()))
}

/// Pre-process a JSON value so that napi-rs (node) binding can deserialize it.
///
/// The napi-rs backend always emits `#[napi(js_name = "kind")]` for the
/// discriminant field of every tagged-data enum, regardless of the original
/// Rust `#[serde(tag = "...")]` attribute. For example, `Message` has
/// `#[serde(tag = "role")]`, but `JsMessage.role_tag` is exposed to
/// TypeScript as `"kind"`. A fixture that sends `{ role: "user" }` causes
/// napi-rs to return `Error: Missing field 'kind'`.
///
/// This function walks the JSON tree and renames any serde_tag key to
/// `"kind"` when the key's value is a string that matches a known variant
/// of the corresponding tagged-data enum. Renaming is limited to exact
/// variant matches so that plain struct fields that happen to share the
/// same key name as a serde_tag (e.g. `type: "function"` on
/// `ChatCompletionTool` where "function" is not a `ContentPart` variant)
/// are left unchanged.
pub(in crate::e2e::codegen::typescript::test_file) fn rename_napi_serde_tags_to_kind(
    value: &serde_json::Value,
    enums: &[EnumDef],
) -> serde_json::Value {
    // Build map: serde_tag_key → (set of variant serde-names, actual_tag_name).
    // Only include tagged-data enums (serde_tag present AND at least one
    // variant with fields so the binding is a flattened struct, not a plain
    // string enum).
    let mut tag_map: std::collections::HashMap<&str, (std::collections::HashSet<String>, &str)> =
        std::collections::HashMap::new();
    for e in enums {
        if let Some(tag) = e.serde_tag.as_deref()
            && e.variants.iter().any(|v| !v.fields.is_empty())
        {
            let variants: std::collections::HashSet<String> = e
                .variants
                .iter()
                .map(|v| v.serde_rename.as_deref().unwrap_or(&v.name).to_string())
                .collect();
            tag_map.insert(tag, (variants, tag));
        }
    }

    rename_napi_serde_tags_recursive(value, &tag_map)
}

fn rename_napi_serde_tags_recursive(
    value: &serde_json::Value,
    tag_map: &std::collections::HashMap<&str, (std::collections::HashSet<String>, &str)>,
) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (key, val) in map {
                // Preserve the original serde_tag key name when:
                //  1. the key is a known serde_tag name, AND
                //  2. the value is a string that matches a known variant of that enum.
                // The actual tag field name is already correct in the fixture; we only need
                // to validate and recurse.
                let new_key = key.clone();
                if let Some((variants, _)) = tag_map.get(key.as_str())
                    && !val.as_str().is_some_and(|s| variants.contains(s))
                {
                    // Not a valid variant value for this tag; leave as-is and recurse
                }
                new_map.insert(new_key, rename_napi_serde_tags_recursive(val, tag_map));
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.iter()
                .map(|item| rename_napi_serde_tags_recursive(item, tag_map))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Convert a JS numeric literal expression to a BigInt-compatible literal
/// (`123n`, `-7n`) for wasm-bindgen `u64`/`i64` setters which reject Number.
/// Non-integer or non-numeric expressions are wrapped in `BigInt(...)` so the
/// runtime conversion still happens.
fn to_bigint_literal(value_expr: &str) -> String {
    let trimmed = value_expr.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return format!("{trimmed}n");
    }
    if let Some(rest) = trimmed.strip_prefix('-')
        && !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit())
    {
        return format!("-{rest}n");
    }
    format!("BigInt({trimmed})")
}

#[allow(clippy::too_many_arguments)]
pub(in crate::e2e::codegen::typescript::test_file) fn ts_builder_expression_inner(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    nested_types: &std::collections::HashMap<String, String>,
    lang: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    wasm_type_prefix: &str,
    docs_files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
    depth: usize,
) -> String {
    // Use a depth-indexed variable name so nested IFEs don't shadow each other.
    // Without this, `const _u = WasmOptions.default(); _u.preprocessing =
    // (() => { const _u = WasmOptions.default(); ... })()` triggers
    // oxlint `no-shadow` on every nested-options expression.
    let var = format!("_u{depth}");
    if lang == "node" || (lang == "wasm" && is_tagged_data_enum(type_name, enums, wasm_type_prefix)) {
        // For node: if this type itself is a tagged-data enum, rename its serde_tag
        // key to "kind". The napi-rs backend hardcodes `#[napi(js_name = "kind")]`
        // for every tagged-data enum discriminant, regardless of the original
        // `#[serde(tag = "...")]` attribute. For wasm tagged-data enums the plain
        // JS object is deserialized via serde_wasm_bindgen which reads the original
        // serde_tag name, so the rename only applies to the node language path.
        let serde_tag_for_this_type = if lang == "node" {
            let ir_name = type_name.strip_prefix(wasm_type_prefix).unwrap_or(type_name);
            enums
                .iter()
                .find(|e| e.name == ir_name && e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty()))
                .and_then(|e| e.serde_tag.as_deref())
        } else {
            None
        };

        let mut fields = Vec::new();
        let owner_type = type_defs.iter().find(|definition| definition.name == type_name);
        for (key, val) in obj {
            let field_pointer = json_pointer_child(pointer, key);
            // Rename serde_tag key → "kind" for node-bound tagged-data enum objects.
            let js_key = if lang == "node" {
                match serde_tag_for_this_type {
                    Some(tag) if key == tag => "kind".to_string(),
                    _ => snake_to_camel(key),
                }
            } else {
                snake_to_camel(key)
            };
            let field_expr = if lang == "node" {
                // Apply the napi serde_tag rename recursively into nested objects
                // and arrays so that tagged-enum elements inside arrays also get
                // their discriminant renamed to "kind".
                let preprocessed = rename_napi_serde_tags_to_kind(val, enums);
                // If the field is an enum (e.g. urlEscapeStyle, codeBlockStyle),
                // napi-rs constants are PascalCase variant names. Fixtures may
                // use the lowercase wire form (e.g. "percent"); convert it.
                let camel_key = snake_to_camel(key);
                let enum_type = enum_fields
                    .get(key.as_str())
                    .or_else(|| enum_fields.get(camel_key.as_str()));
                if let Some(enum_type) = enum_type {
                    if let serde_json::Value::String(s) = &preprocessed {
                        format!("{enum_type}.{}", s.to_upper_camel_case())
                    } else {
                        json_to_js(&preprocessed)
                    }
                } else {
                    let field_type = owner_type
                        .and_then(|definition| definition.fields.iter().find(|field| field.name == *key))
                        .map(|field| &field.ty);
                    node_value_expression(
                        &preprocessed,
                        key,
                        enum_fields,
                        docs_files,
                        &field_pointer,
                        field_type,
                        type_defs,
                        enums,
                    )
                }
            } else {
                match val {
                    serde_json::Value::Object(_) => json_to_js_camel(val),
                    _ => json_to_js(val),
                }
            };
            fields.push(format!("{js_key}: {field_expr}"));
        }
        let obj_literal = format!("{{ {} }}", fields.join(", "));
        return format!("{obj_literal} as {type_name}");
    }

    // WASM path: construct the main type via its synthetic `default()` static
    // factory rather than `new WasmFoo()`. wasm-bindgen's `(constructor)` mirrors
    // the Rust ctor's arity, so any struct with a non-Optional field requires
    // positional args — `new WasmChatCompletionTool()` (no args) throws
    // because `tool_type` and `function` are required. The `default()` factory
    // (emitted unconditionally on every wasm wrapper that derives `Default`)
    // returns a fresh instance the test body can then drive via setters.
    let init_stmt = if type_name.starts_with("Wasm") {
        format!("const {var} = {type_name}.default();")
    } else {
        format!("const {var} = new {type_name}();")
    };

    // Build derived nested_types from the IR registry and merge with the
    // explicit overrides (explicit wins on collision).
    let derived = derive_nested_types_for_wasm(type_name, type_defs, wasm_type_prefix);
    let effective_nested_types: std::collections::HashMap<String, String> = {
        let mut m = derived;
        for (k, v) in nested_types {
            m.insert(k.clone(), v.clone());
        }
        m
    };

    let mut stmts: Vec<String> = vec![init_stmt];
    let ir_owner_name = type_name.strip_prefix(wasm_type_prefix).unwrap_or(type_name);
    let owner_type = type_defs.iter().find(|definition| definition.name == ir_owner_name);
    for (key, val) in obj {
        let camel_key = snake_to_camel(key);
        let field_pointer = json_pointer_child(pointer, key);
        let field_type = owner_type
            .and_then(|definition| definition.fields.iter().find(|field| field.name == *key))
            .map(|field| match &field.ty {
                crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
                other => other,
            });
        if let Some(file) = docs_files.iter().find(|file| file.field == field_pointer) {
            stmts.push(
                crate::e2e::template_env::render(
                    "typescript/docs_file_assignment.jinja",
                    minijinja::context! { target => format!("{var}.{camel_key}"), path => escape_js(&file.path) },
                )
                .trim_end()
                .to_string(),
            );
            continue;
        }
        let is_bigint = bigint_fields.contains(&camel_key) || bigint_fields.contains(key);
        if let serde_json::Value::Object(nested_obj) = val {
            if let Some(nested_type) = effective_nested_types.get(key.as_str()) {
                let nested_expr = ts_builder_expression_inner(
                    nested_obj,
                    nested_type,
                    nested_types,
                    lang,
                    enum_fields,
                    bigint_fields,
                    type_defs,
                    enums,
                    wasm_type_prefix,
                    docs_files,
                    &field_pointer,
                    depth + 1,
                );
                stmts.push(format!("{var}.{camel_key} = {nested_expr};"));
            } else {
                stmts.push(format!("{var}.{camel_key} = {};", json_to_js_camel(val)));
            }
        } else if let serde_json::Value::Array(items) = val {
            // wasm-bindgen rejects plain object literals where it expects class
            // instances. When the array element type is a known binding class
            // (registered in `effective_nested_types`), wrap each object element
            // via the same builder-expression emitter; primitive elements pass
            // through as JS literals.
            if matches!(field_type, Some(crate::core::ir::TypeRef::Bytes)) {
                stmts.push(format!("{var}.{camel_key} = Uint8Array.from({});", json_to_js(val)));
            } else if let Some(elem_type) = effective_nested_types.get(key.as_str()) {
                let element_exprs: Vec<String> = items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        if let serde_json::Value::Object(item_obj) = item {
                            ts_builder_expression_inner(
                                item_obj,
                                elem_type,
                                nested_types,
                                lang,
                                enum_fields,
                                bigint_fields,
                                type_defs,
                                enums,
                                wasm_type_prefix,
                                docs_files,
                                &json_pointer_child(&field_pointer, &index.to_string()),
                                depth + 1,
                            )
                        } else {
                            json_to_js(item)
                        }
                    })
                    .collect();
                stmts.push(format!("{var}.{camel_key} = [{}];", element_exprs.join(", ")));
            } else {
                stmts.push(format!("{var}.{camel_key} = {};", json_to_js(val)));
            }
        } else if let Some(crate::core::ir::TypeRef::Named(enum_type)) = field_type
            && enums.iter().any(|definition| definition.name == *enum_type)
            && let serde_json::Value::String(variant) = val
        {
            let enum_type = wasm_prefixed_wrapped_type(lang, enum_type, type_defs, enums, wasm_type_prefix);
            stmts.push(format!(
                "{var}.{camel_key} = {enum_type}.{};",
                variant.to_upper_camel_case()
            ));
        } else if let Some(enum_type) = enum_fields
            .get(key.as_str())
            .or_else(|| enum_fields.get(camel_key.as_str()))
        {
            // This is an enum field — generate EnumType.EnumValue.
            // Look up by both snake_case (fixture key) and camelCase (alef.toml override key
            // convention) so the alef.toml `enum_fields = { codeBlockStyle = "..." }` style
            // matches fixtures written with snake_case keys.
            //
            // Prefix wasm-wrapped enums exactly as the typed branch above does:
            // the package exports `WasmExtractInputKind`, so a bare
            // `ExtractInputKind.Uri` references an undefined name.
            let enum_type = wasm_prefixed_wrapped_type(lang, enum_type, type_defs, enums, wasm_type_prefix);
            if let serde_json::Value::String(s) = val {
                stmts.push(format!("{var}.{camel_key} = {enum_type}.{};", s.to_upper_camel_case()));
            } else {
                stmts.push(format!("{var}.{camel_key} = {};", json_to_js(val)));
            }
        } else if is_bigint {
            // wasm-bindgen u64/i64 setters require BigInt. Plain numeric
            // literals must be suffixed with `n`; non-literal numeric
            // values are wrapped in `BigInt(...)`.
            let raw = json_to_js(val);
            stmts.push(format!("{var}.{camel_key} = {};", to_bigint_literal(&raw)));
        } else {
            stmts.push(format!("{var}.{camel_key} = {};", json_to_js(val)));
        }
    }

    stmts.push(format!("return {var};"));
    let body = stmts.join(" ");
    crate::e2e::template_env::render(
        "typescript/builder_iife.jinja",
        minijinja::context! { body => body, is_async => !docs_files.is_empty() },
    )
    .trim_end()
    .to_string()
}

fn node_value_expression(
    value: &serde_json::Value,
    field: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    docs_files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
    field_type: Option<&crate::core::ir::TypeRef>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
) -> String {
    if let Some(file) = docs_files.iter().find(|file| file.field == pointer) {
        return crate::e2e::template_env::render(
            "typescript/docs_file_expression.jinja",
            minijinja::context! { path => escape_js(&file.path) },
        )
        .trim_end()
        .to_string();
    }
    let field_type = field_type.map(|field_type| match field_type {
        crate::core::ir::TypeRef::Optional(inner) => inner.as_ref(),
        other => other,
    });
    if matches!(field_type, Some(crate::core::ir::TypeRef::Bytes)) {
        return format!("Uint8Array.from({})", json_to_js(value));
    }
    if let Some(crate::core::ir::TypeRef::Named(type_name)) = field_type
        && enums.iter().any(|definition| definition.name == *type_name)
        && let Some(variant) = value.as_str()
    {
        return format!("{type_name}.{}", variant.to_upper_camel_case());
    }
    let camel_field = snake_to_camel(field);
    if let Some(enum_type) = enum_fields.get(field).or_else(|| enum_fields.get(camel_field.as_str()))
        && let Some(variant) = value.as_str()
    {
        return format!("{enum_type}.{}", variant.to_upper_camel_case());
    }
    match value {
        serde_json::Value::Object(object) => {
            let nested_type = field_type
                .and_then(|field_type| match field_type {
                    crate::core::ir::TypeRef::Named(type_name) => Some(type_name.as_str()),
                    _ => None,
                })
                .and_then(|type_name| type_defs.iter().find(|definition| definition.name == type_name));
            let fields = object
                .iter()
                .map(|(name, value)| {
                    let nested_field_type = nested_type
                        .and_then(|definition| definition.fields.iter().find(|field| field.name == *name))
                        .map(|field| &field.ty);
                    format!(
                        "{}: {}",
                        snake_to_camel(name),
                        node_value_expression(
                            value,
                            name,
                            enum_fields,
                            docs_files,
                            &json_pointer_child(pointer, name),
                            nested_field_type,
                            type_defs,
                            enums,
                        )
                    )
                })
                .collect::<Vec<_>>();
            format!("{{ {} }}", fields.join(", "))
        }
        serde_json::Value::Array(values) => {
            let element_type = field_type.and_then(|field_type| match field_type {
                crate::core::ir::TypeRef::Vec(inner) => Some(inner.as_ref()),
                _ => None,
            });
            let values = values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    node_value_expression(
                        value,
                        "",
                        enum_fields,
                        docs_files,
                        &json_pointer_child(pointer, &index.to_string()),
                        element_type,
                        type_defs,
                        enums,
                    )
                })
                .collect::<Vec<_>>();
            format!("[{}]", values.join(", "))
        }
        _ => json_to_js(value),
    }
}

fn json_pointer_child(pointer: &str, field: &str) -> String {
    let field = field.replace('~', "~0").replace('/', "~1");
    format!("{pointer}/{field}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_strict_typescript_compiles(source: &str) {
        let directory = tempfile::tempdir().expect("temporary TypeScript project");
        let source_path = directory.path().join("snippet.ts");
        std::fs::write(&source_path, source).expect("write TypeScript regression source");
        let Ok(output) = std::process::Command::new("tsc")
            .args([
                "--strict",
                "--noUncheckedIndexedAccess",
                "--noEmit",
                "--target",
                "ES2022",
            ])
            .arg(&source_path)
            .output()
        else {
            return;
        };
        assert!(
            output.status.success(),
            "strict TypeScript rejected generated snippet:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn node_typed_objects_use_importable_enum_members() {
        let expression = ts_builder_expression(
            serde_json::json!({"kind": "uri"}).as_object().expect("object"),
            "DocumentInput",
            &Default::default(),
            "node",
            &[("kind".into(), "InputKind".into())].into_iter().collect(),
            &Default::default(),
            &[],
            &[],
            "",
            &[],
        );

        assert_eq!(expression, "{ kind: InputKind.Uri } as DocumentInput");
    }

    #[test]
    fn node_typed_objects_lower_bytes_and_enums_from_ir() {
        let type_defs = [TypeDef {
            name: "DocumentInput".into(),
            fields: vec![
                crate::core::ir::FieldDef {
                    name: "bytes".into(),
                    ty: crate::core::ir::TypeRef::Bytes,
                    ..Default::default()
                },
                crate::core::ir::FieldDef {
                    name: "kind".into(),
                    ty: crate::core::ir::TypeRef::Named("InputKind".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }];
        let enums = [EnumDef {
            name: "InputKind".into(),
            ..Default::default()
        }];
        let expression = ts_builder_expression(
            serde_json::json!({"bytes": [72, 105], "kind": "bytes"})
                .as_object()
                .expect("object"),
            "DocumentInput",
            &Default::default(),
            "node",
            &Default::default(),
            &Default::default(),
            &type_defs,
            &enums,
            "",
            &[],
        );

        assert_eq!(
            expression,
            "{ bytes: Uint8Array.from([72, 105]), kind: InputKind.Bytes } as DocumentInput"
        );

        let mut fields = std::collections::HashMap::new();
        fields.insert("content".to_string(), "results[0].content".to_string());
        let optional = ["results".to_string()].into_iter().collect();
        let resolver = crate::e2e::field_access::FieldResolver::new(
            &fields,
            &optional,
            &Default::default(),
            &Default::default(),
            &Default::default(),
        );
        let accessor = resolver.accessor("content", "node", "result");
        let source = format!(
            "enum InputKind {{ Bytes }}\ninterface DocumentInput {{ bytes: Uint8Array; kind: InputKind }}\ninterface Output {{ results?: Array<{{ content: string }}> }}\nconst input: DocumentInput = {expression};\ndeclare const result: Output;\nconst content: string | undefined = {accessor};\nvoid input; void content;\n"
        );
        assert_strict_typescript_compiles(&source);
    }

    #[test]
    fn wasm_typed_objects_lower_bytes_and_enums_from_ir() {
        let type_defs = [TypeDef {
            name: "ExtractInput".into(),
            fields: vec![
                crate::core::ir::FieldDef {
                    name: "bytes".into(),
                    ty: crate::core::ir::TypeRef::Bytes,
                    ..Default::default()
                },
                crate::core::ir::FieldDef {
                    name: "kind".into(),
                    ty: crate::core::ir::TypeRef::Named("ExtractInputKind".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }];
        let enums = [EnumDef {
            name: "ExtractInputKind".into(),
            ..Default::default()
        }];
        let expression = ts_builder_expression(
            serde_json::json!({"bytes": [72, 105], "kind": "bytes"})
                .as_object()
                .expect("object"),
            "WasmExtractInput",
            &Default::default(),
            "wasm",
            &Default::default(),
            &Default::default(),
            &type_defs,
            &enums,
            "Wasm",
            &[],
        );
        assert!(
            expression.contains("_u0.bytes = Uint8Array.from([72, 105])"),
            "{expression}"
        );
        assert!(
            expression.contains("_u0.kind = WasmExtractInputKind.Bytes"),
            "{expression}"
        );
        let source = format!(
            "enum WasmExtractInputKind {{ Bytes }}\nclass WasmExtractInput {{ static default(): WasmExtractInput {{ return new WasmExtractInput(); }} bytes!: Uint8Array; kind!: WasmExtractInputKind; }}\nconst input: WasmExtractInput = {expression};\nvoid input;\n"
        );
        assert_strict_typescript_compiles(&source);
    }

    #[test]
    fn node_and_wasm_typed_objects_read_documented_files() {
        let object = serde_json::json!({"bytes": "document.pdf"});
        let object = object.as_object().expect("object");
        let files = [crate::e2e::fixture::FixtureDocsFileInput {
            field: "/bytes".into(),
            path: "document.pdf".into(),
        }];
        for language in ["node", "wasm"] {
            let expression = ts_builder_expression(
                object,
                "DocumentInput",
                &Default::default(),
                language,
                &Default::default(),
                &Default::default(),
                &[],
                &[],
                "",
                &files,
            );
            assert!(
                expression.contains("readFile(\"document.pdf\")"),
                "{language}: {expression}"
            );
            assert!(
                !expression.contains("bytes: \"document.pdf\""),
                "{language}: {expression}"
            );
            if language == "wasm" {
                assert!(expression.starts_with("await (async () =>"), "{expression}");
            }
        }
    }
}
