//! Replacing an inert Kotlin assertion block with an explicit refusal.
//!
//! Split out of `test_method.rs` to keep that file from growing past its ratchet ceiling;
//! deciding how an inert example is reported is its own concern, distinct from rendering a
//! test method, and both `kotlin` and `kotlin_android` share it. ~keep

use super::escape_kotlin;
use crate::e2e::codegen::inert_example::{self, InertCause};
use crate::e2e::fixture::Fixture;

/// Replace an assertion region that asserts nothing with a refusal that a JUnit run can see.
///
/// ~keep Kotlin renders its assertions straight into the shared `out` buffer, so the region is
/// addressed by the offset the caller recorded before the render loop rather than by a separate
/// `String`; `assertions_start` must be that same offset the `fail_on_unavailable_field_markers`
/// scan was given, or the verdict would be read from the wrong text.
///
/// Which refusal is emitted follows who can fix it, exactly as in `ruby/examples.rs`. An
/// unresolved field path is the consumer's to repair, so it gets a `kotlin.test.assertTrue(false,
/// ..)` that FAILS and names the fixture — `assertTrue` rather than `fail()` because `fail()`
/// returns `Nothing` and would make the `client.close()` that follows unreachable. Everything else
/// is alef's generator debt or a language limit no consumer edit clears, so it gets JUnit's own
/// `Assumptions.assumeTrue(false, ..)`, which reports the test as skipped and never as a pass —
/// the same spelling `kotlin/http.rs` already emits, fully qualified so no import is needed.
/// `kotlin_android` shares this renderer and its generated project is JUnit 5 too, so the same
/// construct covers both.
///
/// ~keep `language` is threaded in rather than spelled `"kotlin"` here because this renderer
/// serves two distinct ledger languages. `inert_verdict` reads the skip ledger back through
/// `peek_skip_records`, which filters on an exact language match, so this argument must be the
/// same string `render_test_method` gave `fail_on_unavailable_field_markers` and
/// `fail_on_unsupported_assertion_type_markers` for the same body — otherwise the verdict sees
/// zero markers and misclassifies a fully-skipped example as `RenderedNothing`.
pub(super) fn refuse_inert_example(out: &mut String, assertions_start: usize, fixture: &Fixture, language: &str) {
    let Some(refusal) =
        inert_example::inert_verdict(&out[assertions_start..], language, &fixture.id, &fixture.assertions)
    else {
        return;
    };
    inert_example::record_refusal(&refusal);
    let markers = out[assertions_start..].to_string();
    let reason = escape_kotlin(&refusal.reason());
    let statement = match refusal.cause {
        InertCause::UnresolvedFieldPath => format!("        kotlin.test.assertTrue(false, \"{reason}\")\n"),
        InertCause::AwaitedOrLimited | InertCause::RenderedNothing => {
            format!("        org.junit.jupiter.api.Assumptions.assumeTrue(false, \"{reason}\")\n")
        }
    };
    out.truncate(assertions_start);
    out.push_str(&inert_example::refusal_body(&markers, &statement));
}
