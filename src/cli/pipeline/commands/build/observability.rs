use crate::core::config::Language;
use std::time::Instant;

/// How a backend build ended. The `outcome` field of every `Starting/Completed backend build`
/// event is one of these and nothing else — it is a public, semver-relevant part of alef's log
/// surface, so the variants live in one type rather than as string literals at each call site.
///
/// `Skip` and `UnmetPrecondition` are both "no build happened", but they are not the same claim
/// and must never collapse into one another: `Skip` says this machine or this config has nothing
/// to build here (no backend, no build command, tool absent) and is not the user's problem to
/// fix; `UnmetPrecondition` says the backend was ready to run and the checkout was not prepared,
/// which one command fixes. Neither is `Failure`, which asserts that generated code was actually
/// compiled and was wrong. ~keep
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BackendOutcome {
    Started,
    Success,
    Failure,
    Skip,
    UnmetPrecondition,
}

impl BackendOutcome {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Skip => "skip",
            Self::UnmetPrecondition => "unmet-precondition",
        }
    }
}

/// Below this, a *failed* backend build did not compile anything — it fell over before the
/// compiler started. Real backend builds in this repo's own runs take between seventeen seconds
/// and four minutes; two seconds is far under the slowest plausible no-op incremental compile and
/// far over the tens of milliseconds an environment error takes.
///
/// This threshold has already caught two distinct defects — a precondition that only checked for
/// the tool and not for fetched dependencies, and a worktree-placement bug that made cargo fail
/// every crate in about 100ms. Both presented as a wall of `failure` outcomes that read as
/// catastrophically broken codegen. The duration was the tell in each case, so it is asserted in
/// the log rather than left as something a reader has to notice. ~keep
const IMPLAUSIBLY_FAST_FAILURE_MS: u64 = 2_000;

pub(super) fn observe<T>(language: Language, operation: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    tracing::info!(language = %language, outcome = BackendOutcome::Started.as_str(), "Starting backend build");
    let started = Instant::now();
    let result = operation();
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let outcome = if result.is_ok() {
        BackendOutcome::Success
    } else {
        BackendOutcome::Failure
    };
    record_completion(language, duration_ms, outcome);
    result
}

/// Emit the terminal event for a backend that actually ran, plus the too-fast warning when its
/// duration contradicts the outcome it just reported. ~keep
fn record_completion(language: Language, duration_ms: u64, outcome: BackendOutcome) {
    tracing::info!(language = %language, duration_ms, outcome = outcome.as_str(), "Completed backend build");
    if outcome == BackendOutcome::Failure && duration_ms < IMPLAUSIBLY_FAST_FAILURE_MS {
        tracing::warn!(
            language = %language,
            duration_ms,
            outcome = outcome.as_str(),
            "{language} build failed after {duration_ms}ms -- too fast to have compiled anything. Suspect the \
             environment (unfetched dependencies, a missing interpreter environment, the wrong working directory) \
             before suspecting the generated code."
        );
    }
}

/// Nothing to build here: no binding backend, no build command, or the tool this backend needs is
/// not installed on this machine. Non-fatal by design. ~keep
pub(super) fn skipped(language: Language, reason: &str) {
    tracing::info!(language = %language, outcome = BackendOutcome::Started.as_str(), "Starting backend build");
    tracing::info!(
        language = %language,
        duration_ms = 0_u64,
        outcome = BackendOutcome::Skip.as_str(),
        reason,
        "Completed backend build"
    );
}

/// The backend could have run here and the checkout was not prepared for it. Reported at `warn`
/// with the fixing command attached, because unlike [`skipped`] this is actionable and the caller
/// fails the run over it. ~keep
pub(super) fn unmet_precondition(language: Language, reason: &str, remediation: &str) {
    tracing::info!(language = %language, outcome = BackendOutcome::Started.as_str(), "Starting backend build");
    tracing::warn!(
        language = %language,
        duration_ms = 0_u64,
        outcome = BackendOutcome::UnmetPrecondition.as_str(),
        reason,
        remediation,
        "Completed backend build -- nothing was built for {language}: {reason}. Run: {remediation}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_test::traced_test;

    #[traced_test]
    #[test]
    fn reports_completion_for_distinct_backend_languages() {
        observe(Language::Java, || Ok(())).expect("Java observation");
        observe(Language::Zig, || -> anyhow::Result<()> {
            anyhow::bail!("expected failure")
        })
        .expect_err("Zig failure observation");
        skipped(Language::Dart, "toolchain not on PATH");
        unmet_precondition(
            Language::Elixir,
            "dependencies not fetched",
            "cd packages/elixir && mix deps.get",
        );

        assert!(logs_contain("language=java"));
        assert!(logs_contain("language=zig"));
        assert!(logs_contain("language=dart"));
        assert!(logs_contain("language=elixir"));
        assert!(logs_contain("outcome=\"success\""));
        assert!(logs_contain("outcome=\"failure\""));
        assert!(logs_contain("outcome=\"skip\""));
        assert!(logs_contain("outcome=\"unmet-precondition\""));
        assert!(logs_contain("duration_ms="));
    }

    /// The control for the whole reclassification: an operation that really did fail still reports
    /// `failure`, and never borrows the outcome reserved for an unprepared checkout. ~keep
    #[traced_test]
    #[test]
    fn a_genuine_build_failure_still_reports_failure_and_not_unmet_precondition() {
        observe(Language::Go, || -> anyhow::Result<()> {
            anyhow::bail!("undefined: Foo")
        })
        .expect_err("failing operation");

        assert!(logs_contain("outcome=\"failure\""));
        assert!(!logs_contain("outcome=\"unmet-precondition\""));
    }

    /// A failure this fast could not have compiled anything, and the log has to say so — this
    /// warning is what makes an environmental failure distinguishable from broken codegen at a
    /// glance. ~keep
    #[traced_test]
    #[test]
    fn a_sub_second_failure_is_flagged_as_too_fast_to_have_compiled() {
        observe(Language::Python, || -> anyhow::Result<()> {
            anyhow::bail!("no virtualenv")
        })
        .expect_err("fast failure");

        assert!(logs_contain("too fast to have compiled anything"));
    }

    /// The same warning must not fire for a build that ran long enough to be real, or it would be
    /// noise on every honest compile error and get tuned out. Driven through `record_completion`
    /// with a synthetic duration rather than by sleeping: the assertion is about the threshold,
    /// not about wall-clock time. ~keep
    #[traced_test]
    #[test]
    fn a_slow_failure_is_not_flagged_as_too_fast() {
        record_completion(Language::Ruby, 17_000, BackendOutcome::Failure);

        assert!(logs_contain("outcome=\"failure\""));
        assert!(!logs_contain("too fast to have compiled anything"));
    }

    /// A fast *success* is an incremental no-op build, not a defect — the warning is about a
    /// failure whose duration contradicts it. ~keep
    #[traced_test]
    #[test]
    fn a_fast_success_is_not_flagged_as_too_fast() {
        record_completion(Language::Csharp, 12, BackendOutcome::Success);

        assert!(logs_contain("outcome=\"success\""));
        assert!(!logs_contain("too fast to have compiled anything"));
    }
}
