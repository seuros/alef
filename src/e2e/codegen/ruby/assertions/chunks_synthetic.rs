//! `CHUNKS_RECIPE` synthetic assertion handlers for Ruby.
//!
//! Split out of `assertions.rs`, which is at the repo's 1,000-line file-modularization cap and
//! may not grow. Anchors `.chunks` through `chunks_result_var` — see its doc for why the
//! hardcoded `{result_var}.chunks` these four handlers used before could emit code that does not
//! compile against a consumer whose result type is an envelope. ~keep

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
            let result_var = &chunks_result_var(field_resolver, "ruby", result_var);
            let pred = format!("({result_var}.chunks || []).all? {{ |c| c.content && !c.content.empty? }}");
            render_true_false(out, assertion, field, &pred);
            true
        }
        "chunks_have_heading_context" => {
            let result_var = &chunks_result_var(field_resolver, "ruby", result_var);
            let pred =
                format!("({result_var}.chunks || []).all? {{ |c| c.metadata && !c.metadata.heading_context.nil? }}");
            render_true_false(out, assertion, field, &pred);
            true
        }
        "first_chunk_starts_with_heading" => {
            let result_var = &chunks_result_var(field_resolver, "ruby", result_var);
            let pred = format!("!({result_var}.chunks || []).first&.metadata&.heading_context.nil?");
            render_true_false(out, assertion, field, &pred);
            true
        }
        "chunks_have_embeddings" => {
            let result_var = &chunks_result_var(field_resolver, "ruby", result_var);
            let pred = format!("({result_var}.chunks || []).all? {{ |c| !c.embedding.nil? && !c.embedding.empty? }}");
            render_true_false(out, assertion, field, &pred);
            true
        }
        _ => false,
    }
}

fn render_true_false(out: &mut String, assertion: &Assertion, field: &str, pred: &str) {
    match assertion.assertion_type.as_str() {
        "is_true" => {
            out.push_str(&format!("    expect({pred}).to be(true)\n"));
        }
        "is_false" => {
            out.push_str(&format!("    expect({pred}).to be(false)\n"));
        }
        _ => {
            out.push_str(&format!(
                "    # skipped: unsupported assertion type on synthetic field '{field}'\n"
            ));
        }
    }
}
