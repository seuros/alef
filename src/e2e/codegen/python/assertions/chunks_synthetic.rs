//! `CHUNKS_RECIPE` synthetic assertion handlers for Python.
//!
//! Split out of `assertions.rs`, which is at the repo's 1,000-line file-modularization cap and
//! may not grow. Anchors `.chunks` through `chunks_result_var` — see its doc for why the
//! hardcoded `{result_var}.chunks` these four handlers used before could emit code that does not
//! compile against a consumer whose result type is an envelope. ~keep

use crate::e2e::codegen::assertion_recipes::chunks_result_var;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::Assertion;

use super::emit_bool_assertion;

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
            let result_var = &chunks_result_var(field_resolver, "python", result_var);
            let pred = format!("all(c.content for c in ({result_var}.chunks or []))");
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "chunks_have_heading_context" => {
            let result_var = &chunks_result_var(field_resolver, "python", result_var);
            let pred = format!(
                "all(c.metadata and c.metadata.heading_context is not None for c in ({result_var}.chunks or []))"
            );
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "first_chunk_starts_with_heading" => {
            let result_var = &chunks_result_var(field_resolver, "python", result_var);
            let pred = format!(
                "bool(({result_var}.chunks or []) and ({result_var}.chunks[0].metadata and {result_var}.chunks[0].metadata.heading_context))"
            );
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        "chunks_have_embeddings" => {
            let result_var = &chunks_result_var(field_resolver, "python", result_var);
            let pred =
                format!("all(c.embedding is not None and len(c.embedding) > 0 for c in ({result_var}.chunks or []))");
            emit_bool_assertion(out, &pred, assertion.assertion_type.as_str(), field);
            true
        }
        _ => false,
    }
}
