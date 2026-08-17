//! C e2e assertion and accessor rendering helpers.

use crate::core::config::ResolvedCrateConfig;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::config::{CallConfig, E2eConfig};
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
    config_sources: &FieldConfigSources,
) -> anyhow::Result<Option<String>> {
    let segments: Vec<&str> = resolved.split('.').collect();
    // cbindgen's `[export] prefix` is shouty-snake, not uppercase; re-deriving it here as
    // `to_uppercase` names types the generated header never declares for any prefix carrying an
    // internal word boundary (`SampleCore` -> `SAMPLECORE` vs the header's `SAMPLE_CORE`). ~keep
    let prefix_upper = crate::codegen::c_consumer::export_type_prefix(prefix);

    // Walk the path, starting from the root result type.
    let mut current_snake_type = result_type_name.to_snake_case();
    let mut current_handle = result_var.to_string();
    // True only while `current_snake_type` names a type the IR actually declares, which
    // is the precondition for using the IR as an oracle for the next segment. The `char*`
    // hop below sets `current_snake_type` from a *field* name rather than a type name, and
    // a `fields_c_types` value may name a C type with no IR counterpart at all; in either
    // case an IR type that happens to share the name is a coincidence, not the parent. ~keep
    let mut current_type_from_ir = type_defs.iter().any(|type_def| type_def.name == result_type_name);
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
            // Enum leaf: opaque enum pointer that needs `_to_string` conversion. Must run
            // BEFORE the opaque-struct-leaf check below: `try_emit_enum_accessor` gates
            // itself on `fields_enum` membership, but its `fields_c_types` value (the
            // enum's PascalCase type name, e.g. `DataNodeKind`) is indistinguishable in
            // shape from a struct's opaque type name -- both are non-primitive PascalCase
            // strings. Checking the opaque-struct filter first would swallow every
            // dotted-path enum leaf (it never inspects `fields_enum`) and hand back a bare
            // handle for the caller to `strcmp` against, which aborts at runtime. The flat
            // (single-segment) leaf path a few lines below in `test_function.rs` already
            // orders enum-before-opaque; this nested-path leaf must match it. ~keep
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
            // Every branch above proved the leaf exists — an explicit `fields_c_types`
            // declaration, or an enum registration. This default proves nothing: it emits
            // `{accessor_fn}()` on faith. When the IR knows the type the walk is standing
            // on and that type has no such field, cbindgen never generated that symbol, so
            // the assertion is rendered against a function that does not exist and the
            // failure surfaces at `cc` time inside a consumer — or, if the generated suite
            // is never compiled, not at all. Nothing upstream catches it either:
            // `FieldResolver::is_valid_for_result` only inspects a path's FIRST segment, so
            // `metadata.<anything>` passes as long as `metadata` is a real field, and the
            // `fail_on_unavailable_field_markers` scan only sees skip comments that this
            // path never writes. Fail here, matching the intermediate arm below. ~keep
            ensure_leaf_field_exists(LeafFieldCheck {
                prefix,
                accessor_fn: &accessor_fn,
                resolved,
                raw_field,
                segment,
                parent_snake_type: &current_snake_type,
                parent_is_ir_type: current_type_from_ir,
                declared_in_fields_c_types: fields_c_types.contains_key(&lookup_key),
                result_type_name,
                type_defs,
                result_fields_source: &config_sources.result_fields,
                fields_source: &config_sources.fields,
            })?;
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
                            segment,
                            seg_snake: &seg_snake,
                            segments_walked: &segments[..=i],
                            current_snake_type: &current_snake_type,
                            result_type_name,
                            type_defs,
                            fields_source: &config_sources.fields,
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
                current_type_from_ir = false;
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

            current_type_from_ir = type_defs.iter().any(|type_def| type_def.name == return_type_pascal);
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
/// motivated it (a real consumer's `ScrapeResult.metadata.article.tags` is three hops); a chain
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

/// Every dotted path from `root_type` down to a field whose snake_case name is
/// `field_snake`, one entry per distinct declaring type, shallowest first.
///
/// Only through `TypeRef::Named` struct fields — the same hops [`emit_nested_accessor`]
/// itself can walk, so a path this returns is one the C codegen could actually emit
/// accessors for.
///
/// More than one entry means the field name is ambiguous below `root_type`: two unrelated
/// types happen to share a field name (e.g. `kind` declared on both `DataNode`, values
/// `object`/`array`/`scalar`, and `StructureItem`, values `function`/`class`). A caller that
/// would otherwise propose a single alias fix MUST check `len() > 1` first and refuse to
/// guess — silently picking one binds the fixture to a field with a different value domain
/// instead of failing loudly. Finding this required tslp-owner to catch, by hand, a
/// generated diagnostic that suggested exactly that corrupting alias. ~keep
fn find_all_field_paths(
    root_type: &str,
    field_snake: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Vec<ResolvedFieldChain> {
    fn walk(
        type_name: &str,
        field_snake: &str,
        type_defs: &[crate::core::ir::TypeDef],
        depth: usize,
        seen: &mut HashSet<String>,
        out: &mut Vec<ResolvedFieldChain>,
    ) {
        if depth == 0 || !seen.insert(type_name.to_string()) {
            return;
        }
        let Some(type_def) = type_defs.iter().find(|type_def| type_def.name == type_name) else {
            return;
        };
        if let Some(field) = type_def
            .fields
            .iter()
            .find(|field| field.name.to_snake_case() == field_snake)
        {
            out.push(ResolvedFieldChain {
                path: field.name.to_snake_case(),
                owner_type: type_def.name.clone(),
            });
        }
        // Keep walking nested fields even after a direct hit above: a distinct type
        // reachable through a sibling or deeper field may ALSO declare `field_snake`, and
        // that collision is exactly what this function exists to surface.
        for field in &type_def.fields {
            let Some(nested) = super::named_type(&field.ty) else {
                continue;
            };
            let before = out.len();
            walk(nested, field_snake, type_defs, depth - 1, seen, out);
            for chain in &mut out[before..] {
                chain.path = format!("{}.{}", field.name.to_snake_case(), chain.path);
            }
        }
    }

    let mut out = Vec::new();
    walk(
        root_type,
        field_snake,
        type_defs,
        MAX_FIELD_PATH_SEARCH_DEPTH,
        &mut HashSet::new(),
        &mut out,
    );
    out.sort_by_key(|chain| chain.path.matches('.').count());
    out
}

/// The dotted path from `root_type` down to a field whose snake_case name is
/// `field_snake`, when the name is declared by exactly one reachable type.
///
/// Returns `None` both when no type reachable from `root_type` has such a field AND when
/// more than one distinct type does — see [`find_all_field_paths`] for why an ambiguous
/// name cannot collapse to a single answer here. Callers that need to tell those two cases
/// apart (to phrase a different diagnostic for each) must call `find_all_field_paths`
/// directly instead of this wrapper.
///
/// Test-only: every production caller needs the ambiguous and absent cases phrased differently, so
/// they all call `find_all_field_paths`. The wrapper stays to pin the collapse rule itself. ~keep
#[cfg(test)]
fn find_field_path(
    root_type: &str,
    field_snake: &str,
    type_defs: &[crate::core::ir::TypeDef],
) -> Option<ResolvedFieldChain> {
    let mut chains = find_all_field_paths(root_type, field_snake, type_defs);
    if chains.len() == 1 { chains.pop() } else { None }
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
    /// Which `fields` (alias table) governs the call this hop belongs to — threaded
    /// through so the diagnostic can name the one config key an edit will actually reach.
    fields_source: &'a EffectiveConfigSource,
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
        fields_source,
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

    match find_all_field_paths(result_type_name, seg_snake, type_defs).as_slice() {
        [chain] => {
            let alias_key = match stripped_namespace_prefix(raw_field, resolved) {
                Some(namespace) => format!("{namespace}.{}", segments_walked.join(".")),
                None => segments_walked.join("."),
            };
            let real_path = &chain.path;
            let real_symbol = format!("{prefix}_{}_{seg_snake}", chain.owner_type.to_snake_case());
            // Same shadowing rule as the leaf diagnostic's alias-fix branch, `fields`
            // instead of `result_fields`: a non-empty per-call `fields` override
            // replaces the global alias table outright (`E2eConfig::effective_fields`),
            // so the alias must be spelled under whichever one actually governs this
            // call. ~keep
            let fields_key = match fields_source {
                EffectiveConfigSource::Global => "`[crates.e2e.fields]`".to_string(),
                EffectiveConfigSource::PerCall(label) => format!("`{label}.fields`"),
            };
            let _ = write!(
                message,
                " Field `{seg_snake}` does exist below `{result_type_name}`, at \"{real_path}\" -- it is declared on \
                 `{owner}`, so the accessor that really exists is `{real_symbol}()`. Fix: add \
                 \"{alias_key}\" = \"{real_path}\" under {fields_key} so the fixture path resolves to the \
                 real chain. Only add \"{lookup_key}\" to `[crates.e2e.fields_c_types]` if `{accessor_fn}()` really \
                 is in the generated header.",
                owner = chain.owner_type,
            );
        }
        [] => {
            let _ = write!(
                message,
                " No type reachable from `{result_type_name}` has a field named `{seg_snake}` either, so the \
                 fixture's field path is the thing to check first -- declaring \"{lookup_key}\" cannot make \
                 `{accessor_fn}()` exist."
            );
        }
        chains => {
            let _ = write!(
                message,
                "{}",
                ambiguous_field_name_suffix(seg_snake, result_type_name, chains, fields_source)
            );
        }
    }

    message
}

/// Describe an ambiguous field name (declared by more than one distinct type reachable from
/// `result_type_name`) without picking one for the caller.
///
/// Shared by both diagnostics that would otherwise call [`find_field_path`] and silently take
/// its `None` for "field does not exist" -- an ambiguous name is a different failure mode
/// entirely, and conflating the two is how this diagnostic once recommended a corrupting
/// fix: `find_field_path` returned whichever same-named field it found first (e.g.
/// `DataNode.kind`, values `object`/`array`/`scalar`, vs an unrelated `StructureItem.kind`,
/// values `function`/`class`), and the message confidently suggested aliasing to it. Naming
/// every candidate chain, and refusing to recommend any single one of them, is the fix: the
/// operator -- who knows which chain the fixture actually means -- has to pick. ~keep
fn ambiguous_field_name_suffix(
    seg_snake: &str,
    result_type_name: &str,
    chains: &[ResolvedFieldChain],
    fields_source: &EffectiveConfigSource,
) -> String {
    let candidates: Vec<String> = chains
        .iter()
        .map(|chain| format!("\"{}\" (declared on `{}`)", chain.path, chain.owner_type))
        .collect();
    // Same shadowing rule as every other alias-fix branch in this file: a non-empty
    // per-call `fields` override replaces the global alias table outright
    // (`E2eConfig::effective_fields`), so the manual alias this suggests has to be
    // spelled under whichever one actually governs this call. ~keep
    let fields_key = match fields_source {
        EffectiveConfigSource::Global => "`[crates.e2e.fields]`".to_string(),
        EffectiveConfigSource::PerCall(label) => format!("`{label}.fields`"),
    };
    format!(
        " Field `{seg_snake}` is declared on {count} unrelated types reachable from `{result_type_name}`, with \
         different chains: {candidates} -- alef cannot tell which one the fixture means, and guessing risks \
         binding the assertion to a field with a different value domain than intended. Fix: add \
         \"<fixture path>\" = \"<the correct chain from the list above>\" under {fields_key} yourself, \
         after checking which candidate actually matches this fixture's data.",
        count = chains.len(),
        candidates = candidates.join(", "),
    )
}

/// Where a per-call-overridable e2e config collection (`result_fields`, `fields`, ...)
/// actually came from for a given call: the per-call override, or the global
/// `[crates.e2e]` default that only applies when the call declares no override of its
/// own.
///
/// Exists so a diagnostic can name the ONE config key an edit will actually reach.
/// Every `E2eConfig::effective_*` method (`effective_result_fields`, `effective_fields`,
/// ...) REPLACES the global collection outright when a call's own collection is
/// non-empty — it never merges the two — so a message that always names the global key
/// is actively wrong for every call with an override. That exact wrongness shipped once
/// already for `result_fields`: it told a consumer with a per-call override to edit the
/// global key, they did, nothing changed, and they filed it as a codegen blocker. The
/// same shape lived on, unfixed, in every diagnostic that names `[crates.e2e.fields]` —
/// this type is shared by both so the two checks cannot drift onto different resolution
/// logic the way the two hand-rolled versions of it did before this. ~keep
pub(super) enum EffectiveConfigSource {
    /// The global `[crates.e2e]` collection is what's in effect for this call.
    Global,
    /// A per-call override is what's in effect, named by its TOML table path (e.g.
    /// `"[crates.e2e.calls.crawl]"`, or the unnamed default `"[crates.e2e.call]"`).
    PerCall(String),
}

/// Determine which instance of a per-call-overridable collection governs `call`: pass
/// `call_has_override` as `!call.result_fields.is_empty()`, `!call.fields.is_empty()`,
/// etc. — whichever collection the caller is resolving — since that emptiness check is
/// the only part of [`E2eConfig::effective_result_fields`]/[`E2eConfig::effective_fields`]
/// (and siblings) that differs per collection; the "which key names it" logic that
/// follows is identical for all of them.
///
/// `call` is matched against `e2e_config.calls`/`e2e_config.call` by pointer identity
/// rather than by name, because a caller that reached `call` through
/// `resolve_call_for_fixture`'s `select_when` auto-routing does not get the matched key
/// back — the resolved `&CallConfig` reference is the only thing both the explicit-name
/// path and the auto-routed path have in common. ~keep
pub(super) fn describe_effective_config_source(
    e2e_config: &E2eConfig,
    call: &CallConfig,
    call_has_override: bool,
) -> EffectiveConfigSource {
    if !call_has_override {
        return EffectiveConfigSource::Global;
    }
    match e2e_config
        .calls
        .iter()
        .find(|(_, candidate)| std::ptr::eq(*candidate, call))
    {
        Some((name, _)) => EffectiveConfigSource::PerCall(format!("[crates.e2e.calls.{name}]")),
        None => EffectiveConfigSource::PerCall("[crates.e2e.call]".to_string()),
    }
}

/// The `result_fields` and `fields` sources actually in effect for one call, resolved
/// once per fixture and threaded through every nested-field diagnostic for it. Bundled
/// rather than passed as two loose parameters so a diagnostic that needs both (the leaf
/// diagnostic proposes a `result_fields` fix on one path and a `fields` alias fix on
/// another) cannot accidentally receive one resolved against a different call than the
/// other. ~keep
pub(super) struct FieldConfigSources {
    pub result_fields: EffectiveConfigSource,
    pub fields: EffectiveConfigSource,
}

impl FieldConfigSources {
    pub(super) fn resolve(e2e_config: &E2eConfig, call: &CallConfig) -> Self {
        Self {
            result_fields: describe_effective_config_source(e2e_config, call, !call.result_fields.is_empty()),
            fields: describe_effective_config_source(e2e_config, call, !call.fields.is_empty()),
        }
    }
}

/// Inputs for [`ensure_leaf_field_exists`]. A struct, not a handful of positional
/// `&str`s, for the same reason [`MissingIntermediateType`] is one.
pub(super) struct LeafFieldCheck<'a> {
    /// The crate's FFI symbol prefix, for naming the accessor that really exists.
    pub prefix: &'a str,
    /// The C symbol the caller is about to emit for this leaf.
    pub accessor_fn: &'a str,
    /// The (alias-resolved, already namespace-stripped) path being walked.
    pub resolved: &'a str,
    /// The fixture's own field path, before alias resolution and namespace stripping.
    pub raw_field: &'a str,
    /// The leaf segment itself, in its fixture spelling.
    pub segment: &'a str,
    /// The snake_case name of the type the accessor will be called on.
    pub parent_snake_type: &'a str,
    /// Whether `parent_snake_type` really names an IR type. False after a `char*` hop,
    /// where it holds a *field* name, and for a result type the IR does not model — in
    /// both cases an IR type sharing the name is a coincidence, not the parent.
    pub parent_is_ir_type: bool,
    /// Whether the operator declared this exact leaf in `[crates.e2e.fields_c_types]`.
    /// An explicit declaration is a claim that the accessor exists, and stays authoritative.
    pub declared_in_fields_c_types: bool,
    /// The type the walk started from.
    pub result_type_name: &'a str,
    pub type_defs: &'a [crate::core::ir::TypeDef],
    /// Which `result_fields` set governs the call this leaf belongs to — threaded through
    /// so the diagnostic can name the one config key an edit will actually reach.
    pub result_fields_source: &'a EffectiveConfigSource,
    /// Which `fields` (alias table) governs the call this leaf belongs to — same reason
    /// as `result_fields_source`, for the diagnostic's alias-fix branches.
    pub fields_source: &'a EffectiveConfigSource,
}

/// Reject a leaf field the IR positively says the parent type does not have.
///
/// The C accessor for a leaf is `{prefix}_{parent_snake}_{leaf_snake}`, built from a name
/// rather than looked up, so nothing but the IR can tell a real accessor from a fabricated
/// one. Default-allow everywhere the IR cannot answer: silence is not evidence of absence,
/// and this is a hard generation failure. ~keep
pub(super) fn ensure_leaf_field_exists(check: LeafFieldCheck<'_>) -> anyhow::Result<()> {
    if !check.parent_is_ir_type || check.declared_in_fields_c_types || check.resolved.contains('[') {
        return Ok(());
    }
    let seg_snake = check.segment.to_snake_case();
    let Some(parent) = check
        .type_defs
        .iter()
        .find(|type_def| type_def.name.to_snake_case() == check.parent_snake_type)
    else {
        return Ok(());
    };
    if parent
        .fields
        .iter()
        .any(|field| field.name.to_snake_case() == seg_snake)
    {
        return Ok(());
    }
    anyhow::bail!(
        "{}",
        unknown_leaf_field_diagnostic(UnknownLeafField {
            prefix: check.prefix,
            accessor_fn: check.accessor_fn,
            resolved: check.resolved,
            raw_field: check.raw_field,
            segment: check.segment,
            seg_snake: &seg_snake,
            parent_type: &parent.name,
            result_type_name: check.result_type_name,
            type_defs: check.type_defs,
            result_fields_source: check.result_fields_source,
            fields_source: check.fields_source,
        })
    )
}

/// Inputs for [`unknown_leaf_field_diagnostic`], resolved from a [`LeafFieldCheck`].
struct UnknownLeafField<'a> {
    prefix: &'a str,
    accessor_fn: &'a str,
    resolved: &'a str,
    raw_field: &'a str,
    segment: &'a str,
    seg_snake: &'a str,
    /// The IR type the walk is standing on, in its declared PascalCase spelling.
    parent_type: &'a str,
    result_type_name: &'a str,
    type_defs: &'a [crate::core::ir::TypeDef],
    result_fields_source: &'a EffectiveConfigSource,
    fields_source: &'a EffectiveConfigSource,
}

/// Explain a leaf segment that names no field of the type the walk arrived at.
///
/// The intermediate arm can at least offer "declare the C type"; a leaf cannot, because the
/// leaf accessor is emitted from the parent type and the field name alone. So the only
/// honest remedies are the alias that reconnects the fixture path to the real chain, or
/// fixing the fixture path — and the message has to say which, by looking up where the field
/// really lives. Same three facts as [`missing_intermediate_type_diagnostic`], same
/// resolution machinery, different remedy. ~keep
fn unknown_leaf_field_diagnostic(context: UnknownLeafField<'_>) -> String {
    let UnknownLeafField {
        prefix,
        accessor_fn,
        resolved,
        raw_field,
        segment,
        seg_snake,
        parent_type,
        result_type_name,
        type_defs,
        result_fields_source,
        fields_source,
    } = context;

    let mut message = format!(
        "e2e c codegen: fixture field \"{raw_field}\" (path \"{resolved}\") ends at segment \"{segment}\", but IR \
         type `{parent_type}` has no field `{seg_snake}`. The walk was about to emit `{accessor_fn}()`, a C symbol \
         no binding generates, so this assertion would have been rendered against a function that does not exist. \
         Nothing upstream rejects it: the field-availability oracle (`FieldResolver::is_valid_for_result`) only \
         inspects a path's FIRST segment, which is a real field here."
    );

    let namespace = stripped_namespace_prefix(raw_field, resolved);
    if let Some(namespace) = namespace {
        let _ = write!(
            message,
            " alef stripped the leading \"{namespace}\" from \"{raw_field}\" as a virtual namespace, because no \
             `[crates.e2e.fields]` alias maps it onto a real path and its first segment is not a `result_fields` \
             entry -- which is why the walk started at `{result_type_name}` instead of inside `{namespace}`."
        );
    }

    let chains = find_all_field_paths(result_type_name, seg_snake, type_defs);
    let chain = match chains.as_slice() {
        [chain] => chain,
        [] => {
            let _ = write!(
                message,
                " No type reachable from `{result_type_name}` has a field named `{seg_snake}` either, so the \
                 fixture's field path is the thing to fix -- there is no config entry that can spell a chain which \
                 does not exist."
            );
            return message;
        }
        chains => {
            let _ = write!(
                message,
                "{}",
                ambiguous_field_name_suffix(seg_snake, result_type_name, chains, fields_source)
            );
            return message;
        }
    };

    let real_path = &chain.path;
    let real_symbol = format!("{prefix}_{}_{seg_snake}", chain.owner_type.to_snake_case());
    let _ = write!(
        message,
        " Field `{seg_snake}` does exist below `{result_type_name}`, at \"{real_path}\" -- it is declared on \
         `{owner}`, so the accessor that really exists is `{real_symbol}()`.",
        owner = chain.owner_type,
    );

    // Two different config bugs produce this, and they take opposite fixes. When the real
    // chain starts with the prefix that was stripped, the fixture path was right all along
    // and the stripping was the mistake -- an alias would be an identity mapping and change
    // nothing, because `namespace_stripped_path` consults only `result_fields`. Otherwise the
    // fixture path genuinely names a chain that does not exist and needs an alias. ~keep
    match namespace.filter(|namespace| real_path.starts_with(&format!("{namespace}."))) {
        Some(namespace) => {
            // `result_fields` here means whichever set `effective_result_fields` actually
            // resolved for THIS call -- a non-empty per-call override replaces the global
            // default outright (see `E2eConfig::effective_result_fields`), so naming the
            // global key when a per-call override shadows it sends an edit nowhere: a
            // consumer followed exactly that instruction, edited the global key, and it
            // changed nothing because their call had its own `result_fields`. ~keep
            let result_fields_key = match result_fields_source {
                EffectiveConfigSource::Global => "`[crates.e2e].result_fields`".to_string(),
                EffectiveConfigSource::PerCall(label) => format!("`{label}.result_fields`"),
            };
            let _ = write!(
                message,
                " Fix: add \"{namespace}\" to {result_fields_key} so alef stops treating it as a virtual \
                 namespace prefix and walks it as the real field it is. An alias here would be an identity mapping \
                 and would not stop the stripping."
            );
        }
        None => {
            // Same shadowing rule, `[crates.e2e.fields]` instead of `.result_fields`: a
            // non-empty per-call `fields` override replaces the global alias table
            // outright (`E2eConfig::effective_fields`), so the alias must be spelled
            // under whichever one actually governs this call. ~keep
            let fields_key = match fields_source {
                EffectiveConfigSource::Global => "`[crates.e2e.fields]`".to_string(),
                EffectiveConfigSource::PerCall(label) => format!("`{label}.fields`"),
            };
            let _ = write!(
                message,
                " Fix: add \"{raw_field}\" = \"{real_path}\" under {fields_key} so the fixture path \
                 resolves to the real chain."
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
        let _ = writeln!(
            out,
            "    // skipped: {}",
            FieldSkip::NotAvailableOnResultType.message(f)
        );
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
        let _ = writeln!(
            out,
            "    // skipped: {}",
            FieldSkip::NotAvailableInCFfi.message(&field_expr)
        );
        return;
    }

    let field_is_primitive = primitive_locals.contains_key(&field_expr);
    let field_primitive_type = primitive_locals.get(&field_expr).cloned();
    // Opaque-handle fields (e.g. `usage` → SAMPLELLMUsage*, or an enum field a missing
    // `fields_enum`/IR-enum declaration failed to route through `try_emit_enum_accessor`)
    // cannot be treated as C strings — `strlen`/`strcmp`/`strstr`/`regexec` on a scalar
    // `AlefHandle` (`uint64_t`) is undefined behavior at best and a type error at worst.
    // Every string-shaped assertion arm below guards on this flag and falls back to a
    // non-zero existence check (matching the sentinel the handle actually uses) rather
    // than emitting a comparison against a value the ABI carries as an integer. ~keep
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
                } else if field_is_opaque_handle {
                    if expected.is_number() {
                        // A numeric expected value compares exactly against the handle.
                        let _ = writeln!(
                            out,
                            "    assert({field_expr} == {c_val} && \"equals assertion failed\");"
                        );
                    } else {
                        // A string expected value against a handle means the field should
                        // have been routed through `try_emit_enum_accessor` and wasn't;
                        // `field_expr == "..."` would compile as a pointer comparison that
                        // always lies, so weaken to existence instead of emitting that.
                        let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
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
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
            } else if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert({field_expr} != NULL && strstr({field_expr}, {c_val}) != NULL && \"expected to contain substring\");"
                );
            }
        }
        "contains_all" => {
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
            } else if let Some(values) = &assertion.values {
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
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
            } else if let Some(expected) = &assertion.value {
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
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
            } else if let Some(values) = &assertion.values {
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
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
            } else if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(
                    out,
                    "    assert(strncmp({field_expr}, {c_val}, strlen({c_val})) == 0 && \"expected to start with\");"
                );
            }
        }
        "ends_with" => {
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
            } else if let Some(expected) = &assertion.value {
                let c_val = json_to_c(expected);
                let _ = writeln!(out, "    assert(strlen({field_expr}) >= strlen({c_val}) && ");
                let _ = writeln!(
                    out,
                    "           strcmp({field_expr} + strlen({field_expr}) - strlen({c_val}), {c_val}) == 0 && \"expected to end with\");"
                );
            }
        }
        "min_length" => {
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
            } else if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(
                    out,
                    "    assert(strlen({field_expr}) >= {n} && \"expected minimum length\");"
                );
            }
        }
        "max_length" => {
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
            } else if let Some(val) = &assertion.value
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
            if field_is_opaque_handle {
                let _ = writeln!(out, "    assert({field_expr} != 0 && \"expected non-null handle\");");
            } else if let Some(expected) = &assertion.value {
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

    /// The neutral `FieldConfigSources` most tests want: neither `result_fields` nor
    /// `fields` has a per-call override in effect, so every diagnostic falls back to
    /// naming the global keys — the shape every test that isn't specifically exercising
    /// the per-call branch expects.
    fn global_sources() -> FieldConfigSources {
        FieldConfigSources {
            result_fields: EffectiveConfigSource::Global,
            fields: EffectiveConfigSource::Global,
        }
    }

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

    /// Task 1c backstop: even after the enum-vs-opaque-handle classification gap is
    /// fixed elsewhere, a field `render_assertion` is told is a genuine opaque handle
    /// must never be compared via `strcmp` — the ABI carries it as a scalar `uint64_t`
    /// `AlefHandle`, and `strcmp` on that is undefined behavior, not merely wrong. A
    /// numeric `equals` value must compare exactly instead.
    #[test]
    fn equals_assertion_on_opaque_handle_compares_numerically_not_via_strcmp() {
        let reachable: HashSet<String> = ["status".to_string()].into_iter().collect();
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
            field: Some("status".to_string()),
            value: Some(serde_json::json!(2)),
            ..Default::default()
        };
        let accessed_fields = [("status".to_string(), "status".to_string(), false)];
        let mut opaque_handle_locals = HashMap::new();
        opaque_handle_locals.insert("status".to_string(), "batch_status".to_string());

        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "sample",
            &resolver,
            &accessed_fields,
            &HashMap::new(),
            &opaque_handle_locals,
        );

        assert!(out.contains("status == 2"), "got: {out}");
        assert!(!out.contains("strcmp"), "must not strcmp a uint64_t handle: {out}");
    }

    /// Negative control / companion: a string expected value against an opaque handle
    /// means the field should have matched `try_emit_enum_accessor` and didn't. Rather
    /// than emit `status == "completed"` — a pointer comparison against a string literal
    /// that compiles cleanly and always lies — this weakens to an honest existence check,
    /// mirroring the precedent already established for `not_empty`/`is_empty`.
    #[test]
    fn equals_assertion_on_opaque_handle_with_string_value_falls_back_to_existence_check() {
        let reachable: HashSet<String> = ["status".to_string()].into_iter().collect();
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
            field: Some("status".to_string()),
            value: Some(serde_json::Value::String("completed".to_string())),
            ..Default::default()
        };
        let accessed_fields = [("status".to_string(), "status".to_string(), false)];
        let mut opaque_handle_locals = HashMap::new();
        opaque_handle_locals.insert("status".to_string(), "batch_status".to_string());

        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "sample",
            &resolver,
            &accessed_fields,
            &HashMap::new(),
            &opaque_handle_locals,
        );

        assert!(out.contains("status != 0"), "got: {out}");
        assert!(
            !out.contains("strcmp"),
            "must not compare a uint64_t handle to a string literal: {out}"
        );
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
            &global_sources(),
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
        walk_crawlberg_article_tags_with_sources(&global_sources())
    }

    fn walk_crawlberg_article_tags_with_sources(config_sources: &FieldConfigSources) -> anyhow::Error {
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
            config_sources,
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

    /// The `fields` sibling of the `result_fields` fix: a non-empty per-call `fields`
    /// override REPLACES the global alias table outright (`E2eConfig::effective_fields`),
    /// so when a per-call override is what's in effect, the alias-fix must name that
    /// call's own key -- never the global one, which an edit would not reach.
    #[test]
    fn missing_intermediate_type_names_the_per_call_fields_when_that_is_what_shadows() {
        let sources = FieldConfigSources {
            result_fields: EffectiveConfigSource::Global,
            fields: EffectiveConfigSource::PerCall("[crates.e2e.calls.scrape]".to_string()),
        };
        let message = walk_crawlberg_article_tags_with_sources(&sources).to_string();

        assert!(
            message.contains("\"article.tags\" = \"metadata.article.tags\" under `[crates.e2e.calls.scrape].fields`"),
            "must name the per-call key that actually governs this call: {message}"
        );
        assert!(
            !message.contains("under `[crates.e2e.fields]`"),
            "must not point at the global key when a per-call override shadows it: {message}"
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
            &global_sources(),
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

    /// The `pipeline_regeneration_gate` shape: `CompletionResponse.metadata -> Metadata`,
    /// `Metadata.document -> Document`, `Document.title`. `Metadata` deliberately has NO
    /// `title` field, so `metadata.title` only resolves through the
    /// `"metadata.title" = "metadata.document.title"` alias. ~keep
    fn completion_response_types() -> Vec<TypeDef> {
        vec![
            TypeDef {
                name: "CompletionResponse".into(),
                fields: vec![
                    FieldDef {
                        name: "id".into(),
                        ty: TypeRef::String,
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: "metadata".into(),
                        ty: TypeRef::Named("Metadata".into()),
                        ..FieldDef::default()
                    },
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Metadata".into(),
                fields: vec![FieldDef {
                    name: "document".into(),
                    ty: TypeRef::Named("Document".into()),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Document".into(),
                fields: vec![FieldDef {
                    name: "title".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ]
    }

    fn completion_response_c_types() -> HashMap<String, String> {
        HashMap::from([
            ("completion_response.metadata".to_string(), "Metadata".to_string()),
            ("metadata.document".to_string(), "Document".to_string()),
        ])
    }

    fn walk_completion_response(
        resolved: &str,
        raw_field: &str,
        fields_c_types: &HashMap<String, String>,
    ) -> anyhow::Result<(String, Option<String>)> {
        walk_completion_response_with_sources(resolved, raw_field, fields_c_types, &global_sources())
    }

    fn walk_completion_response_with_sources(
        resolved: &str,
        raw_field: &str,
        fields_c_types: &HashMap<String, String>,
        config_sources: &FieldConfigSources,
    ) -> anyhow::Result<(String, Option<String>)> {
        let mut output = String::new();
        let mut handles = Vec::new();
        let leaf = emit_nested_accessor(
            &mut output,
            "gatelib",
            resolved,
            "metadata_title",
            "result",
            fields_c_types,
            &HashSet::new(),
            &mut handles,
            "CompletionResponse",
            raw_field,
            &completion_response_types(),
            config_sources,
        )?;
        Ok((output, leaf))
    }

    /// The decisive case. Dropping the `[crates.e2e.fields]` alias leaves the fixture
    /// asserting `metadata.title`, whose leaf names no field of `Metadata`. Before this
    /// check the walk emitted `gatelib_metadata_title(metadata_handle)` — a symbol cbindgen
    /// never generates — and generation reported success, so the assertion was lost with no
    /// error, no warning and no skip comment for the
    /// `ALEF_E2E_STRICT_FIELD_AVAILABILITY` scan to find. ~keep
    #[test]
    fn unknown_leaf_field_is_an_error_not_a_phantom_accessor() {
        let error = walk_completion_response("metadata.title", "metadata.title", &completion_response_c_types())
            .expect_err("`title` is not a field of `Metadata`");

        let message = error.to_string();
        assert!(
            message.contains("IR type `Metadata` has no field `title`"),
            "must name the type and the field it lacks: {message}"
        );
        assert!(
            message.contains("gatelib_metadata_title()"),
            "must name the phantom symbol it refused to emit: {message}"
        );
        assert!(
            message.contains("only inspects a path's FIRST segment"),
            "must say why nothing upstream caught it: {message}"
        );
    }

    /// The remedy has to be spelled out, not implied: the fix for this shape is the alias,
    /// and the message must carry both sides of it.
    #[test]
    fn unknown_leaf_field_diagnostic_spells_the_alias_that_fixes_it() {
        let message = walk_completion_response("metadata.title", "metadata.title", &completion_response_c_types())
            .expect_err("`title` is not a field of `Metadata`")
            .to_string();

        assert!(
            message.contains("\"metadata.title\" = \"metadata.document.title\""),
            "must spell the alias that reconnects the fixture path: {message}"
        );
        assert!(
            message.contains("`[crates.e2e.fields]`"),
            "must name the table the alias goes in: {message}"
        );
        assert!(
            message.contains("gatelib_document_title()"),
            "must name the accessor that really exists: {message}"
        );
    }

    /// The `fields` sibling of the per-call `result_fields` test above: a per-call `fields`
    /// override REPLACES the global alias table outright, so the leaf diagnostic's alias-fix
    /// branch must name that call's own key too -- not just the intermediate-hop diagnostic's
    /// identical branch tested above.
    #[test]
    fn unknown_leaf_field_diagnostic_names_the_per_call_fields_when_that_is_what_shadows() {
        let sources = FieldConfigSources {
            result_fields: EffectiveConfigSource::Global,
            fields: EffectiveConfigSource::PerCall("[crates.e2e.calls.complete]".to_string()),
        };
        let message = walk_completion_response_with_sources(
            "metadata.title",
            "metadata.title",
            &completion_response_c_types(),
            &sources,
        )
        .expect_err("`title` is not a field of `Metadata`")
        .to_string();

        assert!(
            message.contains(
                "\"metadata.title\" = \"metadata.document.title\" under `[crates.e2e.calls.complete].fields`"
            ),
            "must name the per-call key that actually governs this call: {message}"
        );
        assert!(
            !message.contains("`[crates.e2e.fields]`"),
            "must not point at the global key when a per-call override shadows it: {message}"
        );
    }

    /// Positive control: with the alias in place the very same fixture field resolves, and
    /// the leaf still renders its accessor. The fix must not turn every nested assertion
    /// into a failure.
    #[test]
    fn resolvable_leaf_still_renders_its_accessor() {
        let (output, leaf) = walk_completion_response(
            "metadata.document.title",
            "metadata.title",
            &completion_response_c_types(),
        )
        .expect("every hop and the leaf resolve");

        assert_eq!(
            leaf, None,
            "a plain string leaf is a char*, not a primitive or a handle"
        );
        assert!(
            output.contains("char* metadata_title = gatelib_document_title(document_handle);"),
            "{output}"
        );
    }

    /// A leaf the operator declared in `[crates.e2e.fields_c_types]` is an explicit claim
    /// that the accessor exists, and stays authoritative — the IR check only governs the
    /// undeclared default. Without this escape hatch a field reached through a C type the
    /// IR does not model would become ungeneratable.
    #[test]
    fn explicitly_declared_leaf_type_overrides_the_ir_check() {
        let mut fields_c_types = completion_response_c_types();
        fields_c_types.insert("metadata.title".to_string(), "char*".to_string());

        let (output, _) = walk_completion_response("metadata.title", "metadata.title", &fields_c_types)
            .expect("an explicit fields_c_types declaration is authoritative");

        assert!(
            output.contains("char* metadata_title = gatelib_metadata_title(metadata_handle);"),
            "{output}"
        );
    }

    /// Default-allow guard: when the walk is standing on a type the IR does not declare,
    /// the IR cannot say whether the leaf exists, and silence must not be read as absence.
    #[test]
    fn leaf_on_a_type_the_ir_does_not_declare_is_not_rejected() {
        let mut output = String::new();
        let mut handles = Vec::new();
        emit_nested_accessor(
            &mut output,
            "gatelib",
            "metadata.title",
            "metadata_title",
            "result",
            &HashMap::from([("unmodelled_result.metadata".to_string(), "AlsoUnmodelled".to_string())]),
            &HashSet::new(),
            &mut handles,
            "UnmodelledResult",
            "metadata.title",
            &completion_response_types(),
            &global_sources(),
        )
        .expect("an unmodelled parent type must not be treated as proof the leaf is absent");

        assert!(
            output.contains("char* metadata_title = gatelib_also_unmodelled_title(metadata_handle);"),
            "{output}"
        );
    }

    /// The shape found shipped in `tree-sitter-language-pack/e2e/c/test_data_extraction.c`:
    /// `ProcessResult.data -> DataNode.kind`, asserted as `data.kind`, with `data` absent
    /// from `result_fields`. Stripping reduces the path to the bare leaf `kind`, which the
    /// availability oracle accepts because `kind` is IR-reachable on *some* type, and the
    /// flat branch then emits `ts_pack_process_result_kind()` — a symbol the generated
    /// header does not declare. ~keep
    fn ts_pack_types() -> Vec<TypeDef> {
        vec![
            TypeDef {
                name: "ProcessResult".into(),
                fields: vec![
                    FieldDef {
                        name: "language".into(),
                        ty: TypeRef::String,
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: "data".into(),
                        ty: TypeRef::Named("DataNode".into()),
                        ..FieldDef::default()
                    },
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "DataNode".into(),
                fields: vec![FieldDef {
                    name: "kind".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ]
    }

    fn check_ts_pack_stripped_leaf(
        declared_in_fields_c_types: bool,
        result_fields_source: &EffectiveConfigSource,
    ) -> anyhow::Result<()> {
        let types = ts_pack_types();
        ensure_leaf_field_exists(LeafFieldCheck {
            prefix: "ts_pack",
            accessor_fn: "ts_pack_process_result_kind",
            resolved: "kind",
            raw_field: "data.kind",
            segment: "kind",
            parent_snake_type: "process_result",
            parent_is_ir_type: true,
            declared_in_fields_c_types,
            result_type_name: "ProcessResult",
            type_defs: &types,
            result_fields_source,
            // Irrelevant to what this helper's callers assert on -- all of them exercise
            // the namespace-stripped-identity branch, which only reads
            // `result_fields_source`. Global is the neutral default. ~keep
            fields_source: &EffectiveConfigSource::Global,
        })
    }

    #[test]
    fn namespace_stripped_leaf_that_is_not_a_result_type_field_is_rejected() {
        let message = check_ts_pack_stripped_leaf(false, &EffectiveConfigSource::Global)
            .expect_err("`kind` is a field of `DataNode`, not of `ProcessResult`")
            .to_string();

        assert!(
            message.contains("IR type `ProcessResult` has no field `kind`"),
            "must name the type the accessor would have been called on: {message}"
        );
        assert!(
            message.contains("stripped the leading \"data\""),
            "must name the stripping that produced the bare leaf: {message}"
        );
        assert!(
            message.contains("ts_pack_data_node_kind()"),
            "must name the accessor that really exists: {message}"
        );
    }

    /// The remedy differs from the aliasable case and the message must not confuse them: an
    /// alias here would be `"data.kind" = "data.kind"`, an identity mapping that leaves
    /// `namespace_stripped_path` (which reads `result_fields`, not the alias table) stripping
    /// exactly as before. This is the global-in-effect case: no per-call override, so the
    /// global key really is the one an edit reaches.
    #[test]
    fn stripped_leaf_diagnostic_names_result_fields_not_an_identity_alias() {
        let message = check_ts_pack_stripped_leaf(false, &EffectiveConfigSource::Global)
            .expect_err("`kind` is a field of `DataNode`, not of `ProcessResult`")
            .to_string();

        assert!(
            message.contains("add \"data\" to `[crates.e2e].result_fields`"),
            "must name the config entry that stops the stripping: {message}"
        );
        assert!(
            !message.contains("\"data.kind\" = \"data.kind\""),
            "must not suggest an identity alias that changes nothing: {message}"
        );
    }

    /// The defect this type exists to prevent: a per-call `result_fields` override
    /// REPLACES the global default outright (`E2eConfig::effective_result_fields`), so
    /// when a per-call override is what's in effect, the "Fix:" must name that call's own
    /// key -- never the global one, which a consumer reported editing to no effect
    /// because their call's per-call list is what actually governed the walk.
    #[test]
    fn stripped_leaf_diagnostic_names_the_per_call_result_fields_when_that_is_what_shadows() {
        let source = EffectiveConfigSource::PerCall("[crates.e2e.calls.crawl]".to_string());
        let message = check_ts_pack_stripped_leaf(false, &source)
            .expect_err("`kind` is a field of `DataNode`, not of `ProcessResult`")
            .to_string();

        assert!(
            message.contains("add \"data\" to `[crates.e2e.calls.crawl].result_fields`"),
            "must name the per-call key that actually governs this call: {message}"
        );
        assert!(
            !message.contains("`[crates.e2e].result_fields`"),
            "must not point at the global key when a per-call override shadows it: {message}"
        );
    }

    /// The unnamed default call (`[crates.e2e.call]`) can also carry its own
    /// `result_fields` override -- it is looked up the same way a named call is, just
    /// with no entry in `e2e_config.calls` to match by pointer. The message must still
    /// name it, not fall back to claiming it's the global key.
    #[test]
    fn describe_effective_config_source_names_the_unnamed_default_call() {
        let e2e_config = E2eConfig::default();
        let call = CallConfig {
            result_fields: HashSet::from(["pages".to_string()]),
            ..CallConfig::default()
        };

        let source = describe_effective_config_source(&e2e_config, &call, !call.result_fields.is_empty());

        match source {
            EffectiveConfigSource::PerCall(label) => assert_eq!(label, "[crates.e2e.call]"),
            EffectiveConfigSource::Global => panic!("call_has_override == true must never resolve to Global"),
        }
    }

    /// The common case: a named call in `[crates.e2e.calls]` with its own override must be
    /// identified by that name, so the operator can find the exact TOML table to edit.
    #[test]
    fn describe_effective_config_source_names_a_call_matched_by_pointer_identity() {
        let mut e2e_config = E2eConfig::default();
        let crawl_call = CallConfig {
            result_fields: HashSet::from(["pages".to_string()]),
            ..CallConfig::default()
        };
        e2e_config.calls.insert("crawl".to_string(), crawl_call);

        let source = describe_effective_config_source(&e2e_config, &e2e_config.calls["crawl"], true);

        match source {
            EffectiveConfigSource::PerCall(label) => assert_eq!(label, "[crates.e2e.calls.crawl]"),
            EffectiveConfigSource::Global => panic!("call_has_override == true must never resolve to Global"),
        }
    }

    /// `call_has_override == false` always resolves to the global default, regardless of
    /// whether `call` is named or the unnamed default call -- the caller-computed
    /// emptiness check is authoritative, the function never re-derives it.
    #[test]
    fn describe_effective_config_source_is_global_when_the_caller_says_there_is_no_override() {
        let e2e_config = E2eConfig::default();
        let call = CallConfig {
            result_fields: HashSet::from(["pages".to_string()]),
            ..CallConfig::default()
        };

        assert!(matches!(
            describe_effective_config_source(&e2e_config, &call, false),
            EffectiveConfigSource::Global
        ));
    }

    /// `FieldConfigSources::resolve` is the one place production code should call this
    /// from: it derives `call_has_override` itself, once per collection, so the two
    /// checks (`result_fields`, `fields`) cannot drift onto different emptiness logic.
    #[test]
    fn field_config_sources_resolve_derives_each_collection_independently() {
        let mut e2e_config = E2eConfig::default();
        let call = CallConfig {
            result_fields: HashSet::from(["pages".to_string()]),
            // `fields` left empty: only `result_fields` has a per-call override.
            ..CallConfig::default()
        };
        e2e_config.calls.insert("crawl".to_string(), call);

        let sources = FieldConfigSources::resolve(&e2e_config, &e2e_config.calls["crawl"]);

        assert!(
            matches!(sources.result_fields, EffectiveConfigSource::PerCall(ref label) if label == "[crates.e2e.calls.crawl]")
        );
        assert!(matches!(sources.fields, EffectiveConfigSource::Global));
    }

    #[test]
    fn explicitly_declared_flat_leaf_type_overrides_the_ir_check() {
        check_ts_pack_stripped_leaf(true, &EffectiveConfigSource::Global)
            .expect("an explicit fields_c_types declaration is authoritative");
    }

    /// The full `ProcessResult.data -> DataNode.kind` shape once `data` is correctly
    /// registered in `result_fields` and `fields_c_types` names both hops (`data` ->
    /// `DataNode`, and the enum leaf `kind` -> `DataNodeKind`) — the "config already correct
    /// and complete" state a fixture author reaches after following `ts_pack_types`'s
    /// diagnostic. `data` is `Optional<Named>` here, matching the real IR (`pub data:
    /// Option<DataNode>`), not the bare `Named` `ts_pack_types` uses — this is the actual
    /// shape `emit_nested_accessor` must walk through the `Option`. ~keep
    fn ts_pack_types_with_optional_data_and_enum_kind() -> Vec<TypeDef> {
        vec![
            TypeDef {
                name: "ProcessResult".into(),
                fields: vec![FieldDef {
                    name: "data".into(),
                    ty: TypeRef::Optional(Box::new(TypeRef::Named("DataNode".into()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "DataNode".into(),
                fields: vec![
                    FieldDef {
                        name: "kind".into(),
                        ty: TypeRef::Named("DataNodeKind".into()),
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: "children".into(),
                        ty: TypeRef::Vec(Box::new(TypeRef::Named("DataNode".into()))),
                        ..FieldDef::default()
                    },
                ],
                ..TypeDef::default()
            },
        ]
    }

    /// Both halves of the ts-pack fix at once: the walk must go through the `Option<DataNode>`
    /// hop AND land on the enum branch, not the opaque-struct branch, for the `DataNodeKind`
    /// leaf. Before the branch-ordering fix, this leaf matched the opaque-struct filter first
    /// (`DataNodeKind` is PascalCase, non-primitive, not `char*`/`skip`) and emitted a bare
    /// handle the caller would `strcmp` against instead of a `_to_string`-converted `char*`.
    #[test]
    fn dotted_path_through_optional_field_reaches_enum_leaf() {
        let types = ts_pack_types_with_optional_data_and_enum_kind();
        let fields_c_types = HashMap::from([
            ("process_result.data".to_string(), "DataNode".to_string()),
            ("data_node.kind".to_string(), "DataNodeKind".to_string()),
        ]);
        let fields_enum: HashSet<String> = ["data.kind".to_string()].into_iter().collect();
        let mut output = String::new();
        let mut handles = Vec::new();

        let result = emit_nested_accessor(
            &mut output,
            "ts_pack",
            "data.kind",
            "data_kind",
            "result",
            &fields_c_types,
            &fields_enum,
            &mut handles,
            "ProcessResult",
            "data.kind",
            &types,
            &global_sources(),
        )
        .expect("the Option<DataNode> hop and the enum leaf both resolve");

        assert_eq!(
            result, None,
            "an enum leaf returns Ok(None) (render_assertion reads it as a plain char*), not \
             Ok(Some(opaque_type)) -- a Some here would mean the opaque-struct branch fired instead"
        );
        assert!(
            output.contains("data_handle = ts_pack_process_result_data(result)"),
            "must walk into the Option<DataNode> field via the FFI accessor: {output}"
        );
        assert!(
            output.contains("ts_pack_data_node_kind_to_string("),
            "must convert the enum leaf via its _to_string accessor, proving the enum branch \
             (not the opaque-struct branch) fired: {output}"
        );
        assert!(
            !output.contains("AlefHandle data_kind = kind_handle"),
            "must not fall through to the opaque-struct branch's bare handle assignment: {output}"
        );
    }

    /// Two unrelated types below the same result type declaring a field with the same name
    /// (`DataNode.kind`, values object/array/scalar, vs `StructureItem.kind`, values
    /// function/class) must not collapse into a single confident alias suggestion — this is
    /// the tslp scenario that motivated the fix: the pre-fix diagnostic would have proposed
    /// exactly `"data.kind" = "structure.kind"`, silently rebinding the assertion to the
    /// wrong field.
    #[test]
    fn ambiguous_leaf_field_name_does_not_suggest_a_specific_alias() {
        let types = vec![
            TypeDef {
                name: "ProcessResult".into(),
                fields: vec![
                    FieldDef {
                        name: "data".into(),
                        ty: TypeRef::Named("DataNode".into()),
                        ..FieldDef::default()
                    },
                    FieldDef {
                        name: "structure".into(),
                        ty: TypeRef::Named("StructureItem".into()),
                        ..FieldDef::default()
                    },
                ],
                ..TypeDef::default()
            },
            TypeDef {
                name: "DataNode".into(),
                fields: vec![FieldDef {
                    name: "kind".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "StructureItem".into(),
                fields: vec![FieldDef {
                    name: "kind".into(),
                    ty: TypeRef::String,
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ];

        let message = ensure_leaf_field_exists(LeafFieldCheck {
            prefix: "ts_pack",
            accessor_fn: "ts_pack_process_result_kind",
            resolved: "kind",
            raw_field: "data.kind",
            segment: "kind",
            parent_snake_type: "process_result",
            parent_is_ir_type: true,
            declared_in_fields_c_types: false,
            result_type_name: "ProcessResult",
            type_defs: &types,
            result_fields_source: &EffectiveConfigSource::Global,
            fields_source: &EffectiveConfigSource::Global,
        })
        .expect_err("`kind` is not a field of `ProcessResult` itself")
        .to_string();

        assert!(
            !message.contains("\"data.kind\" = \"structure.kind\""),
            "must never suggest binding DataNode.kind's field onto the unrelated \
             StructureItem.kind: {message}"
        );
        assert!(
            message.contains("\"data.kind\""),
            "must still name the ambiguous candidate chain rooted at `data`: {message}"
        );
        assert!(
            message.contains("\"structure.kind\""),
            "must still name the ambiguous candidate chain rooted at `structure`: {message}"
        );
        assert!(
            message.contains("DataNode") && message.contains("StructureItem"),
            "must name both declaring types so the operator can tell them apart: {message}"
        );
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
