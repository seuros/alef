//! R e2e test file rendering.

use crate::core::config::ResolvedCrateConfig;
use crate::core::hash::{self, CommentStyle};
use crate::e2e::config::E2eConfig;
use crate::e2e::fixture::Fixture;
use std::fmt::Write as FmtWrite;

use super::test_case::render_test_case;

pub(super) fn render_test_file(
    category: &str,
    fixtures: &[&Fixture],
    result_is_simple: bool,
    result_is_r_list: bool,
    e2e_config: &E2eConfig,
    config: &ResolvedCrateConfig,
    type_defs: &[crate::core::ir::TypeDef],
) -> String {
    let mut out = String::new();
    out.push_str(&hash::header(CommentStyle::Hash));
    let _ = writeln!(out, "# E2e tests for category: {category}");
    let _ = writeln!(out);

    for (i, fixture) in fixtures.iter().enumerate() {
        render_test_case(
            &mut out,
            fixture,
            e2e_config,
            result_is_simple,
            result_is_r_list,
            config,
            type_defs,
        );
        // ~keep R's error path renders `expect_error(...)` and returns, so every other assertion
        // on an error fixture — most often an `equals` against `error.status_code` — leaves no
        // trace in the generated file. The marker sits after the emitted `test_that(...)` call
        // rather than inside its body: the body is built in `test_case.rs`, which another change
        // owns.
        crate::e2e::codegen::error_path_assertions::emit(&mut out, fixture, "# ", "r");
        if i + 1 < fixtures.len() {
            let _ = writeln!(out);
        }
    }

    // Clean up trailing newlines.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod error_path_marker_tests {
    use super::render_test_file;
    use crate::core::config::ResolvedCrateConfig;
    use crate::e2e::config::E2eConfig;
    use crate::e2e::fixture::{Assertion, Fixture};

    fn render(extra: Vec<Assertion>) -> String {
        let mut assertions = vec![Assertion {
            assertion_type: "error".into(),
            ..Assertion::default()
        }];
        assertions.extend(extra);
        let fixture = Fixture {
            id: "rate_limited".into(),
            description: "Rejects the request".into(),
            assertions,
            ..Fixture::default()
        };
        let mut e2e_config = E2eConfig::default();
        e2e_config.call.function = "parse".into();
        let _ = crate::e2e::codegen::take_skip_records();
        render_test_file(
            "error",
            &[&fixture],
            false,
            false,
            &e2e_config,
            &ResolvedCrateConfig::default(),
            &[],
        )
    }

    /// R's error path renders `expect_error(...)` and returns, so every other assertion on the
    /// fixture used to leave no trace in the generated file at all.
    #[test]
    fn r_equals_on_an_error_field_is_named_instead_of_dropped() {
        let out = render(vec![Assertion {
            assertion_type: "equals".into(),
            field: Some("error.status_code".into()),
            ..Assertion::default()
        }]);

        // Positive first: the error block really rendered.
        assert!(out.contains("expect_error("), "the error block must render:\n{out}");
        assert!(
            out.contains(
                "# skipped: assertion type 'equals' has no accessor for error field error.status_code in this backend"
            ),
            "got:\n{out}"
        );

        let records = crate::e2e::codegen::take_skip_records();
        assert_eq!(records.len(), 1, "got: {records:?}");
        assert_eq!(records[0].language, "r");
        assert_eq!(records[0].field, "equals");
    }

    /// Negative control: a lone `error` assertion must leave the generated file marker-free.
    #[test]
    fn r_a_lone_error_assertion_renders_no_marker() {
        let out = render(Vec::new());

        assert!(out.contains("expect_error("), "the error block must render:\n{out}");
        assert!(!out.contains("has no accessor for error field"), "got:\n{out}");
    }
}
