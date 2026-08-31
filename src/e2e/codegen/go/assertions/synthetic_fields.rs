//! Synthetic chunk/embedding assertion fields for the Go e2e generator.
//!
//! These fields (`chunks_have_content`, `embeddings`, `embedding_dimensions`, ...) do not
//! resolve through `FieldResolver` like an ordinary struct field -- they are synthesized
//! predicates over the RAG chunking/embedding recipe's output, so they must be recognized
//! and rendered before any real field resolution runs.
//!
//! Split out of `assertions.rs`, which is over the 1000-line cap and may not grow.

use crate::e2e::codegen::assertion_recipes::chunks_result_var;
use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::fixture::Assertion;
use std::fmt::Write as FmtWrite;

use super::AssertionRenderContext;

/// Render an assertion whose field names a synthetic chunk/embedding predicate.
///
/// Returns `true` when the field matched a synthetic name and the assertion (or its
/// skip-marker fallback) has been written to `out`; `false` when the caller must fall
/// through to ordinary field resolution.
pub(super) fn render_synthetic_field_assertion(
    out: &mut String,
    assertion: &Assertion,
    context: &AssertionRenderContext<'_>,
) -> bool {
    if context.result_is_simple {
        return false;
    }
    let Some(f) = &assertion.field else {
        return false;
    };
    let result_var = context.effective_result_var;
    let field_resolver = context.field_resolver;
    let embed_deref = format!("(*{result_var})");
    if let Some(reason) = crate::e2e::codegen::assertion_recipes::chunks_synthetic_skip_reason(f, field_resolver) {
        let _ = writeln!(out, "\t// skipped: {reason}");
        return true;
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
            return true;
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
            return true;
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
            return true;
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
            return true;
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
            return true;
        }
        "embedding_dimensions" => {
            let expr =
                format!("func() int {{ if len({embed_deref}) == 0 {{ return 0 }}; return len({embed_deref}[0]) }}()");
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
            return true;
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
            return true;
        }
        "keywords" | "keywords_count" => {
            let _ = writeln!(
                out,
                "\t// skipped: {}",
                FieldSkip::NotAvailableOnGoProcessingResult.message(f)
            );
            return true;
        }
        _ => {}
    }
    false
}
