//! Go assertion rendering.

use crate::e2e::codegen::assertion_recipes::chunks_result_var;
use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::escape::go_string_literal;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

use super::assertion_field_shape::resolve_assertion_field_shape;
use super::assertion_render_helpers::{
    contains_value_expression, render_count_assertion, render_guarded_scalar_comparison, render_length_assertion,
    string_value_expression,
};
use super::json_values::json_to_go;
use super::method_calls::build_go_method_call;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    import_alias: &str,
    field_resolver: &FieldResolver,
    optional_locals: &std::collections::HashMap<String, String>,
    numeric_scalar_fields: &std::collections::HashSet<&str>,
    result_is_simple: bool,
    result_is_array: bool,
    is_streaming: bool,
    streaming_item_type: Option<&str>,
) {
    if !result_is_simple && let Some(f) = &assertion.field {
        let embed_deref = format!("(*{result_var})");
        if let Some(reason) = crate::e2e::codegen::assertion_recipes::chunks_synthetic_skip_reason(f, field_resolver) {
            let _ = writeln!(out, "\t// skipped: {reason}");
            return;
        }

        match f.as_str() {
            "chunks_have_content" => {
                let result_var = &chunks_result_var(field_resolver, "go", result_var);
                let pred = format!(
                    "func() bool {{ chunks := {result_var}.Chunks; if chunks == nil {{ return false }}; for _, c := range chunks {{ if c.Content == \"\" {{ return false }} }}; return true }}()"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "\tassert.True(t, {pred}, \"expected true\")");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "\tassert.False(t, {pred}, \"expected false\")");
                    }
                    _ => {
                        let _ = writeln!(out, "\t// skipped: unsupported assertion type on synthetic field '{f}'");
                    }
                }
                return;
            }
            "chunks_have_embeddings" => {
                let result_var = &chunks_result_var(field_resolver, "go", result_var);
                let pred = format!(
                    "func() bool {{ chunks := {result_var}.Chunks; if chunks == nil {{ return false }}; for _, c := range chunks {{ if c.Embedding == nil || len(*c.Embedding) == 0 {{ return false }} }}; return true }}()"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "\tassert.True(t, {pred}, \"expected true\")");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "\tassert.False(t, {pred}, \"expected false\")");
                    }
                    _ => {
                        let _ = writeln!(out, "\t// skipped: unsupported assertion type on synthetic field '{f}'");
                    }
                }
                return;
            }
            "chunks_have_heading_context" => {
                let result_var = &chunks_result_var(field_resolver, "go", result_var);
                let pred = format!(
                    "func() bool {{ chunks := {result_var}.Chunks; if chunks == nil {{ return false }}; for _, c := range chunks {{ if c.Metadata.HeadingContext == nil {{ return false }} }}; return true }}()"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "\tassert.True(t, {pred}, \"expected true\")");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "\tassert.False(t, {pred}, \"expected false\")");
                    }
                    _ => {
                        let _ = writeln!(out, "\t// skipped: unsupported assertion type on synthetic field '{f}'");
                    }
                }
                return;
            }
            "first_chunk_starts_with_heading" => {
                let result_var = &chunks_result_var(field_resolver, "go", result_var);
                let pred = format!(
                    "func() bool {{ chunks := {result_var}.Chunks; if chunks == nil || len(chunks) == 0 {{ return false }}; return chunks[0].Metadata.HeadingContext != nil }}()"
                );
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "\tassert.True(t, {pred}, \"expected true\")");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "\tassert.False(t, {pred}, \"expected false\")");
                    }
                    _ => {
                        let _ = writeln!(out, "\t// skipped: unsupported assertion type on synthetic field '{f}'");
                    }
                }
                return;
            }
            "embeddings" => {
                match assertion.assertion_type.as_str() {
                    "count_equals" => {
                        if let Some(val) = &assertion.value
                            && let Some(n) = val.as_u64()
                        {
                            let _ = writeln!(
                                out,
                                "\tassert.Equal(t, {n}, len({embed_deref}), \"expected exactly {n} elements\")"
                            );
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value
                            && let Some(n) = val.as_u64()
                        {
                            let _ = writeln!(
                                out,
                                "\tassert.GreaterOrEqual(t, len({embed_deref}), {n}, \"expected at least {n} elements\")"
                            );
                        }
                    }
                    "not_empty" => {
                        let _ = writeln!(
                            out,
                            "\tassert.NotEmpty(t, {embed_deref}, \"expected non-empty embeddings\")"
                        );
                    }
                    "is_empty" => {
                        let _ = writeln!(out, "\tassert.Empty(t, {embed_deref}, \"expected empty embeddings\")");
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "\t// skipped: unsupported assertion type on synthetic field 'embeddings'"
                        );
                    }
                }
                return;
            }
            "embedding_dimensions" => {
                let expr = format!(
                    "func() int {{ if len({embed_deref}) == 0 {{ return 0 }}; return len({embed_deref}[0]) }}()"
                );
                match assertion.assertion_type.as_str() {
                    "equals" => {
                        if let Some(val) = &assertion.value
                            && let Some(n) = val.as_u64()
                        {
                            let _ = writeln!(
                                out,
                                "\tif {expr} != {n} {{\n\t\tt.Errorf(\"equals mismatch: got %v\", {expr})\n\t}}"
                            );
                        }
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value
                            && let Some(n) = val.as_u64()
                        {
                            let _ = writeln!(out, "\tassert.Greater(t, {expr}, {n}, \"expected > {n}\")");
                        }
                    }
                    _ => {
                        let _ = writeln!(
                            out,
                            "\t// skipped: unsupported assertion type on synthetic field 'embedding_dimensions'"
                        );
                    }
                }
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!(
                            "func() bool {{ for _, e := range {embed_deref} {{ if len(e) == 0 {{ return false }} }}; return true }}()"
                        )
                    }
                    "embeddings_finite" => {
                        format!(
                            "func() bool {{ for _, e := range {embed_deref} {{ for _, v := range e {{ if v != v || v == float32(1.0/0.0) || v == float32(-1.0/0.0) {{ return false }} }} }}; return true }}()"
                        )
                    }
                    "embeddings_non_zero" => {
                        format!(
                            "func() bool {{ for _, e := range {embed_deref} {{ hasNonZero := false; for _, v := range e {{ if v != 0 {{ hasNonZero = true; break }} }}; if !hasNonZero {{ return false }} }}; return true }}()"
                        )
                    }
                    "embeddings_normalized" => {
                        format!(
                            "func() bool {{ for _, e := range {embed_deref} {{ var n float64; for _, v := range e {{ n += float64(v) * float64(v) }}; if n < 0.999 || n > 1.001 {{ return false }} }}; return true }}()"
                        )
                    }
                    _ => unreachable!(),
                };
                match assertion.assertion_type.as_str() {
                    "is_true" => {
                        let _ = writeln!(out, "\tassert.True(t, {pred}, \"expected true\")");
                    }
                    "is_false" => {
                        let _ = writeln!(out, "\tassert.False(t, {pred}, \"expected false\")");
                    }
                    _ => {
                        let _ = writeln!(out, "\t// skipped: unsupported assertion type on synthetic field '{f}'");
                    }
                }
                return;
            }
            "keywords" | "keywords_count" => {
                let _ = writeln!(
                    out,
                    "\t// skipped: {}",
                    FieldSkip::NotAvailableOnGoProcessingResult.message(f)
                );
                return;
            }
            _ => {}
        }
    }

    if !result_is_simple
        && is_streaming
        && let Some(f) = &assertion.field
        && !f.is_empty()
        && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f)
    {
        if let Some(expr) =
            crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor_with_streaming_context(
                f,
                "go",
                "chunks",
                None,
                streaming_item_type,
            )
        {
            // ~keep The value-narrowing arms below used to fall through to nothing when the
            // fixture's value did not survive `as_u64()` / the string pattern, so the assertion
            // disappeared with no line for any funnel to count.
            let value_skip = || streaming_assertion_value_skip_line("\t", "//", f, &assertion.assertion_type);
            match assertion.assertion_type.as_str() {
                "count_min" => {
                    if let Some(val) = &assertion.value
                        && let Some(n) = val.as_u64()
                    {
                        let _ = writeln!(
                            out,
                            "\tassert.GreaterOrEqual(t, len({expr}), {n}, \"expected >= {n} chunks\")"
                        );
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "count_equals" => {
                    if let Some(val) = &assertion.value
                        && let Some(n) = val.as_u64()
                    {
                        let _ = writeln!(
                            out,
                            "\tassert.Equal(t, {n}, len({expr}), \"expected exactly {n} chunks\")"
                        );
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "equals" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = go_string_literal(s);
                        let is_deep_path = f.contains('.') || f.contains('[');
                        let safe_expr = if is_deep_path {
                            format!("func() string {{ v := {expr}; if v == nil {{ return \"\" }}; return *v }}()")
                        } else {
                            expr.clone()
                        };
                        let _ = writeln!(out, "\tassert.Equal(t, {escaped}, {safe_expr})");
                    } else if let Some(val) = &assertion.value
                        && let Some(n) = val.as_u64()
                    {
                        let _ = writeln!(out, "\tassert.Equal(t, {n}, {expr})");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "not_empty" => {
                    let _ = writeln!(out, "\tassert.NotEmpty(t, {expr}, \"expected non-empty\")");
                }
                "is_empty" => {
                    let _ = writeln!(out, "\tassert.Empty(t, {expr}, \"expected empty\")");
                }
                "is_true" => {
                    let _ = writeln!(out, "\tassert.True(t, {expr}, \"expected true\")");
                }
                "is_false" => {
                    let _ = writeln!(out, "\tassert.False(t, {expr}, \"expected false\")");
                }
                "greater_than" => {
                    if let Some(val) = &assertion.value
                        && let Some(n) = val.as_u64()
                    {
                        let _ = writeln!(out, "\tassert.Greater(t, {expr}, {n}, \"expected > {n}\")");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "greater_than_or_equal" => {
                    if let Some(val) = &assertion.value
                        && let Some(n) = val.as_u64()
                    {
                        let _ = writeln!(out, "\tassert.GreaterOrEqual(t, {expr}, {n}, \"expected >= {n}\")");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                "contains" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = crate::e2e::escape::go_string_literal(s);
                        let _ = writeln!(out, "\tassert.Contains(t, {expr}, {escaped}, \"expected to contain\")");
                    } else {
                        let _ = writeln!(out, "{}", value_skip());
                    }
                }
                _ => {
                    let _ = writeln!(
                        out,
                        "{}",
                        streaming_assertion_type_skip_line("\t", "//", f, &assertion.assertion_type)
                    );
                }
            }
        } else {
            // ~keep The accessor returns `None` for reachable inputs (a `stream.has_*_event`
            // predicate with no resolved item type, for one), and this branch used to be absent:
            // the assertion vanished with no line for `fail_on_unavailable_field_markers` to see,
            // so a clean strict-gate run was indistinguishable from one that dropped it. alef's
            // own streaming adapter owns the gap, so it is counted, never fatal.
            let _ = writeln!(
                out,
                "\t// skipped: {}",
                FieldSkip::StreamingAssertionOnUnsupportedField.message(f)
            );
        }
        return;
    }

    if !result_is_simple
        && let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        let _ = writeln!(out, "\t// skipped: {}", FieldSkip::NotAvailableOnResultType.message(f));
        return;
    }

    // Bracket-wildcard traversal (`links[].link_type`) means "every element", so it must
    // scan the whole slice. Go has no expression-level `any`, so this emits statements
    // rather than folding into `field_expr` below — and it must run before `field_expr`
    // is built, since that path lowers the wildcard to index 0 and would silently assert
    // on one element only. ~keep
    if !result_is_simple
        && let Some(f) = assertion.field.as_deref()
        && !f.is_empty()
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        render_wildcard_assertion(out, assertion, result_var, field_resolver, f, &array_part, &elem_part);
        return;
    }

    let field_expr = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => {
                if let Some(local_var) = optional_locals.get(f.as_str()) {
                    local_var.clone()
                } else {
                    field_resolver.accessor(f, "go", result_var)
                }
            }
            _ => result_var.to_string(),
        }
    };

    let field_shape = resolve_assertion_field_shape(assertion, field_resolver, optional_locals);
    let is_optional = field_shape.is_optional;
    let receiver_is_pointer = field_shape.is_pointer;
    let receiver_is_nullable = field_shape.is_nullable;
    let field_is_array_for_len = field_shape.is_array_for_len;
    let field_is_data_interface = field_shape.is_data_interface;
    let field_expr = if receiver_is_pointer
        && field_expr.starts_with("len(")
        && field_expr.ends_with(')')
        && !field_is_array_for_len
    {
        let inner = &field_expr[4..field_expr.len() - 1];
        format!("len(*{inner})")
    } else {
        field_expr
    };
    let nil_guard_expr = if receiver_is_pointer && field_expr.starts_with("len(*") {
        Some(field_expr[5..field_expr.len() - 1].to_string())
    } else {
        None
    };
    let expression_is_length = field_expr.starts_with("len(");
    let field_is_pointer = receiver_is_pointer && !expression_is_length;
    let field_is_nullable = receiver_is_nullable && !expression_is_length;
    let nullable_guard_expr = nil_guard_expr
        .clone()
        .or_else(|| field_is_nullable.then(|| field_expr.clone()));

    let field_is_slice = field_shape.is_slice;
    let deref_field_expr = if field_is_pointer && !field_expr.starts_with("len(") && !field_is_slice {
        format!("*{field_expr}")
    } else {
        field_expr.clone()
    };

    let array_guard: Option<String> = if let Some(idx) = field_expr.find("[0]") {
        let mut array_expr = field_expr[..idx].to_string();
        if let Some(stripped) = array_expr.strip_prefix("len(") {
            array_expr = stripped.to_string();
        }
        Some(array_expr)
    } else {
        None
    };

    let mut assertion_buf = String::new();
    let out_ref = &mut assertion_buf;

    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                if expected.is_string() {
                    let string_field = string_value_expression(&field_expr, field_is_pointer, field_is_data_interface);
                    let expected_string = if field_is_data_interface {
                        format!("jsonString(t, {go_val})")
                    } else {
                        go_val.clone()
                    };
                    if field_is_nullable && !field_expr.starts_with("len(") {
                        let _ = writeln!(
                            out_ref,
                            "\tif {field_expr} == nil || {string_field} != {expected_string} {{"
                        );
                    } else {
                        let _ = writeln!(out_ref, "\tif {string_field} != {expected_string} {{");
                    }
                } else if field_is_pointer && !field_expr.starts_with("len(") {
                    let _ = writeln!(out_ref, "\tif {field_expr} == nil || {deref_field_expr} != {go_val} {{");
                } else if is_optional && !field_expr.starts_with("len(") {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil && {field_expr} != {go_val} {{");
                } else {
                    let _ = writeln!(out_ref, "\tif {field_expr} != {go_val} {{");
                }
                let _ = writeln!(out_ref, "\t\tt.Errorf(\"equals mismatch: got %v\", {field_expr})");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let resolved_field = assertion.field.as_deref().unwrap_or("");
                let resolved_name = field_resolver.resolve(resolved_field);
                let field_is_array = result_is_array || field_resolver.is_array(resolved_name);
                let is_nullable = field_is_nullable;
                let field_for_contains =
                    contains_value_expression(&field_expr, field_is_pointer, field_is_array, field_is_data_interface);
                if is_nullable {
                    let _ = writeln!(
                        out_ref,
                        "\tif {field_expr} == nil || !strings.Contains({field_for_contains}, {go_val}) {{"
                    );
                    let _ = writeln!(
                        out_ref,
                        "\t\tt.Errorf(\"expected to contain %s, got %v\", {go_val}, {field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t}}");
                } else {
                    let _ = writeln!(out_ref, "\tif !strings.Contains({field_for_contains}, {go_val}) {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\tt.Errorf(\"expected to contain %s, got %v\", {go_val}, {field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t}}");
                }
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                let resolved_field = assertion.field.as_deref().unwrap_or("");
                let resolved_name = field_resolver.resolve(resolved_field);
                let field_is_array = result_is_array || field_resolver.is_array(resolved_name);
                let is_nullable = field_is_nullable;
                for val in values {
                    let go_val = json_to_go(val);
                    let field_for_contains = contains_value_expression(
                        &field_expr,
                        field_is_pointer,
                        field_is_array,
                        field_is_data_interface,
                    );
                    if is_nullable {
                        let _ = writeln!(
                            out_ref,
                            "\tif {field_expr} == nil || !strings.Contains({field_for_contains}, {go_val}) {{"
                        );
                        let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected to contain %s\", {go_val})");
                        let _ = writeln!(out_ref, "\t}}");
                    } else {
                        let _ = writeln!(out_ref, "\tif !strings.Contains({field_for_contains}, {go_val}) {{");
                        let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected to contain %s\", {go_val})");
                        let _ = writeln!(out_ref, "\t}}");
                    }
                }
            }
        }
        "not_contains" => {
            for expected in assertion.expected_values() {
                let go_val = json_to_go(expected);
                let resolved_field = assertion.field.as_deref().unwrap_or("");
                let resolved_name = field_resolver.resolve(resolved_field);
                let field_is_array = result_is_array || field_resolver.is_array(resolved_name);
                let is_nullable = field_is_nullable;
                let field_for_contains =
                    contains_value_expression(&field_expr, field_is_pointer, field_is_array, field_is_data_interface);
                let condition = if is_nullable {
                    format!("{field_expr} != nil && strings.Contains({field_for_contains}, {go_val})")
                } else {
                    format!("strings.Contains({field_for_contains}, {go_val})")
                };
                let _ = writeln!(out_ref, "\tif {condition} {{");
                let _ = writeln!(
                    out_ref,
                    "\t\tt.Errorf(\"expected NOT to contain %s, got %v\", {go_val}, {field_expr})"
                );
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "not_empty" => {
            let resolved_field = assertion.field.as_deref().unwrap_or("");
            let field_is_array = {
                let rn = field_resolver.resolve(resolved_field);
                field_resolver.is_array(rn)
            };
            // `len()` only compiles against a sized Go type (string, slice, array, map,
            // channel). A field that some *other* assertion in this fixture compares
            // numerically (`equals`/`greater_than[_or_equal]`/`less_than[_or_equal]`
            // against a JSON number) is proven to be a scalar number, not a sized type —
            // `not_empty` cannot call `len()` on it without failing to build. A required
            // numeric scalar always carries a value in Go (there is no zero-length state
            // to detect), so the check degrades to a no-op, matching how `not_empty`
            // already treats "no meaningful check applies" for e.g. `not_error`.
            let is_numeric_scalar =
                !field_is_pointer && !field_is_array && numeric_scalar_fields.contains(resolved_field);
            if field_is_pointer && !field_is_array {
                let _ = writeln!(out_ref, "\tif {field_expr} == nil {{");
            } else if field_is_nullable && field_is_slice {
                let _ = writeln!(out_ref, "\tif {field_expr} == nil || len({field_expr}) == 0 {{");
            } else if field_is_nullable {
                let _ = writeln!(out_ref, "\tif {field_expr} == nil || len(*{field_expr}) == 0 {{");
            } else if is_numeric_scalar {
                return;
            } else {
                let _ = writeln!(out_ref, "\tif len({field_expr}) == 0 {{");
            }
            let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected non-empty value\")");
            let _ = writeln!(out_ref, "\t}}");
        }
        "is_empty" => {
            let field_is_array = {
                let rf = assertion.field.as_deref().unwrap_or("");
                let rn = field_resolver.resolve(rf);
                field_resolver.is_array(rn)
            };
            let simple_scalar_result =
                result_is_simple && !result_is_array && assertion.field.as_ref().is_none_or(|f| f.is_empty());
            if simple_scalar_result || field_is_pointer && !field_is_array {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
            } else if field_is_nullable && field_is_slice {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil && len({field_expr}) != 0 {{");
            } else if field_is_nullable {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil && len(*{field_expr}) != 0 {{");
            } else {
                let _ = writeln!(out_ref, "\tif len({field_expr}) != 0 {{");
            }
            let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected empty value, got %v\", {field_expr})");
            let _ = writeln!(out_ref, "\t}}");
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let resolved_field = assertion.field.as_deref().unwrap_or("");
                let resolved_name = field_resolver.resolve(resolved_field);
                let field_is_array = field_resolver.is_array(resolved_name);
                let is_nullable = field_is_nullable;
                let field_for_contains =
                    contains_value_expression(&field_expr, field_is_pointer, field_is_array, field_is_data_interface);
                let _ = writeln!(out_ref, "\t{{");
                let _ = writeln!(out_ref, "\t\tfound := false");
                for val in values {
                    let go_val = json_to_go(val);
                    let condition = if is_nullable {
                        format!("{field_expr} != nil && strings.Contains({field_for_contains}, {go_val})")
                    } else {
                        format!("strings.Contains({field_for_contains}, {go_val})")
                    };
                    let _ = writeln!(out_ref, "\t\tif {condition} {{ found = true }}");
                }
                let _ = writeln!(out_ref, "\t\tif !found {{");
                let _ = writeln!(
                    out_ref,
                    "\t\t\tt.Errorf(\"expected to contain at least one of the specified values\")"
                );
                let _ = writeln!(out_ref, "\t\t}}");
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                let (operator, comparison) = val
                    .as_u64()
                    .map(|value| ("<", (value + 1).to_string()))
                    .unwrap_or_else(|| ("<=", go_val.clone()));
                if render_guarded_scalar_comparison(
                    out_ref,
                    nil_guard_expr.as_deref(),
                    &field_expr,
                    operator,
                    &comparison,
                    &format!("> {go_val}"),
                ) {
                } else if field_is_nullable {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                    if let Some(n) = val.as_u64() {
                        let next = n + 1;
                        let _ = writeln!(out_ref, "\t\tif {deref_field_expr} < {next} {{");
                    } else {
                        let _ = writeln!(out_ref, "\t\tif {deref_field_expr} <= {go_val} {{");
                    }
                    let _ = writeln!(
                        out_ref,
                        "\t\t\tt.Errorf(\"expected > {go_val}, got %v\", {deref_field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t\t}}");
                    let _ = writeln!(out_ref, "\t}}");
                } else if let Some(n) = val.as_u64() {
                    let next = n + 1;
                    let _ = writeln!(out_ref, "\tif {field_expr} < {next} {{");
                    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected > {go_val}, got %v\", {field_expr})");
                    let _ = writeln!(out_ref, "\t}}");
                } else {
                    let _ = writeln!(out_ref, "\tif {field_expr} <= {go_val} {{");
                    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected > {go_val}, got %v\", {field_expr})");
                    let _ = writeln!(out_ref, "\t}}");
                }
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                if render_guarded_scalar_comparison(
                    out_ref,
                    nil_guard_expr.as_deref(),
                    &field_expr,
                    ">=",
                    &go_val,
                    &format!("< {go_val}"),
                ) {
                } else if field_is_nullable && !field_expr.starts_with("len(") {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                    let _ = writeln!(out_ref, "\t\tif {deref_field_expr} >= {go_val} {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\t\tt.Errorf(\"expected < {go_val}, got %v\", {deref_field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t\t}}");
                    let _ = writeln!(out_ref, "\t}}");
                } else {
                    let _ = writeln!(out_ref, "\tif {field_expr} >= {go_val} {{");
                    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected < {go_val}, got %v\", {field_expr})");
                    let _ = writeln!(out_ref, "\t}}");
                }
            }
        }
        "greater_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                if render_guarded_scalar_comparison(
                    out_ref,
                    nil_guard_expr.as_deref(),
                    &field_expr,
                    "<",
                    &go_val,
                    &format!(">= {go_val}"),
                ) {
                } else if field_is_nullable && !field_expr.starts_with("len(") {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                    let _ = writeln!(out_ref, "\t\tif {deref_field_expr} < {go_val} {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\t\tt.Errorf(\"expected >= {go_val}, got %v\", {deref_field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t\t}}");
                    let _ = writeln!(out_ref, "\t}}");
                } else {
                    let _ = writeln!(out_ref, "\tif {field_expr} < {go_val} {{");
                    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected >= {go_val}, got %v\", {field_expr})");
                    let _ = writeln!(out_ref, "\t}}");
                }
            }
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let go_val = json_to_go(val);
                if render_guarded_scalar_comparison(
                    out_ref,
                    nil_guard_expr.as_deref(),
                    &field_expr,
                    ">",
                    &go_val,
                    &format!("<= {go_val}"),
                ) {
                } else if field_is_nullable && !field_expr.starts_with("len(") {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                    let _ = writeln!(out_ref, "\t\tif {deref_field_expr} > {go_val} {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\t\tt.Errorf(\"expected <= {go_val}, got %v\", {deref_field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t\t}}");
                    let _ = writeln!(out_ref, "\t}}");
                } else {
                    let _ = writeln!(out_ref, "\tif {field_expr} > {go_val} {{");
                    let _ = writeln!(out_ref, "\t\tt.Errorf(\"expected <= {go_val}, got %v\", {field_expr})");
                    let _ = writeln!(out_ref, "\t}}");
                }
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_prefix = string_value_expression(&field_expr, field_is_pointer, field_is_data_interface);
                let _ = writeln!(out_ref, "\tif !strings.HasPrefix({field_for_prefix}, {go_val}) {{");
                let _ = writeln!(
                    out_ref,
                    "\t\tt.Errorf(\"expected to start with %s, got %v\", {go_val}, {field_expr})"
                );
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                render_count_assertion(
                    out_ref,
                    &field_expr,
                    n,
                    nullable_guard_expr.as_deref(),
                    field_is_slice,
                    false,
                );
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                render_count_assertion(
                    out_ref,
                    &field_expr,
                    n,
                    nullable_guard_expr.as_deref(),
                    field_is_slice,
                    true,
                );
            }
        }
        "is_true" => {
            if is_optional {
                // `*T`/`[]T`: "is_true" means "present" -- dereferencing to compare against a
                // bool only type-checks when T is bool, and for a struct field (e.g.
                // `Option<DataNode>`) it does not compile at all. `assert.NotNil` is the
                // interpretation that holds for any T, matching the Rust backend's
                // `.is_some()` convention for the same assertion type. ~keep
                let _ = writeln!(out_ref, "\tassert.NotNil(t, {field_expr}, \"expected true (non-nil)\")");
            } else if field_is_pointer {
                let _ = writeln!(out_ref, "\tassert.True(t, {deref_field_expr}, \"expected true\")");
            } else {
                let _ = writeln!(out_ref, "\tassert.True(t, {field_expr}, \"expected true\")");
            }
        }
        "is_false" => {
            if is_optional {
                let _ = writeln!(out_ref, "\tassert.Nil(t, {field_expr}, \"expected false (nil)\")");
            } else if field_is_pointer {
                let _ = writeln!(out_ref, "\tassert.False(t, {deref_field_expr}, \"expected false\")");
            } else {
                let _ = writeln!(out_ref, "\tassert.False(t, {field_expr}, \"expected false\")");
            }
        }
        "method_result" => {
            if let Some(method_name) = &assertion.method {
                let info = build_go_method_call(result_var, method_name, assertion.args.as_ref(), import_alias);
                let check = assertion.check.as_deref().unwrap_or("is_true");
                let deref_expr = if info.is_pointer {
                    format!("*{}", info.call_expr)
                } else {
                    info.call_expr.clone()
                };
                match check {
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            if val.is_boolean() {
                                if val.as_bool() == Some(true) {
                                    let _ = writeln!(out_ref, "\tassert.True(t, {deref_expr}, \"expected true\")");
                                } else {
                                    let _ = writeln!(out_ref, "\tassert.False(t, {deref_expr}, \"expected false\")");
                                }
                            } else {
                                let go_val = if let Some(cast) = info.value_cast {
                                    if val.is_number() {
                                        format!("{cast}({})", json_to_go(val))
                                    } else {
                                        json_to_go(val)
                                    }
                                } else {
                                    json_to_go(val)
                                };
                                let _ = writeln!(
                                    out_ref,
                                    "\tassert.Equal(t, {go_val}, {deref_expr}, \"method_result equals assertion failed\")"
                                );
                            }
                        }
                    }
                    "is_true" => {
                        let _ = writeln!(out_ref, "\tassert.True(t, {deref_expr}, \"expected true\")");
                    }
                    "is_false" => {
                        let _ = writeln!(out_ref, "\tassert.False(t, {deref_expr}, \"expected false\")");
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let cast = info.value_cast.unwrap_or("uint");
                            let _ = writeln!(
                                out_ref,
                                "\tassert.GreaterOrEqual(t, {deref_expr}, {cast}({n}), \"expected >= {n}\")"
                            );
                        }
                    }
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let n = val.as_u64().unwrap_or(0);
                            let _ = writeln!(
                                out_ref,
                                "\tassert.GreaterOrEqual(t, len({deref_expr}), {n}, \"expected at least {n} elements\")"
                            );
                        }
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let go_val = json_to_go(val);
                            let _ = writeln!(
                                out_ref,
                                "\tassert.Contains(t, {deref_expr}, {go_val}, \"expected result to contain value\")"
                            );
                        }
                    }
                    "is_error" => {
                        let _ = writeln!(out_ref, "\t{{");
                        let _ = writeln!(out_ref, "\t\t_, methodErr := {}", info.call_expr);
                        let _ = writeln!(out_ref, "\t\tassert.Error(t, methodErr)");
                        let _ = writeln!(out_ref, "\t}}");
                    }
                    other_check => {
                        panic!("Go e2e generator: unsupported method_result check type: {other_check}");
                    }
                }
            } else {
                panic!("Go e2e generator: method_result assertion missing 'method' field");
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                render_length_assertion(
                    out_ref,
                    &field_expr,
                    n,
                    nullable_guard_expr.as_deref(),
                    field_is_pointer,
                    true,
                );
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                render_length_assertion(
                    out_ref,
                    &field_expr,
                    n,
                    nullable_guard_expr.as_deref(),
                    field_is_pointer,
                    false,
                );
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_suffix = string_value_expression(&field_expr, field_is_pointer, field_is_data_interface);
                let _ = writeln!(out_ref, "\tif !strings.HasSuffix({field_for_suffix}, {go_val}) {{");
                let _ = writeln!(
                    out_ref,
                    "\t\tt.Errorf(\"expected to end with %s, got %v\", {go_val}, {field_expr})"
                );
                let _ = writeln!(out_ref, "\t}}");
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_regex = string_value_expression(&field_expr, field_is_pointer, field_is_data_interface);
                let _ = writeln!(
                    out_ref,
                    "\tassert.Regexp(t, {go_val}, {field_for_regex}, \"expected value to match regex\")"
                );
            }
        }
        "not_error" => {}
        "error" => {}
        other => {
            panic!("Go e2e generator: unsupported assertion type: {other}");
        }
    }

    match &array_guard {
        Some(arr) if !assertion_buf.is_empty() => {
            emit_non_empty_precondition(out, arr);
            out.push_str(&assertion_buf);
        }
        _ => out.push_str(&assertion_buf),
    }
}

#[path = "assertions/wildcard_assertions.rs"]
mod wildcard_assertions;
use wildcard_assertions::{emit_non_empty_precondition, render_wildcard_assertion};

#[cfg(test)]
#[path = "assertions/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "assertions/streaming_skip_marker_tests.rs"]
mod streaming_skip_marker_tests;
