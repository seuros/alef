//! Go assertion rendering.

use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::escape::go_string_literal;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

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
        match f.as_str() {
            "chunks_have_content" => {
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
            match assertion.assertion_type.as_str() {
                "count_min" => {
                    if let Some(val) = &assertion.value
                        && let Some(n) = val.as_u64()
                    {
                        let _ = writeln!(
                            out,
                            "\tassert.GreaterOrEqual(t, len({expr}), {n}, \"expected >= {n} chunks\")"
                        );
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
                    }
                }
                "greater_than_or_equal" => {
                    if let Some(val) = &assertion.value
                        && let Some(n) = val.as_u64()
                    {
                        let _ = writeln!(out, "\tassert.GreaterOrEqual(t, {expr}, {n}, \"expected >= {n}\")");
                    }
                }
                "contains" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = crate::e2e::escape::go_string_literal(s);
                        let _ = writeln!(out, "\tassert.Contains(t, {expr}, {escaped}, \"expected to contain\")");
                    }
                }
                _ => {
                    let _ = writeln!(
                        out,
                        "\t// streaming field '{f}': assertion type '{}' not rendered",
                        assertion.assertion_type
                    );
                }
            }
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

    let is_optional = assertion
        .field
        .as_ref()
        .map(|f| {
            let resolved = field_resolver.resolve(f);
            let check_path = resolved
                .strip_suffix(".length")
                .or_else(|| resolved.strip_suffix(".count"))
                .or_else(|| resolved.strip_suffix(".size"))
                .unwrap_or(resolved);
            field_resolver.is_optional(check_path) && !optional_locals.contains_key(f.as_str())
        })
        .unwrap_or(false);

    let field_is_array_for_len = assertion
        .field
        .as_ref()
        .map(|f| {
            let resolved = field_resolver.resolve(f);
            let check_path = resolved
                .strip_suffix(".length")
                .or_else(|| resolved.strip_suffix(".count"))
                .or_else(|| resolved.strip_suffix(".size"))
                .unwrap_or(resolved);
            field_resolver.is_array(check_path)
        })
        .unwrap_or(false);
    let field_expr =
        if is_optional && field_expr.starts_with("len(") && field_expr.ends_with(')') && !field_is_array_for_len {
            let inner = &field_expr[4..field_expr.len() - 1];
            format!("len(*{inner})")
        } else {
            field_expr
        };
    let nil_guard_expr = if is_optional && field_expr.starts_with("len(*") {
        Some(field_expr[5..field_expr.len() - 1].to_string())
    } else {
        None
    };

    let field_is_slice = assertion
        .field
        .as_ref()
        .map(|f| field_resolver.is_array(field_resolver.resolve(f)))
        .unwrap_or(false);
    let deref_field_expr = if is_optional && !field_expr.starts_with("len(") && !field_is_slice {
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
                    let string_field = if is_optional && !field_expr.starts_with("len(") {
                        format!("string(*{field_expr})")
                    } else {
                        format!("string({field_expr})")
                    };
                    if is_optional && !field_expr.starts_with("len(") {
                        let _ = writeln!(out_ref, "\tif {field_expr} == nil || {string_field} != {go_val} {{");
                    } else {
                        let _ = writeln!(out_ref, "\tif {string_field} != {go_val} {{");
                    }
                } else if is_optional && !field_expr.starts_with("len(") {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil && {deref_field_expr} != {go_val} {{");
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
                let is_opt =
                    is_optional && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()));
                let field_for_contains = if is_opt && field_is_array {
                    format!("jsonString({field_expr})")
                } else if is_opt {
                    format!("string(*{field_expr})")
                } else if field_is_array {
                    format!("jsonString({field_expr})")
                } else {
                    format!("string({field_expr})")
                };
                if is_opt {
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
                let is_opt =
                    is_optional && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()));
                for val in values {
                    let go_val = json_to_go(val);
                    let field_for_contains = if is_opt && field_is_array {
                        format!("jsonString({field_expr})")
                    } else if is_opt {
                        format!("string(*{field_expr})")
                    } else if field_is_array {
                        format!("jsonString({field_expr})")
                    } else {
                        format!("string({field_expr})")
                    };
                    if is_opt {
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
                let is_opt =
                    is_optional && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()));
                let field_for_contains = if is_opt && field_is_array {
                    format!("jsonString({field_expr})")
                } else if is_opt {
                    format!("string(*{field_expr})")
                } else if field_is_array {
                    format!("jsonString({field_expr})")
                } else {
                    format!("string({field_expr})")
                };
                let _ = writeln!(out_ref, "\tif strings.Contains({field_for_contains}, {go_val}) {{");
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
            let is_numeric_scalar = !is_optional && !field_is_array && numeric_scalar_fields.contains(resolved_field);
            if is_optional && !field_is_array {
                let _ = writeln!(out_ref, "\tif {field_expr} == nil {{");
            } else if is_optional && field_is_slice {
                let _ = writeln!(out_ref, "\tif {field_expr} == nil || len({field_expr}) == 0 {{");
            } else if is_optional {
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
            if simple_scalar_result || is_optional && !field_is_array {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
            } else if is_optional && field_is_slice {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil && len({field_expr}) != 0 {{");
            } else if is_optional {
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
                let is_opt =
                    is_optional && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()));
                let field_for_contains = if is_opt && field_is_array {
                    format!("jsonString({field_expr})")
                } else if is_opt {
                    format!("string(*{field_expr})")
                } else if field_is_array {
                    format!("jsonString({field_expr})")
                } else {
                    format!("string({field_expr})")
                };
                let _ = writeln!(out_ref, "\t{{");
                let _ = writeln!(out_ref, "\t\tfound := false");
                for val in values {
                    let go_val = json_to_go(val);
                    let _ = writeln!(
                        out_ref,
                        "\t\tif strings.Contains({field_for_contains}, {go_val}) {{ found = true }}"
                    );
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
                if is_optional {
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
                if let Some(ref guard) = nil_guard_expr {
                    let _ = writeln!(out_ref, "\tif {guard} != nil {{");
                    let _ = writeln!(out_ref, "\t\tif {field_expr} >= {go_val} {{");
                    let _ = writeln!(out_ref, "\t\t\tt.Errorf(\"expected < {go_val}, got %v\", {field_expr})");
                    let _ = writeln!(out_ref, "\t\t}}");
                    let _ = writeln!(out_ref, "\t}}");
                } else if is_optional && !field_expr.starts_with("len(") {
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
                if let Some(ref guard) = nil_guard_expr {
                    let _ = writeln!(out_ref, "\tif {guard} != nil {{");
                    let _ = writeln!(out_ref, "\t\tif {field_expr} < {go_val} {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\t\tt.Errorf(\"expected >= {go_val}, got %v\", {field_expr})"
                    );
                    let _ = writeln!(out_ref, "\t\t}}");
                    let _ = writeln!(out_ref, "\t}}");
                } else if is_optional && !field_expr.starts_with("len(") {
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
                if is_optional && !field_expr.starts_with("len(") {
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
                let field_for_prefix = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("string(*{field_expr})")
                } else {
                    format!("string({field_expr})")
                };
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
                if is_optional {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                    let len_expr = if field_is_slice {
                        format!("len({field_expr})")
                    } else {
                        format!("len(*{field_expr})")
                    };
                    let _ = writeln!(
                        out_ref,
                        "\t\tassert.GreaterOrEqual(t, {len_expr}, {n}, \"expected at least {n} elements\")"
                    );
                    let _ = writeln!(out_ref, "\t}}");
                } else {
                    let _ = writeln!(
                        out_ref,
                        "\tassert.GreaterOrEqual(t, len({field_expr}), {n}, \"expected at least {n} elements\")"
                    );
                }
            }
        }
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                if is_optional {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                    let len_expr = if field_is_slice {
                        format!("len({field_expr})")
                    } else {
                        format!("len(*{field_expr})")
                    };
                    let _ = writeln!(
                        out_ref,
                        "\t\tassert.Equal(t, {len_expr}, {n}, \"expected exactly {n} elements\")"
                    );
                    let _ = writeln!(out_ref, "\t}}");
                } else {
                    let _ = writeln!(
                        out_ref,
                        "\tassert.Equal(t, len({field_expr}), {n}, \"expected exactly {n} elements\")"
                    );
                }
            }
        }
        "is_true" => {
            if is_optional {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                let _ = writeln!(out_ref, "\t\tassert.True(t, *{field_expr}, \"expected true\")");
                let _ = writeln!(out_ref, "\t}}");
            } else {
                let _ = writeln!(out_ref, "\tassert.True(t, {field_expr}, \"expected true\")");
            }
        }
        "is_false" => {
            if is_optional {
                let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                let _ = writeln!(out_ref, "\t\tassert.False(t, *{field_expr}, \"expected false\")");
                let _ = writeln!(out_ref, "\t}}");
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
                if is_optional {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\tassert.GreaterOrEqual(t, len(*{field_expr}), {n}, \"expected length >= {n}\")"
                    );
                    let _ = writeln!(out_ref, "\t}}");
                } else if field_expr.starts_with("len(") {
                    let _ = writeln!(
                        out_ref,
                        "\tassert.GreaterOrEqual(t, {field_expr}, {n}, \"expected length >= {n}\")"
                    );
                } else {
                    let _ = writeln!(
                        out_ref,
                        "\tassert.GreaterOrEqual(t, len({field_expr}), {n}, \"expected length >= {n}\")"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                if is_optional {
                    let _ = writeln!(out_ref, "\tif {field_expr} != nil {{");
                    let _ = writeln!(
                        out_ref,
                        "\t\tassert.LessOrEqual(t, len(*{field_expr}), {n}, \"expected length <= {n}\")"
                    );
                    let _ = writeln!(out_ref, "\t}}");
                } else if field_expr.starts_with("len(") {
                    let _ = writeln!(
                        out_ref,
                        "\tassert.LessOrEqual(t, {field_expr}, {n}, \"expected length <= {n}\")"
                    );
                } else {
                    let _ = writeln!(
                        out_ref,
                        "\tassert.LessOrEqual(t, len({field_expr}), {n}, \"expected length <= {n}\")"
                    );
                }
            }
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let go_val = json_to_go(expected);
                let field_for_suffix = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("string(*{field_expr})")
                } else {
                    format!("string({field_expr})")
                };
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
                let field_for_regex = if is_optional
                    && !optional_locals.contains_key(assertion.field.as_ref().unwrap_or(&String::new()))
                {
                    format!("*{field_expr}")
                } else {
                    field_expr.clone()
                };
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

/// Emit the `len(arr) == 0` precondition that must precede an `arr[0]` assertion.
///
/// A fixture that asserts on element `0` is stating that the collection has an element;
/// wrapping the assertion in `if len(arr) > 0` instead let an empty result satisfy the
/// whole check, so the test could not fail. `t.Fatalf` stops the test function before the
/// index panics, matching how the generator reports every other unmet precondition.
///
/// The precondition is emitted once per collection per function body: `t.Fatalf` aborts,
/// so repeating it ahead of every indexed assertion would add nothing. The de-duplication
/// compares whole lines, so an identical check emitted at a deeper indentation (inside a
/// nil guard, where it may not run) does not suppress the function-level one.
fn emit_non_empty_precondition(out: &mut String, array_expr: &str) {
    let fatal_line = format!(
        "\t\tt.Fatalf(\"expected non-empty %s\", {})",
        go_string_literal(array_expr)
    );
    if out.lines().any(|line| line == fatal_line) {
        return;
    }
    let _ = writeln!(out, "\tif len({array_expr}) == 0 {{");
    let _ = writeln!(out, "{fatal_line}");
    let _ = writeln!(out, "\t}}");
}

/// Per-assertion suffix for Go locals emitted by [`render_wildcard_assertion`].
///
/// Two assertions over the same array would otherwise both declare `found`, which is a
/// redeclaration error in the same function scope. Hashing the assertion's discriminating
/// fields (mirroring the swift backend's `local_suffix`) makes the name unique per
/// assertion while staying stable across regenerations. ~keep
fn wildcard_local_suffix(assertion: &Assertion) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    assertion.assertion_type.hash(&mut hasher);
    assertion.field.hash(&mut hasher);
    assertion
        .value
        .as_ref()
        .map(std::string::ToString::to_string)
        .unwrap_or_default()
        .hash(&mut hasher);
    assertion
        .values
        .as_ref()
        .map(|vs| {
            vs.iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default()
        .hash(&mut hasher);
    format!("{:x}", hasher.finish() & 0xffff_ffff)
}

/// Emit the statement form of an any-element assertion over a bracket-wildcard path.
fn render_wildcard_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    field: &str,
    array_part: &str,
    elem_part: &str,
) {
    let array_accessor = if array_part.is_empty() {
        result_var.to_string()
    } else {
        field_resolver.accessor(array_part, "go", result_var)
    };
    // Passing the loop variable as the result var is what makes a nested element sub-path
    // resolve against the element rather than against the whole result. ~keep
    let elem_accessor = field_resolver.accessor(elem_part, "go", "e");
    let suffix = wildcard_local_suffix(assertion);

    let emit_scan = |out: &mut String, local: &str, cond: &str| {
        let _ = writeln!(out, "\t{local} := false");
        let _ = writeln!(out, "\tfor _, e := range {array_accessor} {{");
        let _ = writeln!(out, "\t\tif {cond} {{");
        let _ = writeln!(out, "\t\t\t{local} = true");
        let _ = writeln!(out, "\t\t\tbreak");
        let _ = writeln!(out, "\t\t}}");
        let _ = writeln!(out, "\t}}");
    };

    match assertion.assertion_type.as_str() {
        "contains" | "not_contains" if assertion.value.is_some() => {
            let expected = assertion.value.as_ref().expect("guarded by the match arm");
            let go_val = json_to_go(expected);
            let local = format!("found{suffix}");
            let cond = format!("strings.Contains(fmt.Sprintf(\"%v\", {elem_accessor}), {go_val})");
            emit_scan(out, &local, &cond);
            if assertion.assertion_type == "contains" {
                let _ = writeln!(out, "\tif !{local} {{");
                let _ = writeln!(
                    out,
                    "\t\tt.Errorf(\"expected some element of '{field}' to contain %v\", {go_val})"
                );
            } else {
                let _ = writeln!(out, "\tif {local} {{");
                let _ = writeln!(
                    out,
                    "\t\tt.Errorf(\"expected no element of '{field}' to contain %v\", {go_val})"
                );
            }
            let _ = writeln!(out, "\t}}");
        }
        "contains" | "contains_all" | "not_contains" => {
            let Some(values) = &assertion.values else {
                let _ = writeln!(out, "\t// skipped: '{field}' traversal assertion has no values");
                return;
            };
            let negated = assertion.assertion_type == "not_contains";
            for (i, val) in values.iter().enumerate() {
                let go_val = json_to_go(val);
                let local = format!("found{suffix}v{i}");
                let cond = format!("strings.Contains(fmt.Sprintf(\"%v\", {elem_accessor}), {go_val})");
                emit_scan(out, &local, &cond);
                if negated {
                    let _ = writeln!(out, "\tif {local} {{");
                    let _ = writeln!(
                        out,
                        "\t\tt.Errorf(\"expected no element of '{field}' to contain %v\", {go_val})"
                    );
                } else {
                    let _ = writeln!(out, "\tif !{local} {{");
                    let _ = writeln!(
                        out,
                        "\t\tt.Errorf(\"expected some element of '{field}' to contain %v\", {go_val})"
                    );
                }
                let _ = writeln!(out, "\t}}");
            }
        }
        "not_empty" => {
            let local = format!("found{suffix}");
            let cond = format!("fmt.Sprintf(\"%v\", {elem_accessor}) != \"\"");
            emit_scan(out, &local, &cond);
            let _ = writeln!(out, "\tif !{local} {{");
            let _ = writeln!(
                out,
                "\t\tt.Errorf(\"expected some element of '{field}' to be non-empty\")"
            );
            let _ = writeln!(out, "\t}}");
        }
        other => {
            let _ = writeln!(
                out,
                "\t// skipped: unsupported traversal assertion '{other}' on '{field}'"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn make_assertion(field: &str, value: &str) -> Assertion {
        Assertion {
            assertion_type: "equals".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Default::default()
        }
    }

    fn contains_assertion(field: &str, value: &str) -> Assertion {
        Assertion {
            assertion_type: "contains".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Default::default()
        }
    }

    fn render_bare(assertion: &Assertion) -> String {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let mut out = String::new();
        render_assertion(
            &mut out,
            assertion,
            "result",
            "pkg",
            &resolver,
            &HashMap::new(),
            &HashSet::new(),
            false,
            false,
            false,
            None,
        );
        out
    }

    #[test]
    fn wildcard_contains_scans_every_element_not_just_index_zero() {
        let out = render_bare(&contains_assertion("links[].link_type", "external"));
        assert!(out.contains("for _, e := range result.Links {"), "got: {out}");
        assert!(out.contains("e.LinkType"), "got: {out}");
        assert!(!out.contains("[0]"), "wildcard must not lower to index 0, got: {out}");
    }

    #[test]
    fn explicit_numeric_index_still_targets_that_element() {
        let out = render_bare(&contains_assertion("links[0].link_type", "external"));
        assert!(out.contains("result.Links[0].LinkType"), "got: {out}");
        assert!(
            !out.contains("range"),
            "explicit index must not become a scan, got: {out}"
        );
    }

    /// Codegen-level canary for the wildcard defect. A fixture array whose match lives in
    /// element 1 is only detected by code that visits every element; the pre-fix renderer
    /// emitted a single `result.Links[0]` accessor, so this assertion pair fails against it.
    /// It cannot execute the generated Go, so it pins the property structurally: an
    /// element-1-only match is caught iff the emitted loop is unbounded and the value is
    /// tested inside it. ~keep
    #[test]
    fn wildcard_match_in_element_one_is_reachable() {
        let out = render_bare(&contains_assertion("links[].link_type", "internal"));
        let loop_start = out
            .find("for _, e := range")
            .expect("expected an unbounded element scan");
        let check = out
            .find("strings.Contains")
            .expect("expected a per-element containment check");
        assert!(
            check > loop_start,
            "containment check must sit inside the scan, got: {out}"
        );
        assert!(
            !out.contains("result.Links[0]"),
            "an index-0 accessor would miss a match in element 1, got: {out}"
        );
    }

    #[test]
    fn two_wildcard_assertions_on_one_array_use_distinct_locals() {
        let first = render_bare(&contains_assertion("links[].link_type", "external"));
        let second = render_bare(&contains_assertion("links[].link_type", "internal"));
        let local_of = |s: &str| {
            let start = s.find("found").expect("expected a found local");
            s[start..start + s[start..].find(' ').expect("local is space-delimited")].to_string()
        };
        assert_ne!(local_of(&first), local_of(&second), "locals must not collide");
    }

    /// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
    /// (present, non-`binding_excluded`, on some IR type) but missing from the
    /// hand-maintained `result_fields` config must still render a real assertion,
    /// not a "skipped: field not available" comment — `go/test_function.rs` (and
    /// the `go/test_file.rs` import-decision resolver) now thread
    /// `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
    #[test]
    fn go_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
        let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(reachable, HashSet::new());
        let assertion = make_assertion("data", "hello");
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "pkg",
            &resolver,
            &HashMap::new(),
            &HashSet::new(),
            false,
            false,
            false,
            None,
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
    fn go_ir_excluded_field_present_in_result_fields_is_still_skipped() {
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
        let assertion = make_assertion("internal_diagnostics", "hello");
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "pkg",
            &resolver,
            &HashMap::new(),
            &HashSet::new(),
            false,
            false,
            false,
            None,
        );
        assert!(out.contains("skipped"), "got: {out}");
    }
}
