use crate::core::config::Language;
use std::time::Instant;

pub(super) fn observe<T>(language: Language, operation: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    tracing::info!(language = %language, outcome = "started", "Starting backend build");
    let started = Instant::now();
    let result = operation();
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let outcome = if result.is_ok() { "success" } else { "failure" };
    tracing::info!(language = %language, duration_ms, outcome, "Completed backend build");
    result
}

pub(super) fn skipped(language: Language, reason: &str) {
    tracing::info!(language = %language, outcome = "started", "Starting backend build");
    tracing::info!(language = %language, duration_ms = 0_u64, outcome = "skip", reason, "Completed backend build");
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
        skipped(Language::Dart, "precondition");

        assert!(logs_contain("language=java"));
        assert!(logs_contain("language=zig"));
        assert!(logs_contain("language=dart"));
        assert!(logs_contain("outcome=\"success\""));
        assert!(logs_contain("outcome=\"failure\""));
        assert!(logs_contain("outcome=\"skip\""));
        assert!(logs_contain("duration_ms="));
    }
}
