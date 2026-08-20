//! Naming each refused e2e example individually, before the aggregate `inert_summary` line.
//!
//! Split into its own module (out of `e2e::mod`, which was already at this repo's 1,000-line
//! cap) rather than left inline, so the naming logic stays testable on its own without pulling
//! in `generate_e2e_with_extensions`'s full pipeline setup.

use crate::e2e::codegen::inert_example::InertExample;
use tracing::warn;

/// Name each refused example individually, before [`crate::e2e::codegen::inert_example::inert_summary`]'s
/// aggregate line.
///
/// ~keep Split out of `generate_e2e_with_extensions` so the naming can be exercised on its own:
/// the aggregate count alone ("9 example(s) across 3 language(s)... 4 that rendered nothing at
/// all") gave an operator nothing to act on without re-running at `-vv` and grepping generated
/// files for the same `reason()` text this just prints directly.
pub(super) fn report_inert_examples(examples: &[InertExample]) {
    for example in examples {
        warn!("[{}] {}", example.language, example.reason());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::e2e::codegen::inert_example::InertCause;
    use tracing_test::traced_test;

    /// The aggregate `inert_summary` line ("N example(s) across M language(s)... X that
    /// rendered nothing at all") named no fixture and no language, so an operator hitting it
    /// had to re-run at `-vv` and grep generated files for the marker text to find out WHICH
    /// example was refused. `report_inert_examples` must put both on the log line directly. ~keep
    #[test]
    #[traced_test]
    fn report_inert_examples_names_language_and_fixture() {
        let examples = vec![InertExample {
            language: "ruby".to_owned(),
            fixture_id: "streaming_chunked_response".to_owned(),
            markers: 0,
            cause: InertCause::RenderedNothing,
        }];

        report_inert_examples(&examples);

        assert!(logs_contain("ruby"), "the affected language must be named in the log");
        assert!(
            logs_contain("streaming_chunked_response"),
            "the affected fixture must be named in the log"
        );
    }

    /// Two refusals for two different languages must both surface — a summary that only
    /// counted "2 across 2 languages" collapsed exactly this case into a number nobody could
    /// act on without re-deriving which two. ~keep
    #[test]
    #[traced_test]
    fn report_inert_examples_names_every_refusal_not_just_the_first() {
        let examples = vec![
            InertExample {
                language: "python".to_owned(),
                fixture_id: "first_fixture".to_owned(),
                markers: 0,
                cause: InertCause::RenderedNothing,
            },
            InertExample {
                language: "go".to_owned(),
                fixture_id: "second_fixture".to_owned(),
                markers: 2,
                cause: InertCause::AwaitedOrLimited,
            },
        ];

        report_inert_examples(&examples);

        assert!(logs_contain("first_fixture"));
        assert!(logs_contain("second_fixture"));
    }
}
