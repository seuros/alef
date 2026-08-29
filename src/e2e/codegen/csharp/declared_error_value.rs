//! The C# backend's declared-`error`-value assertion.
//!
//! Split out of `csharp.rs`, whose tests already live in sibling files — this is the one
//! self-contained production concern small enough to lift without restructuring the generator.

use crate::e2e::escape::escape_csharp;
use crate::e2e::fixture::Fixture;

/// Render the xUnit assertion that checks a declared `error` fixture value against the thrown
/// exception — or, when the declared value names a real error variant this backend's binding
/// cannot substantiate, the registered skip instead of an assertion that can never pass.
///
/// ~keep Mirrors the Rust/Python/Go/Java backends' disjunction (see
/// `crate::e2e::codegen::declared_error_value`): fixture authors name either a message
/// substring (config-validation fixtures) or an exact error-variant name (API-error fixtures)
/// in the assertion's value, never both conventions at once. Which of those two conventions
/// applies, and whether C# can substantiate the second, is decided once by
/// `declared_error_variant::classify` — see its doc for the per-variant dispatch condition.
///
/// The two conventions get genuinely different assertions, not one disjunction serving both:
/// a message-style value keeps the existing `.Contains` check (fuzzy on purpose — it is prose,
/// not an identity), but a real variant name gets `Assert.IsType<{Variant}Exception>`, an exact
/// type check `{Variant}Exception.FromLastError` either satisfies or does not — unlike a
/// substring match, asserting the WRONG variant here fails, because `thrown`'s runtime type is
/// never the wrong variant's class once `FromLastError` actually dispatches it.
pub(super) fn declared_error_value_check(fixture: &Fixture, errors: &[crate::core::ir::ErrorDef]) -> Option<String> {
    use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, declared_variant, skip_line};
    match classify("csharp", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => None,
        DeclaredErrorAssertion::Assert(declared) => {
            if let Some((_, variant)) = declared_variant(fixture, errors) {
                Some(format!("        Assert.IsType<{}Exception>(thrown);", variant.name))
            } else {
                let escaped = escape_csharp(declared);
                Some(format!(
                    "        Assert.True(thrown.Message != null && thrown.Message.Contains(\"{escaped}\") \
|| thrown.GetType().Name.Contains(\"{escaped}\"), \"expected error to match: {escaped}\");"
                ))
            }
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            Some(skip_line("        ", "//", variant, &fixture.id, "csharp"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::declared_error_value_check;
    use crate::core::ir::{ErrorDef, ErrorVariant};
    use crate::e2e::fixture::{Assertion, Fixture};

    fn fixture_with_declared_error(value: &str) -> Fixture {
        Fixture {
            id: "declares_error".to_string(),
            assertions: vec![Assertion {
                assertion_type: "error".to_string(),
                value: Some(serde_json::Value::String(value.to_string())),
                ..Assertion::default()
            }],
            ..Fixture::default()
        }
    }

    /// One `ErrorDef` named `ApiError` carrying every `(variant_name, message_template)` pair
    /// given, mirroring how a real crate declares one error enum with several variants.
    fn api_error(variants: &[(&str, &str)]) -> ErrorDef {
        ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: variants
                .iter()
                .map(|(name, message_template)| ErrorVariant {
                    name: name.to_string(),
                    message_template: Some(message_template.to_string()),
                    is_unit: true,
                    ..ErrorVariant::default()
                })
                .collect(),
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    /// The fix this module exists for: once `FromLastError` can dispatch a variant (it has a
    /// message-template prefix — see `declared_error_variant::substantiates_variant_identity`),
    /// the rendered check must assert the exact `{Variant}Exception` type, not the old
    /// `.Contains` disjunction.
    #[test]
    fn asserts_the_exact_type_for_a_substantiable_variant() {
        let fixture = fixture_with_declared_error("Authentication");
        let errors = vec![api_error(&[("Authentication", "Authentication failed: {reason}")])];
        let check = declared_error_value_check(&fixture, &errors).expect("expected a rendered assertion");
        assert_eq!(check, "        Assert.IsType<AuthenticationException>(thrown);");
    }

    /// Proves discrimination directly: declaring a DIFFERENT known variant against the SAME
    /// error registry renders a DIFFERENT exact assertion, targeting that variant's own class.
    /// If a fixture wrongly declared `"BadRequest"` while the real thrown value were
    /// `AuthenticationException`, this is the line that would fail at C# runtime —
    /// `Assert.IsType` on a concrete type either matches the runtime type or it does not, unlike
    /// a substring check on a message or a shared exception name, which could coincidentally
    /// still pass. This is the "wrong variant fails" property the fix exists to deliver.
    #[test]
    fn renders_a_different_exact_assertion_for_a_different_variant() {
        let errors = vec![api_error(&[
            ("Authentication", "Authentication failed: {reason}"),
            ("BadRequest", "Bad request: {reason}"),
        ])];

        let auth_fixture = fixture_with_declared_error("Authentication");
        let auth_check = declared_error_value_check(&auth_fixture, &errors).expect("expected a rendered assertion");
        assert_eq!(auth_check, "        Assert.IsType<AuthenticationException>(thrown);");

        let bad_request_fixture = fixture_with_declared_error("BadRequest");
        let bad_request_check =
            declared_error_value_check(&bad_request_fixture, &errors).expect("expected a rendered assertion");
        assert_eq!(bad_request_check, "        Assert.IsType<BadRequestException>(thrown);");

        assert_ne!(auth_check, bad_request_check);
    }

    /// A variant with no dispatchable message prefix (no `#[error(...)]` template) still cannot
    /// be substantiated and must still render the registered skip — the fix only turns on the
    /// exact assertion for variants `FromLastError` can actually dispatch, it does not turn
    /// every C# variant assertion on unconditionally.
    #[test]
    fn a_variant_with_no_message_template_still_renders_the_skip() {
        let fixture = fixture_with_declared_error("Unknown");
        let mut error = api_error(&[]);
        error.variants.push(ErrorVariant {
            name: "Unknown".to_string(),
            message_template: None,
            is_unit: true,
            ..ErrorVariant::default()
        });
        let errors = vec![error];

        let check = declared_error_value_check(&fixture, &errors).expect("expected a rendered skip");
        assert_eq!(
            check,
            "        // skipped: declared error variant 'Unknown' not yet preserved as a distinct identity by \
             this backend's generator"
        );
    }
}
