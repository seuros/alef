//! `CHUNKS_RECIPE` synthetic assertion handlers for Zig.
//!
//! Split out of `assertions.rs`, which is at the repo's 1,000-line file-modularization cap and
//! may not grow. Zig walks raw `std.json.Value` rather than typed struct fields, so — unlike
//! every other backend's `assertion_recipes::chunks_result_var` caller — the anchored prefix here
//! is rendered through `json_path_expr` (`super::json_path_expr`), the same JSON-navigation
//! renderer every other zig assertion path already goes through, rather than
//! `FieldResolver::accessor`. See `zig_chunks_result_var` for why the hardcoded
//! `{result_var}.object.get("chunks")` these four handlers used before could reference a key the
//! call's own result JSON object never carries, for a consumer whose result type is an envelope.

use std::fmt::Write as FmtWrite;

use crate::e2e::field_access::{FieldResolver, LeafAnchor};
use crate::e2e::fixture::Assertion;

/// Render one of the four `CHUNKS_RECIPE` synthetic fields, or return `false` when `field` is
/// none of them so the caller's match falls through to its other arms.
pub(super) fn try_render(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field: &str,
    field_resolver: &FieldResolver,
) -> bool {
    match field {
        "chunks_have_content" => {
            let result_var = &zig_chunks_result_var(field_resolver, result_var);
            emit_zig_chunks_predicate(
                out,
                result_var,
                assertion.assertion_type.as_str(),
                "c.object.get(\"content\")",
                "chunks_have_content",
                true,
            );
            true
        }
        "chunks_have_heading_context" => {
            // `heading_context` sitting one JSON hop deeper than `content`/`embedding` (inside
            // `chunk.metadata`) is not the same thing as "not derivable from JSON value alone"
            // -- a chunk missing the key is exactly the signal every other backend's typed
            // `nil`/`None` check reads (serde drops `None` fields), not an unrepresentable one.
            // It is checkable the same way `emit_zig_chunks_predicate` already checks
            // `content`/`embedding`, just with an extra `.object.get(...)` hop. ~keep
            let result_var = &zig_chunks_result_var(field_resolver, result_var);
            emit_zig_chunks_heading_context_predicate(
                out,
                result_var,
                assertion.assertion_type.as_str(),
                "chunks_have_heading_context",
                false,
            );
            true
        }
        "first_chunk_starts_with_heading" => {
            let result_var = &zig_chunks_result_var(field_resolver, result_var);
            emit_zig_chunks_heading_context_predicate(
                out,
                result_var,
                assertion.assertion_type.as_str(),
                "first_chunk_starts_with_heading",
                true,
            );
            true
        }
        "chunks_have_embeddings" => {
            let result_var = &zig_chunks_result_var(field_resolver, result_var);
            emit_zig_chunks_predicate(
                out,
                result_var,
                assertion.assertion_type.as_str(),
                "c.object.get(\"embedding\")",
                "chunks_have_embeddings",
                false,
            );
            true
        }
        _ => false,
    }
}

/// The JSON-navigation expression the `CHUNKS_RECIPE` synthetic handlers should walk to before
/// their own `.object.get("chunks")` step, when the call's own result JSON object doesn't carry
/// `chunks` directly but a `result_fields`-declared envelope prefix does.
///
/// Every other backend's equivalent (`assertion_recipes::chunks_result_var`) delegates to
/// `FieldResolver::accessor`, built for typed struct field access. Zig walks raw
/// `std.json.Value` instead, so this reuses `json_path_expr` — the same JSON-navigation renderer
/// every other zig assertion path already goes through — on the anchored prefix instead.
fn zig_chunks_result_var(field_resolver: &FieldResolver, result_var: &str) -> String {
    match field_resolver.anchor_leaf(crate::e2e::codegen::assertion_recipes::CHUNKS_RECIPE) {
        Some(LeafAnchor::Prefixed(prefix)) => super::json_path_expr(result_var, &prefix, field_resolver),
        _ => result_var.to_string(),
    }
}

fn emit_zig_chunks_predicate(
    out: &mut String,
    result_var: &str,
    assertion_type: &str,
    chunk_field_accessor: &str,
    field_name: &str,
    require_non_empty_string: bool,
) {
    let _ = writeln!(out, "    {{");
    let _ = writeln!(out, "        const _chunks_opt = {result_var}.object.get(\"chunks\");");
    let _ = writeln!(out, "        var _all: bool = true;");
    let _ = writeln!(out, "        if (_chunks_opt) |_chunks_val| {{");
    let _ = writeln!(out, "            if (_chunks_val == .array) {{");
    let _ = writeln!(
        out,
        "                if (_chunks_val.array.items.len == 0) _all = false;"
    );
    let _ = writeln!(out, "                for (_chunks_val.array.items) |c| {{");
    let _ = writeln!(out, "                    if (c != .object) {{ _all = false; break; }}");
    let _ = writeln!(out, "                    const _v = {chunk_field_accessor};");
    if require_non_empty_string {
        let _ = writeln!(
            out,
            "                    if (_v == null or _v.? != .string or _v.?.string.len == 0) {{ _all = false; break; }}"
        );
    } else {
        let _ = writeln!(
            out,
            "                    if (_v == null or _v.? == .null) {{ _all = false; break; }}"
        );
    }
    let _ = writeln!(out, "                }}");
    let _ = writeln!(out, "            }} else {{ _all = false; }}");
    let _ = writeln!(out, "        }} else {{ _all = false; }}");
    match assertion_type {
        "is_true" => {
            let _ = writeln!(out, "        try testing.expect(_all);");
        }
        "is_false" => {
            let _ = writeln!(out, "        try testing.expect(!_all);");
        }
        _ => {
            let _ = writeln!(
                out,
                "        // skipped: unsupported assertion type on synthetic field '{field_name}'"
            );
        }
    }
    let _ = writeln!(out, "    }}");
}

/// Emit a boolean predicate over `result_var.chunks[]`'s `metadata.heading_context` field,
/// read directly off the parsed JSON tree rather than approximated via `content` shape.
/// `heading_context` sits one hop deeper than the fields `emit_zig_chunks_predicate` checks
/// (inside `chunk.metadata`), so it needs its own two-level `.object.get(...)` walk instead of
/// that helper's single accessor string. A chunk missing the `metadata` key, whose `metadata`
/// is not an object, missing the `heading_context` key, or holding JSON `null` there are all
/// read as "no heading context" — the same signal every other backend's typed `nil`/`None`
/// check reads (serde drops `None` fields from the JSON entirely).
///
/// When `only_first` is set, the loop inspects exactly the first chunk (`first_chunk_starts_with_heading`)
/// instead of requiring every chunk to satisfy the predicate (`chunks_have_heading_context`). ~keep
fn emit_zig_chunks_heading_context_predicate(
    out: &mut String,
    result_var: &str,
    assertion_type: &str,
    field_name: &str,
    only_first: bool,
) {
    let _ = writeln!(out, "    {{");
    let _ = writeln!(out, "        const _chunks_opt = {result_var}.object.get(\"chunks\");");
    let _ = writeln!(out, "        var _all: bool = true;");
    let _ = writeln!(out, "        if (_chunks_opt) |_chunks_val| {{");
    let _ = writeln!(out, "            if (_chunks_val == .array) {{");
    let _ = writeln!(
        out,
        "                if (_chunks_val.array.items.len == 0) _all = false;"
    );
    let _ = writeln!(out, "                for (_chunks_val.array.items) |c| {{");
    let _ = writeln!(out, "                    if (c != .object) {{ _all = false; break; }}");
    let _ = writeln!(out, "                    var _has_heading = false;");
    let _ = writeln!(out, "                    if (c.object.get(\"metadata\")) |_meta| {{");
    let _ = writeln!(out, "                        if (_meta == .object) {{");
    let _ = writeln!(
        out,
        "                            if (_meta.object.get(\"heading_context\")) |_hc| {{"
    );
    let _ = writeln!(
        out,
        "                                if (_hc != .null) {{ _has_heading = true; }}"
    );
    let _ = writeln!(out, "                            }}");
    let _ = writeln!(out, "                        }}");
    let _ = writeln!(out, "                    }}");
    let _ = writeln!(out, "                    if (!_has_heading) {{ _all = false; break; }}");
    if only_first {
        let _ = writeln!(out, "                    break;");
    }
    let _ = writeln!(out, "                }}");
    let _ = writeln!(out, "            }} else {{ _all = false; }}");
    let _ = writeln!(out, "        }} else {{ _all = false; }}");
    match assertion_type {
        "is_true" => {
            let _ = writeln!(out, "        try testing.expect(_all);");
        }
        "is_false" => {
            let _ = writeln!(out, "        try testing.expect(!_all);");
        }
        _ => {
            let _ = writeln!(
                out,
                "        // skipped: unsupported assertion type on synthetic field '{field_name}'"
            );
        }
    }
    let _ = writeln!(out, "    }}");
}
