//! Synthetic / virtual field assertions -- fields that are computed rather than direct
//! record accessors (chunk-level predicates, embedding validation, and the like).
//!
//! Split out of `assertions.rs` (file-modularization cap): see that file's module doc.

use crate::e2e::codegen::assertion_recipes::chunks_result_var;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::values::json_to_java;

pub(super) fn try_synthetic_field_assertion(
    out: &mut String,
    assertion: &Assertion,
    result_var: &str,
    field_resolver: &FieldResolver,
    result_is_simple: bool,
) -> bool {
    // Handle synthetic/virtual fields that are computed rather than direct record accessors.
    if let Some(f) = &assertion.field {
        if let Some(reason) = crate::e2e::codegen::assertion_recipes::chunks_synthetic_skip_reason(f, field_resolver) {
            out.push_str(&format!("        // skipped: {reason}\n"));
            return true;
        }

        match f.as_str() {
            // ---- ProcessingResult chunk-level computed predicates ----
            "chunks_have_content" => {
                let result_var = &chunks_result_var(field_resolver, "java", result_var);
                let pred = format!(
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().allMatch(c -> c.content() != null && !c.content().isBlank())"
                );
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_content",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return true;
            }
            "chunks_have_heading_context" => {
                let result_var = &chunks_result_var(field_resolver, "java", result_var);
                let pred = format!(
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().allMatch(c -> c.metadata().headingContext() != null)"
                );
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_heading_context",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return true;
            }
            "chunks_have_embeddings" => {
                let result_var = &chunks_result_var(field_resolver, "java", result_var);
                let pred = format!(
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().allMatch(c -> c.embedding() != null && !c.embedding().isEmpty())"
                );
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "chunks_embeddings",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return true;
            }
            "first_chunk_starts_with_heading" => {
                let result_var = &chunks_result_var(field_resolver, "java", result_var);
                let pred = format!(
                    "java.util.Optional.ofNullable({result_var}.chunks()).orElse(java.util.List.of()).stream().findFirst().map(c -> c.metadata().headingContext() != null).orElse(false)"
                );
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "first_chunk_heading",
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return true;
            }
            // ---- EmbedResponse virtual fields ----
            // When result_is_simple=true the result IS List<List<Float>> (the raw embeddings list).
            // When result_is_simple=false the result has an .embeddings() accessor.
            "embedding_dimensions" => {
                // Dimension = size of the first embedding vector in the list.
                let embed_list = if result_is_simple {
                    result_var.to_string()
                } else {
                    format!("{result_var}.embeddings()")
                };
                let expr = format!("({embed_list}.isEmpty() ? 0 : {embed_list}.get(0).size())");
                let java_val = assertion.value.as_ref().map(json_to_java).unwrap_or_default();
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "embedding_dimensions",
                        assertion_type => assertion.assertion_type.as_str(),
                        expr => expr,
                        java_val => java_val,
                        field_name => f,
                    },
                ));
                return true;
            }
            "embeddings_valid" | "embeddings_finite" | "embeddings_non_zero" | "embeddings_normalized" => {
                // These are validation predicates that require iterating the embedding matrix.
                let embed_list = if result_is_simple {
                    result_var.to_string()
                } else {
                    format!("{result_var}.embeddings()")
                };
                let pred = match f.as_str() {
                    "embeddings_valid" => {
                        format!("{embed_list}.stream().allMatch(e -> e != null && !e.isEmpty())")
                    }
                    "embeddings_finite" => {
                        format!("{embed_list}.stream().flatMap(java.util.Collection::stream).allMatch(Float::isFinite)")
                    }
                    "embeddings_non_zero" => {
                        format!("{embed_list}.stream().allMatch(e -> e.stream().anyMatch(v -> v != 0.0f))")
                    }
                    "embeddings_normalized" => format!(
                        "{embed_list}.stream().allMatch(e -> {{ double n = e.stream().mapToDouble(v -> v * v).sum(); return Math.abs(n - 1.0) < 1e-3; }})"
                    ),
                    _ => unreachable!(),
                };
                let assertion_kind = format!("embeddings_{}", f.strip_prefix("embeddings_").unwrap_or(f));
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => assertion_kind,
                        assertion_type => assertion.assertion_type.as_str(),
                        pred => pred,
                        field_name => f,
                    },
                ));
                return true;
            }
            // ---- Fields not present on the Java ProcessingResult ----
            "keywords" | "keywords_count" => {
                out.push_str(&crate::e2e::template_env::render(
                    "java/synthetic_assertion.jinja",
                    minijinja::context! {
                        assertion_kind => "keywords",
                        field_name => f,
                    },
                ));
                return true;
            }
            // ---- metadata not_empty / is_empty: Metadata is a required record, not Optional ----
            // Metadata has no .isEmpty() method; check that at least one optional field is present.
            "metadata" => {
                match assertion.assertion_type.as_str() {
                    "not_empty" | "is_empty" => {
                        out.push_str(&crate::e2e::template_env::render(
                            "java/synthetic_assertion.jinja",
                            minijinja::context! {
                                assertion_kind => "metadata",
                                assertion_type => assertion.assertion_type.as_str(),
                                result_var => result_var,
                            },
                        ));
                        return true;
                    }
                    _ => {} // fall through to normal handling
                }
            }
            _ => {}
        }
    }
    false
}
