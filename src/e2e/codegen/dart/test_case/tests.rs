use super::*;

#[cfg(test)]
mod vacuous_assertion_fallback_tests {
    use super::has_real_dart_assertion;

    #[test]
    fn has_real_dart_assertion_is_false_for_comment_only_body() {
        let body = "    // skipped: field 'foo' not available on dart result type\n";
        assert!(
            !has_real_dart_assertion(body),
            "comment-only body must not count as asserting"
        );
    }

    #[test]
    fn has_real_dart_assertion_is_false_for_empty_body() {
        assert!(!has_real_dart_assertion(""));
        assert!(!has_real_dart_assertion("   \n  \n"));
    }

    #[test]
    fn has_real_dart_assertion_is_true_when_a_real_statement_is_present() {
        let body = "    // skipped: field 'foo' not available on dart result type\n    expect(result.ok, isTrue);\n";
        assert!(
            has_real_dart_assertion(body),
            "a real expect(...) line must count as asserting"
        );
    }
}

#[cfg(test)]
mod dart_error_matcher_tests {
    use super::dart_error_matcher;
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

    fn coded_error_def(variant_name: &str) -> ErrorDef {
        ErrorDef {
            name: "ApiError".to_string(),
            rust_path: "lib::ApiError".to_string(),
            original_rust_path: String::new(),
            variants: vec![ErrorVariant {
                name: variant_name.to_string(),
                error_code: Some(100),
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

    #[test]
    fn no_declared_value_renders_throws_anything_with_no_comment() {
        let fixture = Fixture {
            id: "no_error".to_string(),
            ..Fixture::default()
        };
        let mut out = String::new();
        let matcher = dart_error_matcher(&mut out, "    ", &fixture, &[]);
        assert_eq!(matcher, "throwsA(anything)");
        assert_eq!(out, "", "no declared value must leave the output buffer untouched");
    }

    /// With no `errors` IR supplied, a value cannot be recognised as a known variant name, so it
    /// renders exactly like a message-style value always did before this fix.
    #[test]
    fn message_style_value_renders_the_message_or_type_predicate() {
        let fixture = fixture_with_declared_error("BadRequest");
        let mut out = String::new();
        let matcher = dart_error_matcher(&mut out, "    ", &fixture, &[]);
        assert_eq!(
            matcher,
            "throwsA(predicate((e) => e.toString().contains('BadRequest') || e.runtimeType.toString().contains('BadRequest')))"
        );
        assert_eq!(out, "");
    }

    /// The defect this fix closes: a declared value that names a real `ErrorVariant` —
    /// flutter_rust_bridge always decodes the thrown value as a raw `String`, never a typed
    /// exception hierarchy — must render `throwsA(anything)` (still an honest "the call must
    /// fail" check) plus a registered skip comment, not a predicate that can never match.
    #[test]
    fn declared_value_naming_a_known_variant_falls_back_to_throws_anything_and_records_a_skip() {
        let fixture = fixture_with_declared_error("Authentication");
        let errors = vec![coded_error_def("Authentication")];
        let mut out = String::new();
        let matcher = dart_error_matcher(&mut out, "    ", &fixture, &errors);
        assert_eq!(matcher, "throwsA(anything)");
        assert_eq!(
            out,
            "    // skipped: declared error variant 'Authentication' not substantiated by this backend's generated \
             error type\n"
        );
    }
}
