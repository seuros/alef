mod enum_members;

use super::*;
use enum_members::{declared_enum_member_for_prefixed, is_tagged_data_enum, wasm_enum_bridged_as_raw_value};

use crate::e2e::codegen::fixture_refusal::RefusalSite;

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
    referenced_enums: &mut std::collections::BTreeSet<String>,
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
        referenced_enums,
    )
}

/// For a node-lang tagged-data enum whose matched variant wraps a single Named-type payload
/// (`enum Message { User(UserMessage), .. }` with `#[serde(tag = "role")]`), napi's `.d.ts`
/// union member nests that payload under a synthesized per-variant field
/// (`{ role: 'user'; user: UserMessage }`) rather than flattening its fields alongside the
/// tag (`{ role: 'user', content: '...' }`) — see `gen_tagged_enum_as_object`, which emits a
/// dedicated `Option<{prefix}{inner}>` field for exactly this shape (one struct-payload tuple
/// variant), keyed by `tagged_enum_binding_field_js_name` (variant/field `serde_rename`, else
/// the lower-camel-case variant name). Building the flattened wire-shape object and casting it
/// `as Message` type-checks against no union member, so `tsc` rejects it with TS2353.
///
/// Returns `None` for anything that doesn't need this treatment (unit variants, struct
/// variants, multi-field tuple variants, or a tag value with no matching variant) so the
/// caller falls back to the ordinary flatten path — still correct there, since napi keeps
/// those variants' fields flattened on the shared binding struct. ~keep
#[allow(clippy::too_many_arguments)]
fn build_node_tagged_enum_variant_literal(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    enum_def: &EnumDef,
    nested_types: &std::collections::HashMap<String, String>,
    enum_fields: &std::collections::HashMap<String, String>,
    bigint_fields: &std::collections::BTreeSet<String>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    docs_files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
    depth: usize,
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> Option<String> {
    let tag_field = enum_def.serde_tag.as_deref()?;
    let tag_value_json = obj.get(tag_field)?;
    let tag_value = tag_value_json.as_str()?;
    let variant = enum_def.variants.iter().find(|v| {
        crate::codegen::naming::wire_variant_value(
            &v.name,
            v.serde_rename.as_deref(),
            enum_def.serde_rename_all.as_deref(),
        ) == tag_value
    })?;
    if !variant.is_tuple || variant.fields.len() != 1 {
        return None;
    }
    let field = &variant.fields[0];
    let TypeRef::Named(inner_type_name) = &field.ty else {
        return None;
    };

    let payload_key = field
        .serde_rename
        .clone()
        .or_else(|| variant.serde_rename.clone())
        .unwrap_or_else(|| crate::codegen::naming::to_node_name(&variant.name));

    let mut remaining = obj.clone();
    remaining.remove(tag_field);
    let nested_with_cast = ts_builder_expression_inner(
        &remaining,
        inner_type_name,
        nested_types,
        "node",
        enum_fields,
        bigint_fields,
        type_defs,
        enums,
        "",
        docs_files,
        pointer,
        depth + 1,
        referenced_enums,
    );
    let cast_suffix = format!(" as {inner_type_name}");
    let nested_expr = nested_with_cast.strip_suffix(&cast_suffix).unwrap_or(&nested_with_cast);

    Some(format!(
        "{{ {tag_field}: {}, {payload_key}: {nested_expr} }} as {type_name}",
        json_to_js(tag_value_json)
    ))
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

/// Find the `FieldDef` on `owner_type` that fixture key `key` refers to, matching either the
/// field's Rust name or its wire name (`#[serde(rename = ...)]` / a container `rename_all`) —
/// a fixture may key a field either way, exactly as `refuse_undeclared_json_keys` accepts both
/// spellings as declared.
fn resolve_owner_field<'a>(owner_type: Option<&'a TypeDef>, key: &str) -> Option<&'a crate::core::ir::FieldDef> {
    let definition = owner_type?;
    definition.fields.iter().find(|field| {
        field.name == key
            || crate::codegen::naming::wire_field_name(
                &field.name,
                field.serde_rename.as_deref(),
                definition.serde_rename_all.as_deref(),
            ) == key
    })
}

/// Resolve the napi-rs / wasm-bindgen public JS field identifier for fixture key `key` on
/// `owner_type`. Both backends compute the public field name as `to_node_name(&field.name)` off
/// the Rust field (see `napi::gen_bindings::types` and `wasm::gen_bindings::types::gen_getter`),
/// never from the field's wire name, so a fixture keyed by the wire spelling (a field whose
/// `#[serde(rename = ...)]` diverges from its `#[napi(js_name = ...)]`/wasm-bindgen `js_name`,
/// e.g. `#[serde(rename = "type")]` + `#[napi(js_name = "toolType")]`) must still resolve
/// through the Rust field, not a generic camelCase of the wire key itself — camelCasing the
/// literal fixture key produced `type: "function"` where the binding required `toolType`,
/// throwing `Missing field 'toolType'` at runtime. Falls back to a generic camelCase of `key`
/// when no declared field matches (arbitrary/opaque payloads; enum tag keys are renamed
/// separately by the caller). ~keep
fn node_field_public_key(owner_type: Option<&TypeDef>, key: &str) -> String {
    resolve_owner_field(owner_type, key)
        .map(|field| crate::codegen::naming::to_node_name(&field.name))
        .unwrap_or_else(|| snake_to_camel(key))
}

/// Refuses `obj` if it contains a key that `type_name`'s declared fields (in `type_defs`) don't
/// cover — the single check shared by every node-lang JSON-object-literal builder in this
/// module, so the snippet path (which binds the literal to a typed `const`, see
/// `typed_binding.jinja`, and so IS excess-property-checked by `tsc`) and the e2e test path
/// (which only ever `as`-casts the same literal, and so is NOT excess-property-checked) cannot
/// silently disagree about the same fixture: an undeclared key would otherwise be a compile
/// error (TS2353) in one and invisible in the other. This applies at every nesting depth —
/// `ts_builder_expression_inner`'s own object literal and `node_value_expression`'s nested
/// struct-field literals both call this, since both build a JSON-object-literal that ends up
/// under the same typed-const-vs-`as`-cast split at the top of whichever call tree it's part of.
/// A `serde_flatten` field makes the owning struct's accepted key set open-ended (it legitimately
/// re-exports its own inner field names, or an arbitrary string-keyed bag, at this JSON level),
/// so those types are exempted rather than filtered. A `type_name` absent from `type_defs`
/// (an external/opaque type) is likewise skipped — there is no declared field set to check
/// against.
///
/// ~keep An undeclared key is REFUSED, not silently dropped: this runs at generation time over a
/// fixture the maintainer wrote, so the only plausible causes are a fixture typo/stale field name,
/// a genuinely missing IR field, or (the case that actually shipped) an `options_type` that
/// resolves this argument to the wrong struct entirely — all three are bugs to fix, not values to
/// discard. A silent drop would still produce a compiling snippet/test that LOOKS like it
/// exercises the field the fixture named, which is the same "check that cannot fail" shape as
/// every other vacuous-assertion fix in this generator (see `apply_vacuous_assertion_fallback`,
/// `inert_example`) — the bug would hide instead of surfacing.
///
/// ~keep The refusal is RECORDED, not panicked. A `panic!` here aborted the entire `alef all`
/// process at exit 101 over one consumer misconfiguration, so every other backend, every later
/// crate and every later stage (README, docs, snippet validation) never ran. `fixture_refusal`
/// carries it to `E2eCodegen::generate_gated`, which turns it into this backend's own `Err` —
/// the failure mode `run_generators` already isolates, and `alef all`'s `StageFailures` already
/// reports as "continuing with the remaining stages". Generation continues to the end of this
/// backend and its output is then discarded wholesale by `run_generators`, so no refused literal
/// ever reaches disk.
fn refuse_undeclared_json_keys(
    obj: &serde_json::Map<String, serde_json::Value>,
    type_name: &str,
    type_defs: &[TypeDef],
    site: RefusalSite,
) {
    let Some(definition) = type_defs.iter().find(|definition| definition.name == type_name) else {
        return;
    };
    if definition.fields.iter().any(|field| field.serde_flatten) {
        return;
    }
    // ~keep Both spellings count as declared. A fixture may key a field by its Rust name or by
    // its wire name (`#[serde(rename)]` / a container `rename_all`), and both reach here
    // unchanged — the camelCase conversion happens per key, after this check, in each caller.
    // Matching only `field.name` would abort generation on a correctly-authored fixture for any
    // renamed field, turning this guard into a worse bug than the one it catches.
    let mut declared: std::collections::HashSet<String> = std::collections::HashSet::new();
    for field in &definition.fields {
        declared.insert(field.name.clone());
        declared.insert(crate::codegen::naming::wire_field_name(
            &field.name,
            field.serde_rename.as_deref(),
            definition.serde_rename_all.as_deref(),
        ));
    }
    // Every undeclared key, not just the first: one wrong `options_type` typically refuses
    // several keys at once, and reporting them one per regeneration is a serial debugging loop
    // against a build that takes minutes. ~keep
    for key in obj.keys().filter(|key| !declared.contains(key.as_str())) {
        crate::e2e::codegen::fixture_refusal::record(type_name, key, site.clone());
    }
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
    referenced_enums: &mut std::collections::BTreeSet<String>,
) -> String {
    // Use a depth-indexed variable name so nested IFEs don't shadow each other.
    // Without this, `const _u = WasmOptions.default(); _u.preprocessing =
    // (() => { const _u = WasmOptions.default(); ... })()` triggers
    // oxlint `no-shadow` on every nested-options expression.
    let var = format!("_u{depth}");
    if lang == "node"
        && let Some(enum_def) = enums
            .iter()
            .find(|e| e.name == type_name && e.serde_tag.is_some() && e.variants.iter().any(|v| !v.fields.is_empty()))
        && let Some(nested_literal) = build_node_tagged_enum_variant_literal(
            obj,
            type_name,
            enum_def,
            nested_types,
            enum_fields,
            bigint_fields,
            type_defs,
            enums,
            docs_files,
            pointer,
            depth,
            referenced_enums,
        )
    {
        return nested_literal;
    }
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
        // The fixture's JSON object is the source of truth for VALUES, but not for which KEYS
        // belong on `type_name` — see `refuse_undeclared_json_keys` for why this is refused
        // rather than silently dropped, and why both the snippet and e2e paths must share this
        // one check.
        // `depth == 0` is the argument value itself; anything deeper is an object the fixture
        // nested inside it, and only the JSON pointer identifies which one. ~keep
        let site = if depth == 0 {
            RefusalSite::Argument
        } else {
            RefusalSite::Nested {
                via: format!("the fixture value at JSON pointer `{pointer}`"),
            }
        };
        refuse_undeclared_json_keys(obj, type_name, type_defs, site);
        for (key, val) in obj {
            let field_pointer = json_pointer_child(pointer, key);
            // Rename serde_tag key → "kind" for node-bound tagged-data enum objects.
            let js_key = if lang == "node" {
                match serde_tag_for_this_type {
                    Some(tag) if key == tag => "kind".to_string(),
                    _ => node_field_public_key(owner_type, key),
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
                let enum_type = resolve_enum_type(enum_fields, Some(type_name), key, &camel_key);
                if let Some(enum_type) = enum_type {
                    if let serde_json::Value::String(s) = &preprocessed {
                        referenced_enums.insert(enum_type.clone());
                        let member = declared_enum_member_for_prefixed(enum_type, enums, wasm_type_prefix, s);
                        format!("{enum_type}.{member}")
                    } else {
                        json_to_js(&preprocessed)
                    }
                } else {
                    let field_type = resolve_owner_field(owner_type, key).map(|field| &field.ty);
                    node_value_expression(
                        &preprocessed,
                        key,
                        enum_fields,
                        docs_files,
                        &field_pointer,
                        field_type,
                        type_defs,
                        enums,
                        Some(type_name),
                        referenced_enums,
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
    // Set when a field expression below contains an `await` (a bytes field whose fixture
    // value is a file path) so the emitted IIFE is declared `async`.
    let mut needs_async = false;
    let ir_owner_name = type_name.strip_prefix(wasm_type_prefix).unwrap_or(type_name);
    let owner_type = type_defs.iter().find(|definition| definition.name == ir_owner_name);
    for (key, val) in obj {
        let camel_key = node_field_public_key(owner_type, key);
        let field_pointer = json_pointer_child(pointer, key);
        let field_type = resolve_owner_field(owner_type, key).map(|field| match &field.ty {
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
        // A `bytes` field's fixture value may be a JSON array of numbers or a JSON string
        // (file path / inline text / base64) — ask `ts_bytes_value_expression` how to lower
        // it rather than assuming array-shaped input, matching every other TypeRef::Bytes
        // call site in this backend. See the `bytes` module docs for why this used to be two
        // independently-guessed rules. ~keep
        if matches!(field_type, Some(crate::core::ir::TypeRef::Bytes)) {
            let (expr, needs_await) = ts_bytes_value_expression(val);
            needs_async = needs_async || needs_await;
            stmts.push(format!("{var}.{camel_key} = {expr};"));
            continue;
        }
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
                    referenced_enums,
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
            if let Some(elem_type) = effective_nested_types.get(key.as_str()) {
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
                                &mut *referenced_enums,
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
            && !wasm_enum_bridged_as_raw_value(enum_type, enums, wasm_type_prefix)
            && let serde_json::Value::String(variant) = val
        {
            let member = declared_enum_member_for_prefixed(enum_type, enums, wasm_type_prefix, variant);
            let enum_type = wasm_prefixed_wrapped_type(lang, enum_type, type_defs, enums, wasm_type_prefix);
            stmts.push(format!("{var}.{camel_key} = {enum_type}.{member};"));
        } else if let Some(enum_type) = resolve_enum_type(enum_fields, Some(ir_owner_name), key, &camel_key)
            && !wasm_enum_bridged_as_raw_value(enum_type, enums, wasm_type_prefix)
        {
            // This is an enum field — generate EnumType.EnumValue.
            // Look up by both snake_case (fixture key) and camelCase (alef.toml override key
            // convention) so the alef.toml `enum_fields = { codeBlockStyle = "..." }` style
            // matches fixtures written with snake_case keys. Prefer an owner-qualified
            // match (from `infer_enum_fields`) over a bare-name one — see
            // `resolve_enum_type`.
            //
            // Prefix wasm-wrapped enums exactly as the typed branch above does:
            // the package exports `WasmExtractInputKind`, so a bare
            // `ExtractInputKind.Uri` references an undefined name.
            let enum_type = wasm_prefixed_wrapped_type(lang, enum_type, type_defs, enums, wasm_type_prefix);
            if let serde_json::Value::String(s) = val {
                let member = declared_enum_member_for_prefixed(&enum_type, enums, wasm_type_prefix, s);
                stmts.push(format!("{var}.{camel_key} = {enum_type}.{member};"));
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
        minijinja::context! { body => body, is_async => !docs_files.is_empty() || needs_async },
    )
    .trim_end()
    .to_string()
}

/// `owner_type` is the IR name of the struct that declares `field`, when known —
/// see `resolve_enum_type` for why this disambiguates same-named fields on
/// unrelated structs.
#[allow(clippy::too_many_arguments)]
fn node_value_expression(
    value: &serde_json::Value,
    field: &str,
    enum_fields: &std::collections::HashMap<String, String>,
    docs_files: &[crate::e2e::fixture::FixtureDocsFileInput],
    pointer: &str,
    field_type: Option<&crate::core::ir::TypeRef>,
    type_defs: &[TypeDef],
    enums: &[EnumDef],
    owner_type: Option<&str>,
    referenced_enums: &mut std::collections::BTreeSet<String>,
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
        // Ask the shared classifier rather than assuming array-shaped input — a fixture's
        // `bytes` value is just as often a JSON string (file path / inline text / base64).
        // `Uint8Array.from(value)` unconditionally wrapped a string here, producing an
        // argument typed `string` where `Uint8Array.from` expects `Iterable<number>`. The
        // resulting expression is spliced inline into an already-`await`-containing snippet
        // body (see the `docs_file_expression.jinja` branch just above), so a file-path
        // expression's own `await` needs no further propagation here. ~keep
        let (expr, _needs_await) = ts_bytes_value_expression(value);
        return expr;
    }
    if let Some(crate::core::ir::TypeRef::Named(type_name)) = field_type
        && enums.iter().any(|definition| definition.name == *type_name)
        && let Some(variant) = value.as_str()
    {
        referenced_enums.insert(type_name.clone());
        let member = declared_enum_member_for_prefixed(type_name, enums, "", variant);
        return format!("{type_name}.{member}");
    }
    let camel_field = snake_to_camel(field);
    if let Some(enum_type) = resolve_enum_type(enum_fields, owner_type, field, &camel_field)
        && let Some(variant) = value.as_str()
    {
        referenced_enums.insert(enum_type.clone());
        let member = declared_enum_member_for_prefixed(enum_type, enums, "", variant);
        return format!("{enum_type}.{member}");
    }
    match value {
        serde_json::Value::Object(object) => {
            let nested_type = field_type
                .and_then(|field_type| match field_type {
                    crate::core::ir::TypeRef::Named(type_name) => Some(type_name.as_str()),
                    _ => None,
                })
                .and_then(|type_name| type_defs.iter().find(|definition| definition.name == type_name));
            // Nested struct-field object literals ("inner" fields reached via this function
            // rather than through `ts_builder_expression_inner` directly) go through the same
            // undeclared-key guard as the top-level object — see `refuse_undeclared_json_keys`.
            if let Some(definition) = nested_type {
                let via = match owner_type {
                    Some(owner) => format!("field `{field}` of `{owner}`"),
                    None => format!("field `{field}`"),
                };
                refuse_undeclared_json_keys(object, &definition.name, type_defs, RefusalSite::Nested { via });
            }
            let fields = object
                .iter()
                .map(|(name, value)| {
                    let nested_field = resolve_owner_field(nested_type, name);
                    let nested_field_type = nested_field.map(|field| &field.ty);
                    let js_key = nested_field
                        .map(|field| crate::codegen::naming::to_node_name(&field.name))
                        .unwrap_or_else(|| snake_to_camel(name));
                    format!(
                        "{}: {}",
                        js_key,
                        node_value_expression(
                            value,
                            name,
                            enum_fields,
                            docs_files,
                            &json_pointer_child(pointer, name),
                            nested_field_type,
                            type_defs,
                            enums,
                            nested_type.map(|definition| definition.name.as_str()),
                            &mut *referenced_enums,
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
                    // `field` is synthetic ("") for array elements, so there is no
                    // owning-type-qualified key to look up here; a nested object
                    // element's own fields resolve their owner from `element_type`
                    // inside the recursive call's `Object` branch above.
                    node_value_expression(
                        value,
                        "",
                        enum_fields,
                        docs_files,
                        &json_pointer_child(pointer, &index.to_string()),
                        element_type,
                        type_defs,
                        enums,
                        None,
                        &mut *referenced_enums,
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
mod tests;
