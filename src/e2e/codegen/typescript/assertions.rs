//! Assertion rendering for TypeScript e2e tests.

use crate::e2e::codegen::assertion_type_skip::{
    streaming_assertion_type_skip_line, streaming_assertion_value_skip_line,
};
use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::json::json_to_js;

/// Render a single assertion into the test body.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
    result_enum_fields: &std::collections::HashMap<String, String>,
    lang: &str,
    is_streaming: bool,
    returns_void: bool,
) {
    // An uncaught throw already fails the test, but the caller (`test_case.rs`)
    // only ever used a separate `has_usable_assertion` predicate to decide
    // whether to bind `const result = ...` — never to assert on it — and that
    // predicate excluded `not_error` outright. A fixture whose only assertion
    // was `not_error` discarded the call's result entirely (`await
    // callExpr();` with no binding, no assertion). `not_error` never carries a
    // `field`, so intercept it here, before any field-specific machinery.
    // For streaming fixtures, assert on the drained `chunks` array (the
    // literal name every streaming-virtual-field accessor in this file
    // already hardcodes) rather than the raw, undrained stream. ~keep
    if assertion.assertion_type == "not_error" {
        if returns_void {
            // A `returns_void` call's binding return is napi-rs's mapping of Rust `()` to JS
            // `undefined`, so `expect(result).toBeDefined()` would fail every successful call,
            // not just an unsuccessful one. There is no result to assert on; `test_case.rs`'s
            // `void_not_error` flag wraps `call_expr` itself in
            // `expect(callExpr()).resolves.not.toThrow()` instead, so nothing renders here.
        } else if is_streaming {
            out.push_str("    expect(chunks).toBeDefined();\n");
        } else {
            out.push_str(&format!("    expect({result_var}).toBeDefined();\n"));
        }
        return;
    }

    // For simple-result methods (e.g., `speech` returning bytes/Buffer), every
    // field-based assertion targets the result itself — there is no struct to
    // access. Drop length-only assertions onto the result directly and skip
    // anything that requires a real struct sub-field.
    if result_is_simple
        && let Some(f) = &assertion.field
        && !f.is_empty()
    {
        match assertion.assertion_type.as_str() {
            "not_empty" => {
                out.push_str(&format!("    expect({result_var}.length).toBeGreaterThan(0);\n"));
                return;
            }
            "is_empty" => {
                out.push_str(&format!("    expect({result_var}.length).toBe(0);\n"));
                return;
            }
            "count_equals" => {
                if let Some(val) = &assertion.value {
                    let js_val = json_to_js(val);
                    out.push_str(&format!("    expect({result_var}.length).toBe({js_val});\n"));
                }
                return;
            }
            "count_min" => {
                if let Some(val) = &assertion.value {
                    let js_val = json_to_js(val);
                    out.push_str(&format!(
                        "    expect({result_var}.length).toBeGreaterThanOrEqual({js_val});\n"
                    ));
                }
                return;
            }
            _ => {
                out.push_str(&format!(
                    "    // skipped: {}\n",
                    FieldSkip::NotApplicableForSimpleResultType.message(f)
                ));
                return;
            }
        }
    }

    // Handle synthetic / derived fields before the is_valid_for_result check
    // so they are never treated as struct property accesses on the result.
    if let Some(f) = &assertion.field
        && render_synthetic_field_assertion(out, assertion, result_var, f, is_streaming)
    {
        return;
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        out.push_str(&format!(
            "    // skipped: {}\n",
            FieldSkip::NotAvailableOnResultType.message(f)
        ));
        return;
    }

    // A `foo[].bar` fixture path means "some element of foo satisfies this", but
    // `FieldResolver::accessor` lowers `[]` to index 0 (`result.foo[0].bar`), which
    // silently checks only the first element and reads as coverage. Quantify over every
    // element instead. This returns before the `field_expr` enum-coercion rewrite below,
    // which would otherwise wrap the whole `.some(...)` predicate. ~keep
    if !result_is_simple
        && let Some(f) = assertion.field.as_deref()
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a
        // second wildcard in `elem_part` that the element accessor below lowers to index 0. ~keep
        if let Some(line) = nested_wildcard_skip_line("    ", "//", f, &elem_part) {
            out.push_str(&line);
            out.push('\n');
            return;
        }
        let array_accessor = if array_part.is_empty() {
            result_var.to_string()
        } else {
            field_resolver.accessor(&array_part, "typescript", result_var)
        };
        let elem_accessor = if elem_part.is_empty() {
            "e".to_string()
        } else {
            field_resolver.accessor(&elem_part, "typescript", "e")
        };
        render_wildcard_assertion(out, assertion, &array_accessor, &elem_accessor, f);
        return;
    }

    let mut field_expr = match &assertion.field {
        Some(f) if !f.is_empty() => field_resolver.accessor(f, "typescript", result_var),
        _ => result_var.to_string(),
    };

    // Check if this field is an enum type that needs coercion
    let mut is_enum_field = false;
    if let Some(f) = assertion.field.as_deref()
        && !f.is_empty()
        && result_enum_fields
            .get(f)
            .or_else(|| result_enum_fields.get(field_resolver.resolve(f)))
            .is_some()
    {
        is_enum_field = true;
    }

    // Check if this is metadata.format field (FormatMetadata tagged enum)
    let is_format_metadata = assertion.field.as_deref().is_some_and(|f| f == "metadata.format");

    // WASM: enum-typed result fields are exposed as numeric discriminants by
    // wasm-bindgen, not strings. Compare against `EnumClass.Variant` instead of
    // running `.trim()` on a number.
    if lang == "wasm" {
        if let Some(enum_class) = assertion.field.as_deref().and_then(|f| {
            result_enum_fields
                .get(f)
                .or_else(|| result_enum_fields.get(field_resolver.resolve(f)))
        }) && render_wasm_enum_assertion(out, assertion, &field_expr, enum_class)
        {
            return;
        }

        // wasm-bindgen maps Rust `u64`/`i64` to JS `BigInt`. Numeric assertions
        // would otherwise fail with `expected 15n to be 15`. Wrap the field
        // expression in `Number(...)` for numeric `equals` / inequality
        // comparisons so both BigInt and Number values compare correctly.
        if assertion_value_is_numeric(assertion) {
            field_expr = format!("Number({field_expr})");
        }
    }

    let field_is_array = assertion
        .field
        .as_deref()
        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));

    render_standard_assertion(
        out,
        assertion,
        result_var,
        &field_expr,
        field_resolver,
        field_is_array,
        is_enum_field,
        is_format_metadata,
    );
}

/// Return `true` when the assertion compares the field against a numeric value.
fn assertion_value_is_numeric(assertion: &Assertion) -> bool {
    match assertion.assertion_type.as_str() {
        "equals" | "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" => {
            assertion.value.as_ref().is_some_and(|v| v.is_number())
        }
        _ => false,
    }
}

/// Render an enum-typed result assertion. WASM optional-enum getters return
/// `Option<String>` (the serde wire format) rather than the wasm-bindgen enum
/// discriminant, so we compare against the original snake_case string value
/// rather than the `EnumClass.Variant` reference. The `enum_class` is no longer
/// referenced in the emitted code but is retained in the signature for clarity
/// at the call site.
/// Returns `true` if the assertion was rendered.
fn render_wasm_enum_assertion(out: &mut String, assertion: &Assertion, field_expr: &str, _enum_class: &str) -> bool {
    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(serde_json::Value::String(s)) = &assertion.value {
                out.push_str(&format!("    expect({field_expr}).toBe(\"{s}\");\n"));
                return true;
            }
        }
        "not_empty" | "is_not_empty" => {
            out.push_str(&format!("    expect({field_expr}).toBeDefined();\n"));
            return true;
        }
        _ => {}
    }
    false
}

/// Try to render a synthetic/virtual field assertion. Returns `true` when the field was handled.
fn render_synthetic_field_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field: &str,
    is_streaming: bool,
) -> bool {
    match field {
        "chunks_have_content" => {
            let pred = format!("({result_var}.chunks ?? []).every((c: {{ content?: string }}) => !!c.content)");
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "chunks_have_embeddings" => {
            let pred = format!(
                "({result_var}.chunks ?? []).every((c: {{ embedding?: number[] }}) => c.embedding != null && c.embedding.length > 0)"
            );
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "chunks_have_heading_context" => {
            let pred = format!(
                "({result_var}.chunks ?? []).every((c: {{ metadata?: {{ headingContext?: string }} }}) => c.metadata?.headingContext != null)"
            );
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "first_chunk_starts_with_heading" => {
            let pred = format!("({result_var}.chunks ?? []).at(0)?.metadata?.headingContext != null");
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "embeddings" => {
            render_embeddings_assertion(out, assertion, result_var);
            true
        }
        "embedding_dimensions" => {
            render_embedding_dimensions(out, assertion, result_var);
            true
        }
        "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
            let pred = match field {
                "embeddings_valid" => {
                    format!("{result_var}.every((e: number[]) => e.length > 0)")
                }
                "embeddings_finite" => {
                    format!("{result_var}.every((e: number[]) => e.every((v: number) => isFinite(v)))")
                }
                "embeddings_non_zero" => {
                    format!("{result_var}.every((e: number[]) => e.some((v: number) => v !== 0))")
                }
                "embeddings_normalized" => {
                    format!(
                        "{result_var}.every((e: number[]) => {{ const n = e.reduce((s: number, v: number) => s + v * v, 0); return Math.abs(n - 1.0) < 1e-3; }})"
                    )
                }
                _ => unreachable!(),
            };
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "keywords" | "keywords_count" => {
            out.push_str(&format!(
                "    // skipped: {}\n",
                FieldSkip::NotAvailableOnNodeProcessingResult.message(field)
            ));
            true
        }
        // Streaming virtual fields resolve against the `chunks` collected-list variable.
        // Skip the streaming interception entirely when the call has opted out
        // (`[e2e.calls.<name>] streaming = false`) — `chunks` then names a plain
        // field on the synchronous result struct and must flow through normal
        // accessor resolution (e.g. `result.chunks`).
        f if is_streaming && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f) => {
            // lang is always "node" or "wasm" here; both use the same JS expressions.
            if let Some(expr) =
                crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor(f, "node", "chunks")
            {
                match assertion.assertion_type.as_str() {
                    "count_min" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            out.push_str(&format!(
                                "    expect({expr}.length).toBeGreaterThanOrEqual({js_val});\n"
                            ));
                        } else {
                            push_streaming_value_skip(out, field, assertion);
                        }
                    }
                    "count_equals" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            out.push_str(&format!("    expect({expr}.length).toBe({js_val});\n"));
                        } else {
                            push_streaming_value_skip(out, field, assertion);
                        }
                    }
                    "equals" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            out.push_str(&format!("    expect({expr}).toBe({js_val});\n"));
                        } else {
                            push_streaming_value_skip(out, field, assertion);
                        }
                    }
                    "not_empty" => {
                        out.push_str(&crate::e2e::template_env::render(
                            "typescript/assertion.jinja",
                            minijinja::context! {
                                assertion_type => "not_empty",
                                field_expr => expr.clone(),
                                field_is_optional => false,
                            },
                        ));
                    }
                    "is_empty" => {
                        out.push_str(&format!("    expect({expr}).toBeFalsy();\n"));
                    }
                    "is_true" => {
                        out.push_str(&format!("    expect({expr}).toBe(true);\n"));
                    }
                    "is_false" => {
                        out.push_str(&format!("    expect({expr}).toBe(false);\n"));
                    }
                    "greater_than" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            out.push_str(&format!("    expect({expr}).toBeGreaterThan({js_val});\n"));
                        } else {
                            push_streaming_value_skip(out, field, assertion);
                        }
                    }
                    "greater_than_or_equal" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            out.push_str(&format!("    expect({expr}).toBeGreaterThanOrEqual({js_val});\n"));
                        } else {
                            push_streaming_value_skip(out, field, assertion);
                        }
                    }
                    "contains" => {
                        if let Some(val) = &assertion.value {
                            let js_val = json_to_js(val);
                            out.push_str(&format!("    expect({expr}).toContain({js_val});\n"));
                        } else {
                            push_streaming_value_skip(out, field, assertion);
                        }
                    }
                    _ => {
                        out.push_str(&format!(
                            "{}\n",
                            streaming_assertion_type_skip_line("    ", "//", field, &assertion.assertion_type)
                        ));
                    }
                }
            } else {
                // ~keep `accessor` returns `None` for reachable inputs — every `stream.has_*_event`
                // predicate does, since this call supplies no item type, and `wasm` has no
                // streaming at all — and this branch used to be absent: the arm still reported
                // "handled" (`true`) while emitting nothing, so the assertion vanished with no line
                // for `fail_on_unavailable_field_markers` to see. alef's streaming adapter owns the
                // gap, so it is counted, never fatal.
                out.push_str(&format!(
                    "    // skipped: {}\n",
                    FieldSkip::StreamingAssertionOnUnsupportedField.message(field)
                ));
            }
            true
        }
        _ => false,
    }
}

/// ~keep Every value-taking streaming arm above narrowed on `assertion.value` and emitted nothing
/// when it was absent, so the assertion disappeared with no line for any funnel to count. One
/// helper keeps the six call sites on the single registered wording.
fn push_streaming_value_skip(out: &mut String, field: &str, assertion: &Assertion) {
    out.push_str(&format!(
        "{}\n",
        streaming_assertion_value_skip_line("    ", "//", field, &assertion.assertion_type)
    ));
}

fn emit_bool_assertion(out: &mut String, pred: &str, assertion_type: &str, field: &str) {
    let pred = pred.to_string();
    let assertion_type = assertion_type.to_string();
    let field_name = field.to_string();
    let rendered = crate::e2e::template_env::render(
        "typescript/synthetic_assertion.jinja",
        minijinja::context! {
            assertion_type,
            pred,
            field_name,
        },
    );
    out.push_str(&rendered);
}

fn render_embeddings_assertion(out: &mut String, assertion: &Assertion, result_var: &str) {
    let assertion_type = assertion.assertion_type.as_str();
    let result_var = result_var.to_string();
    let field_name = "embeddings".to_string();

    match assertion_type {
        "count_equals" | "count_min" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let rendered = crate::e2e::template_env::render(
                    "typescript/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_type,
                        result_var,
                        field_name,
                        js_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_empty" | "is_empty" => {
            let rendered = crate::e2e::template_env::render(
                "typescript/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_type,
                    result_var,
                    field_name,
                },
            );
            out.push_str(&rendered);
        }
        _ => {
            let rendered = crate::e2e::template_env::render(
                "typescript/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_type,
                    field_name,
                },
            );
            out.push_str(&rendered);
        }
    }
}

fn render_embedding_dimensions(out: &mut String, assertion: &Assertion, result_var: &str) {
    let expr = format!("({result_var}.length > 0 ? {result_var}[0].length : 0)");
    let assertion_type = assertion.assertion_type.as_str();
    let expr = expr.clone();
    let field_name = "embedding_dimensions".to_string();

    match assertion_type {
        "equals" | "greater_than" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let rendered = crate::e2e::template_env::render(
                    "typescript/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_type,
                        expr,
                        field_name,
                        js_val,
                    },
                );
                out.push_str(&rendered);
            }
        }
        _ => {
            let rendered = crate::e2e::template_env::render(
                "typescript/synthetic_assertion.jinja",
                minijinja::context! {
                    assertion_type,
                    field_name,
                },
            );
            out.push_str(&rendered);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_standard_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_expr: &str,
    field_resolver: &FieldResolver,
    field_is_array: bool,
    is_enum_field: bool,
    is_format_metadata: bool,
) {
    let assertion_type = assertion.assertion_type.as_str();
    let resolved_field = assertion.field.as_deref().unwrap_or("");
    let field_is_optional =
        !resolved_field.is_empty() && field_resolver.is_optional(field_resolver.resolve(resolved_field));

    // Handle different assertion types
    match assertion_type {
        "equals" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_optional => field_is_optional,
                        is_string_val => expected.is_string(),
                        is_enum_field => is_enum_field && expected.is_string(),
                        is_format_metadata => is_format_metadata && expected.is_string(),
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_optional => field_is_optional,
                        field_is_array => field_is_array,
                        is_string_val => expected.is_string(),
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(json_to_js).collect();
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_optional => field_is_optional,
                        field_is_array => field_is_array,
                        is_string_val => values.iter().all(|v| v.is_string()),
                        values_js => items,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_contains" => {
            for expected in assertion.expected_values() {
                let js_val = json_to_js(expected);
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_array => field_is_array,
                        is_string_val => expected.is_string(),
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_empty" => {
            let rendered = crate::e2e::template_env::render(
                "typescript/assertion.jinja",
                minijinja::context! {
                    assertion_type => assertion_type,
                    field_expr => field_expr,
                    field_is_optional => field_is_optional,
                },
            );
            out.push_str(&rendered);
        }
        "is_empty" => {
            let rendered = crate::e2e::template_env::render(
                "typescript/assertion.jinja",
                minijinja::context! {
                    assertion_type => assertion_type,
                    field_expr => field_expr,
                },
            );
            out.push_str(&rendered);
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(json_to_js).collect();
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_array => field_is_array,
                        is_string_val => values.iter().all(|v| v.is_string()),
                        values_js => items,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let js_val = json_to_js(val);
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "starts_with" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        field_is_optional => field_is_optional,
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "count_min" | "count_equals" | "min_length" | "max_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        n => n,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "is_true" => {
            let rendered = crate::e2e::template_env::render(
                "typescript/assertion.jinja",
                minijinja::context! {
                    assertion_type => assertion_type,
                    field_expr => field_expr,
                    field_is_optional => field_is_optional,
                },
            );
            out.push_str(&rendered);
        }
        "is_false" => {
            let rendered = crate::e2e::template_env::render(
                "typescript/assertion.jinja",
                minijinja::context! {
                    assertion_type => assertion_type,
                    field_expr => field_expr,
                    field_is_optional => field_is_optional,
                },
            );
            out.push_str(&rendered);
        }
        "method_result" => {
            render_method_result_assertion(out, assertion, result_var);
        }
        "ends_with" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        js_val => js_val,
                        has_js_val => true,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "matches_regex" => {
            if let Some(expected) = &assertion.value
                && let Some(pattern) = expected.as_str()
            {
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion_type,
                        field_expr => field_expr,
                        expected_pattern => pattern,
                    },
                );
                out.push_str(&rendered);
            }
        }
        // `not_error` is intercepted earlier, in the top-level `render_assertion`
        // (before `field_expr`/`is_enum_field` are even computed — it never carries
        // a `field`) — see the fix there. Unreachable here; treated as unsupported
        // rather than silently doing nothing if a future refactor ever routes it
        // this far, so the gap surfaces immediately instead of hiding again.
        "error" => {
            // Handled at the test level (early return above).
        }
        other => {
            panic!("TypeScript e2e generator: unsupported assertion type: {other}");
        }
    }
}

fn render_method_result_assertion(out: &mut String, assertion: &Assertion, result_var: &str) {
    if let Some(method_name) = &assertion.method {
        let call_expr = build_ts_method_call(result_var, method_name, assertion.args.as_ref());
        let check = assertion.check.as_deref().unwrap_or("is_true");
        match check {
            "equals" => {
                if let Some(val) = &assertion.value {
                    let js_val = json_to_js(val);
                    let rendered = crate::e2e::template_env::render(
                        "typescript/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "method_result",
                            check => check,
                            call_expr => call_expr,
                            check_js_val => js_val,
                            has_check_js_val => true,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
            "is_true" => {
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "method_result",
                        check => check,
                        call_expr => call_expr,
                    },
                );
                out.push_str(&rendered);
            }
            "is_false" => {
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "method_result",
                        check => check,
                        call_expr => call_expr,
                    },
                );
                out.push_str(&rendered);
            }
            "greater_than_or_equal" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let rendered = crate::e2e::template_env::render(
                        "typescript/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "method_result",
                            check => check,
                            call_expr => call_expr,
                            check_n => n,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
            "count_min" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let rendered = crate::e2e::template_env::render(
                        "typescript/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "method_result",
                            check => check,
                            call_expr => call_expr,
                            check_n => n,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
            "contains" => {
                if let Some(val) = &assertion.value {
                    let js_val = json_to_js(val);
                    let rendered = crate::e2e::template_env::render(
                        "typescript/assertion.jinja",
                        minijinja::context! {
                            assertion_type => "method_result",
                            check => check,
                            call_expr => call_expr,
                            check_js_val => js_val,
                            has_check_js_val => true,
                        },
                    );
                    out.push_str(&rendered);
                }
            }
            "is_error" => {
                let rendered = crate::e2e::template_env::render(
                    "typescript/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "method_result",
                        check => check,
                        call_expr => call_expr,
                    },
                );
                out.push_str(&rendered);
            }
            other_check => {
                panic!("TypeScript e2e generator: unsupported method_result check type: {other_check}");
            }
        }
    } else {
        panic!("TypeScript e2e generator: method_result assertion missing 'method' field");
    }
}

/// Build a TypeScript call expression for a method_result assertion on a tree-sitter Tree.
pub(super) fn build_ts_method_call(result_var: &str, method_name: &str, args: Option<&serde_json::Value>) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.rootNode.childCount"),
        "root_node_type" => format!("{result_var}.rootNode.type"),
        "named_children_count" => format!("{result_var}.rootNode.namedChildCount"),
        "has_error_nodes" => format!("treeHasErrorNodes({result_var})"),
        "error_count" | "tree_error_count" => format!("treeErrorCount({result_var})"),
        "tree_to_sexp" => format!("treeToSexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("treeContainsNodeType({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("findNodesByType({result_var}, \"{node_type}\")")
        }
        "run_query" => {
            let query_source = args
                .and_then(|a| a.get("query_source"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let language = args
                .and_then(|a| a.get("language"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("runQuery({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => {
            if let Some(args_val) = args {
                let arg_str = args_val
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .map(|(k, v)| format!("{}: {}", k, json_to_js(v)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                format!("{result_var}.{method_name}({arg_str})")
            } else {
                format!("{result_var}.{method_name}()")
            }
        }
    }
}

/// Render the `foo[].bar` wildcard forms as an `Array.prototype.some` quantifier over
/// every element, rather than an index-0 lookup. The array expression is `??`-guarded
/// because an absent optional list is `undefined` and `.some` would throw on it.
fn render_wildcard_assertion(
    out: &mut String,
    assertion: &Assertion,
    array_accessor: &str,
    elem_accessor: &str,
    field: &str,
) {
    let guarded = format!("({array_accessor} ?? [])");
    let some_expr = |js_val: &str| format!("{guarded}.some((e) => String({elem_accessor}).includes({js_val}))");
    match assertion.assertion_type.as_str() {
        "contains" => {
            if let Some(expected) = &assertion.value {
                let js_val = json_to_js(expected);
                out.push_str(&format!("    expect({}).toBe(true);\n", some_expr(&js_val)));
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let js_val = json_to_js(val);
                    out.push_str(&format!("    expect({}).toBe(true);\n", some_expr(&js_val)));
                }
            }
        }
        "not_contains" => {
            for expected in assertion.expected_values() {
                let js_val = json_to_js(expected);
                out.push_str(&format!("    expect({}).toBe(false);\n", some_expr(&js_val)));
            }
        }
        "not_empty" => {
            out.push_str(&format!(
                "    expect({guarded}.some((e) => String({elem_accessor}).length > 0)).toBe(true);\n"
            ));
        }
        other => {
            out.push_str(&format!(
                "    // skipped: unsupported traversal assertion '{other}' on '{field}'\n"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;

    fn empty_resolver() -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    fn array_resolver(field: &str) -> FieldResolver {
        let result_fields = HashSet::from([field.to_string()]);
        let array_fields = HashSet::from([field.to_string()]);
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &array_fields,
            &HashSet::new(),
        )
    }

    fn make_assertion(assertion_type: &str, field: Option<&str>, value: Option<serde_json::Value>) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: field.map(|s| s.to_string()),
            value,
            ..Default::default()
        }
    }

    /// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
    /// (present, non-`binding_excluded`, on some IR type) but missing from the
    /// hand-maintained `result_fields` config must still render a real assertion,
    /// not a "skipped: field not available" comment — `typescript/test_file/test_case.rs`
    /// (shared by `typescript` and `wasm`) now threads
    /// `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
    #[test]
    fn typescript_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
        let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(reachable, HashSet::new(), HashSet::new());
        let assertion = make_assertion("equals", Some("data"), Some(serde_json::Value::String("hello".into())));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &HashMap::new(),
            "typescript",
            false,
            false,
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
    fn typescript_ir_excluded_field_present_in_result_fields_is_still_skipped() {
        let result_fields: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
        let excluded: HashSet<String> = ["internal_diagnostics".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(HashSet::new(), excluded, HashSet::new());
        let assertion = make_assertion(
            "equals",
            Some("internal_diagnostics"),
            Some(serde_json::Value::String("hello".into())),
        );
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &HashMap::new(),
            "typescript",
            false,
            false,
        );
        assert!(out.contains("skipped"), "got: {out}");
    }

    /// Regression test for a one-sided-trim bug: `.trim()` wrapped the actual value while
    /// the fixture `expected` literal was emitted verbatim. Fixture expectations may
    /// legitimately end in `\n`, so trimming only one side made those assertions impossible
    /// to satisfy — and trimming both would silently mask real trailing-whitespace
    /// regressions. Equals is exact: neither side is normalized. Covers both the
    /// `typescript` and `wasm` templates, which share this renderer.
    #[test]
    fn render_assertion_equals_string_compares_exactly_without_trim() {
        for lang in ["typescript", "wasm"] {
            let resolver = empty_resolver();
            let assertion = make_assertion("equals", None, Some(serde_json::Value::String("hello\n".into())));
            let mut out = String::new();
            render_assertion(
                &mut out,
                &assertion,
                "result",
                &resolver,
                true,
                &HashMap::new(),
                lang,
                false,
                false,
            );
            assert!(
                !out.contains(".trim()"),
                "[{lang}] equals must not trim either side; got: {out}"
            );
            assert!(out.contains(".toBe("), "[{lang}] got: {out}");
        }
    }

    /// Control for the trim fix: the tightened contract must still DISCRIMINATE values that
    /// differ only in trailing whitespace. If either side were normalized, the emitted
    /// assertion for "hello\n" and for "hello" would be identical and a real trailing-newline
    /// regression would pass unnoticed. They must differ, and the expected literal must be
    /// carried through verbatim.
    #[test]
    fn render_assertion_equals_still_discriminates_trailing_whitespace() {
        for lang in ["typescript", "wasm"] {
            let render_for = |value: &str| {
                let resolver = empty_resolver();
                let assertion = make_assertion("equals", None, Some(serde_json::Value::String(value.into())));
                let mut out = String::new();
                render_assertion(
                    &mut out,
                    &assertion,
                    "result",
                    &resolver,
                    true,
                    &HashMap::new(),
                    lang,
                    false,
                    false,
                );
                out
            };
            let with_newline = render_for("hello\n");
            let without_newline = render_for("hello");
            // The actual side must be the bare expression: any normalizing call (trim/
            // case-folding) wrapped around it would silently accept a mismatched value.
            assert_eq!(
                with_newline, "    expect(result).toBe(\"hello\\n\");\n",
                "[{lang}] emitted assertion drifted: {with_newline}"
            );
            assert_ne!(
                with_newline, without_newline,
                "[{lang}] trailing newline must still change the emitted assertion"
            );
        }
    }

    #[test]
    fn render_assertion_not_empty_emits_length_check() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_empty", None, None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &std::collections::HashMap::new(),
            "node",
            false,
            false,
        );
        assert!(out.contains(".length"), "got: {out}");
    }

    #[test]
    fn streaming_not_empty_rejects_empty_chunks_without_rejecting_zero() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_empty", Some("chunks"), None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &HashMap::new(),
            "node",
            true,
            false,
        );
        assert!(!out.contains("toBeTruthy"), "got: {out}");
        assert!(out.contains("expect(_v.length).toBeGreaterThan(0);"), "got: {out}");
        assert!(out.contains("expect(_v).not.toBeNull();"), "got: {out}");
    }

    #[test]
    fn render_assertion_equals_string_compares_exactly_for_node() {
        let resolver = empty_resolver();
        let assertion = make_assertion("equals", None, Some(serde_json::Value::String("hello\n".into())));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &std::collections::HashMap::new(),
            "node",
            false,
            false,
        );
        assert!(!out.contains(".trim()"), "equals must not trim either side; got: {out}");
    }

    #[test]
    fn render_assertion_is_empty_allows_nullish_simple_results() {
        let resolver = empty_resolver();
        let assertion = make_assertion("is_empty", None, None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &std::collections::HashMap::new(),
            "node",
            false,
            false,
        );
        assert!(out.contains("(result ?? \"\").length"), "got: {out}");
    }

    #[test]
    fn render_assertion_contains_string_array_uses_item_texts() {
        let resolver = array_resolver("structure");
        let assertion = make_assertion(
            "contains",
            Some("structure"),
            Some(serde_json::Value::String("Function".into())),
        );
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &std::collections::HashMap::new(),
            "node",
            false,
            false,
        );
        assert!(out.contains("_alefE2eItemTexts(item)"), "got: {out}");
    }

    #[test]
    fn build_ts_method_call_root_child_count() {
        let expr = build_ts_method_call("tree", "root_child_count", None);
        assert_eq!(expr, "tree.rootNode.childCount");
    }

    /// Regression test for the not_error vacuous-test defect: before this fix,
    /// `not_error` rendered nothing at all (it was a no-op in this file's
    /// `render_standard_assertion`), and `test_case.rs`'s `has_usable_assertion`
    /// predicate separately excluded `not_error`, so a fixture whose only
    /// assertion was `not_error` produced `await callExpr();` with the result
    /// entirely discarded and zero assertions. Must emit a real, visible check.
    #[test]
    fn not_error_emits_a_real_expect_to_be_defined_on_the_result() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_error", None, None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &std::collections::HashMap::new(),
            "node",
            false,
            false,
        );
        assert_eq!(out, "    expect(result).toBeDefined();\n");
    }

    #[test]
    fn not_error_on_a_streaming_fixture_asserts_on_drained_chunks_not_result() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_error", None, None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            false,
            &std::collections::HashMap::new(),
            "node",
            true,
            false,
        );
        assert_eq!(out, "    expect(chunks).toBeDefined();\n");
    }
}

#[cfg(test)]
mod wildcard_tests {
    use super::render_assertion;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn array_resolver(field: &str) -> FieldResolver {
        let result_fields: HashSet<String> = [field.to_string()].into_iter().collect();
        let array_fields: HashSet<String> = [field.to_string()].into_iter().collect();
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &array_fields,
            &HashSet::new(),
        )
    }

    fn render(assertion: &Assertion, resolver: &FieldResolver) -> String {
        let mut out = String::new();
        render_assertion(
            &mut out,
            assertion,
            "result",
            resolver,
            false,
            &HashMap::new(),
            "typescript",
            false,
            false,
        );
        out
    }

    fn contains_on(field: &str, value: &str) -> Assertion {
        Assertion {
            assertion_type: "contains".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Default::default()
        }
    }

    #[test]
    fn typescript_wildcard_contains_quantifies_over_every_element() {
        let out = render(&contains_on("items[].name", "beta"), &array_resolver("items"));
        assert_eq!(
            out, "    expect((result.items ?? []).some((e) => String(e.name).includes(\"beta\"))).toBe(true);\n",
            "got: {out}"
        );
    }

    /// Regression lock: an explicit numeric index is a different, correct feature and must
    /// keep lowering to a positional lookup, not a quantifier. ~keep
    #[test]
    fn typescript_explicit_index_still_lowers_to_a_positional_lookup() {
        let out = render(&contains_on("items[0].name", "beta"), &array_resolver("items"));
        assert!(out.contains("result.items[0].name"), "got: {out}");
        assert!(!out.contains(".some((e)"), "got: {out}");
    }

    /// CANARY. A code-generator unit test cannot execute JavaScript, so it cannot literally
    /// run a fixture whose only match lives in element 1. The observable proxy is exact: the
    /// pre-fix renderer emitted `result.items[0].name`, a lookup pinned to element 0, so a
    /// value present only at element 1 could never be seen. This asserts no positional index
    /// survives and the predicate is quantified over the whole array; it fails against the
    /// pre-fix code, where `result.items[0]` is present. ~keep
    #[test]
    fn typescript_wildcard_match_in_a_non_first_element_is_not_pinned_to_element_zero() {
        let out = render(
            &contains_on("items[].name", "only-in-element-1"),
            &array_resolver("items"),
        );
        assert!(!out.contains("[0]"), "index-pinned lookup survived: {out}");
        assert!(out.contains("(result.items ?? []).some((e) =>"), "got: {out}");
        assert!(out.contains("String(e.name)"), "got: {out}");
    }

    /// `tools[].function` is also used as a DTO *type path* for input construction in
    /// `test_file/render.rs` and `wasm.rs`. Those call sites never reach `render_assertion`,
    /// so the wildcard branch here must not be the thing that keeps them working — this
    /// only pins that the assertion path itself treats such a path as a traversal. ~keep
    #[test]
    fn typescript_wildcard_branch_is_scoped_to_assertion_rendering() {
        let out = render(&contains_on("items[].name", "beta"), &array_resolver("items"));
        assert!(out.starts_with("    expect("), "got: {out}");
    }

    /// `wildcard_split` consumes the first `[].` only, so before the guard the `.some()`
    /// ranged over `pages` while its body read `e.links[0].url` — a whole-array claim that
    /// only ever inspected element zero of the inner list. Pre-guard this test fails: the
    /// skip line is absent and `links[0]` is present. ~keep
    #[test]
    fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
        let out = render(
            &contains_on("pages[].links[].url", "example.test"),
            &array_resolver("pages"),
        );
        assert_eq!(
            out, "    // skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
            "got: {out}"
        );
    }
}

#[cfg(test)]
mod skip_marker_tests {
    use super::render_synthetic_field_assertion;
    use crate::e2e::codegen::assertion_type_skip::AssertionTypeSkip;
    use crate::e2e::codegen::field_skip::FieldSkip;
    use crate::e2e::codegen::{SkipVerdict, fail_on_unavailable_field_markers, take_skip_records};
    use crate::e2e::fixture::Assertion;

    fn render_streaming(assertion_type: &str, field: &str, value: Option<serde_json::Value>) -> (String, bool) {
        let assertion = Assertion {
            assertion_type: assertion_type.to_string(),
            field: Some(field.to_string()),
            value,
            ..Assertion::default()
        };
        let mut out = String::new();
        let handled = render_synthetic_field_assertion(&mut out, &assertion, "result", field, true);
        (out, handled)
    }

    fn field_verdicts(body: &str) -> Vec<SkipVerdict> {
        let _ = take_skip_records();
        fail_on_unavailable_field_markers(body, "node", "stream_smoke", &[]);
        take_skip_records().into_iter().map(|record| record.verdict).collect()
    }

    /// Non-vacuity control: the same harness on a resolvable streaming field must render a real
    /// `expect(...)`, or the marker assertions below would be facts about the harness. ~keep
    #[test]
    fn the_streaming_harness_renders_a_real_expectation_when_the_accessor_resolves() {
        let (out, handled) = render_streaming("count_min", "chunks", Some(serde_json::json!(2)));
        assert!(handled, "the streaming arm must claim the assertion");
        assert!(
            out.contains("expect(chunks.length).toBeGreaterThanOrEqual(2)"),
            "got: {out}"
        );
        assert!(
            field_verdicts(&out).is_empty(),
            "a live assertion records no skip: {out}"
        );
    }

    /// The arm returned `true` — "handled" — while writing nothing, so the assertion vanished and
    /// no later branch could rescue it. `wasm` shares this renderer and has no streaming at all,
    /// so this is the whole-language case, not an edge one. ~keep
    #[test]
    fn a_streaming_field_with_no_accessor_emits_a_counted_marker() {
        let (out, handled) = render_streaming("is_true", "stream.has_page_event", None);
        assert!(handled, "the streaming arm still claims the assertion");
        assert!(!out.is_empty(), "the assertion must not vanish");
        assert_eq!(
            FieldSkip::extract_classified(out.trim_end()),
            Some(("stream.has_page_event", FieldSkip::StreamingAssertionOnUnsupportedField)),
            "got: {out}"
        );
        assert_eq!(field_verdicts(&out), vec![SkipVerdict::AwaitingGeneratorSupport]);
    }

    #[test]
    fn an_unrenderable_streaming_assertion_type_emits_a_registered_marker() {
        let (out, _) = render_streaming("matches_regex", "chunks", None);
        assert_eq!(
            AssertionTypeSkip::extract_classified(out.trim_end()),
            Some(("matches_regex", AssertionTypeSkip::StreamingAssertionTypeNotSupported)),
            "got: {out}"
        );
    }

    #[test]
    fn a_streaming_assertion_with_no_value_emits_a_registered_marker() {
        let (out, _) = render_streaming("count_min", "chunks", None);
        assert_eq!(
            AssertionTypeSkip::extract_classified(out.trim_end()),
            Some(("count_min", AssertionTypeSkip::StreamingAssertionValueNotRenderable)),
            "got: {out}"
        );
    }
}
