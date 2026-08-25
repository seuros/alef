//! Assertion rendering for Rust e2e tests.

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::field_skip::{FieldSkip, nested_wildcard_skip_line};
use crate::e2e::escape::escape_rust;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::assertion_helpers::{
    render_count_equals_assertion, render_count_min_assertion, render_equals_assertion, render_gte_assertion,
    render_is_empty_assertion, render_method_result_assertion, render_not_empty_assertion, wildcard_elem_is_enum,
};
use super::assertion_synthetic::{
    numeric_literal, render_chunks_have_content, render_chunks_have_embeddings, render_chunks_have_heading_context,
    render_embedding_dimensions, render_embedding_quality, render_embeddings_assertion,
    render_first_chunk_starts_with_heading, render_keywords_assertion, render_keywords_count_assertion,
    tree_field_access_expr, value_to_rust_string,
};

/// Returns `true` when the assertion's leaf field resolves to an `Option<T>` where
/// `T` is a scalar (i.e. not a collection). Used to decide whether numeric comparison
/// operators (`>`, `<`, `>=`, `<=`) need to unwrap the field before comparing — directly
/// comparing `Option<usize>` against a numeric literal is a type error.
fn is_optional_scalar_field(assertion: &Assertion, is_unwrapped: bool, field_resolver: &FieldResolver) -> bool {
    assertion.field.as_ref().is_some_and(|f| {
        let resolved = field_resolver.resolve(f);
        let is_opt = !is_unwrapped && field_resolver.is_optional(resolved);
        let is_arr = field_resolver.is_array(resolved);
        is_opt && !is_arr
    })
}

/// Render a single assertion into the test function body.
/// The boolean expression deciding whether `field_access` contains `expected`.
///
/// ~keep Every containment operator shares this, because a fixture author picks the operator
/// and the field independently: an enum field has no `contains` method of its own, and a
/// collection field's `contains` compares whole elements rather than performing the substring
/// search a fixture value expects. An operator that emits the plain form for those two field
/// kinds emits Rust that does not compile, which is invisible until the consumer builds its
/// generated tests.
///
/// ~keep The collection arm's semantics must match the five other e2e backends (Python, Node,
/// Ruby, Java, C#): a substring search over several item keys, not an exact match on `name`
/// alone. Fixture items commonly carry the searched text under `kind` (e.g. `{"kind":
/// "Function","name":"main"}` matched by `{"type":"contains","value":"Function"}`), so pinning
/// the check to `name` with `==` made every such fixture fail. The key list mirrors the
/// Python/Node/Ruby helpers (`kind`, `name`, `source`, `alias`, `text`, `signature`); the
/// whole-value JSON fallback mirrors Java's `.toString()` / C#'s `JsonSerializer.Serialize`
/// approach of searching the serialized item as a whole.
fn containment_predicate(field_access: &str, expected: &str, field_is_enum: bool, field_is_collection: bool) -> String {
    if field_is_enum {
        format!("format!(\"{{:?}}\", {field_access}).to_lowercase().contains(&{expected}.to_lowercase())")
    } else if field_is_collection {
        format!(
            "{field_access}.iter().any(|item| serde_json::to_value(item).ok().is_some_and(|value| match &value {{ serde_json::Value::String(text) => text.contains({expected}), serde_json::Value::Object(fields) => [\"kind\", \"name\", \"source\", \"alias\", \"text\", \"signature\"].iter().any(|key| fields.get(*key).and_then(serde_json::Value::as_str).is_some_and(|text| text.contains({expected}))) || value.to_string().contains({expected}), _ => false }}))"
        )
    } else {
        format!("{field_access}.contains({expected})")
    }
}

/// The failure text describing what [`containment_predicate`] looked for.
fn containment_message(field_is_enum: bool, field_is_collection: bool) -> &'static str {
    if !field_is_enum && field_is_collection {
        "expected collection item to contain"
    } else {
        "expected to contain"
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    module: &str,
    dep_name: &str,
    is_error_context: bool,
    unwrapped_fields: &[(String, String)], // (fixture_field, local_var)
    field_resolver: &FieldResolver,
    result_is_tree: bool,
    result_is_simple: bool,
    result_is_vec: bool,
    result_is_option: bool,
    returns_result: bool,
    streaming_item_type: Option<&str>,
) {
    render_assertion_with_streaming(
        out,
        assertion,
        result_var,
        module,
        dep_name,
        is_error_context,
        unwrapped_fields,
        field_resolver,
        result_is_tree,
        result_is_simple,
        result_is_vec,
        result_is_option,
        returns_result,
        streaming_item_type,
        false,
    )
}

/// Same as [`render_assertion`], but with an `is_streaming` flag so the streaming-virtual
/// field arm can fire when `result_var` is the raw call result rather than the collected
/// `chunks` variable.  Callers that already drained the stream into a `chunks: Vec<_>`
/// local should pass `is_streaming = true`.
#[allow(clippy::too_many_arguments)]
pub fn render_assertion_with_streaming(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    module: &str,
    dep_name: &str,
    is_error_context: bool,
    unwrapped_fields: &[(String, String)], // (fixture_field, local_var)
    field_resolver: &FieldResolver,
    result_is_tree: bool,
    result_is_simple: bool,
    result_is_vec: bool,
    result_is_option: bool,
    returns_result: bool,
    streaming_item_type: Option<&str>,
    _is_streaming: bool,
) {
    // Vec<T> result: iterate per-element so each assertion checks every element.
    // Field-path assertions become `for r in &{result} { <assert using r> }`.
    // Length-style assertions on the Vec itself (no field path) operate on the
    // Vec directly.
    let has_field = assertion.field.as_ref().is_some_and(|f| !f.is_empty());
    if result_is_vec && has_field && !is_error_context {
        let _ = writeln!(out, "    for r in &{result_var} {{");
        render_assertion(
            out,
            assertion,
            "r",
            module,
            dep_name,
            is_error_context,
            unwrapped_fields,
            field_resolver,
            result_is_tree,
            result_is_simple,
            false, // already inside loop
            result_is_option,
            returns_result,
            streaming_item_type,
        );
        let _ = writeln!(out, "    }}");
        return;
    }
    // Option<T> result: map `is_empty`/`not_empty` to `is_none()`/`is_some()`,
    // and unwrap the inner value before any other assertion runs.
    if result_is_option && !is_error_context {
        let assertion_type = assertion.assertion_type.as_str();
        if !has_field && (assertion_type == "is_empty" || assertion_type == "not_empty") {
            let check = if assertion_type == "is_empty" {
                "is_none"
            } else {
                "is_some"
            };
            let _ = writeln!(
                out,
                "    assert!({result_var}.{check}(), \"expected Option to be {check}\");"
            );
            return;
        }
        // For any other assertion shape, unwrap the Option and recurse with a
        // bare reference variable so the rest of the renderer treats the inner
        // value as the result.
        let _ = writeln!(
            out,
            "    let r = {result_var}.as_ref().expect(\"Option<T> should be Some\");"
        );
        render_assertion(
            out,
            assertion,
            "r",
            module,
            dep_name,
            is_error_context,
            unwrapped_fields,
            field_resolver,
            result_is_tree,
            result_is_simple,
            result_is_vec,
            false, // already unwrapped
            returns_result,
            streaming_item_type,
        );
        return;
    }
    // Handle synthetic fields like chunks_have_content (derived assertions).
    // These are computed expressions, not real struct fields — intercept before
    // the is_valid_for_result check so they are never treated as field accesses.
    if let Some(f) = &assertion.field {
        match f.as_str() {
            "chunks_have_content"
            | "chunks_have_embeddings"
            | "chunks_have_heading_context"
            | "first_chunk_starts_with_heading"
                if !crate::e2e::codegen::assertion_recipes::chunks_field_declared_by_result(field_resolver) =>
            {
                let _ = writeln!(
                    out,
                    "    // skipped: {}",
                    FieldSkip::NotAvailableOnResultType.message(f)
                );
                return;
            }
            "chunks_have_content" => {
                render_chunks_have_content(out, result_var, assertion.assertion_type.as_str());
                return;
            }
            "chunks_have_embeddings" => {
                render_chunks_have_embeddings(out, result_var, assertion.assertion_type.as_str());
                return;
            }
            "chunks_have_heading_context" => {
                render_chunks_have_heading_context(out, result_var, assertion.assertion_type.as_str());
                return;
            }
            "first_chunk_starts_with_heading" => {
                render_first_chunk_starts_with_heading(out, result_var, assertion.assertion_type.as_str());
                return;
            }
            "embeddings" => {
                render_embeddings_assertion(out, result_var, assertion);
                return;
            }
            "embedding_dimensions" => {
                render_embedding_dimensions(out, result_var, assertion);
                return;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                render_embedding_quality(out, result_var, f, assertion.assertion_type.as_str());
                return;
            }
            "keywords" => {
                render_keywords_assertion(out, result_var, assertion);
                return;
            }
            "keywords_count" => {
                render_keywords_count_assertion(out, result_var, assertion);
                return;
            }
            _ => {}
        }
    }

    // Streaming virtual fields: intercept before is_valid_for_result so they are
    // never skipped.  These fields resolve against the `chunks` collected-list variable.
    //
    // For streaming fixtures, `chunks` is bound by the collect snippet emitted in
    // `render_test_function`.  For non-streaming fixtures whose result struct has a
    // literal field whose name collides with a streaming-virtual name (e.g. `chunks`,
    // `imports`, `structure`), `render_test_function` emits `let {f} = &result.{f};`
    // before assertions, so the hardcoded `chunks` identifier used below still resolves.
    if let Some(f) = &assertion.field
        && !f.is_empty()
        && crate::e2e::codegen::streaming_assertions::is_streaming_virtual_field(f)
    {
        if let Some(expr) =
            crate::e2e::codegen::streaming_assertions::StreamingFieldResolver::accessor_with_streaming_context(
                f,
                "rust",
                "chunks",
                Some(dep_name),
                streaming_item_type,
            )
        {
            match assertion.assertion_type.as_str() {
                "count_min" => {
                    if let Some(val) = &assertion.value
                        && let Some(n) = val.as_u64()
                    {
                        let expr_for_len = if field_resolver.is_optional(f) {
                            format!("{expr}.as_ref().map_or(0, |v| v.len())")
                        } else {
                            format!("{expr}.len()")
                        };
                        let _ = writeln!(
                            out,
                            "    assert!({expr_for_len} >= {n} as usize, \"expected >= {n} chunks\");"
                        );
                    } else {
                        panic!(
                            "Rust e2e generator: streaming field '{f}' assertion 'count_min' requires a numeric value in the fixture, got {:?}",
                            assertion.value
                        );
                    }
                }
                "count_equals" => {
                    if let Some(val) = &assertion.value
                        && let Some(n) = val.as_u64()
                    {
                        let expr_for_len = if field_resolver.is_optional(f) {
                            format!("{expr}.as_ref().map_or(0, |v| v.len())")
                        } else {
                            format!("{expr}.len()")
                        };
                        let _ = writeln!(
                            out,
                            "    assert_eq!({expr_for_len}, {n} as usize, \"expected exactly {n} chunks\");"
                        );
                    } else {
                        panic!(
                            "Rust e2e generator: streaming field '{f}' assertion 'count_equals' requires a numeric value in the fixture, got {:?}",
                            assertion.value
                        );
                    }
                }
                "equals" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = crate::e2e::escape::escape_rust(s);
                        let _ = writeln!(out, "    assert_eq!({expr}, \"{escaped}\");");
                    } else if let Some(val) = &assertion.value {
                        let lit = super::assertion_synthetic::numeric_literal(val);
                        let _ = writeln!(out, "    assert_eq!({expr}, {lit});");
                    } else {
                        panic!(
                            "Rust e2e generator: streaming field '{f}' assertion 'equals' requires a string or numeric value in the fixture, got {:?}",
                            assertion.value
                        );
                    }
                }
                "not_empty" => {
                    let check_expr = if field_resolver.is_optional(f) {
                        format!("{expr}.as_ref().is_some_and(|v| !v.is_empty())")
                    } else {
                        format!("!{expr}.is_empty()")
                    };
                    let _ = writeln!(out, "    assert!({check_expr}, \"expected non-empty\");");
                }
                "is_empty" => {
                    let check_expr = if field_resolver.is_optional(f) {
                        format!("{expr}.as_ref().is_none_or(|v| v.is_empty())")
                    } else {
                        format!("{expr}.is_empty()")
                    };
                    let _ = writeln!(out, "    assert!({check_expr}, \"expected empty\");");
                }
                "is_true" => {
                    let _ = writeln!(out, "    assert!({expr}, \"expected true\");");
                }
                "is_false" => {
                    let _ = writeln!(out, "    assert!(!{expr}, \"expected false\");");
                }
                "greater_than" => {
                    if let Some(val) = &assertion.value {
                        let lit = super::assertion_synthetic::numeric_literal(val);
                        let _ = writeln!(out, "    assert!({expr} > {lit}, \"expected > {lit}\");");
                    } else {
                        panic!(
                            "Rust e2e generator: streaming field '{f}' assertion 'greater_than' requires a numeric value in the fixture, got {:?}",
                            assertion.value
                        );
                    }
                }
                "greater_than_or_equal" => {
                    if let Some(val) = &assertion.value {
                        let lit = super::assertion_synthetic::numeric_literal(val);
                        let _ = writeln!(out, "    assert!({expr} >= {lit}, \"expected >= {lit}\");");
                    } else {
                        panic!(
                            "Rust e2e generator: streaming field '{f}' assertion 'greater_than_or_equal' requires a numeric value in the fixture, got {:?}",
                            assertion.value
                        );
                    }
                }
                "contains" => {
                    if let Some(serde_json::Value::String(s)) = &assertion.value {
                        let escaped = crate::e2e::escape::escape_rust(s);
                        let _ = writeln!(
                            out,
                            "    assert!({expr}.contains(\"{escaped}\"), \"expected to contain: {escaped}\");"
                        );
                    } else {
                        panic!(
                            "Rust e2e generator: streaming field '{f}' assertion 'contains' requires a string value in the fixture, got {:?}",
                            assertion.value
                        );
                    }
                }
                other => {
                    panic!("Rust e2e generator: unsupported assertion type '{other}' on streaming field '{f}'");
                }
            }
        } else {
            panic!(
                "Rust e2e generator: streaming field '{f}' has no accessor for context (streaming_item_type={streaming_item_type:?}); check the streaming adapter configuration"
            );
        }
        return;
    }

    // Skip assertions on fields that don't exist on the result type.
    // Exception: fields prefixed with "error." target the error value in error-context
    // assertions — they are resolved against the error type via accessor_for_error,
    // not against the success result type, so they must not be skipped here.
    // However, when NOT in error context (i.e. the call site uses .expect() and binds
    // the Ok value), there is no Err to inspect — skip error.* assertions with a comment.
    if let Some(f) = &assertion.field
        && !f.is_empty()
    {
        if f.starts_with("error.") && !is_error_context {
            let _ = writeln!(
                out,
                "    // skipped: {}",
                FieldSkip::NotAvailableOnResultType.message(f)
            );
            return;
        }
        // When result_is_simple the function returns a plain scalar/string type —
        // `field_access` uses `effective_result_var` directly regardless of the
        // field name, so the skip guard must not fire for these calls.
        if !f.starts_with("error.") && !result_is_simple && !field_resolver.is_valid_for_result(f) {
            let _ = writeln!(
                out,
                "    // skipped: {}",
                FieldSkip::NotAvailableOnResultType.message(f)
            );
            return;
        }
    }

    // Check if this field was unwrapped (i.e., it is optional and was bound to a local).
    let is_unwrapped = assertion
        .field
        .as_ref()
        .is_some_and(|f| unwrapped_fields.iter().any(|(ff, _)| ff == f));

    // When in error context with returns_result=true and accessing a field (not an error check),
    // we need to unwrap the Result first. The test generator creates a binding like
    // `let result_ok = result.as_ref().ok();` which we can dereference here.
    // Exception: fields prefixed with "error." access the Err value, not the Ok value.
    let has_field = assertion.field.as_ref().is_some_and(|f| !f.is_empty());
    let is_field_assertion = !matches!(assertion.assertion_type.as_str(), "error" | "not_error");
    let is_error_field = assertion.field.as_ref().is_some_and(|f| f.starts_with("error."));
    let effective_result_var =
        if has_field && is_error_context && returns_result && is_field_assertion && !is_error_field {
            // Dereference the Option<&T> bound as {result_var}_ok
            format!("{result_var}_ok.as_ref().unwrap()")
        } else {
            result_var.to_string()
        };

    // A `foo[].bar` fixture path means EVERY element of `foo`, not element 0. The shared
    // accessor lowers `[]` to `[0]`, so the wildcard has to be expanded here into an
    // `.iter().any(..)` predicate before the accessor is ever built. Deliberately not
    // applied to error.*, Tree, simple-result or already-unwrapped fields: those arms
    // below build their expression by a different route and the wildcard shape does not
    // compose with them. ~keep
    if let Some(f) = assertion.field.as_deref()
        && !f.is_empty()
        && !f.starts_with("error.")
        && !result_is_simple
        && !result_is_tree
        && !is_unwrapped
        && f != result_var
        && let Some((array_part, elem_part)) = field_resolver.wildcard_split(f)
    {
        render_rust_wildcard_assertion(
            out,
            assertion,
            f,
            &array_part,
            &elem_part,
            &effective_result_var,
            field_resolver,
        );
        return;
    }

    // Determine field access expression:
    // 1. If the field was unwrapped to a local var, use that local var name.
    // 2. When result_is_simple, the function returns a plain type (String etc.) — use result_var.
    // 3. When the field path is exactly the result var name (sentinel: `field: "result"`),
    //    refer to the result variable directly to avoid emitting `result.result`.
    // 4. When the result is a Tree, map pseudo-field names to correct Rust expressions.
    // 5. When the field starts with "error.", resolve against the error type.
    // 6. Otherwise, use the field resolver to generate the accessor.
    let field_access = match &assertion.field {
        Some(f) if !f.is_empty() => {
            if let Some((_, local_var)) = unwrapped_fields.iter().find(|(ff, _)| ff == f) {
                local_var.clone()
            } else if result_is_simple && !f.starts_with("error.") {
                // Plain return type (String, Vec<T>, etc.) has no struct fields.
                // Use the result variable directly so assertions operate on the value itself.
                // Exception: error.* fields must resolve against the Err value, not the
                // plain result variable, even when the success type is simple (e.g. Bytes).
                effective_result_var.clone()
            } else if f == result_var {
                // Sentinel: fixture uses `field: "result"` (or matches the result variable name)
                // to refer to the whole return value, not a struct field named "result".
                effective_result_var.clone()
            } else if result_is_tree {
                // Tree is an opaque type — its "fields" are accessed via root_node() or
                // free functions. Map known pseudo-field names to correct Rust expressions.
                tree_field_access_expr(f, &effective_result_var, module)
            } else if let Some(sub) = f.strip_prefix("error.") {
                // Error-path field: access a field on the Err value rather than the Ok value.
                // Inline-bind the error so the expression is self-contained.
                let err_accessor = field_resolver.accessor_for_error(sub, "rust", "__err");
                format!("{{ let __err = {result_var}.as_ref().err().unwrap(); {err_accessor} }}")
            } else {
                field_resolver.accessor(f, "rust", &effective_result_var)
            }
        }
        _ => effective_result_var,
    };
    let field_is_enum = assertion
        .field
        .as_deref()
        .is_some_and(|field| field_resolver.is_enum(field));
    let field_is_collection = assertion
        .field
        .as_deref()
        .is_some_and(|field| field_resolver.is_array(field) || field_resolver.is_collection_root(field))
        || result_is_vec;

    match assertion.assertion_type.as_str() {
        "error" => {
            let _ = writeln!(out, "    assert!({result_var}.is_err(), \"expected call to fail\");");
            if let Some(serde_json::Value::String(msg)) = &assertion.value {
                let escaped = escape_rust(msg);
                // Match against the Debug format (variant-name-style) and the Display format
                // (human-readable text). Fixtures often name the error variant ("BadRequest"),
                // but Display impls typically lowercase with a colon ("bad request: ..."), so
                // checking both lets either kind of fixture value match.
                let _ = writeln!(
                    out,
                    "    {{ let __e = {result_var}.as_ref().err().unwrap(); assert!(format!(\"{{:?}}\", __e).contains(\"{escaped}\") || __e.to_string().contains(\"{escaped}\"), \"error message mismatch\"); }}"
                );
            }
        }
        "not_error" => {
            // Handled at call site; nothing extra needed here.
        }
        "equals" => {
            render_equals_assertion(out, assertion, &field_access, is_unwrapped, field_resolver);
        }
        "contains" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                let predicate = containment_predicate(&field_access, &expected, field_is_enum, field_is_collection);
                let message = containment_message(field_is_enum, field_is_collection);
                let _ = writeln!(out, "    assert!({predicate}, \"{message}: {{}}\", {expected});");
            }
        }
        "contains_all" => {
            if let Some(values) = &assertion.values {
                for val in values {
                    let expected = value_to_rust_string(val);
                    let predicate = containment_predicate(&field_access, &expected, field_is_enum, field_is_collection);
                    let message = containment_message(field_is_enum, field_is_collection);
                    let _ = writeln!(out, "    assert!({predicate}, \"{message}: {{}}\", {expected});");
                }
            }
        }
        "not_contains" => {
            for val in assertion.expected_values() {
                let expected = value_to_rust_string(val);
                let predicate = containment_predicate(&field_access, &expected, field_is_enum, field_is_collection);
                let _ = writeln!(
                    out,
                    "    assert!(!{predicate}, \"expected NOT to contain: {{}}\", {expected});"
                );
            }
        }
        "not_empty" => {
            render_not_empty_assertion(
                out,
                assertion,
                &field_access,
                result_var,
                result_is_option,
                is_unwrapped,
                field_resolver,
            );
        }
        "is_empty" => {
            render_is_empty_assertion(out, assertion, &field_access, is_unwrapped, field_resolver);
        }
        "contains_any" => {
            if let Some(values) = &assertion.values {
                let checks: Vec<String> = values
                    .iter()
                    .map(|v| {
                        let expected = value_to_rust_string(v);
                        containment_predicate(&field_access, &expected, field_is_enum, field_is_collection)
                    })
                    .collect();
                let joined = checks.join(" || ");
                let _ = writeln!(
                    out,
                    "    assert!({joined}, \"expected to contain at least one of the specified values\");"
                );
            }
        }
        "greater_than" => {
            if let Some(val) = &assertion.value {
                // Skip comparisons with negative values against unsigned types (.len() etc.)
                if val.as_f64().is_some_and(|n| n < 0.0) {
                    let _ = writeln!(
                        out,
                        "    // skipped: greater_than with negative value is always true for unsigned types"
                    );
                } else if val.as_u64() == Some(0) {
                    if field_access.ends_with(".len()") {
                        // Clippy prefers !is_empty() over len() > 0 for collections.
                        let base = field_access.strip_suffix(".len()").unwrap();
                        let _ = writeln!(out, "    assert!(!{base}.is_empty(), \"expected > 0\");");
                    } else if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                        // Use 0 for integer comparisons (the common case for > 0).
                        let _ = writeln!(out, "    assert!({field_access}.unwrap_or(0) > 0, \"expected > 0\");");
                    } else {
                        // Scalar types (usize, u64, etc.) — use direct comparison.
                        let _ = writeln!(out, "    assert!({field_access} > 0, \"expected > 0\");");
                    }
                } else {
                    let lit = numeric_literal(val);
                    if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                        // Option<usize>/Option<u64>/Option<f64>: unwrap with appropriate zero literal
                        // before comparing so the assertion fails (rather than fails to compile) on a missing field.
                        let default_literal = if lit.contains("_f64") || lit.contains('.') {
                            "0.0"
                        } else {
                            "0"
                        };
                        let _ = writeln!(
                            out,
                            "    assert!({field_access}.unwrap_or({default_literal}) > {lit}, \"expected > {lit}\");"
                        );
                    } else {
                        let _ = writeln!(out, "    assert!({field_access} > {lit}, \"expected > {lit}\");");
                    }
                }
            }
        }
        "less_than" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                    // Option<usize>/Option<u64>/Option<f64>: unwrap with appropriate zero literal
                    // before comparing. Note this means a missing field will satisfy `< N` for any positive N,
                    // matching the convention used by render_gte_assertion.
                    let default_literal = if lit.contains("_f64") || lit.contains('.') {
                        "0.0"
                    } else {
                        "0"
                    };
                    let _ = writeln!(
                        out,
                        "    assert!({field_access}.unwrap_or({default_literal}) < {lit}, \"expected < {lit}\");"
                    );
                } else {
                    let _ = writeln!(out, "    assert!({field_access} < {lit}, \"expected < {lit}\");");
                }
            }
        }
        "greater_than_or_equal" => {
            render_gte_assertion(out, assertion, &field_access, is_unwrapped, field_resolver);
        }
        "less_than_or_equal" => {
            if let Some(val) = &assertion.value {
                let lit = numeric_literal(val);
                if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                    // Option<usize>/Option<u64>/Option<f64>: unwrap with appropriate zero literal.
                    let default_literal = if lit.contains("_f64") || lit.contains('.') {
                        "0.0"
                    } else {
                        "0"
                    };
                    let _ = writeln!(
                        out,
                        "    assert!({field_access}.unwrap_or({default_literal}) <= {lit}, \"expected <= {lit}\");"
                    );
                } else {
                    let _ = writeln!(out, "    assert!({field_access} <= {lit}, \"expected <= {lit}\");");
                }
            }
        }
        "starts_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                let _ = writeln!(
                    out,
                    "    assert!({field_access}.starts_with({expected}), \"expected to start with: {{}}\", {expected});"
                );
            }
        }
        "ends_with" => {
            if let Some(val) = &assertion.value {
                let expected = value_to_rust_string(val);
                let _ = writeln!(
                    out,
                    "    assert!({field_access}.ends_with({expected}), \"expected to end with: {{}}\", {expected});"
                );
            }
        }
        "min_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                if n == 1 {
                    // Clippy prefers !is_empty() over len() >= 1 for collections.
                    let _ = writeln!(
                        out,
                        "    assert!(!{field_access}.is_empty(), \"expected length >= 1, got {{}}\", {field_access}.len());"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    assert!({field_access}.len() >= {n}, \"expected length >= {n}, got {{}}\", {field_access}.len());"
                    );
                }
            }
        }
        "max_length" => {
            if let Some(val) = &assertion.value
                && let Some(n) = val.as_u64()
            {
                let _ = writeln!(
                    out,
                    "    assert!({field_access}.len() <= {n}, \"expected length <= {n}, got {{}}\", {field_access}.len());"
                );
            }
        }
        "count_min" => {
            render_count_min_assertion(out, assertion, &field_access, is_unwrapped, field_resolver);
        }
        "count_equals" => {
            render_count_equals_assertion(out, assertion, &field_access, is_unwrapped, field_resolver);
        }
        "is_true" => {
            if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                // Option<T>: "is_true" semantically means "present and truthy".
                // For `Option<bool>` that's `Some(true)`; for `Option<serde_json::Value>`
                // (e.g. interact action_results[0].data) it's "Some and not null/false".
                // `is_some()` is the broadest correct interpretation that compiles for any T.
                let _ = writeln!(out, "    assert!({field_access}.is_some(), \"expected true (Some)\");");
            } else {
                let _ = writeln!(out, "    assert!({field_access}, \"expected true\");");
            }
        }
        "is_false" => {
            if is_optional_scalar_field(assertion, is_unwrapped, field_resolver) {
                // Option<T>: "is_false" semantically means "absent or falsy" — `.is_none()`
                // is the safe interpretation that compiles uniformly.
                let _ = writeln!(out, "    assert!({field_access}.is_none(), \"expected false (None)\");");
            } else {
                let _ = writeln!(out, "    assert!(!{field_access}, \"expected false\");");
            }
        }
        "method_result" => {
            render_method_result_assertion(out, assertion, &field_access, result_is_tree, module);
        }
        other => {
            panic!("Rust e2e generator: unsupported assertion type: {other}");
        }
    }
}

/// Build the `.iter().any(|e| ..)` wrapper for a wildcard (`foo[].bar`) path.
///
/// `element_predicate` is the body of the closure, written against the closure
/// parameter `e` (a `&T`, so it must not be re-borrowed).
fn rust_wildcard_any(array_accessor: &str, array_is_optional: bool, element_predicate: &str) -> String {
    if array_is_optional {
        format!("{array_accessor}.as_ref().is_some_and(|v| v.iter().any(|e| {element_predicate}))")
    } else {
        format!("{array_accessor}.iter().any(|e| {element_predicate})")
    }
}

fn render_rust_wildcard_assertion(
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
    if let Some(line) = nested_wildcard_skip_line("    ", "//", field, elem_part) {
        let _ = writeln!(out, "{line}");
        return;
    }
    let array_accessor = if array_part.is_empty() {
        result_var.to_string()
    } else {
        field_resolver.accessor(array_part, "rust", result_var)
    };
    // Passing the closure parameter as the "result var" is what makes nested element
    // sub-paths (`links[].meta.kind`) resolve against the loop variable. ~keep
    let elem_accessor = if elem_part.is_empty() {
        "e".to_string()
    } else {
        field_resolver.accessor(elem_part, "rust", "e")
    };
    let array_is_optional = !array_part.is_empty() && field_resolver.is_optional(array_part);
    let escaped_field = escape_rust(field);
    // Enum-typed elements are not guaranteed to implement `Display` — only `Debug` is a
    // safe assumption (the non-wildcard containment predicate already relies on it below).
    // `{elem_accessor}.to_string()` would fail to compile for an enum that only derives
    // `Debug`, so stringify via Debug instead whenever the traversed leaf is an enum. ~keep
    let elem_is_enum = wildcard_elem_is_enum(field_resolver, elem_part, field);
    let elem_stringified = if elem_is_enum {
        format!("format!(\"{{:?}}\", {elem_accessor})")
    } else {
        format!("{elem_accessor}.to_string()")
    };

    match assertion.assertion_type.as_str() {
        "contains" | "contains_all" | "not_contains" => {
            let negate = assertion.assertion_type == "not_contains";
            let values: Vec<&serde_json::Value> = if assertion.assertion_type == "contains" {
                assertion.value.iter().collect()
            } else {
                assertion.expected_values()
            };
            for val in values {
                let expected = value_to_rust_string(val);
                // `str::contains` needs a string pattern; non-string fixture values
                // (numbers, bools) have to be stringified first. ~keep
                let pattern = if val.is_string() {
                    expected.clone()
                } else {
                    format!("&{expected}.to_string()")
                };
                let predicate = rust_wildcard_any(
                    &array_accessor,
                    array_is_optional,
                    &format!("{elem_stringified}.contains({pattern})"),
                );
                if negate {
                    let _ = writeln!(
                        out,
                        "    assert!(!{predicate}, \"expected no element of {escaped_field} to contain: {{}}\", {expected});"
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "    assert!({predicate}, \"expected some element of {escaped_field} to contain: {{}}\", {expected});"
                    );
                }
            }
        }
        "not_empty" => {
            let predicate = rust_wildcard_any(
                &array_accessor,
                array_is_optional,
                &format!("!{elem_stringified}.is_empty()"),
            );
            let _ = writeln!(
                out,
                "    assert!({predicate}, \"expected some element of {escaped_field} to be non-empty\");"
            );
        }
        other => {
            let _ = writeln!(
                out,
                "    // skipped: unsupported traversal assertion '{other}' on '{field}'"
            );
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

    fn make_assertion(assertion_type: &str, field: Option<&str>, value: Option<serde_json::Value>) -> Assertion {
        Assertion {
            assertion_type: assertion_type.to_string(),
            field: field.map(|s| s.to_string()),
            value,
            ..Default::default()
        }
    }

    fn array_resolver(array_field: &str) -> FieldResolver {
        FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::from([array_field.to_string()]),
            &HashSet::new(),
        )
    }

    fn render_field_contains(resolver: &FieldResolver, field: &str, value: &str) -> String {
        let assertion = make_assertion("contains", Some(field), Some(serde_json::json!(value)));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        out
    }

    #[test]
    fn rust_wildcard_contains_iterates_every_element() {
        let out = render_field_contains(&array_resolver("links"), "links[].link_type", "external");
        assert!(out.contains("result.links.iter().any(|e|"), "got: {out}");
        assert!(out.contains("e.link_type.to_string().contains("), "got: {out}");
        assert!(!out.contains("[0]"), "wildcard must not pin element 0, got: {out}");
    }

    /// Regression for the same `.to_string()`-on-enum defect as the `equals` path, but for
    /// wildcard array-element traversal (`links[].link_type`): the element field's Rust
    /// type may only derive `Debug`, not `Display`, so `e.link_type.to_string()` fails to
    /// compile. The predicate must stringify via `format!("{:?}", ...)` for an enum-typed
    /// traversal leaf, while a non-enum leaf keeps the pre-existing `.to_string()` form. ~keep
    #[test]
    fn rust_wildcard_contains_uses_debug_for_enum_element_field() {
        let resolver = array_resolver("links").with_enum_fields(HashSet::from(["link_type".to_string()]));
        let out = render_field_contains(&resolver, "links[].link_type", "external");
        assert!(out.contains("result.links.iter().any(|e|"), "got: {out}");
        assert!(
            out.contains("format!(\"{:?}\", e.link_type).contains("),
            "enum traversal leaf must stringify via Debug, got: {out}"
        );
        assert!(
            !out.contains("e.link_type.to_string()"),
            "enum traversal leaf must NOT use to_string(), got: {out}"
        );
    }

    #[test]
    fn rust_explicit_index_still_pins_element_zero() {
        let out = render_field_contains(&array_resolver("links"), "links[0].link_type", "external");
        assert!(out.contains("result.links[0].link_type"), "got: {out}");
        assert!(
            !out.contains(".iter().any("),
            "explicit index must not become a traversal, got: {out}"
        );
    }

    /// Canary for the wildcard defect. `links[].link_type` lowered to `links[0]`, so a
    /// fixture whose match lives in element 1 asserted against element 0 and passed by
    /// accident. This test pins the only property observable at codegen level that
    /// distinguishes the two: the emitted predicate must quantify over the whole array
    /// rather than name a single index. Against the pre-fix generator the emitted text is
    /// `result.links[0].link_type` and every assertion below fails. ~keep
    #[test]
    fn rust_wildcard_match_in_second_element_is_not_missed() {
        let out = render_field_contains(&array_resolver("links"), "links[].link_type", "canonical");
        assert!(out.contains(".iter().any("), "got: {out}");
        assert!(!out.contains("links[0]"), "got: {out}");
        assert!(!out.contains("links[1]"), "predicate must be index-free, got: {out}");
    }

    /// `wildcard_split` consumes the first `[].` only, so before the guard the `.iter().any()`
    /// ranged over `pages` while its closure read `e.links[0].url` — a whole-array claim that
    /// only ever inspected element zero of the inner vector. Pre-guard this test fails: the
    /// skip line is absent and `links[0]` is present. ~keep
    #[test]
    fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
        let out = render_field_contains(&array_resolver("pages"), "pages[].links[].url", "example.test");
        assert_eq!(
            out, "    // skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
            "got: {out}"
        );
    }

    #[test]
    fn rust_wildcard_optional_array_guards_with_is_some_and() {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::from(["links".to_string()]),
            &HashSet::new(),
            &HashSet::from(["links".to_string()]),
            &HashSet::new(),
        );
        let out = render_field_contains(&resolver, "links[].link_type", "external");
        assert!(out.contains(".as_ref().is_some_and(|v| v.iter().any(|e|"), "got: {out}");
    }

    /// IR-oracle wiring regression (alef task #64): a field that is IR-reachable
    /// (present, non-`binding_excluded`, on some IR type) but missing from the
    /// hand-maintained `result_fields` config must still render a real assertion,
    /// not a "skipped: field not available" comment — `rust/test_file/test_function.rs`
    /// now threads `FieldResolver::ir_field_sets(type_defs)` into `with_ir_fields`. ~keep
    #[test]
    fn rust_ir_reachable_field_absent_from_result_fields_is_not_skipped() {
        let reachable: HashSet<String> = ["data".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .with_ir_fields(reachable, HashSet::new(), HashSet::new());
        let assertion = make_assertion("equals", Some("data"), Some(serde_json::json!("hello")));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
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
    fn rust_ir_excluded_field_present_in_result_fields_is_still_skipped() {
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
        let assertion = make_assertion("equals", Some("internal_diagnostics"), Some(serde_json::json!("hello")));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(out.contains("skipped"), "got: {out}");
    }

    #[test]
    fn render_assertion_error_type_emits_is_err_check() {
        let resolver = empty_resolver();
        let assertion = make_assertion("error", None, None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            true,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(out.contains("is_err()"), "got: {out}");
    }

    #[test]
    fn render_contains_assertion_uses_raw_string_content() {
        let resolver = empty_resolver();
        let assertion = make_assertion("contains", None, Some(serde_json::json!("line\n\"quoted\"\\path")));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "sample",
            "sample",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(out.contains("result.contains("), "got: {out}");
        assert!(!out.contains("format!(\"{{:?}}\""), "got: {out}");
    }

    #[test]
    fn render_not_contains_emits_each_plural_value() {
        let resolver = empty_resolver();
        let assertion = Assertion {
            assertion_type: "not_contains".into(),
            field: Some("content".into()),
            values: Some(vec![
                serde_json::json!("unsafe markup"),
                serde_json::json!("unsafe handler"),
            ]),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "sample",
            "sample",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );

        assert!(out.contains("unsafe markup"), "got: {out}");
        assert!(out.contains("unsafe handler"), "got: {out}");
        assert_eq!(out.matches("assert!(!").count(), 2, "got: {out}");
    }

    #[test]
    fn render_assertion_vec_result_wraps_in_for_loop() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_empty", Some("content"), None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            true,
            false,
            false,
            None,
        );
        assert!(out.contains("for r in"), "got: {out}");
    }

    #[test]
    fn render_assertion_not_empty_bare_result_uses_is_empty() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_empty", None, None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(out.contains("is_empty()"), "got: {out}");
    }

    #[test]
    fn render_assertion_min_length_one_uses_is_empty_not_len_ge_one() {
        let resolver = empty_resolver();
        let assertion = make_assertion("min_length", Some("content"), Some(serde_json::Value::from(1u64)));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(
            out.contains("is_empty()"),
            "min_length 1 should use !is_empty(); got: {out}"
        );
        assert!(
            !out.contains("len() >= 1"),
            "min_length 1 must not emit len() >= 1 (clippy::len_zero); got: {out}"
        );
    }

    #[test]
    fn render_assertion_min_length_two_still_uses_len_ge() {
        let resolver = empty_resolver();
        let assertion = make_assertion("min_length", Some("content"), Some(serde_json::Value::from(2u64)));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(
            out.contains("len() >= 2"),
            "min_length 2 should emit len() >= 2; got: {out}"
        );
    }

    #[test]
    fn contains_uses_declared_collection_and_enum_types() {
        let result_fields = HashSet::from(["cookies".to_string(), "link_type".to_string()]);
        let array_fields = HashSet::from(["cookies".to_string()]);
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &array_fields,
            &HashSet::new(),
        )
        .with_enum_fields(HashSet::from(["link_type".to_string()]));
        let mut assertions = String::new();
        for (field, expected) in [("cookies", "domain_cookie"), ("link_type", "anchor")] {
            render_assertion(
                &mut assertions,
                &make_assertion("contains", Some(field), Some(serde_json::json!(expected))),
                "result",
                "sample",
                "sample",
                false,
                &[],
                &resolver,
                false,
                false,
                false,
                false,
                false,
                None,
            );
        }
        assert_eq!(assertions.matches("format!(\"{:?}\"").count(), 1, "got: {assertions}");
        assert!(assertions.contains("result.cookies.iter().any"), "got: {assertions}");
        assert!(assertions.contains("fields.get(*key)"), "got: {assertions}");
        assert!(assertions.contains("\"name\""), "got: {assertions}");
    }

    #[test]
    fn contains_uses_the_effective_result_type() {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::from(["items".to_string()]),
            &HashSet::new(),
        );
        let assertion = make_assertion("contains", Some("items"), Some(serde_json::json!("needle")));

        let mut scalar = String::new();
        render_assertion(
            &mut scalar,
            &assertion,
            "result",
            "sample",
            "sample",
            false,
            &[],
            &resolver,
            false,
            true,
            false,
            false,
            false,
            None,
        );
        assert!(!scalar.contains("format!(\"{:?}\""), "got: {scalar}");

        let mut vector = String::new();
        render_assertion(
            &mut vector,
            &make_assertion("contains", None, Some(serde_json::json!("needle"))),
            "result",
            "sample",
            "sample",
            false,
            &[],
            &resolver,
            false,
            true,
            true,
            false,
            false,
            None,
        );
        assert!(vector.contains("result.iter().any"), "got: {vector}");

        assert!(!vector.contains("format!(\"{:?}\""), "got: {vector}");
    }

    /// The four operators that mean "contains".
    const CONTAINMENT_OPERATORS: [&str; 4] = ["contains", "contains_all", "not_contains", "contains_any"];

    /// Render one containment operator, supplying the value under whichever key it reads.
    fn render_containment(operator: &str, field: &str, expected: &str, resolver: &FieldResolver) -> String {
        let assertion = Assertion {
            assertion_type: operator.to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::json!(expected)),
            values: Some(vec![serde_json::json!(expected)]),
            ..Default::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "sample",
            "sample",
            false,
            &[],
            resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        out
    }

    #[test]
    fn every_containment_operator_uses_the_enum_predicate_on_an_enum_field() {
        let resolver = empty_resolver().with_enum_fields(HashSet::from(["link_type".to_string()]));

        for operator in CONTAINMENT_OPERATORS {
            let rendered = render_containment(operator, "link_type", "anchor", &resolver);
            assert!(
                rendered.contains("format!(\"{:?}\", result.link_type).to_lowercase()"),
                "`{operator}` must compare an enum field through its Debug form, or the generated \
                 test cannot compile — an enum has no inherent `contains`; got: {rendered}"
            );
            assert!(
                !rendered.contains("result.link_type.contains("),
                "`{operator}` still calls `contains` directly on an enum field; got: {rendered}"
            );
        }
    }

    #[test]
    fn every_containment_operator_uses_the_collection_predicate_on_a_collection_field() {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::from(["cookies".to_string()]),
            &HashSet::from(["cookies".to_string()]),
            &HashSet::new(),
        );

        for operator in CONTAINMENT_OPERATORS {
            let rendered = render_containment(operator, "cookies", "session", &resolver);
            assert!(
                rendered.contains("result.cookies.iter().any("),
                "`{operator}` must match a collection field element-wise, or it compares a whole \
                 element against a name; got: {rendered}"
            );
            assert!(
                rendered.contains("fields.get(*key)") && rendered.contains("\"name\""),
                "`{operator}` must accept an object element matched by its `name` (among other \
                 keys), not only a whole-element comparison; got: {rendered}"
            );
        }
    }

    #[test]
    fn every_containment_operator_emits_parseable_rust() {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::from(["cookies".to_string()]),
            &HashSet::from(["cookies".to_string()]),
            &HashSet::new(),
        )
        .with_enum_fields(HashSet::from(["link_type".to_string()]));

        for operator in CONTAINMENT_OPERATORS {
            for (field, expected) in [("link_type", "anchor"), ("cookies", "session"), ("content", "plain")] {
                let body = render_containment(operator, field, expected, &resolver);
                let unit = format!("fn generated() {{\n{body}}}\n");
                syn::parse_file(&unit).unwrap_or_else(|error| {
                    panic!("`{operator}` on `{field}` must emit parseable Rust: {error}\n{unit}")
                });
            }
        }
    }

    /// ~keep Pins `contains`'s emitted bytes, not merely its shape. Sharing one predicate across
    /// the four operators is only safe if the operator that already worked emits exactly what it
    /// emitted before — otherwise every consumer regenerates, and reviewers must diff generated
    /// trees to tell a real fix from formatting churn. These three lines are the pre-refactor
    /// output verbatim.
    #[test]
    fn contains_emits_unchanged_bytes_for_every_field_kind() {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::from(["content".to_string(), "link_type".to_string(), "cookies".to_string()]),
            &HashSet::from(["cookies".to_string()]),
            &HashSet::new(),
        )
        .with_enum_fields(HashSet::from(["link_type".to_string()]));

        let expected = [
            (
                "content",
                "needle",
                "    assert!(result.content.contains(r#\"needle\"#), \"expected to contain: {}\", r#\"needle\"#);\n",
            ),
            (
                "link_type",
                "anchor",
                "    assert!(format!(\"{:?}\", result.link_type).to_lowercase().contains(&r#\"anchor\"#.to_lowercase()), \"expected to contain: {}\", r#\"anchor\"#);\n",
            ),
            (
                "cookies",
                "session",
                "    assert!(result.cookies.iter().any(|item| serde_json::to_value(item).ok().is_some_and(|value| match &value { serde_json::Value::String(text) => text.contains(r#\"session\"#), serde_json::Value::Object(fields) => [\"kind\", \"name\", \"source\", \"alias\", \"text\", \"signature\"].iter().any(|key| fields.get(*key).and_then(serde_json::Value::as_str).is_some_and(|text| text.contains(r#\"session\"#))) || value.to_string().contains(r#\"session\"#), _ => false })), \"expected collection item to contain: {}\", r#\"session\"#);\n",
            ),
        ];

        for (field, value, want) in expected {
            let assertion = make_assertion("contains", Some(field), Some(serde_json::json!(value)));
            let mut got = String::new();
            render_assertion(
                &mut got,
                &assertion,
                "result",
                "sample",
                "sample",
                false,
                &[],
                &resolver,
                false,
                false,
                false,
                false,
                false,
                None,
            );
            assert_eq!(got, want, "`contains` on `{field}` changed its emitted bytes");
        }
    }

    /// ~keep The two predicate shapes below are duplicated on purpose: each appears once as a
    /// string the generator must produce and once as real code rustc type-checks in this test
    /// binary. A string-only assertion can be updated to match a broken generator and stay
    /// green — that is how a previous containment fix shipped output that did not compile.
    /// Keeping a compiled copy means the pair cannot both be edited into agreement without
    /// rustc also accepting the result.
    const ENUM_PREDICATE: &str = r#"format!("{:?}", kind).to_lowercase().contains(&"anchor".to_lowercase())"#;

    const COLLECTION_PREDICATE: &str = r#"items.iter().any(|item| serde_json::to_value(item).ok().is_some_and(|value| match &value { serde_json::Value::String(text) => text.contains("needle"), serde_json::Value::Object(fields) => ["kind", "name", "source", "alias", "text", "signature"].iter().any(|key| fields.get(*key).and_then(serde_json::Value::as_str).is_some_and(|text| text.contains("needle"))) || value.to_string().contains("needle"), _ => false }))"#;

    #[test]
    fn the_enum_predicate_is_valid_rust_against_a_real_enum() {
        #[derive(Debug)]
        enum SampleKind {
            Anchor,
        }
        let kind = SampleKind::Anchor;

        assert!(format!("{:?}", kind).to_lowercase().contains(&"anchor".to_lowercase()));

        assert_eq!(containment_predicate("kind", "\"anchor\"", true, false), ENUM_PREDICATE);
    }

    #[test]
    fn the_collection_predicate_is_valid_rust_against_a_real_collection() {
        #[derive(serde::Serialize)]
        struct SampleItem {
            name: String,
        }
        let items = [SampleItem {
            name: "needle".to_string(),
        }];

        assert!(
            items
                .iter()
                .any(|item| serde_json::to_value(item).ok().is_some_and(|value| {
                    match &value {
                        serde_json::Value::String(text) => text.contains("needle"),
                        serde_json::Value::Object(fields) => {
                            ["kind", "name", "source", "alias", "text", "signature"]
                                .iter()
                                .any(|key| {
                                    fields
                                        .get(*key)
                                        .and_then(serde_json::Value::as_str)
                                        .is_some_and(|text| text.contains("needle"))
                                })
                                || value.to_string().contains("needle")
                        }
                        _ => false,
                    }
                }))
        );

        assert_eq!(
            containment_predicate("items", "\"needle\"", false, true),
            COLLECTION_PREDICATE
        );
    }

    #[test]
    fn simple_result_enum_contains_uses_enum_value() {
        let resolver = empty_resolver().with_enum_fields(HashSet::from(["link_type".to_string()]));
        let mut output = String::new();
        render_assertion(
            &mut output,
            &make_assertion("contains", Some("link_type"), Some(serde_json::json!("anchor"))),
            "result.links[0].link_type",
            "sample",
            "sample",
            false,
            &[],
            &resolver,
            false,
            true,
            false,
            false,
            false,
            None,
        );
        assert!(
            output.contains("format!(\"{:?}\", result.links[0].link_type)"),
            "got: {output}"
        );
        assert!(!output.contains("link_type.contains"), "got: {output}");
    }

    #[test]
    #[should_panic(expected = "streaming field 'chunks' assertion 'count_min' requires a numeric")]
    fn streaming_count_min_without_value_fails_loudly() {
        let resolver = empty_resolver();
        let assertion = make_assertion("count_min", Some("chunks"), None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
    }

    #[test]
    #[should_panic(expected = "unsupported assertion type 'bogus_type' on streaming field 'chunks'")]
    fn streaming_assertion_unknown_type_fails_loudly() {
        let resolver = empty_resolver();
        let assertion = make_assertion("bogus_type", Some("chunks"), None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
    }

    #[test]
    #[should_panic(expected = "streaming field 'stream.has_page_event' has no accessor for context")]
    fn streaming_field_without_accessor_fails_loudly() {
        let resolver = empty_resolver();
        // `streaming_item_type: None` makes `accessor_with_streaming_context` return
        // `None` for event-variant fields (see streaming_assertions/accessors.rs), which
        // used to fall through to rendering nothing at all. ~keep
        let assertion = make_assertion("is_true", Some("stream.has_page_event"), None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
    }

    #[test]
    fn streaming_not_empty_still_renders_real_assertion() {
        let resolver = empty_resolver();
        let assertion = make_assertion("not_empty", Some("chunks"), None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(out.contains("assert!"), "got: {out}");
        assert!(out.contains("expected non-empty"), "got: {out}");
    }

    #[test]
    fn streaming_count_min_with_value_still_renders_real_assertion() {
        let resolver = empty_resolver();
        let assertion = make_assertion("count_min", Some("chunks"), Some(serde_json::json!(3)));
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            &resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        assert!(out.contains("assert!(chunks.len() >= 3 as usize"), "got: {out}");
    }

    /// The regression shape: `Envelope { results: Vec<Document> }`, and `Document` (reached only
    /// through `results`) declares `chunks`. Before the fix, `chunks_have_content` hardcoded
    /// `result.chunks` regardless of which type `result` actually was — intercepting ahead of
    /// any field validation. Anchoring the interception at the call's own declared root type is
    /// what tells `Envelope` and `Document` apart. ~keep
    fn envelope_and_document_type_defs() -> Vec<crate::core::ir::TypeDef> {
        use crate::core::ir::{FieldDef, TypeDef, TypeRef};
        vec![
            TypeDef {
                name: "Envelope".to_string(),
                fields: vec![FieldDef {
                    name: "results".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Document".to_string()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
            TypeDef {
                name: "Document".to_string(),
                fields: vec![FieldDef {
                    name: "chunks".to_string(),
                    ty: TypeRef::Vec(Box::new(TypeRef::Named("Chunk".to_string()))),
                    ..FieldDef::default()
                }],
                ..TypeDef::default()
            },
        ]
    }

    fn resolver_anchored_at(root_type: Option<&str>) -> FieldResolver {
        let type_defs = envelope_and_document_type_defs();
        let map = FieldResolver::ir_result_field_facts(&type_defs, "rust");
        let (reachable, excluded, optional) = FieldResolver::ir_field_sets(&type_defs);
        empty_resolver()
            .with_ir_result_fields(map, root_type.map(str::to_string))
            .with_ir_fields(reachable, excluded, optional)
    }

    fn render_chunks_have_content_call(resolver: &FieldResolver) -> String {
        let assertion = make_assertion("is_true", Some("chunks_have_content"), None);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "my_mod",
            "dep",
            false,
            &[],
            resolver,
            false,
            false,
            false,
            false,
            false,
            None,
        );
        out
    }

    /// The confirmed defect: a call whose own root type (`Envelope`) does not declare `chunks`
    /// must not emit `result.chunks` — that struct has no such field and the generated Rust
    /// would not compile.
    #[test]
    fn chunks_have_content_refused_when_call_root_lacks_chunks() {
        let resolver = resolver_anchored_at(Some("Envelope"));
        let out = render_chunks_have_content_call(&resolver);
        assert!(
            !out.contains("result.chunks"),
            "must not hardcode result.chunks against a root type that declares no such field, got: {out}"
        );
        assert!(out.contains("// skipped:"), "got: {out}");
    }

    /// The control: a call whose root type genuinely declares `chunks` must still render the
    /// real assertion — the fix must not turn into "refuse every chunks_have_content fixture."
    #[test]
    fn chunks_have_content_still_renders_when_call_root_declares_chunks() {
        let resolver = resolver_anchored_at(Some("Document"));
        let out = render_chunks_have_content_call(&resolver);
        assert!(out.contains("result.chunks"), "got: {out}");
        assert!(!out.contains("// skipped:"), "got: {out}");
    }

    /// No anchored root type at all (the state of every call site before this fix) must keep
    /// the pre-existing permissive behaviour: nothing here regresses a fixture whose call site
    /// never resolved a root type.
    #[test]
    fn chunks_have_content_renders_when_no_root_type_is_anchored() {
        let resolver = resolver_anchored_at(None);
        let out = render_chunks_have_content_call(&resolver);
        assert!(out.contains("result.chunks"), "got: {out}");
        assert!(!out.contains("// skipped:"), "got: {out}");
    }
}
