//! Whether a fixture's field path can be rendered as a member access on a Rust call's result.
//!
//! Split out of `assertions.rs` so the *same* answer is available to the two places that need it:
//! `render_assertion`, which either emits the access or a skip marker, and `render_test_function`,
//! which decides from the same set of assertions whether the call must bind to a named result
//! variable at all. Those two used to spell the question differently, and a disagreement there is
//! either a phantom member access or an unused binding.

use crate::e2e::codegen::field_skip::FieldSkip;
use crate::e2e::field_access::FieldResolver;

/// The skip marker body for a field path this call's result cannot render, or `None` to render
/// the assertion normally.
///
/// `error.`-prefixed paths name the error value, not the result, and are resolved through
/// `accessor_for_error`; they are never judged here.
///
/// ~keep A `result_is_simple` call reinterprets a field-bearing assertion as an assertion on the
/// whole result, so the availability oracle is deliberately not allowed to veto the path — that
/// carve-out is why the `is_valid_for_result` arm below is gated on it. It is wrong in exactly one
/// case: when the oracle's refusal is that the result has NO fields whatsoever. A
/// `Result<bytes::Bytes>` call is the shape — `FieldResolver::result_has_no_fields` is the same
/// fact `is_valid_for_result` already refuses every path on — and the assertion renderers do not
/// all honour the simple-result decision: `render_not_empty_assertion` (and its optional/array
/// siblings) re-derive `FieldResolver::accessor(field, ..)` instead of reusing the `field_access`
/// the simple-result arm computed, so the skip has to happen before them or the generated test
/// carries `result.<field>` and fails to compile with `E0609: no field <field> on type Bytes`.
/// Every other backend already drops the assertion for this shape; this is rust asking the same
/// oracle instead of keeping a second opinion.
///
/// The wording is `NotAvailableWhenResultIsSimple` rather than `NotAvailableOnResultType`
/// deliberately: this is a property of the declared call shape, not a fixture typo, so it is a
/// `LanguageLimitation` the strict gate counts and reports rather than an `AuthoringGap` that
/// fails generation for a fixture whose author did nothing wrong.
pub(super) fn unrenderable_field_skip(
    field: &str,
    result_var: &str,
    result_is_simple: bool,
    field_resolver: &FieldResolver,
) -> Option<String> {
    if field.starts_with("error.") {
        return None;
    }
    if result_is_simple {
        // ~keep `field == result_var` is the sentinel meaning "the whole return value", which a
        // fieldless result answers perfectly well -- skipping it would drop real coverage.
        return (field != result_var && field_resolver.result_has_no_fields())
            .then(|| FieldSkip::NotAvailableWhenResultIsSimple.message(field));
    }
    (!field_resolver.is_valid_for_result(field)).then(|| FieldSkip::NotAvailableOnResultType.message(field))
}
