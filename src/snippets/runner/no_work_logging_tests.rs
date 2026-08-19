//! The per-snippet `Starting`/`Finished` pair must describe work that actually happened.

use super::*;
use crate::snippets::types::{SnippetAnnotation, SnippetAnnotationKind, SnippetMetadata, SourceOrigin};
use crate::snippets::validators::SnippetValidator;
use tracing_test::traced_test;

/// Always available, never batches, and records every toolchain entry so a test can assert the
/// log against what the validator was actually asked to do.
struct CountingValidator {
    invocations: std::sync::Arc<Mutex<usize>>,
}

impl SnippetValidator for CountingValidator {
    fn language(&self) -> crate::snippets::types::Language {
        crate::snippets::types::Language::Rust
    }

    fn is_available(&self) -> bool {
        true
    }

    fn validate(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
    ) -> Result<(SnippetStatus, Option<String>)> {
        *self.invocations.lock().expect("invocations") += 1;
        Ok((SnippetStatus::Pass, None))
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }
}

fn rust_snippet(annotation: Option<SnippetAnnotation>) -> Snippet {
    Snippet {
        id: None,
        path: "example.md".into(),
        language: crate::snippets::types::Language::Rust,
        title: None,
        code: "fn main() {}".into(),
        start_line: 1,
        block_index: 0,
        annotation,
        metadata: SnippetMetadata::default(),
        source_origin: SourceOrigin {
            path: "example.md".into(),
            line: 1,
            block_index: 0,
        },
    }
}

fn run(snippets: &[Snippet]) -> (usize, RunSummary) {
    let invocations = std::sync::Arc::new(Mutex::new(0_usize));
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(CountingValidator {
        invocations: std::sync::Arc::clone(&invocations),
    }));
    let config = RunnerConfig {
        level: ValidationLevel::Syntax,
        parallelism: 1,
        cache_dir: None,
        ..RunnerConfig::default()
    };
    let summary = run_validation(snippets, &registry, &config).expect("validation completes");
    let count = *invocations.lock().expect("invocations");
    (count, summary)
}

/// Snippets skipped by annotation never reach a toolchain, so announcing a per-snippet validation
/// pass for them is a false statement about what the run is doing.
///
/// ~keep The count came from `validate_batches` leaving an entry unclaimed, which conflates "not
/// batched" with "will be validated one at a time". `validate_one` short-circuits on a cache hit,
/// a skip annotation, a side-effect rejection, a missing validator and an unavailable toolchain.
/// With `changed_only` set and one validation pass per crate, later passes are ~100% cache hits,
/// so fully-cached languages announced `snippet_count=521` while doing nothing -- which is how a
/// run in which only four languages truly fell back read as thirteen.
#[traced_test]
#[test]
fn a_language_that_runs_no_toolchain_does_not_announce_per_snippet_validation() {
    let skipped = SnippetAnnotation {
        kind: SnippetAnnotationKind::Skip,
        reason: Some("not applicable".into()),
    };
    let (invocations, summary) = run(&[rust_snippet(Some(skipped.clone())), rust_snippet(Some(skipped))]);

    assert_eq!(invocations, 0, "skip-annotated snippets must not reach the validator");
    assert_eq!(summary.skipped, 2);
    assert!(
        !logs_contain("Starting per-snippet validation"),
        "a language that invoked no toolchain must not announce a per-snippet pass"
    );
    assert!(
        !logs_contain("Finished per-snippet validation"),
        "and must not report finishing one either"
    );
}

/// The control: real work still produces the pair, so the fix suppresses false announcements
/// rather than the log itself.
#[traced_test]
#[test]
fn a_language_that_runs_a_toolchain_still_announces_per_snippet_validation() {
    let (invocations, summary) = run(&[rust_snippet(None), rust_snippet(None)]);

    assert_eq!(invocations, 2);
    assert_eq!(summary.passed, 2);
    assert!(logs_contain("Starting per-snippet validation"));
    assert!(logs_contain("Finished per-snippet validation"));
}
