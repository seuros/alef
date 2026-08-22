//! The C# backend's declared-`error`-value assertion.
//!
//! Split out of `csharp.rs`, whose tests already live in sibling files — this is the one
//! self-contained production concern small enough to lift without restructuring the generator.

use crate::e2e::escape::escape_csharp;
use crate::e2e::fixture::Fixture;

/// Render the xUnit assertion that checks a declared `error` fixture value against
/// either the thrown exception's message or its type name — or, when the declared value
/// names a real error variant this backend's binding cannot substantiate, the registered
/// skip instead of an assertion that can never pass.
///
/// ~keep Mirrors the Rust/Python/Go/Java backends' disjunction (see
/// `crate::e2e::codegen::declared_error_value`): fixture authors name either a message
/// substring (config-validation fixtures) or a type-name prefix (API-error fixtures) in
/// the assertion's value, never both conventions at once. Checking `.Message` OR
/// `.GetType().Name` lets this single code path serve both, without narrowing the
/// existing fixed `exception_class` the test already asserts is thrown. Which of those two
/// conventions applies, and whether C# can ever satisfy the second, is decided once by
/// `declared_error_variant::classify` — see its doc for why C# lands on "never" today.
pub(super) fn declared_error_value_check(fixture: &Fixture, errors: &[crate::core::ir::ErrorDef]) -> Option<String> {
    use crate::e2e::codegen::declared_error_variant::{DeclaredErrorAssertion, classify, skip_line};
    match classify("csharp", fixture, errors) {
        DeclaredErrorAssertion::Undeclared => None,
        DeclaredErrorAssertion::Assert(declared) => {
            let escaped = escape_csharp(declared);
            Some(format!(
                "        Assert.True(thrown.Message != null && thrown.Message.Contains(\"{escaped}\") \
|| thrown.GetType().Name.Contains(\"{escaped}\"), \"expected error to match: {escaped}\");"
            ))
        }
        DeclaredErrorAssertion::Unsubstantiable(variant) => {
            Some(skip_line("        ", "//", variant, &fixture.id, "csharp"))
        }
    }
}
