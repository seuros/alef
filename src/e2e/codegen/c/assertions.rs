//! C e2e assertion and accessor rendering helpers.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::escape::escape_c;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};
use heck::{ToPascalCase, ToSnakeCase};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use super::{c_optional_sentinel, is_primitive_c_type, is_skipped_c_field, json_to_c, try_emit_enum_accessor};

/// Emit chained FFI accessor calls for a nested resolved field path.
///
/// For a path like `metadata.document.title`, this generates:
/// ```c
/// HTMHtmlMetadata* metadata_handle = htm_conversion_result_metadata(result);
/// assert(metadata_handle != NULL);
/// HTMDocumentMetadata* doc_handle = htm_html_metadata_document(metadata_handle);
/// assert(doc_handle != NULL);
/// char* metadata_title = htm_document_metadata_title(doc_handle);
/// ```
///
/// The type chain is looked up from `fields_c_types` which maps
/// `"{parent_snake_type}.{field}"` -> `"PascalCaseType"`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_nested_accessor(
    out: &mut String,
    prefix: &str,
    resolved: &str,
    local_var: &str,
    result_var: &str,
    fields_c_types: &HashMap<String, String>,
    fields_enum: &HashSet<String>,
    intermediate_handles: &mut Vec<(String, String)>,
    result_type_name: &str,
    raw_field: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> anyhow::Result<Option<String>> {
    let segments: Vec<&str> = resolved.split('.').collect();
    let prefix_upper = prefix.to_uppercase();

    // Walk the path, starting from the root result type.
    let mut current_snake_type = result_type_name.to_snake_case();
    let mut current_handle = result_var.to_string();
    // Set to true when we've traversed a `[]` array element accessor and subsequent
    // fields must be extracted via alef_json_get_string rather than FFI function calls.
    let mut json_extract_mode = false;

    for (i, segment) in segments.iter().enumerate() {
        let is_leaf = i + 1 == segments.len();

        // In JSON extraction mode, the current_handle is a JSON string and all
        // segments name keys to extract via alef_json_get_string (for primitive
        // leaves) or alef_json_get_object (for intermediate object hops).
        if json_extract_mode {
            // Decompose `field` or `field[N]`/`field[]`. Numeric indexing must
            // extract the Nth element so later key lookups don't ambiguously
            // pick the first occurrence (matters for fixtures with multiple
            // array elements like `data[0]`/`data[1]`).
            let (bare_segment, bracket_key): (&str, Option<&str>) = match segment.find('[') {
                Some(pos) => (&segment[..pos], Some(segment[pos + 1..].trim_end_matches(']'))),
                None => (segment, None),
            };
            let seg_snake = bare_segment.to_snake_case();
            if is_leaf {
                let _ = writeln!(
                    out,
                    "    char* {local_var} = alef_json_get_string({current_handle}, \"{seg_snake}\");"
                );
                return Ok(None); // JSON key leaf — char*.
            }
            // Intermediate JSON key — must be an object/array value. Use the
            // object extractor so the substring includes braces/brackets and
            // later primitive lookups against it find their keys
            // (alef_json_get_string would return NULL on non-string values).
            let json_var = format!("{seg_snake}_json");
            if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
                let _ = writeln!(
                    out,
                    "    char* {json_var} = alef_json_get_object({current_handle}, \"{seg_snake}\");"
                );
                intermediate_handles.push((json_var.clone(), "free".to_string()));
            }
            // If the segment also includes a numeric index `[N]`, drill into
            // the Nth element of the extracted array; otherwise stay on the
            // object/array substring.
            if let Some(key) = bracket_key
                && let Ok(idx) = key.parse::<usize>()
            {
                let elem_var = format!("{seg_snake}_{idx}_json");
                if !intermediate_handles.iter().any(|(h, _)| h == &elem_var) {
                    let _ = writeln!(
                        out,
                        "    char* {elem_var} = alef_json_array_get_index({json_var}, {idx});"
                    );
                    intermediate_handles.push((elem_var.clone(), "free".to_string()));
                }
                current_handle = elem_var;
                continue;
            }
            current_handle = json_var;
            continue;
        }

        // Check for map access: "field[key]" or array element access: "field[]"
        if let Some(bracket_pos) = segment.find('[') {
            let field_name = &segment[..bracket_pos];
            let key = segment[bracket_pos + 1..].trim_end_matches(']');
            let field_snake = field_name.to_snake_case();
            let accessor_fn = format!("{prefix}_{current_snake_type}_{field_snake}");

            // The accessor returns a char* (JSON object/array string).
            let json_var = format!("{field_snake}_json");
            if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
                let _ = writeln!(out, "    char* {json_var} = {accessor_fn}({current_handle});");
                let _ = writeln!(out, "    assert({json_var} != NULL);");
                // Track for freeing — use prefix_free_string since it's a char*.
                intermediate_handles.push((json_var.clone(), "free_string".to_string()));
            }

            // Empty key `[]`: array-element substring access (any element matches).
            // Numeric key `[N]` (e.g. `choices[0]`, `data[1]`): extract the exact
            // Nth top-level element so subsequent key lookups don't ambiguously
            // pick the first occurrence — required for fixtures whose results
            // contain multiple array elements (e.g. `data[0].index`/`data[1].index`).
            if key.is_empty() {
                if !is_leaf {
                    current_handle = json_var;
                    json_extract_mode = true;
                    continue;
                }
                return Ok(None);
            }
            if let Ok(idx) = key.parse::<usize>() {
                let elem_var = format!("{field_snake}_{idx}_json");
                if !intermediate_handles.iter().any(|(h, _)| h == &elem_var) {
                    let _ = writeln!(
                        out,
                        "    char* {elem_var} = alef_json_array_get_index({json_var}, {idx});"
                    );
                    intermediate_handles.push((elem_var.clone(), "free".to_string()));
                }
                if !is_leaf {
                    current_handle = elem_var;
                    json_extract_mode = true;
                    continue;
                }
                // Trailing `[N]` — caller asserts on the element JSON.
                return Ok(None);
            }

            // Named map key access: extract the key value from the JSON object.
            let _ = writeln!(
                out,
                "    char* {local_var} = alef_json_get_string({json_var}, \"{key}\");"
            );
            return Ok(None); // Map access leaf — char*.
        }

        let seg_snake = segment.to_snake_case();
        let accessor_fn = format!("{prefix}_{current_snake_type}_{seg_snake}");

        // Skip any assertion that touches a field marked "skip" in fields_c_types.
        if is_skipped_c_field(fields_c_types, &current_snake_type, &seg_snake) {
            return Ok(Some("__skip__".to_string())); // Sentinel: no accessor emitted, assertion skipped later.
        }

        if is_leaf {
            // Leaf may be a primitive scalar (uint64_t, double, ...) when
            // configured in `fields_c_types`. Otherwise default to char*.
            let lookup_key = format!("{current_snake_type}.{seg_snake}");
            if let Some(t) = fields_c_types.get(&lookup_key).filter(|t| is_primitive_c_type(t)) {
                let _ = writeln!(out, "    {t} {local_var} = {accessor_fn}({current_handle});");
                return Ok(Some(t.clone()));
            }
            // Opaque struct leaf: when fields_c_types maps "{parent}.{field}" to a
            // PascalCase type name (not a primitive, not "char*", not "skip"), the
            // accessor returns a struct pointer rather than a string. Emit the typed
            // handle declaration and register it for freeing.
            if let Some(opaque_type) = fields_c_types.get(&lookup_key).filter(|t| {
                *t != "char*"
                    && *t != "skip"
                    && !is_primitive_c_type(t)
                    && t.chars().next().is_some_and(|c| c.is_uppercase())
            }) {
                let handle_var = format!("{seg_snake}_handle");
                let opaque_snake = opaque_type.to_snake_case();
                if !intermediate_handles.iter().any(|(h, _)| h == &handle_var) {
                    let _ = writeln!(
                        out,
                        "    {prefix_upper}AlefHandle {handle_var} = {accessor_fn}({current_handle});"
                    );
                    intermediate_handles.push((handle_var.clone(), opaque_snake.clone()));
                }
                // Treat the handle itself as the local_var for later assertions.
                // Map local_var → handle_var so render_assertion uses the handle name.
                if local_var != handle_var {
                    let _ = writeln!(out, "    {prefix_upper}AlefHandle {local_var} = {handle_var};");
                }
                return Ok(Some(opaque_snake)); // return type name so caller can register opaque handle cleanup
            }
            // Enum leaf: opaque enum pointer that needs `_to_string` conversion.
            if try_emit_enum_accessor(
                out,
                prefix,
                &prefix_upper,
                raw_field,
                &seg_snake,
                &current_snake_type,
                &accessor_fn,
                &current_handle,
                local_var,
                fields_c_types,
                fields_enum,
                intermediate_handles,
            ) {
                return Ok(None);
            }
            let _ = writeln!(out, "    char* {local_var} = {accessor_fn}({current_handle});");
        } else {
            // Intermediate field — check if it's a char* (JSON string/array) or an opaque handle.
            let lookup_key = format!("{current_snake_type}.{seg_snake}");
            let return_type_pascal = match fields_c_types
                .get(&lookup_key)
                .cloned()
                .or_else(|| resolve_intermediate_type(&current_snake_type, &seg_snake, type_defs))
            {
                Some(return_type) => return_type,
                None => {
                    // No silent fallback: deriving the C type from the field name only
                    // works when the Rust return type is the literal PascalCase of the
                    // field identifier. For accessors whose return type carries a
                    // suffix (e.g. `data` -> `DataNode`, `metadata` -> `MetadataConfig`)
                    // the guessed name does not match what cbindgen emits and the
                    // generated C fails to compile with `unknown type name`. Fail loud
                    // here so the operator declares the correct C type explicitly. ~keep
                    anyhow::bail!(
                        "{}",
                        missing_intermediate_type_diagnostic(MissingIntermediateType {
                            prefix,
                            lookup_key: &lookup_key,
                            accessor_fn: &accessor_fn,
                            resolved,
                            raw_field,
                            segment: *segment,
                            seg_snake: &seg_snake,
                            segments_walked: &segments[..=i],
                            current_snake_type: &current_snake_type,
                            result_type_name,
                            type_defs,
                        })
                    );
                }
            };

            // Special case: intermediate char* fields (e.g. links, assets) are JSON
            // strings/arrays, not opaque handles. For a `.length` suffix, emit alef_json_array_count.
            if return_type_pascal == "char*" {
                let json_var = format!("{seg_snake}_json");
                if !intermediate_handles.iter().any(|(h, _)| h == &json_var) {
                    let _ = writeln!(out, "    char* {json_var} = {accessor_fn}({current_handle});");
                    intermediate_handles.push((json_var.clone(), "free_string".to_string()));
                }
                // If the next (and final) segment is "length", emit the count accessor.
                if i + 2 == segments.len() && segments[i + 1] == "length" {
                    let _ = writeln!(out, "    int {local_var} = alef_json_array_count({json_var});");
                    return Ok(Some("int".to_string()));
                }
                current_snake_type = seg_snake.clone();
                current_handle = json_var;
                continue;
            }

            let return_snake = return_type_pascal.to_snake_case();
            let handle_var = format!("{seg_snake}_handle");

            // Only emit the handle if we haven't already (multiple fields may
            // share the same intermediate path prefix).
            if !intermediate_handles.iter().any(|(h, _)| h == &handle_var) {
                let _ = writeln!(
                    out,
                    "    {prefix_upper}AlefHandle {handle_var} = \
                     {accessor_fn}({current_handle});"
                );
                let _ = writeln!(out, "    assert({handle_var} != 0);");
                intermediate_handles.push((handle_var.clone(), return_snake.clone()));
            }

            current_snake_type = return_snake;
            current_handle = handle_var;
        }
    }
    Ok(None)
}

fn resolve_intermediate_type(
    parent_snake: &str,
    field_snake: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<String> {
    let parent = type_defs
        .iter()
        .find(|type_def| type_def.name.to_snake_case() == parent_snake)?;
    let field = parent
        .fields
        .iter()
        .find(|field| field.name.to_snake_case() == field_snake)?;
    super::named_type(&field.ty).map(str::to_string)
}

/// How deep [`find_field_path`] will search for a field name below the result type.
///
/// The bound exists to terminate on a self-referential IR, not to trade off cost -- this
/// only ever runs on the way to returning an error. Six comfortably clears the chains that
/// motivated it (crawlberg's `ScrapeResult.metadata.article.tags` is three hops); a chain
/// deeper than this just loses the "here is where the field really lives" hint, it does not
/// change the error.
const MAX_FIELD_PATH_SEARCH_DEPTH: usize = 6;

/// Where a field named `field_snake` really lives below some root type.
struct ResolvedFieldChain {
    /// The dotted path from the root type down to the field, e.g. `metadata.article.tags`.
    path: String,
    /// The IR type that actually declares the field. The C accessor symbol is built from
    /// this type, not from the root -- naming it is the difference between the diagnostic
    /// pointing at `cberg_article_metadata_tags` and at the `cberg_scrape_result_tags` that
    /// does not exist.
    owner_type: String,
}

/// The dotted path from `root_type` down to the first field whose snake_case name is
/// `field_snake`, or `None` if no type reachable from `root_type` has such a field.
///
/// Shallowest-first, and only through `TypeRef::Named` struct fields — the same hops
/// [`emit_nested_accessor`] itself can walk, so a path this returns is one the C codegen
/// could actually emit accessors for.
fn find_field_path(
    root_type: &str,
    field_snake: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<ResolvedFieldChain> {
    fn walk(
        type_name: &str,
        field_snake: &str,
        type_defs: &[crate::core::ir::TypeDef],
        depth: usize,
        seen: &mut HashSet<String>,
    ) -> Option<ResolvedFieldChain> {
        if depth == 0 || !seen.insert(type_name.to_string()) {
            return None;
        }
        let type_def = type_defs.iter().find(|type_def| type_def.name == type_name)?;
        if let Some(field) = type_def
            .fields
            .iter()
            .find(|field| field.name.to_snake_case() == field_snake)
        {
            return Some(ResolvedFieldChain {
                path: field.name.to_snake_case(),
                owner_type: type_def.name.clone(),
            });
        }
        for field in &type_def.fields {
            let Some(nested) = super::named_type(&field.ty) else {
                continue;
            };
            if let Some(found) = walk(nested, field_snake, type_defs, depth - 1, seen) {
                return Some(ResolvedFieldChain {
                    path: format!("{}.{}", field.name.to_snake_case(), found.path),
                    owner_type: found.owner_type,
                });
            }
        }
        None
    }

    walk(
        root_type,
        field_snake,
        type_defs,
        MAX_FIELD_PATH_SEARCH_DEPTH,
        &mut HashSet::new(),
    )
}

/// The leading segments `resolved` lost to virtual-namespace stripping, if any.
///
/// [`emit_nested_accessor`] is handed the already-stripped path (the callers in
/// `test_function.rs`/`call_patterns.rs` strip before calling), so the only surviving record
/// of a stripped prefix is that `raw_field` ends with `resolved`. Recovering it is what lets
/// the diagnostic tell "add an alias" apart from "add a type mapping": a path that lost a
/// segment is almost always a missing `[crates.e2e.fields]` alias, and declaring the C type
/// the message names would instead emit a call to a symbol that does not exist. ~keep
fn stripped_namespace_prefix<'a>(raw_field: &'a str, resolved: &str) -> Option<&'a str> {
    let prefix_len = raw_field.len().checked_sub(resolved.len())?;
    if prefix_len == 0 || !raw_field.ends_with(resolved) {
        return None;
    }
    raw_field
        .get(..prefix_len)
        .and_then(|prefix| prefix.strip_suffix('.'))
        .filter(|prefix| !prefix.is_empty())
}

/// Why `resolve_intermediate_type` could not derive a C type for `{parent_snake}.{field_snake}`.
///
/// The three ways it returns `None` need three different fixes, and the missing key alone
/// cannot tell them apart: an unknown parent type means the walk arrived somewhere it should
/// never have been (usually namespace stripping), a missing field means the path is wrong,
/// and a non-`Named` field type means the path is right but the accessor returns something
/// no opaque handle can carry. ~keep
fn why_the_type_is_unknown(parent_snake: &str, field_snake: &str, type_defs: &[crate::core::ir::TypeDef]) -> String {
    let Some(parent) = type_defs
        .iter()
        .find(|type_def| type_def.name.to_snake_case() == parent_snake)
    else {
        return format!("No IR type has the snake_case name `{parent_snake}`");
    };
    let Some(field) = parent
        .fields
        .iter()
        .find(|field| field.name.to_snake_case() == field_snake)
    else {
        return format!("Type `{}` has no field `{field_snake}`", parent.name);
    };
    if super::named_type(&field.ty).is_none() {
        return format!(
            "Field `{}.{field_snake}` is not a named struct type, so no opaque accessor type can be derived from it",
            parent.name
        );
    }
    format!("Type `{}` does have a field `{field_snake}`", parent.name)
}

/// Inputs for [`missing_intermediate_type_diagnostic`]. A struct, not a dozen positional
/// `&str`s, so two of them cannot be swapped without the compiler noticing.
struct MissingIntermediateType<'a> {
    /// The crate's FFI symbol prefix, for naming the accessor that really exists.
    prefix: &'a str,
    /// The `"{parent_snake}.{field_snake}"` key that was looked up and missed.
    lookup_key: &'a str,
    /// The C symbol the walk would call if that key were simply declared.
    accessor_fn: &'a str,
    /// The (already namespace-stripped) path being walked.
    resolved: &'a str,
    /// The fixture's own field path, before alias resolution and namespace stripping.
    raw_field: &'a str,
    segment: &'a str,
    seg_snake: &'a str,
    /// `resolved`'s segments up to and including the failing one.
    segments_walked: &'a [&'a str],
    /// The snake_case type the walk is standing on.
    current_snake_type: &'a str,
    /// The type the walk started from.
    result_type_name: &'a str,
    type_defs: &'a [crate::core::ir::TypeDef],
}

/// Explain a missing `fields_c_types` key in terms of the chain that produced it.
///
/// The bare "missing key `{parent}.{field}`" this replaced implied its own remedy — declare
/// that key — and the implied remedy is wrong whenever the key names a field the parent type
/// does not have. Adding it silences the failure and emits a call to a C function that was
/// never generated, which then fails at `cc` time (or, worse, links against an unrelated
/// symbol). So the message has to carry three things the key alone cannot: which prefix
/// alef stripped as a virtual namespace to arrive at this path, which symbol declaring the
/// key would conjure, and where the field really lives under the result type. ~keep
fn missing_intermediate_type_diagnostic(context: MissingIntermediateType<'_>) -> String {
    let MissingIntermediateType {
        prefix,
        lookup_key,
        accessor_fn,
        resolved,
        raw_field,
        segment,
        seg_snake,
        segments_walked,
        current_snake_type,
        result_type_name,
        type_defs,
    } = context;

    let mut message = format!(
        "e2e c codegen: fields_c_types is missing key \"{lookup_key}\" (path \"{resolved}\", segment \"{segment}\"), \
         reached while walking fixture field \"{raw_field}\" from result type `{result_type_name}`. {why}, so \
         declaring \"{lookup_key}\" would make the generated test call `{accessor_fn}()`. (The old fallback guessed \
         `{guess}` from the field name, which silently miscompiled whenever the Rust return type differed, e.g. \
         `DataNode` vs `Data`.)",
        why = why_the_type_is_unknown(current_snake_type, seg_snake, type_defs),
        guess = segment.to_pascal_case(),
    );

    if let Some(namespace) = stripped_namespace_prefix(raw_field, resolved) {
        let _ = write!(
            message,
            " alef stripped the leading \"{namespace}\" from \"{raw_field}\" as a virtual namespace, because no \
             `[crates.e2e.fields]` alias maps it onto a real path and its first segment is not a `result_fields` \
             entry -- which is why the walk started at `{result_type_name}` instead of inside `{namespace}`."
        );
    }

    match find_field_path(result_type_name, seg_snake, type_defs) {
        Some(chain) => {
            let alias_key = match stripped_namespace_prefix(raw_field, resolved) {
                Some(namespace) => format!("{namespace}.{}", segments_walked.join(".")),
                None => segments_walked.join("."),
            };
            let real_path = &chain.path;
            let real_symbol = format!("{prefix}_{}_{seg_snake}", chain.owner_type.to_snake_case());
            let _ = write!(
                message,
                " Field `{seg_snake}` does exist below `{result_type_name}`, at \"{real_path}\" -- it is declared on \
                 `{owner}`, so the accessor that really exists is `{real_symbol}()`. Fix: add \
                 \"{alias_key}\" = \"{real_path}\" under `[crates.e2e.fields]` so the fixture path resolves to the \
                 real chain. Only add \"{lookup_key}\" to `[crates.e2e.fields_c_types]` if `{accessor_fn}()` really \
                 is in the generated header.",
                owner = chain.owner_type,
            );
        }
        None => {
            let _ = write!(
                message,
                " No type reachable from `{result_type_name}` has a field named `{seg_snake}` either, so the \
                 fixture's field path is the thing to check first -- declaring \"{lookup_key}\" cannot make \
                 `{accessor_fn}()` exist."
            );
        }
    }

    message
}

/// Build the C argument string for the function call.
/// When `has_options_handle` is true, json_object args are replaced with
/// the `options_handle` pointer (which was constructed via `from_json`).
pub(super) fn build_args_string_c(
    input: &serde_json::Value,
    args: &[crate::e2e::config::ArgMapping],
    typed_arg_handles: &HashMap<String, String>,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
    fixture: &Fixture,
) -> String {
    if args.is_empty() {
        return json_to_c(input);
    }

    let mut parts: Vec<String> = Vec::new();

    for arg in args {
        // Handle test_backend args: emit the stub and use it.
        if arg.arg_type == "test_backend" {
            // A `test_backend` arg fills a C trait-bridge vtable-pointer parameter.
            // There is no fixture-supplied value to fall back to: an unregistered
            // trait has no vtable to point at, and `emit_test_backend` panics rather
            // than hand back a placeholder for `parts` to splice in as an
            // expression — splicing either would emit C that cannot compile. Unlike a non-null-typed
            // target language, C's type system would happily accept a `NULL` fallback
            // here too (any pointer type admits it), so the compiler can't be relied on
            // to catch a bad default the way it can elsewhere — fail loud here instead,
            // matching every other "cannot render this" case in this file (see
            // `resolve_intermediate_type`'s `None` arm above, and the assertion-type
            // panics below). ~keep
            let Some(trait_name) = &arg.trait_name else {
                panic!(
                    "C e2e generator: fixture `{}` declares a `test_backend` arg `{}` with no `trait_name` configured; cannot generate a C stub without knowing which trait to implement",
                    fixture.id, arg.name
                );
            };
            let Some(trait_bridge) = config.trait_bridges.iter().find(|tb| tb.trait_name == *trait_name) else {
                panic!(
                    "C e2e generator: fixture `{}` requires trait `{trait_name}` for its `test_backend` arg `{}`, but no `[[crates.trait_bridges]]` entry named `{trait_name}` is configured",
                    fixture.id, arg.name
                );
            };
            let mut methods: Vec<&crate::core::ir::MethodDef> = type_defs
                .iter()
                .find(|t| t.name == *trait_name)
                .map(|t| t.methods.iter().collect())
                .unwrap_or_default();
            if let Some(super_trait) = &trait_bridge.super_trait
                && let Some(super_type) = type_defs.iter().find(|t| &t.rust_path == super_trait)
            {
                for method in &super_type.methods {
                    if !methods.iter().any(|m| m.name == method.name) {
                        methods.push(method);
                    }
                }
            }
            // `emit_test_backend` panics rather than return a placeholder when the C
            // test-backend emitter is unimplemented — see `TestBackendEmission`'s and
            // `trait_bridge_snippet::emit_test_backend`'s doc comments. ~keep
            let emission = crate::e2e::codegen::emit_test_backend("c", trait_bridge, &methods, fixture, &[]);
            parts.push(emission.arg_expr);
            continue;
        }

        let val = crate::e2e::codegen::resolve_field(input, &arg.field);
        match val {
            // ~keep Explicit null on optional arg → pass the type-appropriate "none"
            // sentinel: `0` for a scalar `AlefHandle` arg, `NULL` for a real pointer.
            v if v.is_null() && arg.optional => parts.push(c_optional_sentinel(&arg.arg_type).to_string()),
            // Missing required fields resolve to null; skip them so malformed
            // fixture configuration does not crash generation.
            v if v.is_null() => {}
            v => {
                // For json_object args, use the options_handle pointer
                // instead of the raw JSON string.
                if let Some(handle) = typed_arg_handles.get(&arg.name) {
                    parts.push(handle.clone())
                } else {
                    parts.push(json_to_c(v))
                }
            }
        }
    }

    parts.join(", ")
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    ffi_prefix: &str,
    _field_resolver: &FieldResolver,
    accessed_fields: &[(String, String, bool)],
    primitive_locals: &HashMap<String, String>,
    opaque_handle_locals: &HashMap<String, String>,
) {
    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !_field_resolver.is_valid_for_result(f)
    {
        let _ = writeln!(out, "    // skipped: field '{f}' not available on result type");
        return;
    }

    let field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => {
            // Use the local variable extracted from the opaque handle.
            accessed_fields
                .iter()
                .find(|(k, _, _)| k == f)
                .map(|(_, local, _)| local.clone())
                .unwrap_or_else(|| result_var.to_string())
        }
        _ => result_var.to_string(),
    };

    // If the field was marked with the "__skip__" sentinel (fields_c_types = "skip"),
    // the accessor was never emitted — skip the assertion silently.
    if primitive_locals.get(&field_expr).is_some_and(|t| t == "__skip__") {
        let _ = writeln!(out, "    // skipped: field '{field_expr}' not available in C FFI");
        return;
    }

    let field_is_primitive = primitive_locals.contains_key(&field_expr);
    let field_primitive_type = primitive_locals.get(&field_expr).cloned();
    // Opaque-handle fields (e.g. `usage` → SAMPLELLMUsage*) cannot be treated
    // as C strings — `strlen` / `strcmp` on a struct pointer is undefined
    // behavior (SIGABRT in practice). `not_empty` / `is_empty` collapse to
    // NULL checks; other string assertions are skipped for these fields.
    let field_is_opaque_handle = opaque_handle_locals.contains_key(&field_expr);
    // Map-access fields are extracted via `alef_json_get_string` and end up
    // as char*. When the assertion expects a numeric or boolean value, we
    // emit a parsed/literal comparison rather than `strcmp`.
    let field_is_map_access = if let Some(f) = &assertion.field {
        accessed_fields.iter().any(|(k, _, m)| k == f && *m)
    } else {
        false
    };

    // Check if the assertion field is optional — used to emit conditional assertions
    // for optional numeric fields (returns 0 when None, so 0 == "not set").
    // Check both the raw field name and its resolved alias.
    let assertion_field_is_optional = assertion
        .field
        .as_deref()
        .map(|f| {
            if f.is_empty() {
                return false;
            }
            if _field_resolver.is_optional(f) {
                return true;
            }
            // Also check the resolved alias (e.g. "robots.crawl_delay" → "crawl_delay").
            let resolved = _field_resolver.resolve(f);
            _field_resolver.is_optional(resolved)
        })
        .unwrap_or(false);

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                if field_is_primitive {
                    let cmp_val = if field_primitive_type.as_deref() == Some("bool") {
                        match expected.as_bool() {
                            Some(true) => "1".to_string(),
                            Some(false) => "0".to_string(),
                            None => c_val,
                        }
                    } else {
                        c_val
                    };
                    // For optional numeric fields, treat 0 as "not set" and allow it.
                    // This mirrors Go's nil-pointer check for optional fields.
                    let is_numeric = field_primitive_type.as_deref().map(|t| t != "bool").unwrap_or(false);
                    if assertion_field_is_optional && is_numeric {
                        let _ = writeln!(
                            out,
                            "    assert(({field_expr} == 0 || {field_expr} == {cmp_val}) && \"equals assertion failed\");"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert({field_expr} == {cmp_val} && \"equals assertion failed\");"
                        );
                    }
                } else if expected.is_string() {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && strcmp({field_expr}, {c_val}) == 0 && \"equals assertion failed\");"
                    );
                } else if field_is_map_access && expected.is_boolean() {
                    let lit = match expected.as_bool() {
                        Some(true) => "\"true\"",
                        _ => "\"false\"",
                    };
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && strcmp({field_expr}, {lit}) == 0 && \"equals assertion failed\");"
                    );
                } else if field_is_map_access && expected.is_number() {
                    if expected.is_f64() {
                        let _ = writeln!(
                            out,
                            "    assert({field_expr} != NULL && atof({field_expr}) == {c_val} && \"equals assertion failed\");"
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "    assert({field_expr} != NULL && atoll({field_expr}) == {c_val} && \"equals assertion failed\");"
                        );
                    }
                } else {
                    let _ = writeln!(
                        out,
                        "    assert(strcmp({field_expr}, {c_val}) == 0 && \"equals assertion failed\");"
                    );
                }
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert({field_expr} != NULL && strstr({field_expr}, {c_val}) != NULL && \"expected to contain substring\");"
                );
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && strstr({field_expr}, {c_val}) != NULL && \"expected to contain substring\");"
                    );
                }
            }
        }
        "not_contains" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert({field_expr} != NULL && strstr({field_expr}, {c_val}) == NULL && \"expected non-null value without substring\");"
                );
            }
        }
        "not_empty" => {
            if field_is_opaque_handle {
                // ~keep Opaque handle: `strlen` on a scalar `AlefHandle` (uint64_t) is a
                // type error, not just UB on a struct pointer. Weaken to a
                // non-zero check — strictly weaker than the original intent but
                // matches the handle's actual "none" sentinel (`0`, not `NULL`).
                let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
            } else {
                let _ = writeln!(
                    out,
                    "    assert({field_expr} != NULL && strlen({field_expr}) > 0 && \"expected non-empty value\");"
                );
            }
        }
        "is_empty" => {
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} == 0 && \"expected null handle\");");
            } else if assertion_field_is_optional || !field_is_primitive {
                // Optional string fields may return NULL — treat NULL as empty.
                let _ = writeln!(
                    out,
                    "    assert(({field_expr} == NULL || strlen({field_expr}) == 0) && \"expected empty value\");"
                );
            } else {
                let _ = writeln!(
                    out,
                    "    assert(strlen({field_expr}) == 0 && \"expected empty value\");"
                );
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let _ = writeln!(out, "    {{");
                let _ = writeln!(out, "        int found = 0;");
                for val in values {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "        if (strstr({field_expr}, {c_val}) != NULL) {{ found = 1; }}"
                    );
                }
                let _ = writeln!(
                    out,
                    "        assert(found && \"expected to contain at least one of the specified values\");"
                );
                let _ = writeln!(out, "    }}");
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) > {c_val} && \"expected greater than\");"
                    );
                } else {
                    let _ = writeln!(out, "    assert({field_expr} > {c_val} && \"expected greater than\");");
                }
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) < {c_val} && \"expected less than\");"
                    );
                } else {
                    let _ = writeln!(out, "    assert({field_expr} < {c_val} && \"expected less than\");");
                }
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) >= {c_val} && \"expected greater than or equal\");"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} >= {c_val} && \"expected greater than or equal\");"
                    );
                }
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let c_val = json_to_c(val);
                if field_is_map_access && val.is_number() && !field_is_primitive {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} != NULL && atof({field_expr}) <= {c_val} && \"expected less than or equal\");"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    assert({field_expr} <= {c_val} && \"expected less than or equal\");"
                    );
                }
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert(strncmp({field_expr}, {c_val}, strlen({c_val})) == 0 && \"expected to start with\");"
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(out, "    assert(strlen({field_expr}) >= strlen({c_val}) && ");
                let _ = writeln!(
                    out,
                    "           strcmp({field_expr} + strlen({field_expr}) - strlen({c_val}), {c_val}) == 0 && \"expected to end with\");"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(
                    out,
                    "    assert(strlen({field_expr}) >= {n} && \"expected minimum length\");"
                );
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(
                    out,
                    "    assert(strlen({field_expr}) <= {n} && \"expected maximum length\");"
                );
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    {{");
                let _ = writeln!(out, "        /* count_min: count top-level JSON array elements */");
                let _ = writeln!(
                    out,
                    "        assert({field_expr} != NULL && \"expected non-null collection JSON\");"
                );
                let _ = writeln!(out, "        int elem_count = alef_json_array_count({field_expr});");
                let _ = writeln!(
                    out,
                    "        assert(elem_count >= {n} && \"expected at least {n} elements\");"
                );
                let _ = writeln!(out, "    }}");
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    {{");
                let _ = writeln!(out, "        /* count_equals: count elements in array */");
                let _ = writeln!(
                    out,
                    "        assert({field_expr} != NULL && \"expected non-null collection JSON\");"
                );
                let _ = writeln!(out, "        int elem_count = alef_json_array_count({field_expr});");
                let _ = writeln!(out, "        assert(elem_count == {n} && \"expected {n} elements\");");
                let _ = writeln!(out, "    }}");
            }
        }
        "is_true" => {
            let _ = writeln!(out, "    assert({field_expr});");
        }
        "is_false" => {
            let _ = writeln!(out, "    assert(!{field_expr});");
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                render_method_result_assertion(
                    out,
                    result_var,
                    ffi_prefix,
                    method_name,
                    assertion.args.as_ref(),
                    assertion.return_type.as_deref(),
                    assertion.check.as_deref().unwrap_or("is_true"),
                    assertion.value.as_ref(),
                );
            } else {
                panic!("C e2e generator: method_result assertion missing 'method' field");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(out, "    {{");
                let _ = writeln!(out, "        regex_t _re;");
                let _ = writeln!(
                    out,
                    "        assert(regcomp(&_re, {c_val}, REG_EXTENDED) == 0 && \"regex compile failed\");"
                );
                let _ = writeln!(
                    out,
                    "        assert(regexec(&_re, {field_expr}, 0, NULL, 0) == 0 && \"expected value to match regex\");"
                );
                let _ = writeln!(out, "        regfree(&_re);");
                let _ = writeln!(out, "    }}");
            }
        }
        "not_error" => {
            // Already handled — the NULL check above covers this.
        }
        "error" => {
            // Handled at the test function level.
        }
        other => {
            panic!("C e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Render a `method_result` assertion in C.
///
/// Dispatches generically using `{ffi_prefix}_{method_name}` for the FFI call.
/// The `return_type` fixture field controls how the return value is handled:
/// - `"string"` — the method returns a heap-allocated `char*`; the generator
///   emits a scoped block that asserts, then calls `free()`.
/// - absent/other — treated as a primitive integer (or pointer-as-bool); the
///   assertion is emitted inline without any heap management.
#[allow(clippy::too_many_arguments)]
fn render_method_result_assertion(
    out: &mut String,
    result_var: &str,
    ffi_prefix: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
    return_type: Option<&str>,
    check: &str,
    value: Option<&serde_json::Value>,
) {
    let call_expr = build_c_method_call(result_var, ffi_prefix, method_name, args);

    if return_type == Some("string") {
        // Heap-allocated char* return: emit a scoped block, assert, then free.
        let _ = writeln!(out, "    {{");
        let _ = writeln!(out, "        char* _method_result = {call_expr};");
        if check == "is_error" {
            let _ = writeln!(
                out,
                "        assert(_method_result == NULL && \"expected method to return error\");"
            );
            let _ = writeln!(out, "    }}");
            return;
        }
        let _ = writeln!(
            out,
            "        assert(_method_result != NULL && \"method_result returned NULL\");"
        );
        match check {
            "contains" => {
                if let Some(val) = value {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "        assert(strstr(_method_result, {c_val}) != NULL && \"method_result contains assertion failed\");"
                    );
                }
            }
            "equals" => {
                if let Some(val) = value {
                    let c_val = json_to_c(val);
                    let _ = writeln!(
                        out,
                        "        assert(strcmp(_method_result, {c_val}) == 0 && \"method_result equals assertion failed\");"
                    );
                }
            }
            "is_true" => {
                let _ = writeln!(
                    out,
                    "        assert(_method_result != NULL && strlen(_method_result) > 0 && \"method_result is_true assertion failed\");"
                );
            }
            "count_min" => {
                if let Some(val) = value {
                    let n = val.as_u64().unwrap_or(0);
                    let _ = writeln!(out, "        int _elem_count = alef_json_array_count(_method_result);");
                    let _ = writeln!(
                        out,
                        "        assert(_elem_count >= {n} && \"method_result count_min assertion failed\");"
                    );
                }
            }
            other_check => {
                panic!("C e2e generator: unsupported method_result check type for string return: {other_check}");
            }
        }
        let _ = writeln!(out, "        free(_method_result);");
        let _ = writeln!(out, "    }}");
        return;
    }

    // Primitive (integer / pointer-as-bool) return: inline assert, no heap management.
    match check {
        "equals" => {
            if let Some(val) = value {
                let c_val = json_to_c(val);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} == {c_val} && \"method_result equals assertion failed\");"
                );
            }
        }
        "is_true" => {
            let _ = writeln!(
                out,
                "    assert({call_expr} && \"method_result is_true assertion failed\");"
            );
        }
        "is_false" => {
            let _ = writeln!(
                out,
                "    assert(!{call_expr} && \"method_result is_false assertion failed\");"
            );
        }
        "greater_than_or_equal" => {
            if let Some(val) = value {
                let n = val.as_u64().unwrap_or(0);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} >= {n} && \"method_result >= {n} assertion failed\");"
                );
            }
        }
        "count_min" => {
            if let Some(val) = value {
                let n = val.as_u64().unwrap_or(0);
                let _ = writeln!(
                    out,
                    "    assert({call_expr} >= {n} && \"method_result count_min assertion failed\");"
                );
            }
        }
        other_check => {
            panic!("C e2e generator: unsupported method_result check type: {other_check}");
        }
    }
}

/// Build a C call expression for a `method_result` assertion.
///
/// Uses generic dispatch: `{ffi_prefix}_{method_name}(result_var, args...)`.
/// Args from the fixture JSON object are emitted as positional C arguments in
/// insertion order, using best-effort type conversion (strings → C string literals,
/// numbers and booleans → verbatim literals).
fn build_c_method_call(
    result_var: &str,
    ffi_prefix: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
) -> String {
    let extra_args = if let Some(args_val) = args {
        args_val
            .as_object()
            .map(|obj| {
                obj.values()
                    .map(|v| match v {
                        serde_json::Value::String(s) => format!("\"{}\"", escape_c(s)),
                        serde_json::Value::Bool(true) => "1".to_string(),
                        serde_json::Value::Bool(false) => "0".to_string(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Null => "NULL".to_string(),
                        other => format!("\"{}\"", escape_c(&other.to_string())),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    if extra_args.is_empty() {
        format!("{ffi_prefix}_{method_name}({result_var})")
    } else {
        format!("{ffi_prefix}_{method_name}({result_var}, {extra_args})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ir::{FieldDef, TypeDef, TypeRef};

    /// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
    /// (present, non-`binding_excluded`, on some IR type) but missing from the
    /// hand-maintained `result_fields` config must still render a real assertion,
    /// not a "skipped: field not available" comment — `c.rs` (both the main-suite
    /// and snippet resolver construction sites) now threads
    /// `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
    #[test]
    fn c_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
        let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(reachable, HashSet::new());
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("data".to_string()),
            value: Some(serde_json::Value::String("hello".to_string())),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "sample",
            &resolver,
            &[],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert!(!out.contains("skipped"), "got: {out}");
    }

    /// The negative-control half of the same regression: `internal_diagnostics`
    /// represents a field carrying `#[doc(hidden)]` or `#[cfg_attr(alef,
    /// alef(skip))]` in the real struct (a genuine `binding_excluded` field) —
    /// NOT `#[serde(skip)]`, which alone does not exclude a field from the
    /// binding surface. Even though it is listed in `result_fields` (a stale/
    /// wrong config entry), the IR must still win and reject it. ~keep
    #[test]
    fn c_ir_excluded_field_present_in_result_fields_is_still_skipped() {
        let result_fields: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
        let excluded: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(HashSet::new(), excluded);
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("internal_diagnostics".to_string()),
            value: Some(serde_json::Value::String("hello".to_string())),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "sample",
            &resolver,
            &[],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert!(out.contains("skipped"), "got: {out}");
    }

    #[test]
    fn nested_optional_handle_type_comes_from_ir_when_config_mapping_is_absent() {
        let types = [
            TypeDef {
                name: "ExtractionResult".into(),
                fields: vec![FieldDef {
                    name: "summary".into(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("ExtractionSummary".into()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "ExtractionSummary".into(),
                fields: vec![FieldDef {
                    name: "processed".into(),
                    ty: TypeRef::Primitive(crate::core::ir::PrimitiveType::U64),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ];
        let mut output = String::new();
        let mut handles = Vec::new();

        emit_nested_accessor(
            &mut output,
            "sample",
            "summary.processed",
            "summary_processed",
            "result",
            &HashMap::from([("extraction_summary.processed".into(), "uint64_t".into())]),
            &HashSet::new(),
            &mut handles,
            "ExtractionResult",
            "summary.processed",
            &types,
        )
        .expect("every hop resolves");

        assert!(output.contains("SAMPLEAlefHandle summary_handle"), "{output}");
        assert!(output.contains("sample_extraction_result_summary(result)"), "{output}");
        assert!(output.contains("uint64_t summary_processed"), "{output}");
    }

    /// The crawlberg shape: `ScrapeResult.metadata -> PageMetadata.article ->
    /// ArticleMetadata.tags`, asserted by a fixture as `article.tags.length`. With no
    /// `article.*` alias configured, `article` is stripped as a virtual namespace before
    /// this function is called, so the walk starts on `ScrapeResult` and looks for a field
    /// `tags` that lives two hops further down. ~keep
    fn crawlberg_article_types() -> Vec<TypeDef> {
        vec![
            TypeDef {
                name: "ScrapeResult".into(),
                fields: vec![FieldDef {
                    name: "metadata".into(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("PageMetadata".into()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "PageMetadata".into(),
                fields: vec![FieldDef {
                    name: "article".into(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("ArticleMetadata".into()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "ArticleMetadata".into(),
                fields: vec![FieldDef {
                    name: "tags".into(),
                    ty: TypeRef::Vec(Box::new(TypeRef::String)),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ]
    }

    fn walk_crawlberg_article_tags() -> anyhow::Error {
        let mut output = String::new();
        let mut handles = Vec::new();
        emit_nested_accessor(
            &mut output,
            "cberg",
            "tags.length",
            "article_tags_length",
            "result",
            &HashMap::new(),
            &HashSet::new(),
            &mut handles,
            "ScrapeResult",
            "article.tags.length",
            &crawlberg_article_types(),
        )
        .expect_err("`tags` is not a field of ScrapeResult")
    }

    /// A consumer config gap must surface as an error, not a process-killing panic.
    #[test]
    fn missing_intermediate_type_returns_an_error_instead_of_panicking() {
        let message = walk_crawlberg_article_tags().to_string();
        assert!(message.contains("fields_c_types"), "{message}");
        assert!(message.contains("scrape_result.tags"), "{message}");
        assert!(message.contains("tags.length"), "{message}");
    }

    /// Every fact the old panic carried must survive the conversion.
    #[test]
    fn missing_intermediate_type_keeps_the_original_panic_facts() {
        let message = walk_crawlberg_article_tags().to_string();
        assert!(message.contains("path \"tags.length\""), "{message}");
        assert!(message.contains("segment \"tags\""), "{message}");
        assert!(message.contains("`Tags`"), "guessed-name rationale is gone: {message}");
        assert!(message.contains("`DataNode` vs `Data`"), "{message}");
    }

    /// The point of the rewrite. The message must not leave "add the key it named" as the
    /// obvious remedy, because that key would emit `cberg_scrape_result_tags()` -- a symbol
    /// no backend generates. It has to name the stripped namespace, the real chain, and the
    /// alias that reconnects them.
    #[test]
    fn missing_intermediate_type_names_the_real_chain_not_the_phantom_key() {
        let message = walk_crawlberg_article_tags().to_string();

        assert!(
            message.contains("Type `ScrapeResult` has no field `tags`"),
            "must say why the key is missing: {message}"
        );
        assert!(
            message.contains("stripped the leading \"article\""),
            "must name the namespace stripping that produced the path: {message}"
        );
        assert!(
            message.contains("cberg_scrape_result_tags()"),
            "must name the C symbol declaring the key would conjure: {message}"
        );
        assert!(
            message.contains("cberg_article_metadata_tags()"),
            "must name the C symbol that really exists: {message}"
        );
        assert!(
            message.contains("\"metadata.article.tags\""),
            "must name the real resolved chain: {message}"
        );
        assert!(
            message.contains("\"article.tags\" = \"metadata.article.tags\""),
            "must spell the alias that fixes it: {message}"
        );
        assert!(
            message.contains("[crates.e2e.fields]"),
            "must name the alias table, not just fields_c_types: {message}"
        );
    }

    /// The other half of the diagnostic: when the field genuinely does not exist anywhere
    /// under the result type, there is no alias to suggest and the message must say so
    /// rather than inventing a chain.
    #[test]
    fn missing_intermediate_type_says_so_when_no_type_carries_the_field() {
        let mut output = String::new();
        let mut handles = Vec::new();
        let error = emit_nested_accessor(
            &mut output,
            "cberg",
            "nowhere.length",
            "nowhere_length",
            "result",
            &HashMap::new(),
            &HashSet::new(),
            &mut handles,
            "ScrapeResult",
            "nowhere.length",
            &crawlberg_article_types(),
        )
        .expect_err("`nowhere` is not a field of anything");

        let message = error.to_string();
        assert!(
            message.contains("No type reachable from `ScrapeResult` has a field named `nowhere`"),
            "{message}"
        );
        assert!(
            !message.contains("under `[crates.e2e.fields]`"),
            "must not suggest an alias it cannot spell: {message}"
        );
        assert!(
            !message.contains("stripped the leading"),
            "nothing was stripped here: {message}"
        );
    }

    #[test]
    fn stripped_namespace_prefix_recovers_only_a_real_stripped_prefix() {
        assert_eq!(
            stripped_namespace_prefix("article.tags.length", "tags.length"),
            Some("article")
        );
        assert_eq!(
            stripped_namespace_prefix("interaction.action_results[0].x", "action_results[0].x"),
            Some("interaction")
        );
        assert_eq!(stripped_namespace_prefix("tags.length", "tags.length"), None);
        assert_eq!(
            stripped_namespace_prefix("metadata.title", "something.else"),
            None,
            "a raw field that does not end with the resolved path was not produced by stripping"
        );
    }

    #[test]
    fn find_field_path_returns_the_shallowest_chain_and_its_declaring_type() {
        let types = crawlberg_article_types();

        let tags = find_field_path("ScrapeResult", "tags", &types).expect("tags is reachable");
        assert_eq!(tags.path, "metadata.article.tags");
        assert_eq!(
            tags.owner_type, "ArticleMetadata",
            "the C accessor symbol is built from the declaring type, not the root"
        );

        let metadata = find_field_path("ScrapeResult", "metadata", &types).expect("metadata is a direct field");
        assert_eq!(metadata.path, "metadata");
        assert_eq!(metadata.owner_type, "ScrapeResult");

        assert!(find_field_path("ScrapeResult", "nowhere", &types).is_none());
    }

    fn test_backend_arg(trait_name: &str) -> crate::e2e::config::ArgMapping {
        crate::e2e::config::ArgMapping {
            name: "backend".into(),
            field: "backend".into(),
            arg_type: "test_backend".into(),
            optional: false,
            owned: false,
            element_type: None,
            go_type: None,
            vec_inner_is_ref: false,
            trait_name: Some(trait_name.to_string()),
        }
    }

    /// Pin: a `test_backend` arg whose trait IS registered still panics today,
    /// because `c::emit_test_backend` (`trait_bridge_snippet.rs`) is unimplemented —
    /// see its doc comment for why. `emit_test_backend` panics before ever handing
    /// `build_args_string_c` a value, so there is no sentinel left to accidentally
    /// splice into the call's argument list. This is the regression guard: it fails
    /// if that panic is ever replaced with a placeholder return and the call site
    /// stops checking it.
    #[test]
    #[should_panic(expected = "test-backend emitter is unimplemented")]
    fn registered_test_backend_trait_panics_because_c_backend_is_unimplemented() {
        use crate::core::config::TraitBridgeConfig;

        let bridge = TraitBridgeConfig {
            trait_name: "SampleBackend".into(),
            ..TraitBridgeConfig::default()
        };
        let config = ResolvedCrateConfig {
            trait_bridges: vec![bridge],
            ..ResolvedCrateConfig::default()
        };
        let fixture = Fixture {
            id: "register_sample_backend".into(),
            ..Fixture::default()
        };
        let args = vec![test_backend_arg("SampleBackend")];

        build_args_string_c(&fixture.input, &args, &HashMap::new(), &config, &[], &fixture);
    }

    /// An unregistered trait (no matching `[[crates.trait_bridges]]` entry) has no
    /// vtable to point at — generation must fail loudly instead of falling back to
    /// `NULL`. Unlike Kotlin's non-null interface parameter, nothing in C's type
    /// system would catch a bad `NULL` default at compile time, so this loud check
    /// is the only thing standing between a misconfigured `alef.toml` and either an
    /// uncompilable comment or a `NULL` vtable pointer reaching generated C.
    #[test]
    #[should_panic(expected = "no `[[crates.trait_bridges]]` entry")]
    fn unregistered_test_backend_trait_panics_instead_of_falling_back_to_null() {
        let config = ResolvedCrateConfig::default();
        let fixture = Fixture {
            id: "register_sample_backend".into(),
            ..Fixture::default()
        };
        let args = vec![test_backend_arg("SampleBackend")];

        build_args_string_c(&fixture.input, &args, &HashMap::new(), &config, &[], &fixture);
    }
}
