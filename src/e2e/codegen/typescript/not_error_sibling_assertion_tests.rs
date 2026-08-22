//! Regression coverage for `not_error` paired with a sibling assertion.
//!
//! Lives in its own file rather than in `typescript/assertions.rs`: that file is already over
//! the repo's 1,000-line cap and is a documented remediation target, so it must not grow. ~keep

use std::collections::{HashMap, HashSet};

use super::assertions::render_assertion;
use crate::e2e::codegen::not_error_presence::may_assert_presence;
use crate::e2e::field_access::FieldResolver;
use crate::e2e::fixture::{Assertion, Fixture};

fn empty_resolver() -> FieldResolver {
    FieldResolver::new(
        &HashMap::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    )
}

fn make_assertion(assertion_type: &str, field: Option<&str>, value: Option<serde_json::Value>) -> Assertion {
    Assertion {
        assertion_type: assertion_type.to_string(),
        field: field.map(|s| s.to_string()),
        value,
        ..Default::default()
    }
}

/// Regression test for alef task #165: tslp's WASM e2e gate failed on
/// `error_detect_content_empty` and its extension/path siblings — fixtures whose title
/// says "returns null" but paired `not_error` with `is_empty` on an `Option<T>`-returning
/// call. `not_error` used to emit an unconditional `expect(result).toBeDefined();`
/// regardless of sibling assertions, which wasm-bindgen's `None` -> `undefined` mapping
/// genuinely fails (NAPI's `None` -> `null` mapping only passed by accident, since
/// `null !== undefined`). `not_error` must yield to a sibling assertion instead of
/// asserting presence on a call whose success path can legitimately be absent — the same
/// rendering path is shared by "node" and "wasm", so both must agree.
///
/// ~keep The `not_error_may_assert_presence` flag driving `render_assertion` below comes
/// from the real, shared `not_error_presence::may_assert_presence` (not a hand-picked
/// literal), so this test exercises the actual generator plus the actual shared decision,
/// not a hand-written mirror of either.
#[test]
fn not_error_paired_with_is_empty_does_not_assert_presence() {
    for lang in ["node", "wasm"] {
        let resolver = empty_resolver();
        let not_error = make_assertion("not_error", None, None);
        let is_empty = make_assertion("is_empty", None, None);
        let fixture = Fixture {
            assertions: vec![not_error.clone(), is_empty.clone()],
            ..Default::default()
        };
        let not_error_may_assert_presence = may_assert_presence(&fixture, false);
        let mut out = String::new();
        for assertion in [&not_error, &is_empty] {
            render_assertion(
                &mut out,
                assertion,
                "result",
                &resolver,
                false,
                &std::collections::HashMap::new(),
                lang,
                false,
                false,
                not_error_may_assert_presence,
            );
        }
        assert!(
            !out.contains("toBeDefined()"),
            "[{lang}] not_error must not assert presence alongside is_empty; got: {out}"
        );
        assert!(
            out.contains("(result ?? \"\").length"),
            "[{lang}] is_empty must still render its own nullish-safe check; got: {out}"
        );
    }
}

/// New coverage this unification closes: before centralizing the decision, TypeScript only
/// suppressed `not_error`'s presence assertion when a *sibling* assertion existed — a fixture
/// whose *sole* assertion was `not_error` on an `Option<T>`-returning call still got an
/// unconditional `expect(result).toBeDefined()`, which fails whenever the call's success path
/// legitimately returns `None` (wasm-bindgen `undefined`). `may_assert_presence` closes that
/// gap by also consulting `result_is_option`, independent of sibling count.
#[test]
fn not_error_as_sole_assertion_on_option_result_does_not_assert_presence() {
    for lang in ["node", "wasm"] {
        let resolver = empty_resolver();
        let not_error = make_assertion("not_error", None, None);
        let fixture = Fixture {
            assertions: vec![not_error.clone()],
            ..Default::default()
        };
        let not_error_may_assert_presence = may_assert_presence(&fixture, true);
        let mut out = String::new();
        render_assertion(
            &mut out,
            &not_error,
            "result",
            &resolver,
            false,
            &std::collections::HashMap::new(),
            lang,
            false,
            false,
            not_error_may_assert_presence,
        );
        assert_eq!(
            out, "",
            "[{lang}] not_error on a bare Option<T> result must not assert presence even as \
             the sole assertion; got: {out}"
        );
    }
}
