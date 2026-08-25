//! Assertion rendering for Python e2e tests.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::json::{python_string_literal, value_to_python_string};

/// Render a single assertion into the test function body.
pub(super) fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    fields_enum: &HashSet<String>,
    assert_enum_fields: &HashMap<String, String>,
    result_is_simple: bool,
) {
    // When result_is_simple, skip fields that reference struct sub-fields.
    if result_is_simple && let Some(f) = &assertion.field {
        let f_lower = f.to_lowercase();
        if !f.is_empty()
            && f_lower != "content"
            && f_lower != "result"
            && (f_lower.starts_with("metadata")
                || f_lower.starts_with("document")
                || f_lower.starts_with("structure")
                || f_lower.starts_with("pages")
                || f_lower.starts_with("chunks")
                || f_lower.starts_with("tables")
                || f_lower.starts_with("images")
                || f_lower.starts_with("mime_type")
                || f_lower.starts_with("is_")
                || f_lower == "byte_length"
                || f_lower == "page_count"
                || f_lower == "output_format"
                || f_lower == "extraction_method")
        {
            let _ = writeln!(
                out,
                "    # skipped: {}",
                FieldSkip::NotApplicableForSimpleResultType.message(f)
            );
            return;
        }
    }

    // Handle synthetic / derived fields.
    if let Some(f) = &assertion.field {
        if let Some(reason) = crate::e2e::codegen::assertion_recipes::chunks_synthetic_skip_reason(f, field_resolver) {
            let _ = writeln!(out, "    # skipped: {reason}");
            return;
        }
        if render_synthetic_field(out, assertion, result_var, f) {
            return;
        }
    }

    // Skip assertions on fields that don't exist on the result type.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && !field_resolver.is_valid_for_result(f)
    {
        let _ = writeln!(out, "    # skipped: {}", FieldSkip::NotAvailableOnResultType.message(f));
        return;
    }

    // A `foo[].bar` fixture path means EVERY element of `foo`. The shared accessor lowers
    // `[]` to `[0]`, so the wildcard must be expanded into an `any(..)` comprehension
    // before the accessor is built. Returning here also keeps the wildcard away from the
    // `contains("[0]")` enum heuristic below, which only classifies explicit-index paths. ~keep
    if !result_is_simple
        && let Some(f) = assertion.field.as_deref()
        && !f.is_empty()
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        render_python_wildcard_assertion(out, assertion, f, &array_part, &elem_part, result_var, field_resolver);
        return;
    }

    let field_access = if result_is_simple {
        result_var.to_string()
    } else {
        match &assertion.field {
            Some(f) if !f.is_empty() => field_resolver.accessor(f, "python", result_var),
            _ => result_var.to_string(),
        }
    };

    let field_is_enum = assertion.field.as_deref().is_some_and(|f| {
        // Per-call assertion override wins over the global set.
        if assert_enum_fields.contains_key(f) {
            return true;
        }
        if fields_enum.contains(f) {
            return true;
        }
        let resolved = field_resolver.resolve(f);
        if fields_enum.contains(resolved) {
            return true;
        }
        // Neither the explicit config nor the per-call override named this field. Fall back
        // to the IR-derived classification (`with_ir_enum_map`, anchored at the call's
        // declared Rust return type via `resolve_declared_result_type`) so a consumer that
        // never configured `fields_enum` still gets a correct classification instead of the
        // dynamically-typed default of "compare as a plain string" — which asserts the wire
        // value against the Python enum's `repr`/member name instead of coercing it first.
        // This is purely additive: it only turns a `false` into a `true`. ~keep
        if field_resolver.is_enum(f) {
            return true;
        }
        field_resolver.accessor(f, "python", result_var).contains("[0]")
    });

    let field_is_optional = match &assertion.field {
        Some(f) if !f.is_empty() => {
            let resolved = field_resolver.resolve(f);
            field_resolver.is_optional(resolved)
        }
        _ => false,
    };
    let field_is_array = assertion
        .field
        .as_deref()
        .is_some_and(|f| field_resolver.is_array(field_resolver.resolve(f)));

    render_standard_assertion(
        out,
        assertion,
        result_var,
        &field_access,
        field_is_enum,
        field_is_optional,
        field_is_array,
    );
}

fn render_python_wildcard_assertion(
    out: &mut String,
    assertion: &Assertion,
    field: &str,
    array_part: &str,
    elem_part: &str,
    result_var: &str,
    field_resolver: &FieldResolver,
) {
    // `wildcard_split` consumes the first `[].` only, so a doubly-nested path leaves a second
    // wildcard in `elem_part` that the element accessor below would lower to index 0. ~keep
    if let Some(line) = nested_wildcard_skip_line("    ", "#", field, elem_part) {
        let _ = writeln!(out, "{line}");
        return;
    }
    let array_accessor = if array_part.is_empty() {
        result_var.to_string()
    } else {
        field_resolver.accessor(array_part, "python", result_var)
    };
    // Passing the comprehension variable as the "result var" is what makes nested element
    // sub-paths (`links[].meta.kind`) resolve against the loop variable. ~keep
    let elem_accessor = if elem_part.is_empty() {
        "_e".to_string()
    } else {
        field_resolver.accessor(elem_part, "python", "_e")
    };
    let iterable = format!("({array_accessor} or [])");

    match assertion.assertion_type.as_str() {
        "contains" | "contains_all" | "not_contains" => {
            let negate = assertion.assertion_type == "not_contains";
            let values: Vec<&serde_json::Value> = if assertion.assertion_type == "contains" {
                assertion.value.iter().collect()
            } else {
                assertion.expected_values()
            };
            for val in values {
                let expected = value_to_python_string(val);
                // `in str(..)` needs a str left operand; non-string fixture values
                // (numbers, bools) have to be stringified first. ~keep
                let needle = if val.is_string() {
                    expected
                } else {
                    format!("str({expected})")
                };
                let pred = format!("any({needle} in str({elem_accessor}) for _e in {iterable})");
                if negate {
                    let _ = writeln!(out, "    assert not {pred}  # noqa: S101");
                } else {
                    let _ = writeln!(out, "    assert {pred}  # noqa: S101");
                }
            }
        }
        "not_empty" => {
            let pred = format!("any(str({elem_accessor}) != \"\" for _e in {iterable})");
            let _ = writeln!(out, "    assert {pred}  # noqa: S101");
        }
        other => {
            let _ = writeln!(
                out,
                "    # skipped: unsupported traversal assertion '{other}' on '{field}'"
            );
        }
    }
}

fn render_synthetic_field(out: &mut String, assertion: &Assertion, result_var: &str, field: &str) -> bool {
    match field {
        "chunks_have_content" => {
            let pred = format!("all(c.content for c in ({result_var}.chunks or []))");
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "chunks_have_heading_context" => {
            let pred = format!(
                "all(c.metadata and c.metadata.heading_context is not None for c in ({result_var}.chunks or []))"
            );
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "first_chunk_starts_with_heading" => {
            let pred = format!(
                "bool(({result_var}.chunks or []) and ({result_var}.chunks[0].metadata and {result_var}.chunks[0].metadata.heading_context))"
            );
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "chunks_have_embeddings" => {
            let pred =
                format!("all(c.embedding is not None and len(c.embedding) > 0 for c in ({result_var}.chunks or []))");
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
                "embeddings_valid" => format!("all(bool(e) for e in {result_var})"),
                "embeddings_finite" => {
                    format!("all(v == v and abs(v) != float('inf') for e in {result_var} for v in e)")
                }
                "embeddings_non_zero" => {
                    format!("all(any(v != 0.0 for v in e) for e in {result_var})")
                }
                "embeddings_normalized" => {
                    format!("all(abs(sum(v * v for v in e) - 1.0) < 1e-3 for e in {result_var})")
                }
                _ => unreachable!(),
            };
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "keywords" | "keywords_count" => {
            let _ = writeln!(
                out,
                "    # skipped: {}",
                FieldSkip::NotAvailableOnPythonProcessingResult.message(field)
            );
            true
        }
        _ => false,
    }
}

fn emit_bool_assertion(out: &mut String, pred: &str, assertion_type: &str, field: &str) {
    match assertion_type {
        "is_true" => {
            let _ = writeln!(out, "    assert {pred}  # noqa: S101");
        }
        "is_false" => {
            let _ = writeln!(out, "    assert not ({pred})  # noqa: S101");
        }
        other => {
            panic!("Python e2e generator: unsupported assertion type '{other}' on synthetic field '{field}'");
        }
    }
}

fn render_embeddings_assertion(out: &mut String, assertion: &Assertion, result_var: &str) {
    match assertion.assertion_type.as_str() {
        "count_equals" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    assert len({result_var}) == {n}  # noqa: S101");
            }
        }
        "count_min" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(out, "    assert len({result_var}) >= {n}  # noqa: S101");
            }
        }
        "not_empty" => {
            let _ = writeln!(out, "    assert len({result_var}) > 0  # noqa: S101");
        }
        "is_empty" => {
            let _ = writeln!(out, "    assert len({result_var}) == 0  # noqa: S101");
        }
        other => {
            panic!("Python e2e generator: unsupported assertion type '{other}' on synthetic field 'embeddings'");
        }
    }
}

fn render_embedding_dimensions(out: &mut String, assertion: &Assertion, result_var: &str) {
    let expr = format!("(len({result_var}[0]) if {result_var} else 0)");
    match assertion.assertion_type.as_str() {
        "equals" => {
            if let Some(val) = &assertion.value {
                let py_val = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} == {py_val}  # noqa: S101");
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                let py_val = value_to_python_string(val);
                let _ = writeln!(out, "    assert {expr} > {py_val}  # noqa: S101");
            }
        }
        other => {
            panic!(
                "Python e2e generator: unsupported assertion type '{other}' on synthetic field 'embedding_dimensions'"
            );
        }
    }
}

fn render_standard_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_access: &str,
    field_is_enum: bool,
    field_is_optional: bool,
    field_is_array: bool,
) {
    let _ = (result_var, python_string_literal); // available for potential future use
    match assertion.assertion_type.as_str() {
        "error" | "not_error" => {
            // Handled at call site.
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                let values_list: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let expected = value_to_python_string(v);
                        python_contains_expr(field_access, &expected, field_is_enum, field_is_array, v.is_string())
                    })
                    .collect();
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains_all",
                        field_access => field_access,
                        field_is_optional => field_is_optional,
                        values_list => values_list,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let items: Vec<String> = values.iter().map(value_to_python_string).collect();
                let list_str = items.join(", ");
                let cmp_expr = if field_is_array {
                    format!(
                        "any(any(v in text for text in _alef_e2e_item_texts(item)) for item in {field_access} for v in [{list_str}])"
                    )
                } else if field_is_enum {
                    format!("any(v.lower() in str({field_access}).lower() for v in [{list_str}])")
                } else {
                    format!("any(v in {field_access} for v in [{list_str}])")
                };
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains_any",
                        field_access => field_access,
                        field_is_optional => field_is_optional,
                        cmp_expr_any => cmp_expr,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "method_result" => {
            render_method_result(out, assertion, result_var);
        }
        "equals" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let op = if val.is_boolean() || val.is_null() { "is" } else { "==" };
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "equals",
                        field_access => field_access,
                        field_is_optional => field_is_optional,
                        is_enum => field_is_enum,
                        expected_val => expected,
                        op => op,
                        is_string_val => val.is_string(),
                    },
                );
                out.push_str(&rendered);
            }
        }
        "contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let cmp_expr =
                    python_contains_expr(field_access, &expected, field_is_enum, field_is_array, val.is_string());
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "contains",
                        field_access => field_access,
                        field_is_optional => field_is_optional,
                        cmp_expr => cmp_expr,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_contains" => {
            for val in assertion.expected_values() {
                let expected = value_to_python_string(val);
                let cmp_expr =
                    python_contains_expr(field_access, &expected, field_is_enum, field_is_array, val.is_string());
                let negated_cmp_expr = negate_contains_expr(&cmp_expr, field_is_array, field_is_enum);
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "not_contains",
                        field_access => field_access,
                        field_is_optional => field_is_optional,
                        negated_cmp_expr => negated_cmp_expr,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "not_empty" => {
            let rendered = crate::e2e::template_env::render(
                "python/assertion.jinja",
                minijinja::context! {
                    assertion_type => "not_empty",
                    field_access => field_access,
                },
            );
            out.push_str(&rendered);
        }
        "is_empty" => {
            let rendered = crate::e2e::template_env::render(
                "python/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_empty",
                    field_access => field_access,
                },
            );
            out.push_str(&rendered);
        }
        "greater_than" | "less_than" | "greater_than_or_equal" | "less_than_or_equal" | "min" | "max" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion.assertion_type.as_str(),
                        field_access => field_access,
                        expected_val => expected,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "starts_with" | "ends_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion.assertion_type.as_str(),
                        field_access => field_access,
                        expected_val => expected,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "min_length" | "max_length" | "count_min" | "count_equals" => {
            if let Some(val) = &assertion.value {
                let n = val.as_u64().unwrap_or(0);
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => assertion.assertion_type.as_str(),
                        field_access => field_access,
                        n => n,
                    },
                );
                out.push_str(&rendered);
            }
        }
        "is_true" => {
            let rendered = crate::e2e::template_env::render(
                "python/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_true",
                    field_access => field_access,
                    field_is_optional => field_is_optional,
                },
            );
            out.push_str(&rendered);
        }
        "is_false" => {
            let rendered = crate::e2e::template_env::render(
                "python/assertion.jinja",
                minijinja::context! {
                    assertion_type => "is_false",
                    field_access => field_access,
                    field_is_optional => field_is_optional,
                },
            );
            out.push_str(&rendered);
        }
        "matches_regex" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_python_string(val);
                let rendered = crate::e2e::template_env::render(
                    "python/assertion.jinja",
                    minijinja::context! {
                        assertion_type => "matches_regex",
                        field_access => field_access,
                        expected_val => expected,
                    },
                );
                out.push_str(&rendered);
            }
        }
        other => {
            panic!("unsupported assertion type: {other}");
        }
    }
}

fn python_contains_expr(
    field_access: &str,
    expected: &str,
    field_is_enum: bool,
    field_is_array: bool,
    expected_is_string: bool,
) -> String {
    if field_is_array && expected_is_string {
        return format!("any({expected} in text for item in {field_access} for text in _alef_e2e_item_texts(item))");
    }
    if field_is_enum && expected_is_string {
        return format!("{expected}.lower() in str({field_access}).lower()");
    }
    format!("{expected} in {field_access}")
}

/// Negate a comparison expression for use in `not_contains` assertions.
/// For simple membership tests (e.g., `x in y`), emits `x not in y` directly
/// instead of wrapping with `not (...)` to avoid ruff E713 flip-flop issues.
fn negate_contains_expr(cmp_expr: &str, field_is_array: bool, field_is_enum: bool) -> String {
    // Simple membership test: `expected in field_access` → `expected not in field_access`
    if !field_is_array && !field_is_enum && cmp_expr.contains(" in ") && !cmp_expr.contains("not in") {
        return cmp_expr.replace(" in ", " not in ");
    }

    // For complex expressions (any(...), lower() calls, etc.), wrap with `not (...)`
    format!("not ({cmp_expr})")
}

fn render_method_result(out: &mut String, assertion: &Assertion, result_var: &str) {
    if let Some(method_name) = &assertion.method {
        let call_expr = build_python_method_call(result_var, method_name, assertion.args.as_ref());
        let check = assertion.check.as_deref().unwrap_or("is_true");
        match check {
            "equals" => {
                if let Some(val) = &assertion.value {
                    if val.is_boolean() {
                        if val.as_bool() == Some(true) {
                            let _ = writeln!(out, "    assert {call_expr} is True  # noqa: S101");
                        } else {
                            let _ = writeln!(out, "    assert {call_expr} is False  # noqa: S101");
                        }
                    } else {
                        let expected = value_to_python_string(val);
                        let _ = writeln!(out, "    assert {call_expr} == {expected}  # noqa: S101");
                    }
                }
            }
            "is_true" => {
                let _ = writeln!(out, "    assert {call_expr}  # noqa: S101");
            }
            "is_false" => {
                let _ = writeln!(out, "    assert not {call_expr}  # noqa: S101");
            }
            "greater_than_or_equal" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let _ = writeln!(out, "    assert {call_expr} >= {n}  # noqa: S101");
                }
            }
            "count_min" => {
                if let Some(val) = &assertion.value {
                    let n = val.as_u64().unwrap_or(0);
                    let _ = writeln!(out, "    assert len({call_expr}) >= {n}  # noqa: S101");
                }
            }
            "contains" => {
                if let Some(val) = &assertion.value {
                    let expected = value_to_python_string(val);
                    let _ = writeln!(out, "    assert {expected} in {call_expr}  # noqa: S101");
                }
            }
            "is_error" => {
                let _ = writeln!(out, "    with pytest.raises(Exception):  # noqa: B017");
                let _ = writeln!(out, "        {call_expr}");
            }
            other_check => {
                panic!("unsupported method_result check type: {other_check}");
            }
        }
    } else {
        panic!("method_result assertion missing 'method' field");
    }
}

pub(super) fn build_python_method_call(
    result_var: &str,
    method_name: &str,
    args: Option<&serde_json::Value>,
) -> String {
    match method_name {
        "root_child_count" => format!("{result_var}.root_node().child_count()"),
        "root_node_type" => format!("{result_var}.root_node().kind()"),
        "named_children_count" => format!("{result_var}.root_node().named_child_count()"),
        "has_error_nodes" => format!("tree_has_error_nodes({result_var})"),
        "error_count" | "tree_error_count" => format!("tree_error_count({result_var})"),
        "tree_to_sexp" => format!("tree_to_sexp({result_var})"),
        "contains_node_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("tree_contains_node_type({result_var}, \"{node_type}\")")
        }
        "find_nodes_by_type" => {
            let node_type = args
                .and_then(|a| a.get("node_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("find_nodes_by_type({result_var}, \"{node_type}\")")
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
            format!("run_query({result_var}, \"{language}\", \"{query_source}\", source)")
        }
        _ => {
            if let Some(args_val) = args {
                let arg_str = args_val
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .map(|(k, v)| format!("{}={}", k, value_to_python_string(v)))
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

    fn resolver_with_array_field(field: &str) -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::from([field.to_string()]),
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

    fn render_field_contains(resolver: &FieldResolver, field: &str, value: &str) -> String {
        let assertion = make_assertion("contains", Some(field), Some(serde_json::json!(value)));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            resolver,
            &HashSet::new(),
            &HashMap::new(),
            false,
        );
        out
    }

    fn resolver_with_optional_field(field: &str) -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::from([field.to_string()]),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
    }

    fn render_field_assertion(resolver: &FieldResolver, assertion: &Assertion) -> String {
        let mut out = String::new();
        render_assertion(
            &mut out,
            assertion,
            "result",
            resolver,
            &HashSet::new(),
            &HashMap::new(),
            false,
        );
        out
    }

    /// `Option<DataNode>` presence: before the fix this rendered `assert result.data is
    /// True`, which is never true for a present non-bool object (Python's `is` compares
    /// identity, and no struct instance is ever the singleton `True`).
    #[test]
    fn is_true_on_optional_struct_field_checks_presence() {
        let out = render_field_assertion(
            &resolver_with_optional_field("data"),
            &make_assertion("is_true", Some("data"), None),
        );
        assert_eq!(out, "    assert result.data is not None  # noqa: S101\n");
    }

    #[test]
    fn is_false_on_optional_struct_field_checks_absence() {
        let out = render_field_assertion(
            &resolver_with_optional_field("data"),
            &make_assertion("is_false", Some("data"), None),
        );
        assert_eq!(out, "    assert result.data is None  # noqa: S101\n");
    }

    /// A follow-on member access through the optional field: Python's dynamic typing means
    /// `result.data.kind` needs no unwrap ceremony at the codegen level (unlike Rust/Java/
    /// Kotlin) -- it only needs `is_true`'s presence check (above) to be correct so the
    /// assertion the fixture actually declares runs before this one, rather than always
    /// failing first regardless of whether `data` is present.
    #[test]
    fn equals_on_nested_field_through_optional_parent_is_unchanged() {
        let out = render_field_assertion(
            &resolver_with_optional_field("data"),
            &make_assertion("equals", Some("data.kind"), Some(serde_json::json!("KeyValue"))),
        );
        assert!(out.contains("result.data.kind"), "got: {out}");
    }

    #[test]
    fn is_true_on_non_optional_field_is_unchanged() {
        let out = render_field_assertion(&empty_resolver(), &make_assertion("is_true", Some("active"), None));
        assert_eq!(out, "    assert result.active is True  # noqa: S101\n");
    }

    #[cfg(test)]
    #[path = "wildcard_tests.rs"]
    mod wildcard_tests;

    #[test]
    fn not_empty_for_python_rejects_empty_sized_values_but_accepts_zero() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_empty", None, None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            &HashSet::new(),
            &HashMap::new(),
            false,
        );
        // Bare `assert result` fails on a legitimate 0, 0.0 or False.
        assert_eq!(
            out.trim(),
            "assert result is not None and (not hasattr(result, \"__len__\") or len(result) > 0)  # noqa: S101"
        );
    }

    /// Regression test for a one-sided-strip bug: `.strip()` was applied to the actual value
    /// while the fixture `expected` literal was emitted verbatim. Fixture expectations may
    /// legitimately end in `\n`, so stripping only one side made those assertions impossible
    /// to satisfy — and stripping both would silently mask real trailing-whitespace
    /// regressions. Equals is exact: neither side is normalized.
    /// Control for the trim fix: the tightened contract must still DISCRIMINATE values that
    /// differ only in trailing whitespace. If either side were normalized, the emitted
    /// assertion for "hello\n" and for "hello" would be identical and a real trailing-newline
    /// regression would pass unnoticed.
    #[test]
    fn render_assertion_equals_still_discriminates_trailing_whitespace() {
        let render_for = |value: &str| {
            let resolver = empty_resolver();
            let assertion = make_assertion("equals", None, Some(serde_json::Value::String(value.into())));
            let mut out = String::new();
            render_assertion(
                &mut out,
                &assertion,
                "result",
                &resolver,
                &HashSet::new(),
                &HashMap::new(),
                false,
            );
            out
        };
        let emitted = render_for("hello\n");
        // The actual side must be the bare expression: any normalizing call (trim/strip/
        // case-folding) wrapped around it would silently accept a mismatched value.
        assert_eq!(
            emitted, "    assert result == \"hello\\n\"  # noqa: S101\n",
            "emitted assertion drifted: {emitted}"
        );
        // And a value differing only by the trailing newline must still produce a
        // different expectation, proving trailing whitespace is discriminated.
        assert_ne!(
            emitted,
            render_for("hello"),
            "trailing newline must still change the emitted assertion"
        );
    }

    #[test]
    fn render_assertion_equals_string_compares_exactly_without_strip() {
        let resolver = empty_resolver();
        let assertion = make_assertion("equals", None, Some(serde_json::Value::String("hello\n".into())));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            &resolver,
            &HashSet::new(),
            &HashMap::new(),
            false,
        );
        assert!(
            !out.contains(".strip()"),
            "equals must not strip either side; got: {out}"
        );
        assert!(out.contains("assert result =="), "got: {out}");
    }

    #[test]
    fn render_assertion_contains_string_array_uses_item_texts() {
        let resolver = resolver_with_array_field("structure");
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
            &HashSet::new(),
            &HashMap::new(),
            false,
        );

        assert!(out.contains("_alef_e2e_item_texts(item)"), "got: {out}");
        assert!(out.contains("for item in result.structure"), "got: {out}");
    }

    #[test]
    fn build_python_method_call_root_child_count() {
        let expr = build_python_method_call("tree", "root_child_count", None);
        assert_eq!(expr, "tree.root_node().child_count()");
    }

    #[test]
    fn negate_contains_expr_simple_membership_not_in() {
        let expr = "\"test\" in result.content";
        let negated = negate_contains_expr(expr, false, false);
        assert_eq!(negated, "\"test\" not in result.content");
    }

    #[test]
    fn negate_contains_expr_array_uses_not_wrapper() {
        let expr = "any(\"test\" in text for item in result.structure for text in _alef_e2e_item_texts(item))";
        let negated = negate_contains_expr(expr, true, false);
        assert!(
            negated.contains("not ("),
            "expected `not (...)` wrapper for array expression"
        );
    }

    #[test]
    fn negate_contains_expr_enum_uses_not_wrapper() {
        let expr = "\"test\".lower() in str(result.status).lower()";
        let negated = negate_contains_expr(expr, false, true);
        assert!(
            negated.contains("not ("),
            "expected `not (...)` wrapper for enum expression"
        );
    }

    #[test]
    fn negate_contains_expr_preserves_already_negated() {
        let expr = "\"test\" not in result.content";
        let negated = negate_contains_expr(expr, false, false);
        // Should not double-negate: ` not in ` already present, so wrap with `not (...)`
        assert!(
            negated.contains("not ("),
            "expected `not (...)` wrapper for already-negated expression"
        );
    }

    #[test]
    #[should_panic(expected = "unsupported assertion type 'bogus_type' on synthetic field 'chunks_have_content'")]
    fn python_synthetic_chunks_unsupported_type_fails_loudly() {
        let assertion = make_assertion("bogus_type", Some("chunks_have_content"), None);
        let mut out = String::new();
        render_synthetic_field(&mut out, &assertion, "result", "chunks_have_content");
    }

    #[test]
    fn python_synthetic_chunks_supported_type_renders_assertion() {
        let assertion = make_assertion("is_true", Some("chunks_have_content"), None);
        let mut out = String::new();
        let handled = render_synthetic_field(&mut out, &assertion, "result", "chunks_have_content");
        assert!(handled);
        assert_eq!(
            out.trim(),
            "assert all(c.content for c in (result.chunks or []))  # noqa: S101"
        );
    }

    #[test]
    #[should_panic(expected = "unsupported assertion type 'bogus_type' on synthetic field 'embeddings'")]
    fn python_synthetic_embeddings_unsupported_type_fails_loudly() {
        let assertion = make_assertion("bogus_type", Some("embeddings"), None);
        let mut out = String::new();
        render_synthetic_field(&mut out, &assertion, "result", "embeddings");
    }

    #[test]
    fn python_synthetic_embeddings_supported_type_renders_assertion() {
        let assertion = make_assertion("not_empty", Some("embeddings"), None);
        let mut out = String::new();
        let handled = render_synthetic_field(&mut out, &assertion, "result", "embeddings");
        assert!(handled);
        assert_eq!(out.trim(), "assert len(result) > 0  # noqa: S101");
    }

    #[test]
    fn python_embedding_dimensions_unsupported_type_no_longer_emits_invalid_syntax() {
        let assertion = make_assertion("bogus_type", Some("embedding_dimensions"), None);
        let mut out = String::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render_synthetic_field(&mut out, &assertion, "result", "embedding_dimensions");
        }));
        assert!(result.is_err(), "expected a panic for unsupported assertion type");
        assert!(
            !out.contains("//"),
            "generated output must never contain a `//` (invalid Python comment token): {out}"
        );
    }

    #[test]
    fn python_embedding_dimensions_supported_type_renders_assertion() {
        let assertion = make_assertion(
            "greater_than",
            Some("embedding_dimensions"),
            Some(serde_json::Value::from(10)),
        );
        let mut out = String::new();
        let handled = render_synthetic_field(&mut out, &assertion, "result", "embedding_dimensions");
        assert!(handled);
        assert_eq!(
            out.trim(),
            "assert (len(result[0]) if result else 0) > 10  # noqa: S101"
        );
    }
}
