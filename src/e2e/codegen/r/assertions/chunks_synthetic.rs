//! `CHUNKS_RECIPE` synthetic assertion handlers for R.
//!
//! Split out of `assertions.rs`, which is at the repo's 1,000-line file-modularization cap and
//! may not grow. Anchors `.chunks` through `chunks_result_var` — see its doc for why the
//! hardcoded `{result_var}$chunks` these four handlers used before could emit code that does not
//! compile against a consumer whose result type is an envelope. ~keep

use std::fmt::Write as FmtWrite;

use crate::e2e::codegen::assertion_recipes::chunks_result_var;
use crate::e2e::field_access::FieldResolver;
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
            let result_var = &chunks_result_var(field_resolver, "r", result_var);
            let pred = format!("all(sapply({result_var}$chunks %||% list(), function(c) nchar(c$content) > 0))");
            render_true_false(out, assertion, field, &pred);
            true
        }
        "chunks_have_embeddings" => {
            let result_var = &chunks_result_var(field_resolver, "r", result_var);
            let pred = format!(
                "all(sapply({result_var}$chunks %||% list(), function(c) !is.null(c$embedding) && length(c$embedding) > 0))"
            );
            render_true_false(out, assertion, field, &pred);
            true
        }
        "chunks_have_heading_context" => {
            // extendr exposes `Chunk.metadata` and its nested `heading_context` the same way it
            // exposes `content`/`embedding` above (both accessed via plain `$`) -- an
            // `Option<T>::None` maps to R `NULL`, so the field itself is directly checkable. A
            // predicate over `content` length would be a proxy: it can pass on a chunk whose
            // heading metadata was never attached, and fail on one where it was but the content
            // happens to be short. ~keep
            let result_var = &chunks_result_var(field_resolver, "r", result_var);
            let pred = format!(
                "!is.null({result_var}$chunks) && length({result_var}$chunks) > 0 && all(sapply({result_var}$chunks, function(c) !is.null(c$metadata) && !is.null(c$metadata$heading_context)))"
            );
            render_true_false(out, assertion, field, &pred);
            true
        }
        "first_chunk_starts_with_heading" => {
            // Same field as `chunks_have_heading_context` above, restricted to the first chunk
            // -- not a `content`-prefix proxy. ~keep
            let result_var = &chunks_result_var(field_resolver, "r", result_var);
            let pred = format!(
                "!is.null({result_var}$chunks) && length({result_var}$chunks) > 0 && !is.null({result_var}$chunks[[1]]$metadata) && !is.null({result_var}$chunks[[1]]$metadata$heading_context)"
            );
            render_true_false(out, assertion, field, &pred);
            true
        }
        _ => false,
    }
}

fn render_true_false(out: &mut String, assertion: &Assertion, field: &str, pred: &str) {
    match assertion.assertion_type.as_str() {
        "is_true" => {
            let _ = writeln!(out, "  expect_true({pred})");
        }
        "is_false" => {
            let _ = writeln!(out, "  expect_false({pred})");
        }
        other => {
            panic!("R e2e generator: unsupported assertion type '{other}' on synthetic field '{field}'");
        }
    }
}
