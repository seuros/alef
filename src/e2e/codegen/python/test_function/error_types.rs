//! Which typed exception classes a rendered test file's error assertions reference.
//!
//! `pyo3::create_exception!` gives every `ErrorVariant` its own exception class unconditionally
//! (`declared_error_variant::substantiates_variant_identity`'s `"python" => true` arm), so
//! `error_assertions::emit_error_assertion` catches that class directly —
//! `pytest.raises(BadRequestError)` — for a fixture whose declared `error` value names a real
//! variant, rather than the broad `Exception` (xberg #1525). That class name must be imported
//! into the generated file exactly like any other type it references.

use std::collections::BTreeSet;

use crate::e2e::fixture::Fixture;

/// The typed exception class names `emit_error_assertion` will reference in `pytest.raises(...)`
/// for any fixture in `fixtures` whose declared `error` value names a real `ErrorVariant`.
///
/// HTTP fixtures route through `render_http_test_function`, not `emit_error_assertion`, so they
/// are excluded here to match which fixtures can actually emit that class name.
pub(crate) fn collect_used_error_types(
    fixtures: &[&Fixture],
    errors: &[crate::core::ir::ErrorDef],
) -> BTreeSet<String> {
    let mut used_error_types = BTreeSet::new();
    for fixture in fixtures.iter().filter(|f| !f.is_http_test()) {
        if let Some(branch) = crate::e2e::codegen::snippet_error_branch::for_fixture("python", fixture, errors) {
            used_error_types.insert(branch.host_type);
        }
    }
    used_error_types
}

#[cfg(test)]
mod tests {
    use super::collect_used_error_types;
    use crate::core::ir::{ErrorDef, ErrorVariant};
    use crate::e2e::fixture::{Assertion, Fixture};

    fn error_def_with_variant(error_name: &str, variant_name: &str) -> ErrorDef {
        ErrorDef {
            name: error_name.to_string(),
            rust_path: format!("lib::{error_name}"),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: variant_name.to_string(),
                is_unit: true,
                ..ErrorVariant::default()
            }],
            doc: String::new(),
            methods: vec![],
            binding_excluded: false,
            binding_exclusion_reason: None,
            version: Default::default(),
        }
    }

    fn fixture_declaring(value: &str) -> Fixture {
        Fixture {
            id: "auth_fails".to_string(),
            assertions: vec![Assertion {
                assertion_type: "error".to_string(),
                value: Some(serde_json::Value::String(value.to_string())),
                ..Assertion::default()
            }],
            ..Fixture::default()
        }
    }

    /// The regression this module exists to close: before it, nothing collected the class name
    /// `emit_error_assertion` now renders, so `pytest.raises(AuthenticationError)` resolved
    /// against no import at all — a `NameError`, not a passing test.
    #[test]
    fn a_fixture_naming_a_real_variant_yields_its_exception_class() {
        let fixture = fixture_declaring("Authentication");
        let errors = vec![error_def_with_variant("ApiError", "Authentication")];
        let fixtures = vec![&fixture];
        assert_eq!(
            collect_used_error_types(&fixtures, &errors),
            ["AuthenticationError".to_string()].into_iter().collect()
        );
    }

    /// A message-style declared value (not a known variant name) names no class — must not
    /// spuriously import anything.
    #[test]
    fn a_message_style_value_yields_nothing() {
        let fixture = fixture_declaring("size must be positive");
        let errors = vec![error_def_with_variant("ApiError", "Authentication")];
        let fixtures = vec![&fixture];
        assert!(collect_used_error_types(&fixtures, &errors).is_empty());
    }
}
