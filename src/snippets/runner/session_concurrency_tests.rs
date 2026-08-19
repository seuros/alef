//! Only validators that share fixed-name files in the session workspace may be serialized.

use super::*;
use crate::snippets::session::SessionSpec;
use crate::snippets::types::{SnippetMetadata, SourceOrigin};
use crate::snippets::validators::SnippetValidator;
use std::sync::{Arc, Condvar};
use std::time::Duration;

/// Blocks each call until `PAIR` calls are in flight, so the test can only pass if the runner
/// really does run them at the same time. A serialized runner never reaches the second call while
/// the first is waiting, so the wait expires and the observed peak stays at 1 — the assertion
/// fails on a timeout rather than hanging the suite. ~keep
const PAIR: usize = 2;
const RENDEZVOUS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Rendezvous {
    in_flight: usize,
    peak: usize,
}

struct RendezvousValidator {
    state: Arc<(Mutex<Rendezvous>, Condvar)>,
    exclusive: bool,
}

impl SnippetValidator for RendezvousValidator {
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
        self.rendezvous()
    }

    // ~keep The default `validate_in_session` rejects any snippet carrying a session, so a probe
    // that only overrode `validate` would never run -- which is exactly what this test is for.
    fn validate_in_session(
        &self,
        _snippet: &Snippet,
        _level: ValidationLevel,
        _timeout_secs: u64,
        _session: Option<&crate::snippets::session::ValidationSession>,
    ) -> Result<(SnippetStatus, Option<String>)> {
        self.rendezvous()
    }

    fn max_level(&self) -> ValidationLevel {
        ValidationLevel::Run
    }

    fn requires_session_exclusivity(&self) -> bool {
        self.exclusive
    }
}

impl RendezvousValidator {
    fn rendezvous(&self) -> Result<(SnippetStatus, Option<String>)> {
        let (lock, condvar) = &*self.state;
        let mut state = lock.lock().expect("rendezvous");
        state.in_flight += 1;
        state.peak = state.peak.max(state.in_flight);
        condvar.notify_all();
        while state.in_flight < PAIR {
            let (next, timeout) = condvar
                .wait_timeout(state, RENDEZVOUS_TIMEOUT)
                .expect("rendezvous wait");
            state = next;
            if timeout.timed_out() {
                break;
            }
        }
        state.in_flight -= 1;
        Ok((SnippetStatus::Pass, None))
    }
}

fn snippet() -> Snippet {
    Snippet {
        id: None,
        path: "example.md".into(),
        language: crate::snippets::types::Language::Rust,
        title: None,
        code: "fn main() {}".into(),
        start_line: 1,
        block_index: 0,
        annotation: None,
        metadata: SnippetMetadata {
            target: Some("shared".into()),
            ..SnippetMetadata::default()
        },
        source_origin: SourceOrigin {
            path: "example.md".into(),
            line: 1,
            block_index: 0,
        },
    }
}

/// Peak concurrency observed for two snippets sharing one session.
fn peak_concurrency(exclusive: bool) -> usize {
    let state = Arc::new((Mutex::new(Rendezvous::default()), Condvar::new()));
    let mut registry = ValidatorRegistry::new();
    registry.register(Box::new(RendezvousValidator {
        state: Arc::clone(&state),
        exclusive,
    }));
    let directory = tempfile::tempdir().expect("session directory");
    let config = RunnerConfig {
        level: ValidationLevel::Syntax,
        parallelism: PAIR,
        cache_dir: None,
        sessions: HashMap::from([(
            "shared".to_string(),
            SessionSpec {
                language: crate::snippets::types::Language::Rust,
                working_directory: directory.path().into(),
                manifest: None,
                before: Vec::new(),
                env: Default::default(),
                include_paths: Vec::new(),
                rust_features: Vec::new(),
                rust_dependencies: Default::default(),
            },
        )]),
        ..RunnerConfig::default()
    };

    let summary = run_validation(&[snippet(), snippet()], &registry, &config).expect("validation completes");
    assert_eq!(summary.passed, PAIR);
    let peak = state.0.lock().expect("rendezvous").peak;
    peak
}

/// A validator that writes only into its own per-call scratch directory must not be serialized by
/// the session mutex.
///
/// ~keep The mutex was introduced by `6ee684237` alongside the change that moved TypeScript, C#
/// and Java onto a shared fingerprint-keyed workspace with fixed filenames -- but it was then
/// taken for every language, on a path that already runs inside a rayon pool. That made the
/// per-snippet fallback strictly serial per session: 521 zig snippets of ~5.9s each ran for half
/// an hour, and because the elapsed timer sat outside the lock they were each *recorded* at a 58s
/// median, disguising the queueing as per-invocation cost.
#[test]
fn a_validator_without_shared_session_state_runs_concurrently() {
    assert_eq!(
        peak_concurrency(false),
        PAIR,
        "snippets sharing a session must run concurrently when the validator declares no shared state"
    );
}

/// The control: declaring shared session state still serializes, because concurrent snippets there
/// would overwrite each other's fixed-name sources mid-compile.
#[test]
fn a_validator_with_shared_session_state_is_still_serialized() {
    assert_eq!(
        peak_concurrency(true),
        1,
        "a validator that shares fixed-name files in the session workspace must not run concurrently"
    );
}
