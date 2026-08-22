use super::*;

#[cfg(test)]
mod strict_field_availability_marker_tests {
    use super::render_assertion;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    /// Regression test for alef task #81: Kotlin's "skipped: field not available"
    /// comment text must survive as the exact marker the shared
    /// `crate::e2e::codegen::fail_on_unavailable_field_markers` mechanism matches on
    /// (wired into `kotlin/test_method.rs`, shared by `kotlin` and `kotlin_android`),
    /// so arming `ALEF_E2E_STRICT_FIELD_AVAILABILITY` turns a dropped field assertion
    /// into a generation-time failure instead of a silently-passing comment.
    #[test]
    fn unavailable_field_skip_comment_carries_the_strict_mode_marker() {
        let result_fields: HashSet<String> = ["content".to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &result_fields,
            &HashSet::new(),
            &HashSet::new(),
        );
        let assertion = Assertion {
            assertion_type: "equals".to_string(),
            field: Some("nonexistent_field".to_string()),
            value: Some(serde_json::json!("x")),
            ..Assertion::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClient",
            &resolver,
            false,
            false,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            true,
        );
        assert!(out.contains("field 'nonexistent_field' not available"), "got: {out}");
    }
}

#[cfg(test)]
mod is_true_optional_field_tests {
    use super::render_assertion;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn render(assertion: &Assertion, optional_field: &str, kotlin_android_style: bool) -> String {
        let optional: HashSet<String> = [optional_field.to_string()].into_iter().collect();
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &optional,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let mut out = String::new();
        render_assertion(
            &mut out,
            assertion,
            "result",
            "SampleClient",
            &resolver,
            false,
            false,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            false,
            kotlin_android_style,
            true,
        );
        out
    }

    fn is_true_assertion(field: &str) -> Assertion {
        Assertion {
            assertion_type: "is_true".to_string(),
            field: Some(field.to_string()),
            ..Assertion::default()
        }
    }

    /// `Option<DataNode>` presence on the Kotlin (JVM) target: before the fix this rendered
    /// `assertTrue(result.data() == true, ...)`, which compiles (`==` is Any?-to-Any?
    /// structural equality) but is always false for a present non-Boolean nullable.
    #[test]
    fn kotlin_is_true_on_optional_struct_field_checks_presence() {
        let out = render(&is_true_assertion("data"), "data", false);
        assert_eq!(
            out,
            "        assertTrue(result.data() != null, \"expected true (non-null)\")\n"
        );
    }

    /// Same fixture, kotlin_android target: properties (no `()`), same nullability fix.
    #[test]
    fn kotlin_android_is_true_on_optional_struct_field_checks_presence() {
        let out = render(&is_true_assertion("data"), "data", true);
        assert_eq!(
            out,
            "        assertTrue(result.data != null, \"expected true (non-null)\")\n"
        );
    }

    #[test]
    fn kotlin_android_is_false_on_optional_struct_field_checks_absence() {
        let out = render(
            &Assertion {
                assertion_type: "is_false".to_string(),
                field: Some("data".to_string()),
                ..Assertion::default()
            },
            "data",
            true,
        );
        assert_eq!(
            out,
            "        assertTrue(result.data == null, \"expected false (null)\")\n"
        );
    }

    #[test]
    fn kotlin_android_is_true_on_non_optional_field_is_unchanged() {
        let resolver = FieldResolver::new(
            &HashMap::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        let mut out = String::new();
        render_assertion(
            &mut out,
            &is_true_assertion("active"),
            "result",
            "SampleClient",
            &resolver,
            false,
            false,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            false,
            true,
            true,
        );
        assert_eq!(out, "        assertTrue(result.active == true, \"expected true\")\n");
    }
}

#[cfg(test)]
mod wildcard_tests {
    use super::render_assertion;
    use crate::e2e::field_access::FieldResolver;
    use crate::e2e::fixture::Assertion;
    use std::collections::{HashMap, HashSet};

    fn array_resolver(field: &str) -> FieldResolver {
        let names: HashSet<String> = [field.to_string()].into_iter().collect();
        FieldResolver::new(&HashMap::new(), &HashSet::new(), &names, &names, &HashSet::new())
    }

    fn render_contains(resolver: &FieldResolver, field: &str, value: &str) -> String {
        let assertion = Assertion {
            assertion_type: "contains".to_string(),
            field: Some(field.to_string()),
            value: Some(serde_json::Value::String(value.to_string())),
            ..Assertion::default()
        };
        let mut out = String::new();
        render_assertion(
            &mut out,
            &assertion,
            "result",
            "SampleClient",
            resolver,
            false,
            false,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
            false,
            false,
            true,
        );
        out
    }

    /// Baseline: a single wildcard still quantifies over the whole list, so the refusal added
    /// for the nested case cannot have been implemented by refusing wildcards generally. ~keep
    #[test]
    fn single_wildcard_still_quantifies_over_every_element() {
        let out = render_contains(&array_resolver("links"), "links[].url", "example.test");
        assert!(out.contains(".any {"), "got: {out}");
        assert!(!out.contains(".first()"), "wildcard must not pin element 0, got: {out}");
    }

    /// `wildcard_split` consumes the first `[].` only, so before the guard the `any {}` ranged
    /// over `pages` while its lambda read `e.links().first().url()` — a whole-array claim that
    /// only ever inspected element zero of the inner list. Kotlin is the worst case for
    /// spotting this in review: `index == 0` renders as `.first()`, never as a literal `[0]`,
    /// so an index-free assertion would have passed vacuously. Pre-guard this test fails: the
    /// skip line is absent and `.first()` is present. ~keep
    #[test]
    fn nested_wildcard_should_emit_a_visible_skip_rather_than_an_index_zero_check() {
        let out = render_contains(&array_resolver("pages"), "pages[].links[].url", "example.test");
        assert_eq!(
            out, "        // skipped: nested array-wildcard field 'pages[].links[].url' not supported\n",
            "got: {out}"
        );
    }
}
